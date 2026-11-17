//! Covers in the formats the scan stores but the route did not serve.
//!
//! `waveflow_core` takes a `cover.gif` or a `cover.bmp` as an album's folder
//! cover and stores it under that format, and `artwork_mime` knew only JPEG,
//! PNG and WebP: the album had a cover row, and every route answered 404 for
//! it.

use super::*;

/// One frame of a small picture in the container `path` names, by FFmpeg.
fn write_picture(path: &std::path::Path) {
    let output = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "color=c=red:s=8x8", "-frames:v", "1"])
        .arg(path)
        .output()
        .expect("FFmpeg is required for the cover fixtures");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn a_gif_or_bmp_folder_cover_is_served_as_what_it_is() {
    let (_temp, config, state) = test_app().await;
    let (owner, token) = admin_with_api_token(&state, "covers").await;
    let music = config.data_dir.join("covers");
    for (album, cover) in [("Gif", "cover.gif"), ("Bmp", "cover.bmp")] {
        std::fs::create_dir_all(music.join(album)).unwrap();
        write_tagged_flac(&music.join(album).join("Song.flac"), album, album, 440);
        write_picture(&music.join(album).join(cover));
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
