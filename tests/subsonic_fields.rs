//! The shape of what the facade emits, in both encodings.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use tower::ServiceExt;
use waveflow_server::authentication::now_ms;
use waveflow_server::database::AccountRole;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::security;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
mod support;
use support::*;

/// OpenSubsonic reports support for a field by emitting it even when the value
/// is unknown, so a track with nothing tagged is as much a test of the contract
/// as a fully tagged one — in both encodings, since XML and JSON build the
/// arrays through different code paths.
#[tokio::test]
async fn media_items_carry_the_modern_opensubsonic_fields_in_both_encodings() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_fields-key";
    let subsonic_password = "fields-secret";
    let owner = state
        .db
        .create_account(
            "fields-owner",
            &security::hash_password("correct horse battery staple").unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state
                .secret_box
                .encrypt(subsonic_password.as_bytes())
                .unwrap(),
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("fields-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Fields",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 2, false).await.unwrap();

    // Two credited artists and two genres, so tag order and the split are both
    // observable. The album is a compilation, which is stored and was never
    // surfaced before.
    let mut tagged = browse_input(80, "Tagged", "Sun Bloom", "Aria Lux", Some(1), Some(1));
    tagged.artist = Some("Aria Lux; Mono Field".into());
    tagged.album_artist = Some("Aria Lux".into());
    tagged.genre = Some("Rock; Jazz".into());
    tagged.is_compilation = true;
    tagged.musicbrainz_recording_id = Some("9f4c1d2e-recording".into());
    tagged.musicbrainz_release_id = Some("3a8b7c6d-release".into());
    tagged.musicbrainz_artist_id = Some("1b2c3d4e-artist".into());
    tagged.replay_gain_track_gain = Some(-7.32);
    tagged.replay_gain_track_peak = Some(0.988_525);
    tagged.replay_gain_album_gain = Some(-6.5);
    tagged.replay_gain_album_peak = Some(1.0);
    tagged.bpm = Some(128);
    tagged.sort_title = Some("Tagged, The".into());
    tagged.comment = Some("ripped from vinyl".into());
    tagged.isrc = Some("FRZ039800212; GBAYE0601498".into());
    tagged.moods = Some("Melancholic; Warm".into());
    tagged.explicit_status = Some("clean".into());
    state
        .db
        .apply_catalog_track(library, scan, &tagged, None, false)
        .await
        .unwrap();

    // Nothing tagged and nothing decoded: the case where every added field has
    // to be present with its default rather than omitted.
    let mut bare = browse_input(81, "Bare", "Sun Bloom", "Aria Lux", Some(2), Some(1));
    bare.artist = None;
    bare.album_artist = Some("Aria Lux".into());
    bare.genre = None;
    bare.is_compilation = true;
    bare.sample_rate = None;
    bare.channels = None;
    bare.bit_depth = None;
    state
        .db
        .apply_catalog_track(library, scan, &bare, None, false)
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();

    let songs = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .songs;
    let song_id = |title: &str| {
        songs
            .iter()
            .find(|song| song.title == title)
            .unwrap_or_else(|| panic!("{title} was indexed"))
            .id
    };
    let tagged_id = song_id("Tagged");
    let bare_id = song_id("Bare");
    state
        .services
        .scrobble(owner, tagged_id, true, Some(1_700_000_000_000))
        .await
        .unwrap();

    let router = waveflow_server::app(&config, state.clone());

    let tagged_json = subsonic_json(&router, "getSong", api_key, &format!("&id={tagged_id}")).await;
    let tagged_json = &tagged_json["subsonic-response"]["song"];
    assert_eq!(tagged_json["samplingRate"], 44_100);
    assert_eq!(tagged_json["channelCount"], 2);
    assert_eq!(tagged_json["bitDepth"], 16);
    assert_eq!(tagged_json["mediaType"], "song");
    assert_eq!(tagged_json["isVideo"], false);
    assert_eq!(tagged_json["playCount"], 1);
    assert!(tagged_json["played"].is_string());
    assert_eq!(tagged_json["displayArtist"], "Aria Lux; Mono Field");
    // Tag order, not alphabetical: "Mono Field" is credited second.
    let artists = tagged_json["artists"].as_array().unwrap();
    assert_eq!(artists.len(), 2);
    assert_eq!(artists[0]["name"], "Aria Lux");
    assert_eq!(artists[1]["name"], "Mono Field");
    assert!(artists[0]["id"].is_string());
    // The primary credit still matches the frozen artistId.
    assert_eq!(tagged_json["artistId"], artists[0]["id"]);
    assert_eq!(tagged_json["musicBrainzId"], "9f4c1d2e-recording");
    assert_eq!(tagged_json["bpm"], 128);
    assert_eq!(tagged_json["sortName"], "Tagged, The");
    assert_eq!(tagged_json["comment"], "ripped from vinyl");
    // Multi-valued like artists and genres, and split the same way.
    assert_eq!(
        tagged_json["isrc"],
        serde_json::json!(["FRZ039800212", "GBAYE0601498"])
    );
    assert_eq!(
        tagged_json["replayGain"],
        serde_json::json!({
            "trackGain": -7.32,
            "trackPeak": 0.988_525,
            "albumGain": -6.5,
            "albumPeak": 1.0
        })
    );
    assert_eq!(
        tagged_json["moods"],
        serde_json::json!(["Melancholic", "Warm"])
    );
    assert_eq!(tagged_json["explicitStatus"], "clean");
    // Genres are ordered by name so two identical catalogues answer identically.
    let genres = tagged_json["genres"]
        .as_array()
        .unwrap()
        .iter()
        .map(|genre| genre["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(genres, vec!["Jazz", "Rock"]);

    let bare_json = subsonic_json(&router, "getSong", api_key, &format!("&id={bare_id}")).await;
    let bare_json = &bare_json["subsonic-response"]["song"];
    // Present with their defaults. Omitting them would tell the client this
    // server does not implement the fields at all.
    assert_eq!(bare_json["samplingRate"], 0);
    assert_eq!(bare_json["channelCount"], 0);
    assert_eq!(bare_json["bitDepth"], 0);
    assert_eq!(bare_json["playCount"], 0);
    assert_eq!(bare_json["displayArtist"], "");
    assert_eq!(bare_json["artists"], serde_json::json!([]));
    assert_eq!(bare_json["genres"], serde_json::json!([]));
    assert_eq!(bare_json["musicBrainzId"], "");
    assert_eq!(bare_json["bpm"], 0);
    assert_eq!(bare_json["sortName"], "");
    assert_eq!(bare_json["comment"], "");
    assert_eq!(bare_json["isrc"], serde_json::json!([]));
    assert_eq!(bare_json["moods"], serde_json::json!([]));
    assert_eq!(bare_json["explicitStatus"], "");
    // replayGain is the one addition whose members the specification says to
    // omit when unknown. The container still has to be there: it is what says
    // the server reads gain tags at all.
    assert_eq!(bare_json["replayGain"], serde_json::json!({}));
    // The documented exception: an empty string is not a timestamp, and
    // playCount already signals that play statistics are supported.
    assert!(bare_json.get("played").is_none());

    // XML builds the arrays as repeated child elements rather than as a JSON
    // array, so it is asserted separately rather than assumed.
    let plain_auth = format!("u=fields-owner&p={subsonic_password}&v=1.16.1&c=golden");
    let xml_song = |id: uuid::Uuid| {
        let router = router.clone();
        let plain_auth = plain_auth.clone();
        async move {
            let response = router
                .oneshot(
                    Request::get(format!("/rest/getSong.view?{plain_auth}&id={id}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            body_text(response).await
        }
    };
    let tagged_xml = xml_song(tagged_id).await;
    assert!(tagged_xml.contains("samplingRate=\"44100\""));
    assert!(tagged_xml.contains("channelCount=\"2\""));
    assert!(tagged_xml.contains("bitDepth=\"16\""));
    assert!(tagged_xml.contains("mediaType=\"song\""));
    assert!(tagged_xml.contains("isVideo=\"false\""));
    assert!(tagged_xml.contains("<artists "));
    assert!(tagged_xml.contains("name=\"Mono Field\""));
    assert!(tagged_xml.contains("<genres name=\"Jazz\"/>"));
    assert!(tagged_xml.contains("<genres name=\"Rock\"/>"));
    assert!(tagged_xml.contains("played="));
    assert!(tagged_xml.contains("musicBrainzId=\"9f4c1d2e-recording\""));
    assert!(tagged_xml.contains("bpm=\"128\""));
    assert!(tagged_xml.contains("<isrc>FRZ039800212</isrc>"));
    assert!(tagged_xml.contains("<isrc>GBAYE0601498</isrc>"));
    assert!(tagged_xml.contains("trackGain=\"-7.32\""));
    assert!(tagged_xml.contains("<moods>Melancholic</moods>"));
    assert!(tagged_xml.contains("explicitStatus=\"clean\""));

    let bare_xml = xml_song(bare_id).await;
    assert!(bare_xml.contains("samplingRate=\"0\""));
    assert!(bare_xml.contains("bitDepth=\"0\""));
    assert!(bare_xml.contains("displayArtist=\"\""));
    // An empty array has no repeated element to render, which is exactly why
    // the JSON branch needs its own rule and gets its own assertion above.
    assert!(!bare_xml.contains("<artists "));
    assert!(!bare_xml.contains("<genres "));
    assert!(!bare_xml.contains("played="));
    assert!(bare_xml.contains("musicBrainzId=\"\""));
    assert!(bare_xml.contains("bpm=\"0\""));
    assert!(!bare_xml.contains("<isrc>"));
    // Present but empty, in both encodings.
    assert!(bare_xml.contains("<replayGain/>"));

    // Albums carry their own additions.
    let album_id = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .albums[0]
        .id;
    let album = subsonic_json(&router, "getAlbum", api_key, &format!("&id={album_id}")).await;
    let album = &album["subsonic-response"]["album"];
    assert_eq!(album["isCompilation"], true);
    assert_eq!(album["playCount"], 1);
    assert_eq!(album["displayArtist"], "Aria Lux");
    assert!(album["played"].is_string());

    // The native surface reads the same projection, so the structured relations
    // reach it without a second implementation.
    let token = login_token(&router, "fields-owner", "correct horse battery staple").await;
    let native = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/tracks/{tagged_id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(native.status(), StatusCode::OK);
    let native = json_body(native).await;
    assert_eq!(native["sample_rate"], 44_100);
    assert_eq!(native["play_count"], 1);
    assert_eq!(
        native["artists"]
            .as_array()
            .unwrap()
            .iter()
            .map(|artist| artist["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>(),
        vec!["Aria Lux", "Mono Field"]
    );
    assert_eq!(native["genres"], serde_json::json!(["Jazz", "Rock"]));
    assert_eq!(native["musicbrainz_id"], "9f4c1d2e-recording");
    assert_eq!(native["bpm"], 128);
    assert_eq!(
        native["isrc"],
        serde_json::json!(["FRZ039800212", "GBAYE0601498"])
    );
}

/// Two artist projections that are deliberately not the same shape, pinned in
/// both encodings so neither drifts into the other.
///
/// `artists[]` and `albumArtists[]` are *references*: an identifier and a
/// display name, and nothing else. The entries `getMusicDirectory` renders as
/// `child` are the artist and album nodes themselves, minus `musicBrainzId`,
/// so they do carry `sortName` and the rest. A field added to the node reaches
/// the second and must not leak into the first.
#[tokio::test]
async fn an_artist_reference_is_not_an_artist_record() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_reference-key";
    let admin = state
        .db
        .create_account(
            "reference-admin",
            &security::hash_password("correct horse battery staple").unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    let encrypted = state.secret_box.encrypt(b"subsonic-secret-123").unwrap();
    state
        .db
        .set_subsonic_credential(
            admin,
            admin,
            &encrypted,
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("reference-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(admin, "Refs", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(admin), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 1, false).await.unwrap();
    let mut input = catalog_input(0, "The Nocturnes");
    input.title = "Opening".into();
    input.album = Some("The Night Sessions".into());
    input.album_artist = Some("The Nocturnes".into());
    input.is_compilation = false;
    input.sort_album = Some("Night Sessions, The".into());
    input.sort_album_artist = Some("Nocturnes, The".into());
    input.sort_artist = Some("Nocturnes, The".into());
    // A contributor, so the whitelist below actually has one to check.
    input.roles = vec![(
        waveflow_server::tags::Role::Producer,
        vec!["Rita Sound".into()],
    )];
    state
        .db
        .apply_catalog_track(library, scan, &input, None, false)
        .await
        .unwrap();
    state
        .db
        .consolidate_catalog_derivations(library)
        .await
        .unwrap();
    state.db.consolidate_sort_names(library).await.unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let router = waveflow_server::app(&config, state.clone());

    // JSON: the album carries its own sortName; its `artists[]` entry carries
    // an identifier and a name, and no third key.
    let albums = subsonic_json(
        &router,
        "getAlbumList2",
        api_key,
        "&type=alphabeticalByName",
    )
    .await;
    let album = albums["subsonic-response"]["albumList2"]["album"][0].clone();
    assert_eq!(album["sortName"], serde_json::json!("Night Sessions, The"));
    let reference_keys = |value: &serde_json::Value, what: &str| {
        let entry = value
            .as_object()
            .unwrap_or_else(|| panic!("{what} is an object: {value}"))
            .clone();
        let mut keys: Vec<String> = entry.keys().cloned().collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["id".to_owned(), "name".to_owned()],
            "{what} is a reference, not an ArtistID3: {entry:?}"
        );
    };
    reference_keys(&album["artists"][0], "an album's artists[] entry");

    // `albumArtists[]` is a reference too, and it is emitted on media items
    // rather than on the album — the album's own credit is `artists[]`. So it
    // is pinned on a song of the album.
    let album_json = subsonic_json(
        &router,
        "getAlbum",
        api_key,
        &format!("&id={}", album["id"].as_str().expect("the album id")),
    )
    .await;
    let song = album_json["subsonic-response"]["album"]["song"][0].clone();
    reference_keys(&song["artists"][0], "a song's artists[] entry");
    reference_keys(&song["albumArtists"][0], "a song's albumArtists[] entry");
    // A contributor's artist is a reference on the same terms, and the
    // contributor itself carries only what names the credit.
    // Unconditional: the fixture credits a producer, so a contributor that
    // stopped being emitted would fail here rather than skip the check.
    {
        let credit = song["contributors"][0].clone();
        reference_keys(&credit["artist"], "a contributor's artist");
        let mut keys: Vec<String> = credit
            .as_object()
            .expect("a contributor is an object")
            .keys()
            .cloned()
            .collect();
        keys.sort_unstable();
        assert!(
            keys == vec!["artist".to_owned(), "role".to_owned()]
                || keys == vec!["artist".to_owned(), "role".to_owned(), "subRole".to_owned()],
            "a contributor names the credit and nothing more: {credit:?}"
        );
    }

    // XML: the same statement, in the encoding where an absent attribute is
    // absent rather than a missing key.
    let directory = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getMusicDirectory.view?apiKey={api_key}&v=1.16.1&c=fixtures&id={library}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(directory.status(), StatusCode::OK);
    let directory = body_text(directory).await;
    // The browsing child is the artist node minus musicBrainzId, so the sort
    // name reaches it, carrying the tagged value.
    assert!(
        directory.contains(r#"sortName="Nocturnes, The""#),
        "a getMusicDirectory child carries the artist's sortName: {directory}"
    );
    // ...while the references carry neither that field nor any other one
    // belonging to the record. They live on the album and on its songs, so
    // this reads the album document rather than the folder listing — where a
    // library holding no album-less track has no song child at all.
    let album_xml = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getAlbum.view?apiKey={api_key}&v=1.16.1&c=fixtures&id={}",
                album["id"].as_str().expect("the album id")
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(album_xml.status(), StatusCode::OK);
    let album_xml = body_text(album_xml).await;
    for name in ["<artists ", "<albumArtists "] {
        let mut references = 0;
        for element in album_xml.split(name).skip(1) {
            let element = element.split("/>").next().unwrap_or_default();
            references += 1;
            // The whitelist, rather than a list of fields to reject: a field
            // added to the artist node has to fail here even if nobody thought
            // to name it.
            // Split on the quote rather than on whitespace: an attribute
            // value holds spaces, and `name="The Nocturnes"` would otherwise
            // read as two attributes.
            let mut attributes: Vec<String> = element
                .split('"')
                .step_by(2)
                .map(|key| key.trim().trim_end_matches('=').trim().to_owned())
                .filter(|key| !key.is_empty())
                .collect();
            // Sorted before comparing, like the JSON side: which attributes
            // are present is the contract, the order they are written in is
            // not, and pinning it would fail a reordering that changes
            // nothing observable.
            attributes.sort_unstable();
            assert_eq!(
                attributes,
                vec!["id".to_owned(), "name".to_owned()],
                "{name} is a reference, not an ArtistID3: {element}"
            );
        }
        assert!(
            references > 0,
            "the fixture must exercise {name}: {album_xml}"
        );
    }
}

/// The credits OpenSubsonic asks for, and the presence rule they follow.
///
/// These three fields were absent because the columns they need did not
/// exist — and under the presence rule absent is a statement: it says the
/// server does not read them. Now that it does, they are emitted with their
/// default on a track that names nobody, which is the difference between
/// "unsupported" and "this file credits no composer".
#[tokio::test]
async fn credits_reach_the_wire_in_both_encodings() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_credits-key";
    let admin = state
        .db
        .create_account(
            "credit-admin",
            &security::hash_password("correct horse battery staple").unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    let encrypted = state.secret_box.encrypt(b"subsonic-secret-123").unwrap();
    state
        .db
        .set_subsonic_credential(
            admin,
            admin,
            &encrypted,
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("credit-wire");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            admin,
            "Credits",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(admin), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 2, false).await.unwrap();

    let mut credited = catalog_input(0, "Nova Kern");
    credited.title = "Credited".into();
    credited.album = Some("The Record".into());
    credited.album_artist = Some("Nova Kern; Lior Sand".into());
    credited.is_compilation = false;
    credited.roles = vec![
        (
            waveflow_server::tags::Role::Composer,
            vec!["Otto Pen; Ada Vale".into()],
        ),
        (
            waveflow_server::tags::Role::Producer,
            vec!["Rita Sound".into()],
        ),
    ];
    credited.performer_pairs = vec![("guitar".into(), "Jimmy Page".into())];
    state
        .db
        .apply_catalog_track(library, scan, &credited, None, false)
        .await
        .unwrap();

    let mut bare = catalog_input(1, "Nova Kern");
    bare.title = "Bare".into();
    bare.album = Some("The Record".into());
    bare.album_artist = Some("Nova Kern; Lior Sand".into());
    bare.is_compilation = false;
    state
        .db
        .apply_catalog_track(library, scan, &bare, None, false)
        .await
        .unwrap();
    state
        .db
        .consolidate_catalog_derivations(library)
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let router = waveflow_server::app(&config, state.clone());

    let albums = subsonic_json(
        &router,
        "getAlbumList2",
        api_key,
        "&type=alphabeticalByName",
    )
    .await;
    let album_id = albums["subsonic-response"]["albumList2"]["album"][0]["id"]
        .as_str()
        .expect("the album is listed")
        .to_owned();
    let album = subsonic_json(&router, "getAlbum", api_key, &format!("&id={album_id}")).await;
    let songs = album["subsonic-response"]["album"]["song"]
        .as_array()
        .expect("the album lists its songs")
        .clone();
    let song = |title: &str| {
        songs
            .iter()
            .find(|song| song["title"] == title)
            .unwrap_or_else(|| panic!("missing {title}"))
            .clone()
    };

    let credited = song("Credited");
    let contributors = credited["contributors"]
        .as_array()
        .expect("contributors is an array");
    let mut named: Vec<(String, String)> = contributors
        .iter()
        .map(|credit| {
            (
                credit["role"].as_str().unwrap().to_owned(),
                credit["artist"]["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    named.sort();
    assert_eq!(
        named,
        vec![
            ("composer".to_owned(), "Ada Vale".to_owned()),
            ("composer".to_owned(), "Otto Pen".to_owned()),
            ("performer".to_owned(), "Jimmy Page".to_owned()),
            ("producer".to_owned(), "Rita Sound".to_owned()),
        ],
        "every role but artist and albumartist is a contributor"
    );
    let performer = contributors
        .iter()
        .find(|credit| credit["role"] == "performer")
        .expect("the performer is credited");
    assert_eq!(
        performer["subRole"], "Guitar",
        "a performer carries the instrument, title-cased"
    );
    assert_eq!(
        credited["displayComposer"], "Otto Pen \u{2022} Ada Vale",
        "the composers, in tag order, joined the way the reference joins them"
    );
    let album_artists: Vec<&str> = credited["albumArtists"]
        .as_array()
        .expect("albumArtists is an array")
        .iter()
        .map(|artist| artist["name"].as_str().unwrap())
        .collect();
    assert_eq!(album_artists, vec!["Nova Kern", "Lior Sand"]);

    let bare = song("Bare");
    assert_eq!(bare["contributors"], serde_json::json!([]));
    assert_eq!(bare["displayComposer"], "");

    let artists = subsonic_json(&router, "getArtists", api_key, "").await;
    let indexed: Vec<serde_json::Value> = artists["subsonic-response"]["artists"]["index"]
        .as_array()
        .expect("the artist index")
        .iter()
        .flat_map(|index| index["artist"].as_array().unwrap().clone())
        .collect();
    let named = |name: &str| {
        indexed
            .iter()
            .find(|artist| artist["name"] == name)
            .unwrap_or_else(|| panic!("{name} is indexed"))
            .clone()
    };
    assert_eq!(
        named("Nova Kern")["roles"],
        serde_json::json!(["albumartist", "artist"]),
        "an album artist who also performs says both, in a stable order"
    );
    // A composer holds no album, so the index does not list them — but the
    // artist is still in the catalogue, findable by name, and still says what
    // it is. The second half is what makes the first acceptable: filtering the
    // index would otherwise put a credited artist out of reach entirely.
    let found = subsonic_json(&router, "search3", api_key, "&query=Otto").await;
    let found_artists: Vec<&str> = found["subsonic-response"]["searchResult3"]["artist"]
        .as_array()
        .map(|all| {
            all.iter()
                .filter_map(|artist| artist["name"].as_str())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        found_artists.contains(&"Otto Pen"),
        "a composer is reachable by searching their own name: {found_artists:?}"
    );
    assert!(
        !indexed.iter().any(|artist| artist["name"] == "Otto Pen"),
        "an artist credited on no album is not one of the library's artists"
    );
    let catalogue = state
        .services
        .list_artists(admin, None, Default::default())
        .await
        .unwrap();
    let composer = catalogue
        .iter()
        .find(|summary| summary.artist.name == "Otto Pen")
        .expect("the composer is in the catalogue");
    assert_eq!(composer.artist.roles, vec!["composer".to_owned()]);

    // An artist whose credits nothing has counted yet still says the field is
    // supported. The presence rule turns on emitting the default, and a record
    // that omits its own array is a server saying it does not read roles at
    // all — which is what the classification above must not do.
    let indexed_artist = named("Nova Kern");
    assert!(
        indexed_artist["roles"].is_array(),
        "a full artist record carries its roles array: {indexed_artist:?}"
    );
    // The same record through a second surface, because `getStarred2` builds
    // its artists from the same projection and the same node.
    let artist_id = indexed_artist["id"].as_str().expect("the artist id");
    subsonic_json(&router, "star", api_key, &format!("&artistId={artist_id}")).await;
    let starred = subsonic_json(&router, "getStarred2", api_key, "").await;
    let starred_artist = starred["subsonic-response"]["starred2"]["artist"]
        .as_array()
        .expect("the starred artists are an array")
        .iter()
        .find(|artist| artist["id"] == artist_id)
        .expect("the artist that was just starred")
        .clone();
    assert!(
        starred_artist["roles"].is_array(),
        "and carries it there too: {starred_artist:?}"
    );
    // A browsing child is an artist or an album under the element name a song
    // uses, and only `isDir` tells them apart. A folder entry must not answer
    // with a track's relations: an artist reporting `isrc: []` would be the
    // server claiming it read a recording identifier off a directory.
    let folders = subsonic_json(&router, "getMusicFolders", api_key, "").await;
    let folder_id = folders["subsonic-response"]["musicFolders"]["musicFolder"][0]["id"]
        .as_str()
        .expect("the library is listed")
        .to_owned();
    let directory = subsonic_json(
        &router,
        "getMusicDirectory",
        api_key,
        &format!("&id={folder_id}"),
    )
    .await;
    let entry = directory["subsonic-response"]["directory"]["child"]
        .as_array()
        .and_then(|children| children.iter().find(|child| child["isDir"] == true))
        .expect("the folder lists its artists")
        .clone();
    for absent in [
        "isrc",
        "moods",
        "albumArtists",
        "contributors",
        "artists",
        "genres",
    ] {
        assert!(
            entry.get(absent).is_none(),
            "a directory entry carries no {absent}: {entry:?}"
        );
    }
    assert!(
        entry["roles"].is_array(),
        "but it keeps the artist record shape it is rendered from: {entry:?}"
    );

    let xml = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getAlbum.view?apiKey={api_key}&v=1.16.1&c=fixtures&id={album_id}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let xml = body_text(xml).await;
    assert!(
        xml.contains("<contributors role=\"performer\" subRole=\"Guitar\">"),
        "the performer credit is an element carrying its instrument: {xml}"
    );
    assert!(
        xml.contains("displayComposer=\"Otto Pen \u{2022} Ada Vale\""),
        "the composer display string reaches XML too: {xml}"
    );
    // And the case that needs the field injected rather than rendered: an
    // artist whose files have all gone missing keeps its row and its credits,
    // but nothing counts them any more, so it holds no role at all. It still
    // has to say the field is supported.
    sqlx::query("UPDATE track SET is_available = 0 WHERE library_id = ?")
        .bind(library.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    state
        .db
        .consolidate_catalog_derivations(library)
        .await
        .unwrap();
    let bereft = subsonic_json(&router, "getArtist", api_key, &format!("&id={artist_id}")).await;
    assert_eq!(
        bereft["subsonic-response"]["artist"]["roles"],
        serde_json::json!([]),
        "empty, not absent: absent would say the server does not read roles"
    );
}

#[tokio::test]
async fn an_album_reports_its_release_details_and_its_disc_titles() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("release", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let encrypted = state
        .secret_box
        .encrypt(b"dedicated-subsonic-secret")
        .unwrap();
    let api_key = "wfsk_release-key";
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &encrypted,
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("release-music");
    std::fs::create_dir_all(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Release library",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan_id = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan_id, 2, false).await.unwrap();

    for (index, disc, subtitle) in [(0usize, 1i64, "The Session"), (1, 2, "The Rehearsal")] {
        let mut input = catalog_input(index, "Nova Kern");
        input.disc_number = Some(disc);
        input.disc_subtitle = Some(subtitle.into());
        if index == 0 {
            // Only the first track carries them: the album takes the first
            // value it is given and later tracks do not overwrite it.
            input.original_release_date = Some("1998-11".into());
            input.release_date = Some("2019-04-05".into());
            input.release_types = Some("Album; Compilation".into());
            input.record_labels = Some("Nightfall Records; Second Imprint".into());
        }
        state
            .db
            .apply_catalog_track(library_id, scan_id, &input, None, false)
            .await
            .unwrap();
    }
    // What a real scan does after applying its rows, before closing the job:
    // it builds the artist index and the role statistics a folder listing
    // reads. Driving the pipeline in another order would prove something about
    // an order nothing runs.
    state
        .db
        .consolidate_catalog_derivations(library_id)
        .await
        .unwrap();
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    let album_id = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .albums[0]
        .id;
    let detail = state.services.album(owner, album_id).await.unwrap();
    assert_eq!(
        detail.album.record_labels,
        vec!["Nightfall Records", "Second Imprint"]
    );
    assert_eq!(detail.album.release_types, vec!["Album", "Compilation"]);
    assert_eq!(
        detail
            .album
            .disc_titles
            .iter()
            .map(|disc| (disc.disc, disc.title.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "The Session"), (2, "The Rehearsal")]
    );

    let album = subsonic_json(&router, "getAlbum", api_key, &format!("&id={album_id}")).await;
    let album = &album["subsonic-response"]["album"];
    assert_eq!(
        album["recordLabels"],
        serde_json::json!([{"name": "Nightfall Records"}, {"name": "Second Imprint"}])
    );
    assert_eq!(
        album["releaseTypes"],
        serde_json::json!(["Album", "Compilation"])
    );
    assert_eq!(
        album["discTitles"],
        serde_json::json!([
            {"disc": 1, "title": "The Session"},
            {"disc": 2, "title": "The Rehearsal"}
        ])
    );
    // A tag naming a year and a month is a year and a month. Reporting a day
    // it never claimed would be inventing precision.
    assert_eq!(
        album["originalReleaseDate"],
        serde_json::json!({"year": 1998, "month": 11})
    );
    assert_eq!(
        album["releaseDate"],
        serde_json::json!({"year": 2019, "month": 4, "day": 5})
    );

    // An album with none of these tags declares the fields supported and
    // unset — empty arrays rather than absent keys — and names no date at all.
    let bare_scan = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(bare_scan, 1, false).await.unwrap();
    let mut bare = catalog_input(7, "Quiet Hand");
    bare.album = Some("Bare release".into());
    bare.is_compilation = false;
    state
        .db
        .apply_catalog_track(library_id, bare_scan, &bare, None, false)
        .await
        .unwrap();
    // What a real scan does after applying its rows, before closing the job:
    // it builds the artist index and the role statistics a folder listing
    // reads. Driving the pipeline in another order would prove something about
    // an order nothing runs.
    state
        .db
        .consolidate_catalog_derivations(library_id)
        .await
        .unwrap();
    state.db.finish_scan_job(bare_scan, 0).await.unwrap();
    let bare_id = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .albums
        .into_iter()
        .find(|album| album.title == "Bare release")
        .unwrap()
        .id;
    // A song is not a release. The three arrays share their element name with
    // an album inside a directory, and a recording answering `recordLabels: []`
    // would claim the server reads a label off a track.
    let song = &album["song"][0];
    assert!(omits(song, "recordLabels"));
    assert!(omits(song, "releaseTypes"));
    assert!(omits(song, "discTitles"));
    // Its own arrays are still there, empty rather than absent.
    assert_eq!(song["moods"], serde_json::json!([]));

    let bare = subsonic_json(&router, "getAlbum", api_key, &format!("&id={bare_id}")).await;
    let bare = &bare["subsonic-response"]["album"];
    assert_eq!(bare["recordLabels"], serde_json::json!([]));
    assert_eq!(bare["releaseTypes"], serde_json::json!([]));
    assert_eq!(bare["discTitles"], serde_json::json!([]));
    assert!(omits(bare, "originalReleaseDate"));
    assert!(omits(bare, "releaseDate"));

    // A third album carrying exactly one of each. This is the shape the array
    // rule is for: with one child and no rule, a record label renders as a bare
    // object instead of a list of one.
    let solo_scan = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(solo_scan, 1, false).await.unwrap();
    let mut solo_input = catalog_input(9, "Solo Hand");
    solo_input.album = Some("Single imprint".into());
    solo_input.is_compilation = false;
    solo_input.disc_number = Some(1);
    solo_input.disc_subtitle = Some("Only disc".into());
    solo_input.release_types = Some("EP".into());
    solo_input.record_labels = Some("Solo Records".into());
    state
        .db
        .apply_catalog_track(library_id, solo_scan, &solo_input, None, false)
        .await
        .unwrap();
    // What a real scan does after applying its rows, before closing the job:
    // it builds the artist index and the role statistics a folder listing
    // reads. Driving the pipeline in another order would prove something about
    // an order nothing runs.
    state
        .db
        .consolidate_catalog_derivations(library_id)
        .await
        .unwrap();
    state.db.finish_scan_job(solo_scan, 0).await.unwrap();
    let solo_id = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .albums
        .into_iter()
        .find(|album| album.title == "Single imprint")
        .unwrap()
        .id;
    let solo = subsonic_json(&router, "getAlbum", api_key, &format!("&id={solo_id}")).await;
    let solo = &solo["subsonic-response"]["album"];
    assert_eq!(
        solo["recordLabels"],
        serde_json::json!([{"name": "Solo Records"}]),
        "one record label is a list of one, not a bare object"
    );
    assert_eq!(solo["releaseTypes"], serde_json::json!(["EP"]));
    assert_eq!(
        solo["discTitles"],
        serde_json::json!([{"disc": 1, "title": "Only disc"}])
    );

    // `getMusicDirectory` renames an album to `child`, and both the array rule
    // and the injection guard are keyed on that name. Under it an album has to
    // answer exactly what it answers as `album` — for several values, for one,
    // and for none — or a client browsing folders reads a single record label
    // as a bare object and an empty list as "not supported".
    for (id, rendered) in [(album_id, album), (solo_id, solo), (bare_id, bare)] {
        let artist_id: String =
            sqlx::query_scalar("SELECT album_artist_id FROM album WHERE id = ?")
                .bind(id.to_string())
                .fetch_one(state.db.pool())
                .await
                .unwrap();
        let directory = subsonic_json(
            &router,
            "getMusicDirectory",
            api_key,
            &format!("&id={artist_id}"),
        )
        .await;
        let children = directory["subsonic-response"]["directory"]["child"]
            .as_array()
            .unwrap()
            .clone();
        let child = children
            .iter()
            .find(|child| child["id"] == id.to_string())
            .unwrap_or_else(|| panic!("the folder of {artist_id} has to list {id}"));
        for key in ["recordLabels", "releaseTypes", "discTitles"] {
            assert_eq!(
                child[key], rendered[key],
                "{key} must read the same under `child` as under `album`"
            );
        }
        // And a song wearing that same element name carries none of the three.
        // Browsed from the album's own folder rather than the artist's: an
        // artist folder lists albums, so filtering it for songs would assert
        // nothing at all.
        let songs =
            subsonic_json(&router, "getMusicDirectory", api_key, &format!("&id={id}")).await;
        let songs = songs["subsonic-response"]["directory"]["child"]
            .as_array()
            .unwrap();
        assert!(!songs.is_empty(), "every album here has at least one track");
        for song in songs {
            assert_eq!(song["isDir"], false);
            for key in ["recordLabels", "releaseTypes", "discTitles"] {
                assert!(
                    omits(song, key),
                    "a song must not answer {key}: {}",
                    song["title"]
                );
            }
        }
    }
}
