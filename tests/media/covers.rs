//! Covers in the formats the scan stores but the route did not serve.
//!
//! `waveflow_core` takes a `cover.gif` or a `cover.bmp` as an album's folder
//! cover and stores it under that format, and `artwork_mime` knew only JPEG,
//! PNG and WebP: the album had a cover row, and every route answered 404 for
//! it.

use super::*;

/// A 1×1 red GIF89a, whole.
///
/// Written out rather than asked of FFmpeg: this target also runs against the
/// minimal FFmpeg the standalone archives ship (`ffmpeg.yml`), which has no
/// GIF or BMP encoder and must not gain one for a test. The route serves the
/// bytes as they are, so any valid file of the format will do.
const GIF: &[u8] = &[
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0xff, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44,
    0x01, 0x00, 0x3b,
];

/// A 1×1 red BMP, 24 bits per pixel, whole: the 14-byte file header, the
/// 40-byte info header, and one pixel padded to four bytes.
const BMP: &[u8] = &[
    0x42, 0x4d, 0x3a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x36, 0x00, 0x00, 0x00, 0x28, 0x00,
    0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x18, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x13, 0x0b, 0x00, 0x00, 0x13, 0x0b, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0x00,
];

#[tokio::test]
async fn a_gif_or_bmp_folder_cover_is_served_as_what_it_is() {
    let (_temp, config, state) = test_app().await;
    let (owner, token) = admin_with_api_token(&state, "covers").await;
    let music = config.data_dir.join("covers");
    for (album, cover, bytes) in [("Gif", "cover.gif", GIF), ("Bmp", "cover.bmp", BMP)] {
        std::fs::create_dir_all(music.join(album)).unwrap();
        write_tagged_flac(&music.join(album).join("Song.flac"), album, album, 440);
        std::fs::write(music.join(album).join(cover), bytes).unwrap();
    }
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(owner, "covers", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    scan_once(
        &state,
        owner,
        LibraryRecord {
            id: library_id,
            name: "covers".into(),
            root_path: root.clone(),
        },
    )
    .await;
    let router = waveflow_server::app(&config, state.clone());

    for (album, format, mime) in [("Gif", "gif", "image/gif"), ("Bmp", "bmp", "image/bmp")] {
        let (track, hash, stored): (String, String, String) = sqlx::query_as(
            "SELECT t.id, a.hash, a.format FROM track t JOIN artwork a ON a.hash = t.artwork_hash \
             WHERE t.relative_path = ?",
        )
        .bind(format!("{album}/Song.flac"))
        .fetch_one(state.db.pool())
        .await
        .unwrap();
        assert_eq!(stored, format, "the scan stores the cover as a {format}");

        // By its hash and by the track that carries it, as clients ask.
        for id in [&hash, &track] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/v2/artwork/{id}"))
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{album} by {id}");
            assert_eq!(response.headers()["content-type"], mime);
            let body = response.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(
                &body[..],
                std::fs::read(root.join(album).join(format!("cover.{format}"))).unwrap(),
                "the file as it is"
            );
        }
    }
}
