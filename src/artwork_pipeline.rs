//! Synchronous resize pipeline for uploaded artwork (Phase 1.h.3).
//!
//! Every upload runs through [`generate_variants`] to produce two
//! re-encoded JPEGs:
//!
//! - `thumb`   — fits inside a 128 × 128 box, preserving aspect.
//! - `preview` — fits inside a 480 × 480 box, preserving aspect.
//!
//! The "full" variant is the byte-perfect original kept in
//! `metadata_artwork`; the pipeline never re-encodes it.
//!
//! Variants are always JPEG q85: covers are opaque (alpha doesn't
//! survive a JPEG round-trip, but cover art doesn't carry alpha
//! anyway), and JPEG q85 buys roughly 60% file-size reduction over
//! PNG / WebP-lossless for visually identical output.
//!
//! Decoding accepts the same MIME set the upload endpoint accepts —
//! JPEG, PNG, WebP. The `image` crate auto-detects from the byte
//! signature, so the MIME header is informational (we don't pass it
//! in).
//!
//! Phase 1.i.1 will move this off the request thread into an
//! `apalis` job; for 1.h.3 the resize stays synchronous to keep the
//! end-to-end test loop simple (a JPEG resize is on the order of
//! tens of milliseconds for the 4 MiB ceiling we enforce upstream).

use bytes::Bytes;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageError};

/// JPEG quality factor for re-encoded variants. q85 is the sweet
/// spot from `cwebp`'s baseline study — visually indistinguishable
/// from q95 for cover art while shaving ~30% extra bytes vs q90.
const JPEG_QUALITY: u8 = 85;

/// Variant size buckets. `kind()` returns the canonical string we
/// store in the `metadata_artwork_variant.variant` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariantKind {
    Thumb,
    Preview,
}

impl VariantKind {
    /// Canonical string for the DB column + the URL suffix. Kept
    /// stable across releases — adding a new variant size means a
    /// new enum case + a CHECK-list update on the SQL side.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Thumb => "thumb",
            Self::Preview => "preview",
        }
    }

    /// Maximum pixels on the longest edge. Aspect ratio is
    /// preserved; the output dimensions are bounded by `(max, max)`.
    pub fn max_edge(&self) -> u32 {
        match self {
            Self::Thumb => 128,
            Self::Preview => 480,
        }
    }

    /// Parse the URL-tail variant suffix into the enum. Anything
    /// outside the closed set returns `None`, which the HTTP layer
    /// maps to 400. Named `parse` rather than `from_str` to avoid
    /// clashing with the `std::str::FromStr` trait shape (and the
    /// clippy lint that flags the collision).
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "thumb" => Some(Self::Thumb),
            "preview" => Some(Self::Preview),
            _ => None,
        }
    }
}

/// Result of one resize pass. Carries every field the caller needs
/// to write the variant row + the bytes themselves to object_store.
#[derive(Debug)]
pub struct Variant {
    pub kind: VariantKind,
    pub bytes: Bytes,
    /// BLAKE3 hex of the re-encoded bytes — the row identity and
    /// the object_store key.
    pub hash: String,
    pub mime: &'static str,
    pub byte_size: i64,
    pub width: u32,
    pub height: u32,
}

/// Decode + resize the source bytes into the full variant set.
/// Returns the variants in deterministic order (thumb, preview) so
/// a caller that wants to iterate doesn't depend on enum-order
/// nuances.
///
/// Failure modes:
/// - The byte stream isn't a format we accept (JPEG / PNG / WebP).
///   The `image` crate's `load_from_memory` surfaces this as
///   `ImageError::Unsupported`.
/// - The decoded image has zero pixels — guarded explicitly because
///   `image` happily decodes a 0 × 0 PNG and the resize pass would
///   then divide by zero on the aspect ratio.
///
/// Callers convert any error to a 400 (client-side input problem)
/// or a 500 (server-side encoder failure) — the [`PipelineError`]
/// variants discriminate the two.
pub fn generate_variants(source_bytes: &[u8]) -> Result<Vec<Variant>, PipelineError> {
    if source_bytes.is_empty() {
        return Err(PipelineError::EmptySource);
    }

    let img = image::load_from_memory(source_bytes).map_err(PipelineError::Decode)?;
    if img.width() == 0 || img.height() == 0 {
        return Err(PipelineError::ZeroDimension);
    }

    let mut out = Vec::with_capacity(2);
    for kind in [VariantKind::Thumb, VariantKind::Preview] {
        out.push(resize_one(&img, kind)?);
    }
    Ok(out)
}

/// Resize one variant. `DynamicImage::thumbnail` preserves the
/// aspect ratio by fitting the longest edge into the supplied
/// bounding box — a 1600 × 900 cover shrinks to 480 × 270, not
/// 480 × 480 with letterboxing. We do NOT call `thumbnail`
/// unconditionally because it scales the source UP to the box when
/// the source is smaller — a 64 × 48 cover would balloon to 128 × 96
/// fake pixels. Manual long-edge check first; pass the original
/// through (re-encoded to JPEG q85 for byte-format consistency)
/// when no shrink is needed.
fn resize_one(source: &DynamicImage, kind: VariantKind) -> Result<Variant, PipelineError> {
    let max = kind.max_edge();
    let long_edge = source.width().max(source.height());
    // `DynamicImage::to_rgb8` is `&self`, so the no-shrink branch
    // skips the resize-induced copy without cloning the whole
    // DynamicImage tree. The shrink branch does need to materialise
    // the resized DynamicImage before pulling the RGB buffer.
    let rgb = if long_edge <= max {
        source.to_rgb8()
    } else {
        source.thumbnail(max, max).to_rgb8()
    };
    let (width, height) = (rgb.width(), rgb.height());

    let mut buf = Vec::with_capacity((width * height * 3 / 4) as usize);
    {
        let mut encoder = JpegEncoder::new_with_quality(&mut buf, JPEG_QUALITY);
        encoder
            .encode(&rgb, width, height, image::ColorType::Rgb8.into())
            .map_err(PipelineError::Encode)?;
    }

    let bytes = Bytes::from(buf);
    let hash = blake3::hash(&bytes).to_hex().to_string();
    let byte_size = i64::try_from(bytes.len()).expect("variant bytes fit in i64");

    Ok(Variant {
        kind,
        bytes,
        hash,
        mime: "image/jpeg",
        byte_size,
        width,
        height,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("source bytes are empty")]
    EmptySource,
    #[error("source has zero dimension")]
    ZeroDimension,
    #[error("failed to decode source image: {0}")]
    Decode(#[source] ImageError),
    #[error("failed to encode variant: {0}")]
    Encode(#[source] ImageError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};
    use std::io::Cursor;

    /// Encode a synthetic test image to JPEG. Used by every test
    /// that needs a real (i.e. decodable) bitmap on the wire —
    /// hand-rolled bytes like `\xff\xd8\xff\xe0fake` are rejected by
    /// the decoder.
    fn synth_jpeg(width: u32, height: u32) -> Vec<u8> {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(width, height, |x, y| {
            // A diagonal gradient — visually distinct, avoids the
            // pure-flat-colour edge case some JPEG decoders short-
            // circuit on.
            let r = ((x * 255) / width.max(1)) as u8;
            let g = ((y * 255) / height.max(1)) as u8;
            Rgb([r, g, 128])
        });
        let mut buf = Vec::new();
        let mut cursor = Cursor::new(&mut buf);
        img.write_to(&mut cursor, image::ImageFormat::Jpeg).unwrap();
        buf
    }

    #[test]
    fn variant_kind_round_trips_strings() {
        for k in [VariantKind::Thumb, VariantKind::Preview] {
            assert_eq!(VariantKind::parse(k.as_str()), Some(k));
        }
        assert!(VariantKind::parse("full").is_none());
        assert!(VariantKind::parse("").is_none());
        assert!(VariantKind::parse("THUMB").is_none()); // case-sensitive
    }

    #[test]
    fn generates_thumb_and_preview_in_order() {
        let src = synth_jpeg(800, 600);
        let variants = generate_variants(&src).expect("resize succeeded");
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0].kind, VariantKind::Thumb);
        assert_eq!(variants[1].kind, VariantKind::Preview);
    }

    #[test]
    fn thumb_fits_in_128_box_with_aspect_preserved() {
        // 800 × 600 → thumb should be ≤ 128 on the long edge.
        let src = synth_jpeg(800, 600);
        let variants = generate_variants(&src).unwrap();
        let thumb = variants
            .iter()
            .find(|v| v.kind == VariantKind::Thumb)
            .unwrap();
        assert!(thumb.width <= 128);
        assert!(thumb.height <= 128);
        // 800:600 = 4:3 → 128 × 96 (long-edge clamp).
        assert_eq!(thumb.width, 128);
        assert_eq!(thumb.height, 96);
    }

    #[test]
    fn preview_fits_in_480_box() {
        let src = synth_jpeg(1600, 900);
        let variants = generate_variants(&src).unwrap();
        let preview = variants
            .iter()
            .find(|v| v.kind == VariantKind::Preview)
            .unwrap();
        assert!(preview.width <= 480);
        assert!(preview.height <= 480);
        // 1600:900 = 16:9 → 480 × 270.
        assert_eq!(preview.width, 480);
        assert_eq!(preview.height, 270);
    }

    #[test]
    fn variants_have_distinct_hashes_and_jpeg_mime() {
        let src = synth_jpeg(800, 600);
        let variants = generate_variants(&src).unwrap();
        assert_ne!(variants[0].hash, variants[1].hash);
        for v in &variants {
            assert_eq!(v.mime, "image/jpeg");
            assert_eq!(v.hash.len(), 64);
            assert!(v.byte_size > 0);
            assert_eq!(v.byte_size as usize, v.bytes.len());
        }
    }

    #[test]
    fn small_source_is_not_upscaled() {
        // 64 × 48 — already smaller than the thumb cap, so the
        // pipeline should pass it through at native size, not
        // pretend-upscale to 128.
        let src = synth_jpeg(64, 48);
        let variants = generate_variants(&src).unwrap();
        let thumb = &variants[0];
        assert_eq!(thumb.width, 64);
        assert_eq!(thumb.height, 48);
    }

    #[test]
    fn rejects_empty_bytes() {
        let err = generate_variants(&[]).unwrap_err();
        assert!(matches!(err, PipelineError::EmptySource));
    }

    #[test]
    fn rejects_non_image_bytes() {
        let err = generate_variants(b"this is not an image").unwrap_err();
        assert!(matches!(err, PipelineError::Decode(_)));
    }
}
