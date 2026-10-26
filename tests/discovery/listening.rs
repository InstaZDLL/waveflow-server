//! What the caller's listens make: the statistics and the mixes.

use super::*;

// ── Statistics: decision 6 ──

/// Two accounts, each with a library; the first also reaches the second's
/// for a while, then leaves it.
#[tokio::test]
async fn statistics_count_the_caller_s_listens_on_tracks_they_still_reach() {
    let (_temp, config, state) = support::test_app().await;
    let me = account(&state, "stats-me").await;
    let other = account(&state, "stats-other").await;
    let (_, mine) = library(
        &state,
        &config,
        me,
        "stats-mine",
        &[
            file("One", "Björk", Some("Pop"), Some(1995)),
            file("Two", "Björk", Some("Pop; Electronic"), Some(1997)),
            file("Three", "Sigur Rós", Some("Post-Rock"), Some(2002)),
        ],
    )
    .await;
    let (theirs_id, theirs) = library(
        &state,
        &config,
        other,
        "stats-theirs",
        &[file("Elsewhere", "Björk", Some("Pop"), Some(2001))],
    )
    .await;
    // Joined for a while: my listen there counts while I am a member.
    sqlx::query(
        "INSERT INTO library_member (library_id, user_id, role, created_at) \
         VALUES (?, ?, 'listener', 0)",
    )
    .bind(theirs_id.to_string())
    .bind(me.to_string())
    .execute(state.db.pool())
    .await
    .unwrap();
    listen(&state, me, theirs[0], 3 * MINUTE).await;
    for ago in [MINUTE, 2 * DAY, 40 * DAY] {
        listen(&state, me, mine[1], ago).await;
    }
    listen(&state, me, mine[0], 5 * DAY).await;
    listen(&state, me, mine[2], 6 * DAY).await;
    // The other account's listens are never mine.
    for _ in 0..5 {
        listen(&state, other, theirs[0], MINUTE).await;
    }
    // A now playing is not a listen.
    state
        .services
        .scrobble(me, mine[2], false, None)
        .await
        .unwrap();

    let query = |period| waveflow_server::services::StatsQuery::new(Some(period), None, None);
    let week = state
        .services
        .listening_stats(me, query("7d").unwrap())
        .await
        .unwrap();
    assert_eq!(week.totals.plays, 5);
    assert_eq!(week.totals.listening_ms, 5 * 234_000);
    assert_eq!(week.totals.tracks, 4);
    // Björk in two libraries is one artist.
    assert_eq!(week.totals.artists, 2);
    assert_eq!(week.top_tracks[0].song.title, "Two");
    assert_eq!(week.top_tracks[0].plays, 2);
    assert_eq!(week.top_artists[0].artist.name, "Björk");
    assert_eq!(week.top_artists[0].plays, 4);
    assert_eq!(week.top_genres[0].name, "Pop");
    assert_eq!(week.top_genres[0].plays, 4);
    assert_eq!(week.timeline.len(), 7);
    assert_eq!(
        week.timeline.iter().map(|bucket| bucket.plays).sum::<i64>(),
        5
    );
    assert_eq!(week.first_play.as_ref().unwrap().song.title, "Three");
    assert_eq!(week.last_play.as_ref().unwrap().song.title, "Two");
    assert!(week.top_albums.iter().all(|entry| entry.plays > 0));

    let month = state
        .services
        .listening_stats(me, query("30d").unwrap())
        .await
        .unwrap();
    assert_eq!(month.totals.plays, 5);
    assert_eq!(month.timeline.len(), 30);
    let all = state
        .services
        .listening_stats(me, query("all").unwrap())
        .await
        .unwrap();
    assert_eq!(all.totals.plays, 6);
    assert!(all.from.is_none());
    let year = state
        .services
        .listening_stats(me, query("365d").unwrap())
        .await
        .unwrap();
    assert_eq!(year.timeline.len(), 12);

    // Leaving the library takes its listens out, as it takes the track out
    // of every other answer.
    sqlx::query("DELETE FROM library_member WHERE library_id=? AND user_id=?")
        .bind(theirs_id.to_string())
        .bind(me.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let after = state
        .services
        .listening_stats(me, query("7d").unwrap())
        .await
        .unwrap();
    assert_eq!(after.totals.plays, 4);
    assert!(after
        .top_tracks
        .iter()
        .all(|entry| entry.song.title != "Elsewhere"));

    // And the other account sees only its own five.
    let theirs_stats = state
        .services
        .listening_stats(other, query("7d").unwrap())
        .await
        .unwrap();
    assert_eq!(theirs_stats.totals.plays, 5);
}

/// The route: its parameters are validated and it needs a credential.
#[tokio::test]
async fn the_statistics_route_validates_its_parameters() {
    let (_temp, config, state) = support::test_app().await;
    let me = account(&state, "stats-route").await;
    let router = waveflow_server::app(&config, state.clone());
    let bearer = token(&state, me, "stats-route").await;

    let (status, body) = get_json(
        &router,
        "/api/v2/stats?period=7d&utc_offset_minutes=120",
        &bearer,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["period"], "7d");
    assert_eq!(body["bucket"], "day");
    assert_eq!(body["totals"]["plays"], 0);
    assert_eq!(body["timeline"].as_array().unwrap().len(), 7);
    for bad in [
        "period=90d",
        "limit=0",
        "limit=51",
        "utc_offset_minutes=900",
    ] {
        let (status, _) = get_json(&router, &format!("/api/v2/stats?{bad}"), &bearer).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    let anonymous = support::send_json(&router, "GET", "/api/v2/stats", "not-a-token", None).await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}

// ── Mixes: decision 7 ──

#[tokio::test]
async fn mixes_are_listed_and_an_empty_one_is_left_out() {
    let (_temp, config, state) = support::test_app().await;
    let me = account(&state, "mix-list").await;
    let files: Vec<File> = (0..12)
        .map(|n| {
            file(
                &format!("Song {n}"),
                &format!("Artist {n}"),
                Some("Jazz"),
                Some(1960),
            )
        })
        .collect();
    library(&state, &config, me, "mix-list", &files).await;
    let query = waveflow_server::services::MixQuery::new(Vec::new(), None, None).unwrap();

    let mixes = state.services.list_mixes(me, &query).await.unwrap();
    let ids: Vec<&str> = mixes.iter().map(|mix| mix.id.as_str()).collect();
    // Nothing played and nothing starred: those two are not offered.
    assert_eq!(
        ids,
        [
            "recently-added",
            "never-played",
            "genre:jazz",
            "decade:1960"
        ],
        "{ids:?}"
    );
    assert_eq!(mixes[2].genre.as_deref(), Some("Jazz"));
    assert_eq!(mixes[3].decade, Some(1960));
    assert!(mixes.iter().all(|mix| mix.track_count == 12));
    assert!(matches!(
        state.services.mix(me, "most-played", &query).await,
        Err(ServiceError::NotFound)
    ));
    assert!(matches!(
        state.services.mix(me, "no-such-mix", &query).await,
        Err(ServiceError::NotFound)
    ));
}

/// Variety and determinism: no artist holds more than its share, two asks
/// on one day answer one mix, and a listened track leaves "never played".
#[tokio::test]
async fn a_mix_is_varied_and_the_same_all_day() {
    let (_temp, config, state) = support::test_app().await;
    let me = account(&state, "mix-vary").await;
    let mut files: Vec<File> = (0..30)
        .map(|n| {
            file(
                &format!("Prolific {n}"),
                "Prolific",
                Some("Rock"),
                Some(1990),
            )
        })
        .collect();
    files.extend((0..10).map(|n| {
        file(
            &format!("Other {n}"),
            &format!("Other {n}"),
            Some("Rock"),
            Some(1990),
        )
    }));
    let (_, ids) = library(&state, &config, me, "mix-vary", &files).await;
    let query = waveflow_server::services::MixQuery::new(Vec::new(), Some(20), None).unwrap();

    let first = state
        .services
        .mix(me, "never-played", &query)
        .await
        .unwrap();
    let again = state
        .services
        .mix(me, "never-played", &query)
        .await
        .unwrap();
    let order = |mix: &waveflow_server::services::Mix| {
        mix.songs.iter().map(|song| song.id).collect::<Vec<_>>()
    };
    assert_eq!(order(&first), order(&again));
    let prolific = first
        .songs
        .iter()
        .filter(|song| song.artist.as_deref() == Some("Prolific"))
        .count();
    assert_eq!(prolific, 2, "a tenth of twenty, and never fewer than two");
    assert_eq!(first.songs.len(), 12);
    for pair in first.songs.windows(2) {
        assert_ne!(pair[0].artist, pair[1].artist);
    }

    // Most played is a ranking: twice before once, whatever came last.
    listen(&state, me, ids[31], 3 * MINUTE).await;
    listen(&state, me, ids[31], 2 * MINUTE).await;
    listen(&state, me, ids[30], MINUTE).await;
    let played = state.services.mix(me, "most-played", &query).await.unwrap();
    assert_eq!(order(&played), [ids[31], ids[30]]);
    let never = state
        .services
        .mix(me, "never-played", &query)
        .await
        .unwrap();
    assert!(!order(&never).contains(&ids[30]));
    assert!(!order(&never).contains(&ids[31]));
}

/// A favourite is forgotten when starred long ago and not listened to
/// lately; a fresh star and a recent listen both keep it out.
#[tokio::test]
async fn forgotten_favourites_are_old_stars_not_listened_to_lately() {
    let (_temp, config, state) = support::test_app().await;
    let me = account(&state, "mix-forgotten").await;
    let files: Vec<File> = (0..3)
        .map(|n| file(&format!("Star {n}"), &format!("Starred {n}"), None, None))
        .collect();
    let (_, ids) = library(&state, &config, me, "mix-forgotten", &files).await;
    let now = waveflow_server::auth::authentication::now_ms();
    for (track, starred_ago) in [(ids[0], 60 * DAY), (ids[1], DAY), (ids[2], 60 * DAY)] {
        sqlx::query(
            "INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) \
             VALUES (?, 'track', ?, ?)",
        )
        .bind(me.to_string())
        .bind(track.to_string())
        .bind(now - starred_ago)
        .execute(state.db.pool())
        .await
        .unwrap();
    }
    listen(&state, me, ids[2], 10 * DAY).await;
    let query = waveflow_server::services::MixQuery::new(Vec::new(), None, None).unwrap();
    let mix = state
        .services
        .mix(me, "forgotten-favourites", &query)
        .await
        .unwrap();
    assert_eq!(
        mix.songs.iter().map(|song| song.id).collect::<Vec<_>>(),
        [ids[0]]
    );
}

/// Tenancy: another account's tracks are never in a mix, and a genre only
/// another account holds is not found.
#[tokio::test]
async fn a_mix_holds_only_the_caller_s_tracks() {
    let (_temp, config, state) = support::test_app().await;
    let me = account(&state, "mix-me").await;
    let other = account(&state, "mix-other").await;
    let (_, mine) = library(
        &state,
        &config,
        me,
        "mix-me",
        &[file("Mine", "Me", Some("Folk"), Some(1970))],
    )
    .await;
    library(
        &state,
        &config,
        other,
        "mix-other",
        &[file("Theirs", "Them", Some("Grime"), Some(2005))],
    )
    .await;
    let router = waveflow_server::app(&config, state.clone());
    let bearer = token(&state, me, "mix-me").await;

    let (status, body) = get_json(&router, "/api/v2/mixes/never-played", &bearer).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(support::song_ids(&body["songs"]), [mine[0].to_string()]);
    let (status, _) = get_json(&router, "/api/v2/mixes/genre%3Agrime", &bearer).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get_json(&router, "/api/v2/mixes/decade%3A2000", &bearer).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = get_json(&router, "/api/v2/mixes", &bearer).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body
        .as_array()
        .unwrap()
        .iter()
        .all(|mix| mix["id"] != "genre:grime"));
    let (status, _) = get_json(&router, "/api/v2/mixes?limit=101", &bearer).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}
