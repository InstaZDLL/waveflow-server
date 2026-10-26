use super::*;

/// 2026-10-25T10:00:00Z.
const NOW: i64 = 1_792_922_400_000;

#[test]
fn a_period_starts_at_the_listener_s_midnight() {
    let week = period_start(StatsPeriod::Week, NOW, 0).unwrap();
    assert_eq!(
        local_date(week, 0).format("%Y-%m-%d").to_string(),
        "2026-10-19"
    );
    assert_eq!(week % DAY_MS, 0);
    // Two hours ahead of UTC, the day began two hours earlier.
    let east = period_start(StatsPeriod::Week, NOW, 2 * 3_600_000).unwrap();
    assert_eq!(week - east, 2 * 3_600_000);
    let year = period_start(StatsPeriod::Year, NOW, 0).unwrap();
    assert_eq!(
        local_date(year, 0).format("%Y-%m-%d").to_string(),
        "2025-11-01"
    );
    assert_eq!(period_start(StatsPeriod::All, NOW, 0), None);
}

#[test]
fn days_are_named_and_folded_into_months() {
    // 2026-10-30, 2026-10-31 and 2026-11-01, as day numbers.
    let first = NaiveDate::from_ymd_opt(2026, 10, 30)
        .unwrap()
        .signed_duration_since(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
        .num_days();
    let days = [(first, 1, 10), (first + 1, 2, 20), (first + 2, 4, 40)];
    let daily = buckets_of(StatsBucketSize::Day, &days);
    assert_eq!(
        daily.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(),
        ["2026-10-30", "2026-10-31", "2026-11-01"]
    );
    let monthly = buckets_of(StatsBucketSize::Month, &days);
    assert_eq!(monthly.len(), 2);
    assert_eq!(monthly[0].label, "2026-10");
    assert_eq!((monthly[0].plays, monthly[0].listening_ms), (3, 30));
    assert_eq!((monthly[1].plays, monthly[1].listening_ms), (4, 40));
}

#[test]
fn a_bounded_timeline_has_every_bucket() {
    let filled = fill_timeline(
        StatsPeriod::Week,
        NOW,
        0,
        vec![StatsBucket {
            label: "2026-10-21".into(),
            plays: 3,
            listening_ms: 600_000,
        }],
    );
    assert_eq!(filled.len(), 7);
    assert_eq!(filled[0].label, "2026-10-19");
    assert_eq!(filled[6].label, "2026-10-25");
    assert_eq!(filled[2].plays, 3);
    assert_eq!(
        fill_timeline(StatsPeriod::Year, NOW, 0, Vec::new()).len(),
        12
    );
}

#[test]
fn a_query_outside_its_bounds_is_refused() {
    assert!(StatsQuery::new(Some("90d"), None, None).is_err());
    assert!(StatsQuery::new(None, Some(15 * 60), None).is_err());
    assert!(StatsQuery::new(None, None, Some(0)).is_err());
    assert!(StatsQuery::new(None, None, Some(51)).is_err());
    let default = StatsQuery::new(None, None, None).unwrap();
    assert_eq!(default.period, StatsPeriod::Month);
    assert_eq!(default.limit, 10);
}
