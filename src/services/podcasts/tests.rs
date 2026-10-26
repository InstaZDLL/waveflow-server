use super::covers::{cover_plan, CoverPlan, KnownCover, COVER_TTL_MS};
use super::*;

#[test]
fn a_reader_is_shown_the_feeds_host_and_nothing_after_it() {
    assert_eq!(
        feed_origin("https://feeds.example.org/rss/secret-path.xml?token=secret#top"),
        "https://feeds.example.org/"
    );
    assert_eq!(
        feed_origin("http://user:pass@127.0.0.1:8080/feed/token/podcast.xml"),
        "http://127.0.0.1:8080/"
    );
    assert_eq!(feed_origin("not a url"), "");
}

#[test]
fn a_cover_is_fetched_when_it_is_new_changed_missing_or_stale() {
    let url = "https://cdn.example.org/cover.jpg";
    let now = 10 * COVER_TTL_MS;
    let held = KnownCover {
        hash: Some("a".repeat(64)),
        url: Some(url.to_owned()),
        fetched_at: Some(now - 1_000),
    };
    // Held, from this address, recently, with its file: nothing to do.
    assert_eq!(cover_plan(Some(url), &held, true, now), CoverPlan::Keep);
    // Nothing held yet, or a fetch that failed last time.
    assert_eq!(
        cover_plan(Some(url), &KnownCover::default(), false, now),
        CoverPlan::Fetch(url)
    );
    // The feed moved to another cover.
    let moved = "https://cdn.example.org/cover-2.jpg";
    assert_eq!(
        cover_plan(Some(moved), &held, true, now),
        CoverPlan::Fetch(moved)
    );
    // The file is gone from the store.
    assert_eq!(
        cover_plan(Some(url), &held, false, now),
        CoverPlan::Fetch(url)
    );
    // Older than the TTL.
    let stale = KnownCover {
        fetched_at: Some(now - COVER_TTL_MS),
        ..held
    };
    assert_eq!(
        cover_plan(Some(url), &stale, true, now),
        CoverPlan::Fetch(url)
    );
    // The feed names none any more: forgotten, and nothing to forget when
    // nothing was held.
    assert_eq!(cover_plan(None, &stale, true, now), CoverPlan::Clear);
    assert_eq!(
        cover_plan(None, &KnownCover::default(), false, now),
        CoverPlan::Keep
    );
}
