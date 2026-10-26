//! The credits page: its candidates, who reaches it, a conflicting second
//! decision, and the track categories it lists.

use super::*;

#[tokio::test]
async fn the_candidates_are_the_desktops_sorted_as_the_desktop_sorts_them() {
    let shelf = shelf("candidates").await;
    shelf
        .apply(&file(0, "Megan Thee Stallion, TWICE", Some("Mamushi")))
        .await;
    shelf.apply(&file(1, "TWICE", Some("Fancy"))).await;
    shelf
        .apply(&file(2, "Didier Super, The Aro String Band", Some("A")))
        .await;
    shelf
        .apply(&file(3, "Didier Super, The Aro String Band", Some("B")))
        .await;
    shelf
        .apply(&file(4, "Tyler, The Creator", Some("Igor")))
        .await;
    shelf
        .apply(&file(5, "Simon & Garfunkel", Some("Bookends")))
        .await;
    shelf
        .apply(&file(
            6,
            "Ian Asher, Olly Alexander (Years, Years)",
            Some("Sun"),
        ))
        .await;
    shelf.apply(&file(7, "Solo,", Some("Alone"))).await;
    shelf.settle().await;

    let candidates = shelf
        .state
        .services
        .credit_candidates(shelf.owner, shelf.library)
        .await
        .unwrap();
    let listed: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        listed,
        [
            "Megan Thee Stallion, TWICE",
            "Didier Super, The Aro String Band",
            "Ian Asher, Olly Alexander (Years, Years)",
            "Tyler, The Creator",
        ],
        "a known fragment first, then the most tracks, then the name"
    );
    let megan = &candidates[0];
    assert_eq!(megan.track_count, 1);
    assert_eq!(megan.fragments[1].name, "TWICE");
    assert_eq!(
        megan.fragments[1].artist_id,
        Some(shelf.artist("TWICE").await)
    );
    assert_eq!(megan.fragments[0].artist_id, None);

    let tyler = shelf.artist("Tyler, The Creator").await;
    shelf
        .state
        .services
        .decide_credit(shelf.owner, shelf.library, tyler, CreditDecisionKind::Keep)
        .await
        .unwrap();
    assert!(
        shelf
            .state
            .services
            .credit_candidates(shelf.owner, shelf.library)
            .await
            .unwrap()
            .iter()
            .all(|candidate| candidate.name != "Tyler, The Creator"),
        "a keep takes the credit off the list"
    );
    assert_eq!(
        shelf.credited(4, "artist").await,
        names(&["Tyler, The Creator"]),
        "and changes nothing it credits"
    );
}

#[tokio::test]
async fn only_owners_and_managers_reach_the_page_and_its_decisions() {
    let shelf = shelf("page-access").await;
    shelf
        .apply(&file(0, "Chris Lake, Abel Balder", Some("A")))
        .await;
    shelf.apply(&file(1, "Plain", Some("B"))).await;
    shelf.settle().await;
    let listener = account(&shelf.state, "listener").await;
    let manager = account(&shelf.state, "manager").await;
    for (user, role) in [
        (listener, LibraryRole::Listener),
        (manager, LibraryRole::Manager),
    ] {
        shelf
            .state
            .db
            .add_library_member(shelf.owner, shelf.library, user, role, now_ms())
            .await
            .unwrap();
    }
    let router = waveflow_server::app(&shelf.config, shelf.state.clone());
    let library = shelf.library;
    let send = |token: String, request: Request<Body>| {
        let router = router.clone();
        async move {
            let (mut parts, body) = request.into_parts();
            parts
                .headers
                .insert("authorization", format!("Bearer {token}").parse().unwrap());
            router
                .oneshot(Request::from_parts(parts, body))
                .await
                .unwrap()
        }
    };
    let listening = shelf.token(listener, "listener").await;
    let managing = shelf.token(manager, "manager").await;
    let joined = shelf.artist("Chris Lake, Abel Balder").await;
    let plain = shelf.artist("Plain").await;
    let get = |path: &str| {
        Request::get(format!("/api/v2/libraries/{library}{path}"))
            .body(Body::empty())
            .unwrap()
    };
    let decide = |artist: Uuid| {
        json_request(
            &format!("/api/v2/libraries/{library}/credit-decisions"),
            json!({ "artist_id": artist, "decision": "split" }),
        )
    };

    for path in [
        "/attention",
        "/attention/credits",
        "/attention/tracks?category=missing_year",
        "/credit-decisions",
    ] {
        assert_eq!(
            send(listening.clone(), get(path)).await.status(),
            StatusCode::NOT_FOUND,
            "a listener is told nothing at {path}"
        );
        assert_eq!(
            send(managing.clone(), get(path)).await.status(),
            StatusCode::OK,
            "a manager reads {path}"
        );
    }
    assert_eq!(
        send(listening.clone(), decide(joined)).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            managing.clone(),
            get("/attention/tracks?category=split_credits")
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "credits are not a track category"
    );
    assert_eq!(
        send(managing.clone(), decide(plain)).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "a name with nothing to split is not a candidate"
    );

    let created = send(managing.clone(), decide(joined)).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json_body(created).await;
    assert_eq!(created["decision"], "split");
    assert_eq!(created["parts"], json!(["Chris Lake", "Abel Balder"]));
    assert_eq!(created["decided_by"], "manager");

    // Decided, the joined artist is gone: deciding again names nothing.
    assert_eq!(
        send(managing.clone(), decide(joined)).await.status(),
        StatusCode::NOT_FOUND
    );
    let id = created["id"].as_str().unwrap().to_owned();
    let remove = || {
        Request::delete(format!("/api/v2/libraries/{library}/credit-decisions/{id}"))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        send(listening.clone(), remove()).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(managing.clone(), remove()).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(managing.clone(), remove()).await.status(),
        StatusCode::NOT_FOUND
    );

    // Undone, the credit can be decided again; a part of it is no candidate.
    assert_eq!(
        send(managing.clone(), decide(joined)).await.status(),
        StatusCode::CREATED
    );
    let keep = json_request(
        &format!("/api/v2/libraries/{library}/credit-decisions"),
        json!({ "artist_id": shelf.artist("Chris Lake").await, "decision": "keep" }),
    );
    assert_eq!(
        send(managing.clone(), keep).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn a_second_decision_on_one_credit_is_a_conflict() {
    let shelf = shelf("conflict").await;
    shelf.apply(&file(0, "Jonas Blue, Malive", Some("A"))).await;
    shelf.settle().await;
    let joined = shelf.artist("Jonas Blue, Malive").await;
    let services = &shelf.state.services;
    services
        .decide_credit(shelf.owner, shelf.library, joined, CreditDecisionKind::Keep)
        .await
        .unwrap();
    let again = services
        .decide_credit(
            shelf.owner,
            shelf.library,
            joined,
            CreditDecisionKind::Split,
        )
        .await;
    // Asserted on the variant alone: printing the result would put the
    // decision's identifiers in the failure message for nothing.
    assert!(
        matches!(
            again,
            Err(waveflow_server::services::ServiceError::Conflict)
        ),
        "a second decision on one credit is a conflict"
    );
}

#[tokio::test]
async fn each_track_category_lists_what_its_criterion_names() {
    let shelf = shelf("categories").await;
    let mut tracks = Vec::new();
    // A gap on disc one: 1, 2, 4.
    for (index, number) in [(0, 1), (1, 2), (2, 4)] {
        let mut input = file(index, "Gap Band", Some("Gaps"));
        input.track_number = Some(number);
        tracks.push(input);
    }
    // Two tracks numbered 1 on the same disc.
    for index in [3, 4] {
        let mut input = file(index, "Dup Band", Some("Dups"));
        input.track_number = Some(1);
        tracks.push(input);
    }
    // Two years on one album.
    for (index, year) in [(5, 2001), (6, 2002)] {
        let mut input = file(index, "Year Band", Some("Years"));
        input.year = Some(year);
        tracks.push(input);
    }
    // No year, no number, no album.
    let mut bare = file(7, "Bare Band", None);
    bare.year = None;
    bare.track_number = None;
    tracks.push(bare);
    // The same recording twice, a second apart.
    for (index, album, duration) in [(8, "First", 180_000), (9, "Second", 181_000)] {
        let mut input = file(index, "Echo Band", Some(album));
        input.title = "Echo".into();
        input.duration_ms = duration;
        tracks.push(input);
    }
    // A compilation that names no album artist.
    let mut various = file(10, "Some Band", Some("Hits"));
    various.is_compilation = true;
    tracks.push(various);
    for input in &tracks {
        shelf.apply(input).await;
    }
    shelf.settle().await;

    let listed = |category: AttentionCategory| {
        let shelf = &shelf;
        async move {
            let mut paths: Vec<String> = shelf
                .state
                .services
                .attention_tracks(
                    shelf.owner,
                    shelf.library,
                    category,
                    BrowsePage::new(None, None).unwrap(),
                )
                .await
                .unwrap()
                .into_iter()
                .map(|track| track.song.relative_path)
                .collect();
            paths.sort();
            paths
        }
    };
    let paths = |indices: &[usize]| -> Vec<String> {
        let mut paths: Vec<String> = indices
            .iter()
            .map(|index| format!("track-{index}.flac"))
            .collect();
        paths.sort();
        paths
    };
    assert_eq!(
        listed(AttentionCategory::TrackNumberGap).await,
        paths(&[0, 1, 2])
    );
    assert_eq!(
        listed(AttentionCategory::DuplicateTrackNumber).await,
        paths(&[3, 4])
    );
    assert_eq!(
        listed(AttentionCategory::AlbumYearConflict).await,
        paths(&[5, 6])
    );
    assert_eq!(listed(AttentionCategory::MissingYear).await, paths(&[7]));
    assert_eq!(
        listed(AttentionCategory::MissingTrackNumber).await,
        paths(&[7])
    );
    assert_eq!(listed(AttentionCategory::MissingAlbum).await, paths(&[7]));
    assert_eq!(
        listed(AttentionCategory::ProbableDuplicate).await,
        paths(&[8, 9])
    );
    assert_eq!(
        listed(AttentionCategory::CompilationNoAlbumArtist).await,
        paths(&[10])
    );
    assert!(listed(AttentionCategory::MissingArtist).await.is_empty());
    assert_eq!(
        listed(AttentionCategory::MissingCover).await.len(),
        tracks.len(),
        "no file here carries a cover"
    );

    // A correction is an effective value: the corrected track leaves its
    // category at once, with no rescan.
    let bare: String =
        sqlx::query_scalar("SELECT id FROM track WHERE library_id = ? AND relative_path = ?")
            .bind(shelf.library.to_string())
            .bind("track-7.flac")
            .fetch_one(shelf.state.db.pool())
            .await
            .unwrap();
    shelf
        .state
        .services
        .set_track_metadata(
            shelf.owner,
            Uuid::parse_str(&bare).unwrap(),
            TrackMetadataPatch {
                year: Some(Some(1999)),
                ..TrackMetadataPatch::default()
            },
            None,
        )
        .await
        .unwrap();
    assert!(listed(AttentionCategory::MissingYear).await.is_empty());

    let summary = shelf
        .state
        .services
        .attention_summary(shelf.owner, shelf.library)
        .await
        .unwrap();
    let count = |category: AttentionCategory| {
        summary
            .iter()
            .find(|entry| entry.category == category)
            .map(|entry| entry.count)
    };
    assert_eq!(summary.len(), AttentionCategory::ALL.len());
    assert_eq!(summary[0].category, AttentionCategory::SplitCredits);
    assert_eq!(count(AttentionCategory::TrackNumberGap), Some(3));
    assert_eq!(count(AttentionCategory::ProbableDuplicate), Some(2));
    assert_eq!(count(AttentionCategory::MissingYear), Some(0));

    let duplicates = shelf
        .state
        .services
        .attention_tracks(
            shelf.owner,
            shelf.library,
            AttentionCategory::ProbableDuplicate,
            BrowsePage::new(None, None).unwrap(),
        )
        .await
        .unwrap();
    assert!(
        duplicates
            .iter()
            .all(|track| track.duplicate_group == Some(1)),
        "both are one group"
    );
}
