//! Reading what a candidate file really is, and stripping its audio.

use super::*;

/// How long `ffprobe` gets to read a canvas.
///
/// The same five seconds `check_tool` allows the version probe. A loop of a few
/// seconds and a few hundred kilobytes is read in milliseconds, so this is not
/// a budget anything legitimate spends — it is the bound on something that will
/// never finish.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// What `ffprobe` had to say about the bytes offered.
pub(super) struct Probed {
    pub(super) format: String,
    pub(super) duration_secs: f64,
    /// Whether the file carries sound nobody will ever hear. See
    /// [`DomainServices::strip_canvas_audio`].
    pub(super) has_audio: bool,
}

impl DomainServices {
    /// Rewrites a staged canvas without its soundtrack.
    ///
    /// A **remux, never a re-encode**: `-c copy` moves the video packets across
    /// untouched, so this is not what decision 8 refuses. That decision's
    /// argument was that transcoding would make a canvas a compute charge *per
    /// playback*; this is one stream copy, once, at ingestion, and the picture
    /// comes out bit for bit what it went in as.
    ///
    /// `-map_metadata -1` and `-fflags +bitexact` are load-bearing rather than
    /// tidy. The store is content-addressed, so two members offering the same
    /// file must produce the same output or the deduplication of decision 1
    /// quietly stops working and each of them is charged for a blob. Those two
    /// flags drop the creation timestamp and the encoder string that would
    /// otherwise differ between two runs a second apart.
    ///
    /// It holds within one deployment. Two different FFmpeg versions may still
    /// mux the same packets differently, so a server upgrade can give the same
    /// source a second blob — which costs a duplicate, never a wrong answer,
    /// and is the reason this is written down rather than assumed.
    ///
    /// **The suite does not cover those two flags, and does not claim to.**
    /// `canvas` asserts that the same bytes offered twice give the same blob,
    /// and that holds with the flags removed: one FFmpeg build muxing the same
    /// packets twice agrees with itself either way. What the flags defend
    /// against is the version string this muxer writes, which differs between
    /// builds — a difference no test running against a single build can see.
    pub(super) async fn strip_canvas_audio(
        &self,
        staging: &std::path::Path,
        format: &str,
    ) -> Result<PathBuf, ServiceError> {
        // A name the caller can predict without knowing the container, so its
        // cleanup reaches this file on every path out — including the ones that
        // fail after it exists. The muxer is named with `-f` rather than
        // inferred from an extension, which is what lets the name stay plain.
        let silent = staging.with_extension("silent");
        let output = tokio::time::timeout(
            PROBE_TIMEOUT,
            crate::media::file_input(
                tokio::process::Command::new(&self.ffmpeg_path)
                    .arg("-hide_banner")
                    .arg("-loglevel")
                    .arg("error")
                    .arg("-y")
                    .arg("-fflags")
                    .arg("+bitexact"),
                staging,
            )
            .arg("-map")
            .arg("0:v")
            .arg("-c")
            .arg("copy")
            .arg("-an")
            .arg("-map_metadata")
            .arg("-1")
            .arg("-fflags")
            .arg("+bitexact")
            .arg("-f")
            .arg(format)
            .arg(&silent)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
        )
        .await
        .map_err(|_| {
            tracing::warn!("ffmpeg timed out taking the sound out of a canvas");
            ServiceError::Invalid
        })?
        .map_err(|error| {
            tracing::error!(%error, "cannot run ffmpeg on a canvas");
            ServiceError::Unavailable
        })?;
        if !output.status.success() {
            // The probe already said this is a container we accept, so a remux
            // that fails is about this file rather than about the server.
            tracing::warn!("a canvas could not be stripped of its audio");
            return Err(ServiceError::Invalid);
        }
        Ok(silent)
    }

    /// Reads the container and the duration out of the bytes themselves.
    ///
    /// Never from a `Content-Type` or an extension: anything at all can be
    /// called `.mp4`. `ffprobe` is already a dependency and already on the
    /// `PATH` of every deployment, and reading the file is the only thing that
    /// is evidence.
    ///
    /// Bounded, and killed if this future is dropped. The bytes being read were
    /// chosen by whoever sent them, so a container crafted to make a demuxer
    /// spin is an input this route has to survive: without the bound one
    /// request holds a task forever, and without `kill_on_drop` a client that
    /// hangs up leaves the process behind to finish spinning alone. Both
    /// already exist in `MediaService` — the five-second bound on `check_tool`,
    /// and `kill_on_drop` on the transcode child.
    pub(super) async fn probe_canvas(
        &self,
        path: &std::path::Path,
    ) -> Result<Probed, ServiceError> {
        let output = tokio::time::timeout(
            PROBE_TIMEOUT,
            crate::media::file_input(
                tokio::process::Command::new(&self.ffprobe_path)
                    .arg("-v")
                    .arg("error")
                    .arg("-print_format")
                    .arg("json")
                    .arg("-show_format")
                    .arg("-show_streams"),
                path,
            )
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
        )
        .await
        .map_err(|_| {
            // The server is fine; this file is not something it will finish
            // reading. Refusing the offer says that without pretending the
            // deployment is broken.
            tracing::warn!("ffprobe timed out on a canvas");
            ServiceError::Invalid
        })?
        .map_err(|error| {
            tracing::error!(%error, "cannot run ffprobe on a canvas");
            ServiceError::Unavailable
        })?;
        if !output.status.success() {
            // Not an outage: what arrived is not something ffprobe can read,
            // which is a verdict on the offer rather than on the server.
            return Err(ServiceError::Invalid);
        }
        let probed: serde_json::Value =
            serde_json::from_slice(&output.stdout).map_err(|_| ServiceError::Unavailable)?;

        // A container reports a family — "matroska,webm", or the mov/mp4 group
        // — so the whitelist is matched against the members rather than the
        // whole string.
        let format_names = probed
            .get("format")
            .and_then(|format| format.get("format_name"))
            .and_then(serde_json::Value::as_str)
            .ok_or(ServiceError::Invalid)?;
        let format = ACCEPTED_FORMATS
            .iter()
            .find(|accepted| format_names.split(',').any(|name| name == **accepted))
            .ok_or(ServiceError::Invalid)?;

        // A canvas without a video stream is not a canvas, whatever it is.
        let has_video = probed
            .get("streams")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|streams| {
                streams.iter().any(|stream| {
                    stream.get("codec_type").and_then(serde_json::Value::as_str) == Some("video")
                })
            });
        if !has_video {
            return Err(ServiceError::Invalid);
        }
        // Reported rather than refused. RFC-009 decision 9: a canvas loops over a
        // track that is already playing and the desktop's `<video>` is muted in
        // its markup, so nobody can hear this — and keeping it would spend the
        // ceiling on bytes that buy nothing. `identify_and_place` remuxes the
        // stream out before the file reaches the store; this only says whether
        // there is one to take out.
        let has_audio = probed
            .get("streams")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|streams| {
                streams.iter().any(|stream| {
                    stream.get("codec_type").and_then(serde_json::Value::as_str) == Some("audio")
                })
            });

        // A container that does not say how long it runs cannot be checked
        // against the ceiling, and an unchecked ceiling is not one.
        let duration_secs = probed
            .get("format")
            .and_then(|format| format.get("duration"))
            .and_then(serde_json::Value::as_str)
            .and_then(|duration| duration.parse::<f64>().ok())
            .filter(|duration| duration.is_finite() && *duration >= 0.0)
            .ok_or(ServiceError::Invalid)?;

        Ok(Probed {
            format: (*format).to_owned(),
            duration_secs,
            has_audio,
        })
    }
}
