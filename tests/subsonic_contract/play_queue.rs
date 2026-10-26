//! The play queue and now-playing entries.

use super::*;

/// `getPlayQueue` and `getNowPlaying` name their children `entry`, never `song`.
///
/// The schema renames a media item inside these two containers exactly as it
/// does inside a playlist, and this server sent `song` in both until
/// 2026-09-16. Three client campaigns passed over it because every client that
/// met it was lenient; a client that decodes against the schema reads an empty
/// queue and an empty now-playing list.
///
/// Asserted in XML *and* JSON, and asserted negatively as well: the point is
/// not that `entry` appears, it is that `song` does not come back. Without the
/// negative half, emitting both names would pass.
#[tokio::test]
async fn play_queue_and_now_playing_name_their_children_entry() {
    let (_temp, config, state) = test_app().await;
    let web_hash = security::hash_password("web-password-for-test").unwrap();
    let owner = state
        .db
        .create_account("queue-owner", &web_hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state.secret_box.encrypt(b"queue-password").unwrap(),
            &security::token_hash("wfsk_queue"),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("queue-subsonic");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Queued.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Queue Subsonic",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        LibraryRecord {
            id: library,
            name: "Queue Subsonic".into(),
            root_path: root,
        },
    )
    .await;
    let song = state.db.list_tracks_for_user(owner, library).await.unwrap()[0].id;
    let router = waveflow_server::app(&config, state);

    // Nothing is saved yet, and the answer carries **no `playQueue` at all**
    // (#229). The schema marks `username`, `changed` and `changedBy` all
    // three required on that element, so there is no valid shape for a queue
    // that has never been written, and the reference documents the same
    // answer: "an empty <subsonic-response> if no play queue has been saved".
    //
    // The envelope is asserted beside the absence on purpose. Without it this
    // test would also pass on a failure — an error response has no
    // `playQueue` either, and absence alone cannot tell the two apart.
    let unsaved = subsonic_json(&router, "getPlayQueue", "wfsk_queue", "").await;
    assert_eq!(unsaved["subsonic-response"]["status"], "ok", "{unsaved}");
    assert!(
        omits(&unsaved["subsonic-response"], "playQueue"),
        "an unsaved queue still sends a container: {unsaved}"
    );

    // And in XML, where an element present but empty renders as `<playQueue/>`
    // and would read as absent to any JSON-only assertion.
    let unsaved_xml = subsonic_xml(&router, "getPlayQueue", "wfsk_queue").await;
    assert!(unsaved_xml.contains("status=\"ok\""), "{unsaved_xml}");
    assert!(
        !unsaved_xml.contains("playQueue"),
        "an unsaved queue still sends a container: {unsaved_xml}"
    );

    // Something has to be in each container, or an assertion that `song` is
    // absent would hold on an empty answer and prove nothing.
    subsonic_json(
        &router,
        "savePlayQueue",
        "wfsk_queue",
        &format!("&id={song}&current={song}&position=25"),
    )
    .await;
    subsonic_json(
        &router,
        "scrobble",
        "wfsk_queue",
        &format!("&id={song}&submission=false"),
    )
    .await;

    let queue = subsonic_json(&router, "getPlayQueue", "wfsk_queue", "").await;
    let queue = &queue["subsonic-response"]["playQueue"];
    assert_eq!(queue["entry"][0]["id"], song.to_string());
    assert!(queue["song"].is_null(), "playQueue still carries `song`");
    // Required by the schema beside `changed` and `changedBy`, and absent until
    // the same change: a queue belongs to somebody, and said so nowhere.
    assert_eq!(queue["username"], "queue-owner");

    let playing = subsonic_json(&router, "getNowPlaying", "wfsk_queue", "").await;
    let playing = &playing["subsonic-response"]["nowPlaying"];
    assert_eq!(playing["entry"][0]["id"], song.to_string());
    assert!(playing["song"].is_null(), "nowPlaying still carries `song`");

    // The XML half. A JSON-only assertion would miss a renaming applied to the
    // array table alone, which is where the JSON shape is decided.
    for (method, container) in [
        ("getPlayQueue", "playQueue"),
        ("getNowPlaying", "nowPlaying"),
    ] {
        let xml = subsonic_xml(&router, method, "wfsk_queue").await;
        assert!(xml.contains(&format!("<{container}")), "{method}: {xml}");
        assert!(xml.contains("<entry "), "{method} has no `entry`: {xml}");
        assert!(
            !xml.contains("<song "),
            "{method} still sends `song`: {xml}"
        );
    }
}
