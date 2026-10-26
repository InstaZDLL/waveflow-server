//! Evaluating a rule set under its budget, and what each reader was last
//! shown.

use super::statement::EVALUATE;
use super::*;

impl DomainServices {
    /// The track ids a rule set answers for `owner`, in its order, at most its
    /// limit.
    ///
    /// **Under a budget** (decision 4). The statement runs on a connection of
    /// its own with a SQLite progress handler installed: every
    /// [`BUDGET_STEP_OPS`] virtual-machine instructions it is asked whether to
    /// go on, and it stops the statement once the budget of instructions, or
    /// [`SMART_EVALUATION_DEADLINE`], is spent. A stopped evaluation answers
    /// `Unavailable` and returns nothing partial.
    ///
    /// **Spawned, so it always finishes what it started.** The handler is a
    /// property of the connection, and the connection goes back to the pool:
    /// were the request dropped between installing the handler and removing
    /// it, the next query on that connection would inherit an exhausted
    /// budget. A spawned task is not cancelled with the request, so the
    /// removal always runs.
    pub(super) async fn evaluate(
        &self,
        owner: Uuid,
        rules: &SmartRules,
    ) -> Result<Vec<Uuid>, ServiceError> {
        self.smart
            .evaluations
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let limit = rules.limit.unwrap_or(MAX_SMART_PLAYLIST_TRACKS);
        let mode = match rules.match_kind {
            SmartMatch::All => "all",
            SmartMatch::Any => "any",
        };
        let conditions = rules.bound_conditions(now_ms());
        let count = rules.conditions.len() as i64;
        let sort = rules.sort_code();
        let budget = self
            .smart
            .budget_ops
            .load(std::sync::atomic::Ordering::Relaxed);
        let pool = self.db.pool().clone();
        let exhausted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&exhausted);
        // Waited for holding no connection, and moved into the task, so it is
        // held for the whole evaluation even when the request is dropped.
        let permit = Arc::clone(&self.smart.permits)
            .acquire_owned()
            .await
            .map_err(|_| ServiceError::Unavailable)?;
        let running = Arc::clone(&self.smart.running);
        let peak = Arc::clone(&self.smart.peak);
        let ids = tokio::spawn(async move {
            let _permit = permit;
            let now = running.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            peak.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
            let ids = evaluate_on_its_own_connection(
                pool, owner, mode, conditions, count, sort, limit, budget, flag,
            )
            .await;
            running.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            ids
        })
        .await
        .map_err(|_| ServiceError::Unavailable)?;
        if exhausted.load(std::sync::atomic::Ordering::Relaxed) {
            tracing::warn!("a smart playlist evaluation passed its budget and was stopped");
            return Err(ServiceError::Unavailable);
        }
        ids?.into_iter()
            .map(|id| parse_uuid(id).map_err(ServiceError::from))
            .collect()
    }

    /// The most evaluations that have run at once in this process.
    #[doc(hidden)]
    pub fn smart_playlist_peak_evaluations(&self) -> u64 {
        self.smart.peak.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// The statement on a connection of its own, under the progress handler.
#[allow(clippy::too_many_arguments)]
async fn evaluate_on_its_own_connection(
    pool: sqlx::SqlitePool,
    owner: Uuid,
    mode: &'static str,
    conditions: String,
    count: i64,
    sort: i64,
    limit: u32,
    budget: u64,
    flag: Arc<std::sync::atomic::AtomicBool>,
) -> Result<Vec<String>, sqlx::Error> {
    let mut connection = pool.acquire().await?;
    let started = std::time::Instant::now();
    let mut spent: u64 = 0;
    connection
        .lock_handle()
        .await?
        .set_progress_handler(BUDGET_STEP_OPS, move || {
            spent = spent.saturating_add(BUDGET_STEP_OPS as u64);
            let go_on = spent <= budget && started.elapsed() < SMART_EVALUATION_DEADLINE;
            if !go_on {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            go_on
        });
    let ids: Result<Vec<String>, sqlx::Error> = sqlx::query_scalar(EVALUATE)
        .bind(owner.to_string())
        .bind(mode)
        .bind(conditions)
        .bind(count)
        .bind(sort)
        .bind(owner.to_string())
        .bind(i64::from(limit))
        .fetch_all(&mut *connection)
        .await;
    connection.lock_handle().await?.remove_progress_handler();
    ids
}

impl DomainServices {
    /// How many rule sets this process has evaluated. Observability, and what
    /// the suite counts to prove a listing evaluates none.
    #[doc(hidden)]
    pub fn smart_playlist_evaluations(&self) -> u64 {
        self.smart
            .evaluations
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Lowers the evaluation budget, in virtual-machine instructions. For the
    /// suite, which cannot build a library large enough to reach the real one.
    #[doc(hidden)]
    pub fn limit_smart_playlist_evaluation(&self, ops: u64) {
        self.smart
            .budget_ops
            .store(ops, std::sync::atomic::Ordering::Relaxed);
    }

    /// Records what one viewer was last shown of one playlist, for listings.
    pub(super) fn remember_extent(&self, playlist: &SmartPlaylistItem, viewer: Uuid) {
        if self.smart.extents.len() >= MAX_REMEMBERED_EXTENTS {
            self.smart.extents.clear();
        }
        self.smart.extents.insert(
            (playlist.id, viewer),
            RememberedExtent {
                updated_at: playlist.updated_at,
                evaluated_at: playlist.evaluated_at.unwrap_or_else(now_ms),
                song_count: playlist.song_count.unwrap_or(0),
                duration_ms: playlist.duration_ms.unwrap_or(0),
            },
        );
    }
}
