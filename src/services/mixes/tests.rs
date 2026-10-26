use super::*;

fn candidate(artist: &str) -> Candidate {
    Candidate {
        id: Uuid::new_v4(),
        artist: artist.to_owned(),
        artwork_hash: None,
    }
}

#[test]
fn no_artist_dominates_and_none_repeats_back_to_back() {
    let mut pool: Vec<Candidate> = (0..30).map(|_| candidate("prolific")).collect();
    pool.extend((0..5).map(|index| candidate(&format!("other-{index}"))));
    let mixed = vary(pool, 20);
    let prolific = mixed.iter().filter(|c| c.artist == "prolific").count();
    // A tenth of twenty is two.
    assert_eq!(prolific, 2);
    assert_eq!(mixed.len(), 7);
    for pair in mixed.windows(2) {
        assert_ne!(pair[0].artist, pair[1].artist);
    }
}

#[test]
fn the_same_day_draws_the_same_mix_and_the_next_day_another() {
    let user = Uuid::new_v4();
    let ids: Vec<Uuid> = (0..50).map(|_| Uuid::new_v4()).collect();
    let order = |day| {
        let seed = seed_of(user, &MixKind::NeverPlayed, day);
        let mut ids = ids.clone();
        ids.sort_by_cached_key(|id| draw_rank(&seed, *id));
        ids
    };
    assert_eq!(order(20_000), order(20_000));
    assert_ne!(order(20_000), order(20_001));
    assert_ne!(
        seed_of(user, &MixKind::NeverPlayed, 1),
        seed_of(Uuid::new_v4(), &MixKind::NeverPlayed, 1)
    );
}

#[test]
fn a_mix_id_reads_back_to_itself() {
    for id in [
        "most-played",
        "never-played",
        "forgotten-favourites",
        "recently-added",
        "decade:1990",
    ] {
        assert_eq!(MixKind::parse(id).unwrap().id(), id);
    }
    assert_eq!(
        MixKind::parse("genre:Hip-Hop").unwrap().id(),
        MixKind::parse("genre:hip hop").unwrap().id()
    );
    assert!(MixKind::parse("decade:1995").is_none());
    assert!(MixKind::parse("genre:").is_none());
    assert!(MixKind::parse("top-secret").is_none());
}
