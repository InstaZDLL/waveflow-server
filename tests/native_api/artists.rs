//! Artist pages and the images chosen for an artist.

use super::*;

/// Artist images chosen by hand (RFC-013 step 2), through the router: a chosen
/// portrait leads every reader and outlives the provider's fill, a banner
/// rides on the artist alone, the sweep keeps both, and only owners and
/// managers choose. The images are bytes with the right magic numbers: the
/// server recognises a format from the bytes and never decodes an image.
/// Passwords are drawn at run time, for CodeQL.
#[tokio::test]
async fn chosen_artist_images_lead_every_reader_and_outlive_the_portrait_fill() {
    let (_temp, config, state) = test_app().await;
    let password = uuid::Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let mut accounts = Vec::new();
    for name in ["picture-owner", "picture-listener", "picture-stranger"] {
        accounts.push(
            state
                .db
                .create_account(name, &hash, AccountRole::User, now_ms())
                .await
                .unwrap(),
        );
    }
    let (owner, listener) = (accounts[0], accounts[1]);
    let root = std::fs::canonicalize(&config.data_dir).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Pictures",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    state
        .db
        .add_library_member(
            owner,
            library_id,
            listener,
            waveflow_server::database::LibraryRole::Listener,
            now_ms(),
        )
        .await
        .unwrap();
    let scan_id = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan_id, 1, false).await.unwrap();
    let mut input = browse_input(50_000, "Hyperballad", "Post", "Björk", Some(1), Some(1));
    input.relative_path = "Björk/Post/01.flac".into();
    state
        .db
        .apply_catalog_track(library_id, scan_id, &input, None, false)
        .await
        .unwrap();
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    // What RFC-011's fill does: an artwork row from a provider, linked on the
    // artist's own column.
    let provider_portrait = |seed: &str| format!("{:0>64}", seed);
    let link_provider_portrait = |hash: String| {
        let pool = state.db.pool().clone();
        async move {
            sqlx::query(
                "INSERT INTO artwork (hash, format, source, byte_size, created_at) \
                 VALUES (?, 'jpg', 'deezer', 1, 0) ON CONFLICT (hash) DO NOTHING",
            )
            .bind(&hash)
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("UPDATE artist SET artwork_hash = ? WHERE name = 'Björk'")
                .bind(&hash)
                .execute(&pool)
                .await
                .unwrap();
        }
    };
    let deezer = provider_portrait("d1");
    link_provider_portrait(deezer.clone()).await;

    let router = waveflow_server::app(&config, state.clone());
    let owner_token = login_token(&router, "picture-owner", &password).await;
    let listener_token = login_token(&router, "picture-listener", &password).await;
    let stranger_token = login_token(&router, "picture-stranger", &password).await;
    let send = |method: Method, uri: String, token: &str, body: Vec<u8>| {
        let router = router.clone();
        let token = token.to_owned();
        async move {
            router
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };
    let artists =
        json_body(send(Method::GET, "/api/v2/artists".into(), &owner_token, vec![]).await).await;
    let artist = artists
        .as_array()
        .unwrap()
        .iter()
        .find(|artist| artist["name"] == "Björk")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let artist_uri = format!("/api/v2/artists/{artist}");
    let image_uri = |kind: &str| format!("{artist_uri}/image/{kind}");
    let png = |seed: u8| {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend(std::iter::repeat_n(seed, 64));
        bytes
    };
    let jpeg = |seed: u8| {
        let mut bytes = vec![0xff, 0xd8, 0xff];
        bytes.extend(std::iter::repeat_n(seed, 64));
        bytes
    };
    let hash_of = |bytes: &[u8]| blake3::hash(bytes).to_hex().to_string();
    let events = || {
        let pool = state.db.pool().clone();
        let artist = artist.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM library_event WHERE entity_type = 'artist' AND entity_id = ?",
            )
            .bind(artist)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };

    // Before any choice: the provider's portrait, no banner, nothing chosen.
    let before = json_body(send(Method::GET, artist_uri.clone(), &owner_token, vec![]).await).await;
    assert_eq!(before["artwork_hash"], deezer.as_str());
    assert!(omits(&before, "banner_hash"));
    assert!(omits(&before, "image_choice"));

    // A chosen portrait leads, says it was chosen, and tells clients so.
    let events_before = events().await;
    let portrait = png(1);
    let chosen = send(
        Method::PUT,
        image_uri("portrait"),
        &owner_token,
        portrait.clone(),
    )
    .await;
    assert_eq!(chosen.status(), StatusCode::OK);
    let chosen = json_body(chosen).await;
    assert_eq!(chosen["artwork_hash"], hash_of(&portrait).as_str());
    assert_eq!(chosen["image_choice"]["portrait"]["source"], "upload");
    assert!(omits(&chosen["image_choice"], "banner"));
    assert_eq!(events().await, events_before + 1);

    // A banner rides on the artist alone; the portrait stays.
    let banner = jpeg(2);
    let with_banner = json_body(
        send(
            Method::PUT,
            image_uri("banner"),
            &owner_token,
            banner.clone(),
        )
        .await,
    )
    .await;
    assert_eq!(with_banner["banner_hash"], hash_of(&banner).as_str());
    assert_eq!(with_banner["artwork_hash"], hash_of(&portrait).as_str());

    // Every reader: the listing, the artist the Subsonic façade renders from,
    // and the artwork route by hash and by the artist's id.
    let listed = json_body(
        send(
            Method::GET,
            "/api/v2/artists".into(),
            &listener_token,
            vec![],
        )
        .await,
    )
    .await;
    let listed = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "Björk")
        .unwrap()
        .clone();
    assert_eq!(listed["artwork_hash"], hash_of(&portrait).as_str());
    let artist_id = uuid::Uuid::parse_str(&artist).unwrap();
    let detail = state.services.artist(owner, artist_id).await.unwrap();
    assert_eq!(
        detail.artist.artwork_hash.as_deref(),
        Some(hash_of(&portrait).as_str())
    );
    for (id, expected) in [(hash_of(&banner), &banner), (artist.clone(), &portrait)] {
        let served = send(
            Method::GET,
            format!("/api/v2/artwork/{id}"),
            &listener_token,
            vec![],
        )
        .await;
        assert_eq!(served.status(), StatusCode::OK, "{id}");
        let bytes = axum::body::to_bytes(served.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&bytes[..], &expected[..], "{id}");
    }
    assert_eq!(
        send(
            Method::GET,
            format!("/api/v2/artwork/{}", hash_of(&banner)),
            &stranger_token,
            vec![]
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );

    // The provider's fill rewrites the artist's own column; the choice holds.
    let refreshed = provider_portrait("d2");
    link_provider_portrait(refreshed.clone()).await;
    let after_fill =
        json_body(send(Method::GET, artist_uri.clone(), &owner_token, vec![]).await).await;
    assert_eq!(after_fill["artwork_hash"], hash_of(&portrait).as_str());

    // The sweep keeps what a choice names, row and file.
    state.services.sweep_artwork_store().await.unwrap();
    for image in [&portrait, &banner] {
        let named: Option<String> = sqlx::query_scalar("SELECT hash FROM artwork WHERE hash = ?")
            .bind(hash_of(image))
            .fetch_optional(state.db.pool())
            .await
            .unwrap();
        assert!(named.is_some());
    }
    let served = send(
        Method::GET,
        format!("/api/v2/artwork/{}", hash_of(&banner)),
        &owner_token,
        vec![],
    )
    .await;
    assert_eq!(served.status(), StatusCode::OK);

    // Only owners and managers choose; everyone else is answered as a stranger.
    for token in [&listener_token, &stranger_token] {
        assert_eq!(
            send(Method::PUT, image_uri("portrait"), token, png(3))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(Method::DELETE, image_uri("banner"), token, vec![])
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        send(Method::GET, artist_uri.clone(), &stranger_token, vec![])
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // A refused upload leaves nothing in the store: access is asked before the
    // file is written, not only under the gate after it.
    assert!(!config
        .artwork_dir
        .join(format!("{}.png", hash_of(&png(3))))
        .exists());
    // Not an image, too large, or no such kind.
    assert_eq!(
        send(
            Method::PUT,
            image_uri("portrait"),
            &owner_token,
            b"not an image".to_vec()
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut huge = png(4);
    huge.resize(10 * 1024 * 1024 + 1, 0);
    assert_eq!(
        send(Method::PUT, image_uri("portrait"), &owner_token, huge)
            .await
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        send(Method::PUT, image_uri("cover"), &owner_token, png(5))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // Exactly at the cap is still an image.
    let mut largest = png(6);
    largest.resize(10 * 1024 * 1024, 0);
    assert_eq!(
        send(Method::PUT, image_uri("banner"), &owner_token, largest)
            .await
            .status(),
        StatusCode::OK
    );

    // Back to automatic: the provider's latest portrait, and no banner.
    let events_before = events().await;
    for kind in ["portrait", "banner"] {
        assert_eq!(
            send(Method::DELETE, image_uri(kind), &owner_token, vec![])
                .await
                .status(),
            StatusCode::NO_CONTENT,
            "{kind}"
        );
    }
    assert_eq!(events().await, events_before + 2);
    let automatic =
        json_body(send(Method::GET, artist_uri.clone(), &owner_token, vec![]).await).await;
    assert_eq!(automatic["artwork_hash"], refreshed.as_str());
    assert!(omits(&automatic, "banner_hash"));
    assert!(omits(&automatic, "image_choice"));
    assert_eq!(
        send(Method::DELETE, image_uri("portrait"), &owner_token, vec![])
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

// An artist page groups the discography by release type and lists what the
/// artist appears on (RFC-022 decisions 2 and 3).
///
/// The fixture holds one album per group, one album with no type at all, and
/// two albums by someone else: one with a guest track by the artist, which is
/// an appearance, and one crediting the artist only as composer, which is not.
/// An album the artist is album artist of credits them on every track as
/// well, so those six are the case that proves the exclusion.
#[tokio::test]
async fn an_artist_page_groups_its_discography_and_lists_its_appearances() {
    let (_temp, config, state) = test_app().await;
    // Drawn at run time: a literal reaching the hash is a hard-coded
    // credential to CodeQL.
    let password = uuid::Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("discography-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("discography-outsider", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let root = config.data_dir.join("discography");
    std::fs::create_dir_all(&root).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Discography",
            &std::fs::canonicalize(&root).unwrap(),
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
    // (title, album, album artist, track artist, release types, year)
    let fixture = [
        (
            "One",
            "Long Player",
            "Disco Artist",
            "Disco Artist",
            Some("album"),
            2001,
        ),
        (
            "Two",
            "Short Player",
            "Disco Artist",
            "Disco Artist",
            Some("EP"),
            2002,
        ),
        (
            "Three",
            "Lone Song",
            "Disco Artist",
            "Disco Artist",
            Some("single"),
            2003,
        ),
        (
            "Four",
            "On Stage",
            "Disco Artist",
            "Disco Artist",
            Some("album/live"),
            2004,
        ),
        (
            "Five",
            "Best Of",
            "Disco Artist",
            "Disco Artist",
            Some("album; compilation"),
            2005,
        ),
        ("Six", "Untyped", "Disco Artist", "Disco Artist", None, 2006),
        (
            "Seven",
            "Guest Spot",
            "Host Artist",
            "Host Artist; Disco Artist",
            Some("album"),
            2010,
        ),
        (
            "Eight",
            "Guest Spot",
            "Host Artist",
            "Host Artist",
            Some("album"),
            2010,
        ),
        (
            "Nine",
            "Written For",
            "Host Artist",
            "Host Artist",
            Some("album"),
            2011,
        ),
    ];
    state
        .db
        .start_scan_job(scan_id, fixture.len() as i64, false)
        .await
        .unwrap();
    for (index, (title, album, album_artist, artist, types, year)) in
        fixture.into_iter().enumerate()
    {
        let mut input = browse_input(index + 3_000, title, album, album_artist, Some(1), Some(1));
        input.artist = Some(artist.into());
        input.release_types = types.map(str::to_owned);
        input.year = Some(year);
        input.track_number = Some(index as i64 + 1);
        if title == "Nine" {
            input.roles = vec![(
                waveflow_server::tags::Role::Composer,
                vec!["Disco Artist".into()],
            )];
        }
        state
            .db
            .apply_catalog_track(library_id, scan_id, &input, None, false)
            .await
            .unwrap();
    }
    state
        .db
        .consolidate_catalog_derivations(library_id)
        .await
        .unwrap();
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    let router = waveflow_server::app(&config, state);
    let owner_token = login_token(&router, "discography-owner", &password).await;
    let outsider_token = login_token(&router, "discography-outsider", &password).await;
    let get = |uri: String, token: String| {
        let router = router.clone();
        async move {
            router
                .oneshot(
                    Request::get(uri)
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };
    let artists = json_body(get("/api/v2/artists".into(), owner_token.clone()).await).await;
    let artist_id = artists
        .as_array()
        .unwrap()
        .iter()
        .find(|artist| artist["name"] == "Disco Artist")
        .expect("the artist is listed")["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let page = get(format!("/api/v2/artists/{artist_id}"), owner_token.clone()).await;
    assert_eq!(page.status(), StatusCode::OK);
    let page = json_body(page).await;
    let groups = page["albums"]
        .as_array()
        .unwrap()
        .iter()
        .map(|album| {
            (
                album["title"].as_str().unwrap().to_owned(),
                album["release_group"].as_str().unwrap().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        groups,
        [
            ("Long Player", "album"),
            ("Short Player", "ep"),
            ("Lone Song", "single"),
            ("On Stage", "live"),
            ("Best Of", "compilation"),
            ("Untyped", "album"),
        ]
        .map(|(title, group)| (title.to_owned(), group.to_owned()))
    );
    // What the files said is still carried beside the decision about it.
    assert_eq!(
        page["albums"][3]["release_types"],
        serde_json::json!(["album/live"])
    );

    // The guest track's album, and only it: not the albums whose every track
    // credits the artist, and not the one that credits them as composer.
    let appears_on = page["appears_on"]
        .as_array()
        .expect("the native page lists appearances");
    assert_eq!(appears_on.len(), 1, "{appears_on:?}");
    assert_eq!(appears_on[0]["title"], "Guest Spot");
    assert_eq!(appears_on[0]["artist"], "Host Artist");

    // The album-artist list on its own is unchanged: appearances are the
    // page's, not the artist's albums.
    let albums_of = json_body(
        get(
            format!("/api/v2/artists/{artist_id}/albums"),
            owner_token.clone(),
        )
        .await,
    )
    .await;
    assert_eq!(albums_of, page["albums"]);

    // Someone outside the library is told nothing, appearances included.
    let hidden = get(format!("/api/v2/artists/{artist_id}"), outsider_token).await;
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
}
