//! The legacy wire: a client named in `WAVEFLOW_SUBSONIC_LEGACY_CLIENTS` is
//! answered in Subsonic 1.16.1, without what OpenSubsonic added.
//!
//! Found with DSub 5.5.3 on 2026-10-28: its XML parser collects every
//! `<artist>` element wherever it sits, so the reference nested inside a
//! `contributors` credit became a starred artist in `getStarred` and the
//! header of an album in `getMusicDirectory`. The fixture is that catalogue in
//! small: one starred track by The Nocturnes on the album Session Work,
//! composed by Bach and Gounod and produced by Hal Brennan, and a sketch with
//! no album and the same credits, which `getIndexes` lists at the root.

use super::*;

/// Every attribute the 1.16.1 schema gives the types a media item is rendered
/// as — `Child`, `AlbumID3`, `ArtistID3` and `Artist` — and nothing else.
///
/// A whitelist, where the server removes a named list: a field added to a
/// node builder later and forgotten in `src/subsonic/legacy.rs` fails here,
/// even if nobody thought to name it.
const SUBSONIC_ITEM_ATTRIBUTES: &[&str] = &[
    "album",
    "albumCount",
    "albumId",
    "artist",
    "artistId",
    "artistImageUrl",
    "averageRating",
    "bitRate",
    "bookmarkPosition",
    "contentType",
    "coverArt",
    "created",
    "discNumber",
    "duration",
    "genre",
    "id",
    "isDir",
    "isVideo",
    "name",
    "originalHeight",
    "originalWidth",
    "parent",
    "path",
    "playCount",
    "size",
    "songCount",
    "starred",
    "suffix",
    "title",
    "track",
    "transcodedContentType",
    "transcodedSuffix",
    "type",
    "userRating",
    "year",
];

/// The element names a media item is rendered under on the four methods
/// these tests read.
const MEDIA_ITEMS: &[&str] = &["song", "child", "album", "artist", "entry"];

/// The names a credit put on the wire. None may reach a legacy client, in an
/// attribute or anywhere else.
const CREDITED: &[&str] = &["Bach", "Gounod", "Hal Brennan"];

struct Fixture {
    router: axum::Router,
    api_key: String,
    song: String,
    album: String,
}

/// One track starred with its album and its artist, credited to two composers
/// and a producer, and carrying the tags the other OpenSubsonic fields read.
async fn starred_session(
    config: &waveflow_server::Config,
    state: &waveflow_server::AppState,
) -> Fixture {
    let (account, _token) = admin_with_api_token(state, "legacy-listener").await;
    // Drawn at run time, so no credential is ever written into the source.
    let api_key = format!("wfsk_{}", uuid::Uuid::new_v4().simple());
    state
        .db
        .set_subsonic_credential(
            account,
            account,
            &state
                .secret_box
                .encrypt(uuid::Uuid::new_v4().to_string().as_bytes())
                .unwrap(),
            &security::token_hash(&api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("session-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            account,
            "Sessions",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(account), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 1, false).await.unwrap();
    let mut input = browse_input(
        0,
        "Ave Maria",
        "Session Work",
        "The Nocturnes",
        Some(1),
        Some(1),
    );
    input.roles = vec![
        (
            waveflow_server::library::tags::Role::Composer,
            vec!["Bach".into(), "Gounod".into()],
        ),
        (
            waveflow_server::library::tags::Role::Producer,
            vec!["Hal Brennan".into()],
        ),
    ];
    input.isrc = Some("GBAYE0601498".into());
    input.moods = Some("Calm".into());
    input.bpm = Some(72);
    input.comment = Some("Second take".into());
    input.replay_gain_track_gain = Some(-6.5);
    input.musicbrainz_recording_id = Some(uuid::Uuid::new_v4().to_string());
    input.sort_title = Some("Ave Maria".into());
    input.record_labels = Some("Nocturne Records".into());
    input.release_date = Some("2024-03-01".into());
    state
        .db
        .apply_catalog_track(library, scan, &input, None, false)
        .await
        .unwrap();
    // The same credits on a file tagged with no album: a root `child` of
    // `getIndexes` (#372), the one media item no other method here reaches.
    let mut loose = browse_input(1, "Loose Sketch", "", "The Nocturnes", None, None);
    loose.album = None;
    loose.album_artist = None;
    loose.relative_path = "Loose/sketch.flac".into();
    loose.roles = input.roles.clone();
    state
        .db
        .apply_catalog_track(library, scan, &loose, None, false)
        .await
        .unwrap();
    state
        .db
        .consolidate_catalog_derivations(library)
        .await
        .unwrap();
    state.db.consolidate_sort_names(library).await.unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let router = waveflow_server::app(config, state.clone());

    let song: String =
        sqlx::query_scalar("SELECT id FROM track WHERE library_id=? AND title='Ave Maria'")
            .bind(library.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    let full = subsonic_json(&router, "getSong", &api_key, &format!("&id={song}")).await;
    let item = &full["subsonic-response"]["song"];
    let album = item["albumId"].as_str().expect("the album id").to_owned();
    let artist = item["artistId"].as_str().expect("the artist id").to_owned();
    // The fixture is only worth something if the full wire carries the
    // credits: otherwise a legacy answer without them proves nothing.
    assert_eq!(
        item["contributors"].as_array().map(Vec::len),
        Some(3),
        "the full wire credits both composers and the producer: {item}"
    );
    let starred = subsonic_json(
        &router,
        "star",
        &api_key,
        &format!("&id={song}&albumId={album}&artistId={artist}"),
    )
    .await;
    assert_eq!(starred["subsonic-response"]["status"], "ok");
    Fixture {
        router,
        api_key,
        song,
        album,
    }
}

/// The four methods DSub reads the symptom through, with their arguments.
fn methods(fixture: &Fixture) -> [(&'static str, String); 4] {
    [
        ("getStarred", String::new()),
        ("getMusicDirectory", format!("&id={}", fixture.album)),
        ("getSong", format!("&id={}", fixture.song)),
        ("getAlbum", format!("&id={}", fixture.album)),
    ]
}

/// One `/rest` request as the client named `client`, in either encoding.
async fn ask(fixture: &Fixture, client: &str, method: &str, extra: &str, json: bool) -> String {
    let format = if json { "&f=json" } else { "" };
    let response = fixture
        .router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/{method}.view?apiKey={}&v=1.16.1&c={client}{format}{extra}",
                fixture.api_key
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_text(response).await;
    assert!(
        body.contains(r#"status="ok""#) || body.contains(r#""status":"ok""#),
        "{method} as {client} succeeds: {body}"
    );
    body
}

/// Holds an XML answer to the legacy wire: no media item carries an attribute
/// the 1.16.1 schema does not give it, no child element at all except the
/// songs of an album, and no `<artist>` sits anywhere but directly under the
/// `starred` container.
fn assert_legacy_xml(method: &str, body: &str) {
    let document = roxmltree::Document::parse(body)
        .unwrap_or_else(|error| panic!("{method} is well-formed XML ({error}): {body}"));
    // DSub's symptom first, on its own, so a regression that brings it back
    // is reported as itself rather than as whichever attribute comes first.
    for element in document
        .descendants()
        .filter(|node| node.has_tag_name("artist"))
    {
        let parent = element
            .parent_element()
            .map(|parent| parent.tag_name().name());
        assert_eq!(
            parent,
            Some("starred"),
            "{method}: an <artist> nested in a {parent:?} is what DSub lifts out: {body}"
        );
    }
    let mut items = 0;
    for element in document.descendants().filter(|node| node.is_element()) {
        let name = element.tag_name().name();
        if !MEDIA_ITEMS.contains(&name) {
            continue;
        }
        items += 1;
        for attribute in element.attributes() {
            assert!(
                SUBSONIC_ITEM_ATTRIBUTES.contains(&attribute.name()),
                "{method}: <{name} {}> is not a Subsonic 1.16.1 attribute: {body}",
                attribute.name()
            );
        }
        for child in element.children().filter(|node| node.is_element()) {
            assert!(
                name == "album" && child.tag_name().name() == "song",
                "{method}: <{name}> carries <{}>, which Subsonic 1.16.1 does not define: {body}",
                child.tag_name().name()
            );
        }
    }
    assert!(items > 0, "{method} answered a media item to check: {body}");
}

/// The same rule over a JSON answer: every object holding an `id` is a media
/// item here — or the directory, whose 1.16.1 attributes are a subset of the
/// same list — and its keys are the 1.16.1 attributes, plus the list of songs
/// an album holds and of children a directory holds.
fn assert_legacy_json(method: &str, value: &serde_json::Value, body: &str) -> usize {
    let mut items = 0;
    match value {
        serde_json::Value::Object(map) => {
            if map.contains_key("id") {
                items += 1;
                for (key, nested) in map {
                    if matches!(key.as_str(), "song" | "child") && nested.is_array() {
                        continue;
                    }
                    assert!(
                        SUBSONIC_ITEM_ATTRIBUTES.contains(&key.as_str()),
                        "{method}: `{key}` is not a Subsonic 1.16.1 attribute: {body}"
                    );
                }
            }
            for nested in map.values() {
                items += assert_legacy_json(method, nested, body);
            }
        }
        serde_json::Value::Array(list) => {
            for nested in list {
                items += assert_legacy_json(method, nested, body);
            }
        }
        _ => {}
    }
    items
}

/// DSub's two symptoms, in the encoding it reads: a credited name reaches no
/// element of a legacy answer, and nothing OpenSubsonic added does either.
#[tokio::test]
async fn a_legacy_client_meets_no_nested_artist_in_xml() {
    let (_temp, config, state) = test_app().await;
    let fixture = starred_session(&config, &state).await;
    for (method, extra) in methods(&fixture) {
        let body = ask(&fixture, "DSub", method, &extra, false).await;
        for credited in CREDITED {
            assert!(
                !body.contains(credited),
                "{method} as DSub names {credited}, credited on the track only: {body}"
            );
        }
        for name in ["<contributors", "<artists", "<albumArtists", "<replayGain"] {
            assert!(
                !body.contains(name),
                "{method} as DSub carries {name}: {body}"
            );
        }
        assert_legacy_xml(method, &body);
        // The Subsonic fields themselves are untouched.
        assert!(
            body.contains(r#"title="Ave Maria""#) && body.contains(r#"artist="The Nocturnes""#),
            "{method} as DSub still describes the track: {body}"
        );
    }
    // The starred artist is the one that was starred, and the only one.
    let starred = ask(&fixture, "DSub", "getStarred", "", false).await;
    assert_eq!(starred.matches("<artist ").count(), 1, "{starred}");
    // The header DSub showed Hal Brennan in is the album's own name again.
    let directory = ask(
        &fixture,
        "DSub",
        "getMusicDirectory",
        &format!("&id={}", fixture.album),
        false,
    )
    .await;
    assert!(directory.contains(r#"name="Session Work""#), "{directory}");
    // The envelope still says what the server is: Navidrome keeps it too.
    assert!(directory.contains(r#"openSubsonic="true""#), "{directory}");
}

/// The same answers in JSON, where the presence rule would otherwise put the
/// removed arrays back as `[]`.
#[tokio::test]
async fn a_legacy_client_meets_no_open_subsonic_field_in_json() {
    let (_temp, config, state) = test_app().await;
    let fixture = starred_session(&config, &state).await;
    for (method, extra) in methods(&fixture) {
        let body = ask(&fixture, "DSub", method, &extra, true).await;
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        let payload = value["subsonic-response"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(key, value)| value.is_object() && *key != "error")
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| panic!("{method} answers a payload: {body}"));
        for credited in CREDITED {
            assert!(
                !body.contains(credited),
                "{method} as DSub names {credited}, credited on the track only: {body}"
            );
        }
        assert!(
            assert_legacy_json(method, &payload, &body) > 0,
            "{method} answered a media item to check: {body}"
        );
        assert_eq!(value["subsonic-response"]["openSubsonic"], true, "{body}");
    }
    let song = ask(
        &fixture,
        "DSub",
        "getSong",
        &format!("&id={}", fixture.song),
        true,
    )
    .await;
    let song: serde_json::Value = serde_json::from_str(&song).unwrap();
    let song = &song["subsonic-response"]["song"];
    assert_eq!(song["title"], "Ave Maria", "{song}");
    assert_eq!(song["artist"], "The Nocturnes", "{song}");
    for key in [
        "contributors",
        "artists",
        "albumArtists",
        "replayGain",
        "genres",
    ] {
        assert!(omits(song, key), "`{key}` reaches DSub: {song}");
    }
}

/// Every other client keeps the whole wire: the setting names DSub, not the
/// clients that read OpenSubsonic.
#[tokio::test]
async fn another_client_keeps_every_open_subsonic_field() {
    let (_temp, config, state) = test_app().await;
    let fixture = starred_session(&config, &state).await;
    for (method, extra) in methods(&fixture) {
        let xml = ask(&fixture, "Feishin", method, &extra, false).await;
        for name in ["<contributors", "<artists", "<albumArtists", "Hal Brennan"] {
            // The album itself has no credits of its own on getAlbum's root,
            // but its song does, and every method here carries the song.
            assert!(xml.contains(name), "{method} as Feishin lost {name}: {xml}");
        }
        let json = ask(&fixture, "Feishin", method, &extra, true).await;
        for key in [r#""contributors""#, r#""albumArtists""#, r#""replayGain""#] {
            assert!(json.contains(key), "{method} as Feishin lost {key}: {json}");
        }
    }
    let song = ask(
        &fixture,
        "Feishin",
        "getSong",
        &format!("&id={}", fixture.song),
        true,
    )
    .await;
    let song: serde_json::Value = serde_json::from_str(&song).unwrap();
    let song = &song["subsonic-response"]["song"];
    assert_eq!(song["displayComposer"], "Bach \u{2022} Gounod", "{song}");
    assert_eq!(song["contributors"].as_array().map(Vec::len), Some(3));
    assert_eq!(song["mediaType"], "song", "{song}");
}

/// The list is the operator's: a name it does not hold is answered in full,
/// DSub included once it is left out, and the comparison is exact.
#[tokio::test]
async fn the_legacy_list_is_the_operators() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    config.subsonic_legacy_clients = vec!["Audinaut".into()];
    let state = waveflow_server::initialize(&config).await.unwrap();
    let fixture = starred_session(&config, &state).await;
    let extra = format!("&id={}", fixture.song);
    let named = ask(&fixture, "Audinaut", "getSong", &extra, false).await;
    assert_legacy_xml("getSong", &named);
    for client in ["DSub", "audinaut", "Audinaut2"] {
        let full = ask(&fixture, client, "getSong", &extra, false).await;
        assert!(
            full.contains("<contributors"),
            "{client} is not on the list and is answered in full: {full}"
        );
    }
}

/// What the legacy wire removes is the media items' additions, not the arrays
/// a method is made of: `getLyricsBySongId`, which a legacy client may still
/// ask for, keeps its empty `structuredLyrics` rather than answering a list
/// without the list.
#[tokio::test]
async fn a_legacy_client_keeps_the_lyrics_list_shape() {
    let (_temp, config, state) = test_app().await;
    let fixture = starred_session(&config, &state).await;
    let body = ask(
        &fixture,
        "DSub",
        "getLyricsBySongId",
        &format!("&id={}", fixture.song),
        true,
    )
    .await;
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        value["subsonic-response"]["lyricsList"]["structuredLyrics"],
        serde_json::json!([]),
        "{body}"
    );
}

/// The root files `getIndexes` lists (#372) are media items like any other:
/// a legacy client meets the sketch there without its credits, in XML and in
/// JSON, while every other client still reads them.
#[tokio::test]
async fn a_legacy_client_meets_the_root_files_of_get_indexes_bare() {
    let (_temp, config, state) = test_app().await;
    let fixture = starred_session(&config, &state).await;
    let full = ask(&fixture, "Feishin", "getIndexes", "", false).await;
    assert!(
        full.contains("Loose Sketch") && full.contains("Hal Brennan"),
        "the full wire lists the sketch with its credits: {full}"
    );

    let xml = ask(&fixture, "DSub", "getIndexes", "", false).await;
    for credited in CREDITED {
        assert!(
            !xml.contains(credited),
            "getIndexes as DSub names {credited}: {xml}"
        );
    }
    let document = roxmltree::Document::parse(&xml).unwrap();
    let children: Vec<_> = document
        .descendants()
        .filter(|node| node.has_tag_name("child"))
        .collect();
    assert_eq!(children.len(), 1, "the sketch is a root file: {xml}");
    assert_eq!(
        children[0].attribute("title"),
        Some("Loose Sketch"),
        "{xml}"
    );
    for attribute in children[0].attributes() {
        assert!(
            SUBSONIC_ITEM_ATTRIBUTES.contains(&attribute.name()),
            "getIndexes: <child {}> is not a Subsonic 1.16.1 attribute: {xml}",
            attribute.name()
        );
    }
    assert_eq!(
        children[0]
            .children()
            .filter(|node| node.is_element())
            .count(),
        0,
        "getIndexes: the root file carries a child element: {xml}"
    );

    let json = ask(&fixture, "DSub", "getIndexes", "", true).await;
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let child = &value["subsonic-response"]["indexes"]["child"];
    assert_eq!(child.as_array().map(Vec::len), Some(1), "{json}");
    for (key, _) in child[0].as_object().unwrap() {
        assert!(
            SUBSONIC_ITEM_ATTRIBUTES.contains(&key.as_str()),
            "getIndexes: `{key}` is not a Subsonic 1.16.1 attribute: {json}"
        );
    }
}

/// The `Content-Type` of a `/rest` answer as the client named `client`.
async fn media_type(
    fixture: &Fixture,
    client: &str,
    method: &str,
    extra: &str,
) -> (String, String) {
    let response = fixture
        .router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/{method}.view?apiKey={}&v=1.16.1&c={client}{extra}",
                fixture.api_key
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let media_type = response.headers()[axum::http::header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .to_owned();
    (media_type, body_text(response).await)
}

/// A legacy client is answered in `text/xml`, errors included.
///
/// Found with DSub 5.5.3 on 2026-10-28: it tells an error in place of a stream
/// from audio only by a type beginning `text/xml`, so the `application/xml`
/// error 70 for a removed track went into its cache as a 234-byte FLAC and
/// was requested again every second. The full wire keeps `application/xml`,
/// and JSON is JSON on both.
#[tokio::test]
async fn a_legacy_client_is_answered_in_text_xml_errors_included() {
    let (_temp, config, state) = test_app().await;
    let fixture = starred_session(&config, &state).await;
    let gone = format!("&id={}", uuid::Uuid::new_v4());
    for (client, expected) in [
        ("DSub", "text/xml; charset=utf-8"),
        ("Feishin", "application/xml; charset=utf-8"),
    ] {
        let (ping, body) = media_type(&fixture, client, "ping", "").await;
        assert_eq!(ping, expected, "ping as {client}: {body}");
        let (stream, body) = media_type(&fixture, client, "stream", &gone).await;
        assert!(body.contains(r#"code="70""#), "stream as {client}: {body}");
        assert_eq!(stream, expected, "stream error as {client}: {body}");
        let (json, body) = media_type(&fixture, client, "stream", &format!("{gone}&f=json")).await;
        assert!(body.contains(r#""code":70"#), "stream as {client}: {body}");
        assert_eq!(json, "application/json; charset=utf-8", "{client}: {body}");
    }
}

/// A legacy client that names itself in the query is answered as itself even
/// when its `formPost` body cannot be read: the refusal is `text/xml` too.
#[tokio::test]
async fn a_legacy_client_keeps_text_xml_when_its_post_body_is_refused() {
    let (_temp, config, state) = test_app().await;
    let fixture = starred_session(&config, &state).await;
    for (client, expected) in [
        ("DSub", "text/xml; charset=utf-8"),
        ("Feishin", "application/xml; charset=utf-8"),
    ] {
        let response = fixture
            .router
            .clone()
            .oneshot(
                Request::post(format!(
                    "/rest/ping.view?apiKey={}&v=1.16.1&c={client}",
                    fixture.api_key
                ))
                .header(axum::http::header::CONTENT_TYPE, "text/plain")
                .body(Body::from("f=json"))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let media_type = response.headers()[axum::http::header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .to_owned();
        let body = body_text(response).await;
        assert!(body.contains(r#"code="10""#), "{client}: {body}");
        assert_eq!(media_type, expected, "refused POST as {client}: {body}");
    }
}

/// A client that names itself only in a `formPost` body is answered as itself
/// too: the wire is read again once the body is merged, so a `stream` error
/// posted by DSub is `text/xml`.
#[tokio::test]
async fn a_legacy_client_naming_itself_in_its_post_body_is_answered_in_text_xml() {
    let (_temp, config, state) = test_app().await;
    let fixture = starred_session(&config, &state).await;
    let gone = uuid::Uuid::new_v4();
    for (client, expected) in [
        ("DSub", "text/xml; charset=utf-8"),
        ("Feishin", "application/xml; charset=utf-8"),
    ] {
        let response = fixture
            .router
            .clone()
            .oneshot(
                Request::post(format!(
                    "/rest/stream.view?apiKey={}&v=1.16.1",
                    fixture.api_key
                ))
                .header(
                    axum::http::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(Body::from(format!("c={client}&id={gone}")))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let media_type = response.headers()[axum::http::header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .to_owned();
        let body = body_text(response).await;
        assert!(body.contains(r#"code="70""#), "{client}: {body}");
        assert_eq!(
            media_type, expected,
            "posted stream error as {client}: {body}"
        );
    }
}
