// One account's sync loop, as a state machine.
//
// A pass pushes before it pulls, so a write that has just landed comes back as the provider's own
// row in the same pass rather than a tick later. Then it fills or polls, evicts when a day has
// gone by, backfills when a widened window asked for one, and prefetches a few bodies if there is
// nothing more urgent to do.
//
// The circuit breaker is Mailspring's, and it is the difference between an account that is broken
// and an app that is broken: after repeated failures the account stops trying, says so in its
// phase, and waits for the cooldown or for somebody to press sync.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::dto::SyncStatus;
use crate::mirror::{evict, read, write};
use crate::provider::ProviderError;

use super::{changes, hydrate, outbox, Outcome, Remote, Sink, Store};
use crate::log;

/// Four failures in a row is a pattern rather than an accident.
pub const BREAKER_TRIPS: u32 = 4;
pub const BREAKER_COOLDOWN_MS: i64 = 5 * 60_000;
/// How many failures in a row an account keeps to itself before the chip says anything. One is
/// an accident: the connection the machine slept through, a 5xx Google was having. Mailspring
/// shows nothing for a single failure of any kind and a red error only after five in five
/// minutes, and its users never see one; this app used to change the chip and raise a toast on
/// the first, and its user saw one every time the laptop woke up.
pub const QUIET_FAILURES: u32 = 1;
/// A short rest after a single failure, so a poll every twelve seconds does not become a retry
/// every twelve seconds.
const RETRY_REST_MS: i64 = 30_000;
/// Bodies fetched per pass by the body cache.
///
/// The arithmetic, against Gmail's 6,000 units a minute: one body is one `messages.get` at 20
/// units, and the foreground poll is a pass every twelve seconds. Forty bodies is 800 units a
/// pass and about 4,000 a minute at the very busiest, which leaves a third of the minute for
/// everything else. That remainder is the point of the number: it is a hundred message opens a
/// minute of headroom, so a thread somebody has just opened is never queueing behind the cache.
///
/// It was five, once a pass, which is 25 bodies a minute. On a window of fourteen hundred messages
/// that never caught up, so every thread anybody opened was a cold fetch, for ever, and the cache
/// was decoration.
const CACHE_PER_PASS: usize = 40;
/// Stale renders redone per pass. A render is a few milliseconds and the odd newsletter is a
/// hundred and fifty; twenty five keeps the connection held for well under a second.
const RERENDER_PER_PASS: usize = 25;

/// What the account says while the backlog drains. It names the work rather than the mechanism,
/// which is Mailspring's line and the right one.
const CACHING_MESSAGE: &str = "Caching recent mail";

/// How many refused messages the cache remembers before it starts forgetting the oldest.
///
/// Five hundred is more than any mailbox should have and small enough to be a rounding error in
/// memory. The cap is here because the set is only ever added to within a session, and an
/// unbounded list of ids kept by a process that runs for weeks is a leak however slow.
///
/// It is a list in memory rather than a mark on disk, and the obvious place for a mark on disk is
/// closed: `body_pending` in the frozen contract means "no row in `bodies`", so a failure noted
/// there would make every message the cache gave up on claim to have a body. Remembering this
/// across launches would need a column on `messages`, a schema step and a rule for when it clears,
/// which is a great deal of machinery for a set that is mostly transient failures worth retrying.
const POISON_MEMORY: usize = 500;

pub struct Engine {
    pub account_id: String,
    state: Mutex<State>,
    /// Whether a pass is on this account right now. A pass somebody asked for lands on top of the
    /// poll loop's, and an account that has just been connected gets one from the connect and one
    /// from the next tick; two first syncs at once list the mailbox twice and hydrate the same
    /// batches twice. The second caller gets the status the first is producing and nothing else.
    running: AtomicBool,
}

/// Lowers the flag when the pass ends, however it ends: a future dropped at a quit ends too.
struct Running<'a>(&'a AtomicBool);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct State {
    status: SyncStatus,
    failures: u32,
    hold_until: i64,
    /// What the body cache's backlog was when it started draining, so its progress counts up
    /// rather than standing still while the number left goes down.
    cache_total: u32,
    /// The messages the provider would not give a body for, which the cache stops asking for.
    ///
    /// A message deleted upstream between the listing and the fetch answers 404 for ever, and it
    /// sorts newest first like anything else, so without this the cache asks for the same handful
    /// at the head of the queue on every pass and never reaches the mail behind them. That is the
    /// same shape as the first sync that wrote its cursor last and re-crawled for ever, and one of
    /// those is enough.
    ///
    /// In memory and for the life of the process on purpose. Most body failures are transient, so
    /// the retry worth having is the next launch or the next time somebody presses sync, not a
    /// column that would remember one bad hour on a train until the mirror is cleared.
    poisoned: Vec<String>,
}

impl Engine {
    pub fn new(account_id: &str) -> Engine {
        Engine {
            account_id: account_id.to_string(),
            state: Mutex::new(State {
                status: SyncStatus::idle(account_id),
                failures: 0,
                hold_until: 0,
                cache_total: 0,
                poisoned: Vec::new(),
            }),
            running: AtomicBool::new(false),
        }
    }

    pub fn status(&self) -> SyncStatus {
        self.state
            .lock()
            .map(|state| state.status.clone())
            .unwrap_or_else(|_| SyncStatus::idle(&self.account_id))
    }

    /// Closes the breaker. A sync somebody asked for is a person saying "try anyway", and they can
    /// see the failure for themselves if it comes back.
    ///
    /// Which is also why it forgets the bodies the cache gave up on. "Try anyway" means all of it,
    /// and pressing sync is the only way back for a message the provider refused earlier in the
    /// session short of restarting the app.
    pub fn resume(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.failures = 0;
            state.hold_until = 0;
            state.poisoned.clear();
            if state.status.phase == "paused" {
                state.status.phase = "idle".to_string();
                state.status.message = None;
            }
        }
    }

    pub fn paused(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.status.phase == "paused")
            .unwrap_or(false)
    }

    /// Ends the rest after a failure without forgetting the failure, which is what a test that
    /// wants the second failure in a row needs and nothing else does: the poll loop waits it out.
    #[cfg(test)]
    pub(crate) fn unrest(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.hold_until = 0;
        }
    }

    fn resting(&self, now: i64) -> bool {
        self.state
            .lock()
            .map(|state| state.hold_until > now)
            .unwrap_or(false)
    }

    fn keep(&self, status: &SyncStatus) {
        if let Ok(mut state) = self.state.lock() {
            state.status = status.clone();
        }
    }

    /// Records a failure and decides whether this account has had enough. Returns the phase the
    /// pass should report and the word the account chip prints for it, or nothing when this
    /// failure is still the account's own business.
    ///
    /// The word is decided here because only the engine sees the error's kind, and the kind is
    /// the whole question: a token Google refused is "Signed out", and nothing else is, however
    /// it used to read. A 400 for one label change on a signed-in account said "Signed out" for as
    /// long as the row was retried, which was for ever. The detail goes in `status.error` for the
    /// toast and the log; the chip gets two words.
    ///
    /// The first `QUIET_FAILURES` of a run are kept quiet unless the kind is one that will not
    /// mend on its own: Google refusing the token or the scope is true on the first answer and
    /// stays true, and a rate limit already carries how long to wait. A network failure or a
    /// 5xx is answered by the next poll, and the next poll is twelve seconds away.
    fn note_failure(&self, now: i64, error: &ProviderError) -> Option<(String, String)> {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => return Some(("error".to_string(), "Sync trouble".to_string())),
        };
        state.failures += 1;
        if state.failures >= BREAKER_TRIPS {
            state.hold_until = now + BREAKER_COOLDOWN_MS;
            return Some((
                "paused".to_string(),
                "Paused after repeated failures. Sync now to try again.".to_string(),
            ));
        }
        let rest = match error {
            ProviderError::RateLimited { retry_after_ms } => *retry_after_ms as i64,
            // A dropped connection has already been tried again three times by the call that
            // reported it. The next poll is the retry; there is nothing to wait for.
            ProviderError::Network(_) => 0,
            _ => RETRY_REST_MS,
        };
        state.hold_until = now + rest;
        let (phase, word) = match error {
            ProviderError::Network(_) => ("offline", "Offline"),
            ProviderError::Auth(_) => ("error", "Signed out"),
            ProviderError::Scope(_) => ("error", "Needs permission"),
            ProviderError::RateLimited { .. } => ("error", "Rate limited"),
            ProviderError::NeedsFullSync | ProviderError::NotFound | ProviderError::Other(_) => {
                ("error", "Sync trouble")
            }
        };
        let loud = matches!(
            error,
            ProviderError::Auth(_) | ProviderError::Scope(_) | ProviderError::RateLimited { .. }
        );
        if !loud && state.failures <= QUIET_FAILURES {
            return None;
        }
        Some((phase.to_string(), word.to_string()))
    }

    /// The denominator the caching progress divides by. New mail arriving raises it; nothing
    /// lowers it until the backlog is empty, because a total that moved down as work was done
    /// would report a bar that never fills.
    fn cache_total(&self, backlog: u32) -> u32 {
        let Ok(mut state) = self.state.lock() else {
            return backlog;
        };
        state.cache_total = state.cache_total.max(backlog);
        state.cache_total
    }

    fn cache_settled(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.cache_total = 0;
        }
    }

    /// What the cache is not going to ask for, with anything that has picked up a body since taken
    /// back out. A message the cache refused to chase is still fetched when somebody opens its
    /// thread, and one that has arrived is not one to keep skipping.
    fn skipped<S: Store>(&self, store: &S) -> Vec<String> {
        let held = match self.state.lock() {
            Ok(state) => state.poisoned.clone(),
            Err(_) => return Vec::new(),
        };
        if held.is_empty() {
            return held;
        }
        let still = store
            .with(|conn| read::still_bodiless(conn, &held))
            .unwrap_or_else(|_| held.clone());
        if let Ok(mut state) = self.state.lock() {
            state.poisoned = still.clone();
        }
        still
    }

    /// Notes what the provider refused, oldest forgotten first once the set is at its cap.
    fn note_poisoned(&self, refused: Vec<String>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        for id in refused {
            if !state.poisoned.contains(&id) {
                state.poisoned.push(id);
            }
        }
        let over = state.poisoned.len().saturating_sub(POISON_MEMORY);
        if over > 0 {
            state.poisoned.drain(..over);
        }
    }

    fn note_success(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.failures = 0;
            state.hold_until = 0;
        }
    }

    /// One pass. Returns the status it left the account in, which is also what it emitted.
    pub async fn run_pass<S: Store>(
        &self,
        store: &S,
        remote: &dyn Remote,
        sink: &dyn Sink,
        foreground: bool,
    ) -> SyncStatus {
        if self.running.swap(true, Ordering::AcqRel) {
            return self.status();
        }
        let _running = Running(&self.running);
        let now = write::now_ms();
        if self.resting(now) {
            return self.status();
        }

        let mut status = SyncStatus {
            phase: "syncing".to_string(),
            ..SyncStatus::idle(&self.account_id)
        };
        status.last_sync_ms = store
            .with(|conn| write::meta_i64(conn, write::LAST_SYNC_KEY))
            .ok()
            .flatten();
        status.pending_writes = store.with(write::pending_writes).unwrap_or(0);
        sink.status(&status);

        let pushed = outbox::drain_reporting(store, remote, now + outbox::PUSH_BUDGET_MS).await;
        if pushed.outcome.changed {
            sink.changed("outbox");
        }
        // A write the provider refused for good is said once, here, and is not a failure of the
        // pass: the row is gone, the queue is moving, and the status at the end will be clean. It
        // travels as a status with the error on it and the phase still `syncing`, which is how the
        // frontend tells "this change did not take" from "this account has stopped".
        for reason in &pushed.dropped {
            log::note(&self.account_id, reason);
            status.error = Some(reason.clone());
            sink.status(&status);
        }
        status.error = None;

        let pulled = self.pull(store, remote, sink, &mut status).await;

        if pulled.is_ok() && pushed.outcome.error.is_none() {
            // After the pull rather than inside it, because the pull is what drains the backlog
            // the seed is waiting on and asking beforehand would answer "not yet" on the very pass
            // that made it ready.
            self.seed_when_ready(store, sink);
            self.housekeeping(store, remote, sink, &mut status, foreground)
                .await;
        }

        status.pending_writes = store.with(write::pending_writes).unwrap_or(0);
        status.oldest_ms = store
            .with(|conn| {
                conn.query_row(
                    "SELECT MIN(date_ms) FROM messages WHERE hydrated = 1 AND date_ms > 0",
                    [],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .map_err(|e| e.to_string())
            })
            .ok()
            .flatten();

        match pulled.err().or(pushed.outcome.error) {
            Some(error) => {
                // In the log as well as in the status. A person who has run the app from a
                // terminal because the mail is not arriving should not have to open the developer
                // tools to read what the provider actually said, and a person who has not still
                // has the file.
                log::note(&self.account_id, &format!("{}: {error}", error.kind()));
                match self.note_failure(now, &error) {
                    Some((phase, message)) => {
                        status.phase = phase;
                        status.error = Some(error.to_string());
                        status.message = Some(message);
                    }
                    // Written down and otherwise kept to ourselves: the status the frontend sees
                    // is the one it saw before, and the next poll is the retry.
                    None => {
                        status.phase = "idle".to_string();
                        status.error = None;
                        status.message = None;
                    }
                }
            }
            None => {
                self.note_success();
                let stamp = write::now_ms();
                let _ = store
                    .with(|conn| write::meta_set(conn, write::LAST_SYNC_KEY, &stamp.to_string()));
                // The body cache speaks for itself. A pass that ended with a backlog still
                // draining is not idle, and overwriting it here is what would make the status bar
                // flicker between the two every twelve seconds.
                if status.phase != "caching" {
                    status.phase = "idle".to_string();
                    status.message = None;
                }
                status.error = None;
                status.last_sync_ms = Some(stamp);
            }
        }

        self.keep(&status);
        sink.status(&status);
        status
    }

    async fn pull<S: Store>(
        &self,
        store: &S,
        remote: &dyn Remote,
        sink: &dyn Sink,
        status: &mut SyncStatus,
    ) -> Result<Outcome, ProviderError> {
        let cursor = store
            .with(|conn| write::meta_get(conn, write::CURSOR_KEY))
            .map_err(ProviderError::Other)?;
        if cursor.is_none() {
            hydrate::first_sync(store, remote, sink, status).await?;
            return Ok(Outcome {
                changed: true,
                error: None,
            });
        }
        match changes::incremental(store, remote, sink, status).await {
            Err(ProviderError::NeedsFullSync) => {
                changes::reconcile(store, remote, sink, status).await
            }
            other => other,
        }
    }

    /// Screening in everyone the account already knows, once there is a mailbox to look at.
    ///
    /// This runs on every pass rather than at the end of the first one. A mailbox of a few thousand
    /// messages takes several passes to hydrate and any one of them can fail on a flaky connection,
    /// so hanging the seed off a clean first sync meant that on a real account it never ran: every
    /// sender waited in the Screener, and the Inbox showed a handful of threads out of a thousand.
    ///
    /// It waits for the crawl to finish because it reads the correspondents the crawl writes.
    /// Seeding halfway through would screen in whoever happened to be hydrated and leave the rest
    /// waiting, which is worse than waiting a minute, and `seed_once` is guarded so there is no
    /// second chance to get it right.
    fn seed_when_ready<S: Store>(&self, store: &S, sink: &dyn Sink) {
        match store.with(crate::routing::seed_if_ready) {
            // The rules the seed wrote are what move a thread from the Screener into the Inbox,
            // and the pull that made the mirror ready has already said "threads" before this ran.
            // Without a word from here the Inbox stayed empty until the next thing changed.
            Ok(crate::routing::Seeded::Ran(screened)) if screened > 0 => {
                sink.changed("threads screener state");
            }
            Ok(_) => {}
            // Loudly, because a seed that quietly failed is an Inbox that is quietly empty, and
            // that is exactly the failure this whole function exists to stop happening twice.
            Err(e) => log::note(&self.account_id, &format!("seed: {e}")),
        }
    }

    /// Eviction, the backfill and the prefetch, in that order. None of them is allowed to fail a
    /// pass: they are maintenance, and the mail arriving matters more than the tidying.
    async fn housekeeping<S: Store>(
        &self,
        store: &S,
        remote: &dyn Remote,
        sink: &dyn Sink,
        status: &mut SyncStatus,
        foreground: bool,
    ) {
        let now = write::now_ms();
        if store.with(|conn| evict::due(conn, now)).unwrap_or(false) {
            let report = store.with(|conn| evict::run(conn, now)).unwrap_or_default();
            if report.threads > 0 {
                sink.changed("threads");
            }
        }

        if let Ok(Some(target)) = store.with(evict::backfill_target) {
            let after = (target > 0).then_some(target);
            if hydrate::backfill(store, remote, sink, status, after)
                .await
                .is_ok()
            {
                let _ = store.with(evict::backfill_done);
                sink.changed("threads");
            }
        }

        // Renders made by an older sanitiser are made again here, a few a pass, so that opening
        // a thread never pays for them: the open path still catches whatever this has not reached,
        // but on a mailbox this has finished with it finds nothing to do. Local work, no quota.
        let renewed = store
            .with(|conn| {
                let options = hydrate::render_options(conn)?;
                let mut renewed = 0u32;
                for message_id in read::stale_renders_any(conn, RERENDER_PER_PASS)? {
                    if write::rerender(conn, &message_id, &options)? {
                        renewed += 1;
                    }
                }
                Ok(renewed)
            })
            .unwrap_or(0);
        if renewed > 0 {
            sink.changed("thread");
        }

        // Never during a first sync: the metadata crawl owns the budget until it is finished.
        let caching = store
            .with(|conn| write::meta_get(conn, "prefetch"))
            .ok()
            .flatten()
            .map(|value| value != "0")
            .unwrap_or(true);
        if foreground && caching && status.total == 0 {
            self.cache_bodies(store, remote, sink, status).await;
        }
    }

    /// The body cache, and the only part of a pass that says what it is doing while it does it.
    ///
    /// A mail client where moving between two messages is instant is one that fetched them before
    /// anybody asked. This drains the messages that have no body yet, newest first, a batch a
    /// pass, and reports the drain as `caching` with a count, because an account takes several
    /// minutes to warm up and an app that looks idle for several minutes looks broken.
    async fn cache_bodies<S: Store>(
        &self,
        store: &S,
        remote: &dyn Remote,
        sink: &dyn Sink,
        status: &mut SyncStatus,
    ) {
        let skip = self.skipped(store);
        let backlog = store
            .with(|conn| read::prefetch_backlog(conn, &skip))
            .unwrap_or(0);
        if backlog == 0 {
            self.cache_settled();
            return;
        }

        let total = self.cache_total(backlog);
        status.phase = "caching".to_string();
        status.message = Some(CACHING_MESSAGE.to_string());
        status.total = total;
        status.hydrated = total.saturating_sub(backlog);
        sink.status(status);

        let cached = hydrate::prefetch(store, remote, CACHE_PER_PASS, &skip)
            .await
            .unwrap_or_default();
        let stored = cached.stored;
        self.note_poisoned(cached.failed);

        // Finished, or getting nowhere. A pass that stored nothing is not caching whatever the
        // backlog says, and an account reporting itself busy while nothing moves is worse than one
        // that says nothing at all.
        // Taken before the count rather than inside it: `store.with` holds the account's one
        // connection, and asking for it again from within the closure is a deadlock.
        let skip = self.skipped(store);
        let left = store
            .with(|conn| read::prefetch_backlog(conn, &skip))
            .unwrap_or(0);
        if left == 0 || stored == 0 {
            self.cache_settled();
            status.phase = "idle".to_string();
            status.message = None;
            status.hydrated = 0;
            status.total = 0;
        } else {
            status.hydrated = total.saturating_sub(left);
        }
        if stored > 0 {
            // The scope is the thread rather than the list: a body landing changes what an open
            // message shows and nothing whatever about the row above it.
            sink.changed("thread");
        }
        sink.status(status);
    }

    /// The backfill on its own, for the command that a widened window fires.
    pub async fn backfill<S: Store>(
        &self,
        store: &S,
        remote: &dyn Remote,
        sink: &dyn Sink,
    ) -> Result<(), String> {
        let target = store.with(evict::backfill_target)?;
        let Some(target) = target else {
            return Ok(());
        };
        let mut status = SyncStatus::idle(&self.account_id);
        hydrate::backfill(store, remote, sink, &mut status, (target > 0).then_some(target))
            .await
            .map_err(|e| e.to_string())?;
        store.with(evict::backfill_done)?;
        self.keep(&status);
        sink.changed("threads");
        Ok(())
    }
}
