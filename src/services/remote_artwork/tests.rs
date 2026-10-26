use super::*;

#[test]
fn an_account_spends_its_hourly_budget_and_gets_it_back_an_hour_later() {
    let budget = FetchBudget::default();
    let (one, other) = (Uuid::new_v4(), Uuid::new_v4());
    for _ in 0..STATION_IMAGE_FETCHES_PER_HOUR {
        assert!(budget.take(one, 1_000));
    }
    assert!(!budget.take(one, 1_000 + BUDGET_WINDOW_MS - 1));
    // Another account's budget is its own.
    assert!(budget.take(other, 1_000));
    assert!(budget.take(one, 1_000 + BUDGET_WINDOW_MS));
}
