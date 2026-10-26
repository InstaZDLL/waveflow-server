//! The modern OpenSubsonic fields a media item carries, in both encodings.

use super::*;

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
