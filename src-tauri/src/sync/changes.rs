// The change log, and what to do when it has expired.
//
// Records arrive in order and are applied in order. A message can be added and deleted inside the
// same window, and a fetch can come back empty for a record that has not been processed yet; both
// are normal and neither is a failure.
//
// The cursor is committed only when the last page has been applied. A cursor from the middle of a
// chain points at the middle of a page, and storing one is how a client silently loses the mail
// that arrived between two pages.

use crate::mirror::write;
use crate::provider::{Change, ProviderError};

use super::{hydrate, Outcome, Remote, Sink, Store};
use crate::dto::SyncStatus;

fn oops(e: String) -> ProviderError {
    ProviderError::Other(e)
}

/// One incremental pass. `NeedsFullSync` is returned rather than handled, because recovering from
/// it is a different shape of pass and the engine is the thing that decides to run one.
pub async fn incremental<S: Store>(
    store: &S,
    remote: &dyn Remote,
    sink: &dyn Sink,
    status: &mut SyncStatus,
) -> Result<Outcome, ProviderError> {
    let cursor = store
        .with(|conn| write::meta_get(conn, write::CURSOR_KEY))
        .map_err(oops)?
        .ok_or_else(|| ProviderError::Other("there is no cursor to poll from".to_string()))?;

    let mut outcome = Outcome::default();
    let mut page: Option<String> = None;
    let mut latest;
    loop {
        let changes = remote.changes_since(&cursor, page.as_deref()).await?;
        if !changes.changes.is_empty() {
            outcome.changed = true;
            apply(store, &changes.changes)?;
        }
        latest = changes.cursor;
        page = changes.next_page;
        if page.is_none() {
            break;
        }
    }

    // The cursor advances as soon as the changes are on disk, before any hydration, for the same
    // reason the first sync commits its own early: the changes have already been applied, so a
    // crawl that fails after this point must not make the next pass read the same window of the
    // change log again.
    store
        .with(|conn| write::meta_set(conn, write::CURSOR_KEY, &latest))
        .map_err(oops)?;

    // Drained on every pass, not only on a pass where the change log had something in it. A first
    // sync that was interrupted leaves rows waiting for their metadata, and tying the drain to new
    // mail arriving meant that on a quiet mailbox those rows waited for ever: they sat in the list
    // with no sender and no subject until somebody happened to write.
    //
    // A backlog longer than one batch is a first sync being picked up where a quit or a tunnel
    // left it, and it is reported the way the first sync reports itself: as hydrating, with the
    // count carrying on from where it stopped. It used to drain under the plain "syncing" of an
    // ordinary pass with no sentence and no number, so an account that was still bringing in a
    // thousand messages looked idle and its Inbox looked empty on purpose. Under a batch it stays
    // quiet: that is the handful of messages every pass with new mail brings, gone in one call.
    let waiting = store
        .with(|conn| write::count(conn, "SELECT COUNT(*) FROM messages WHERE hydrated = 0"))
        .map_err(oops)?;
    let resumed = waiting as usize > hydrate::HYDRATE_BATCH;
    if resumed {
        let held = store
            .with(|conn| write::count(conn, "SELECT COUNT(*) FROM messages"))
            .map_err(oops)?;
        status.phase = "hydrating".to_string();
        status.message = Some(hydrate::FETCHING_MESSAGE.to_string());
        status.total = held;
        status.hydrated = held.saturating_sub(waiting);
        sink.status(status);
    }
    let drained = hydrate::drain_unhydrated(store, remote, sink, status).await?;
    if resumed {
        // Back to what an ordinary pass reports, and the counts to nothing: the housekeeping that
        // follows reads a total of zero as "the crawl is over, the body cache may have the budget".
        status.phase = "syncing".to_string();
        status.message = None;
        status.hydrated = 0;
        status.total = 0;
        sink.status(status);
    }
    if outcome.changed || drained > 0 {
        outcome.changed = true;
        sink.changed("threads");
    }

    // New mail is looked for here and nowhere else: the first sync, the recovery listing and a
    // backfill bring in mail that is old. On every pass rather than only one that changed
    // something, because a message hydrated during a recovery listing is new mail this pass did
    // not bring in. A failure is logged and not returned, since the mail is already on the screen
    // and a notification about it is not worth failing the pass that put it there.
    match store.with(|conn| crate::notify::arrivals(conn, &status.account_id, write::now_ms())) {
        Ok(arrivals) if !arrivals.is_empty() => sink.arrived(&status.account_id, &arrivals),
        Ok(_) => {}
        Err(e) => crate::log::note(&status.account_id, &format!("notify: {e}")),
    }
    Ok(outcome)
}

fn apply<S: Store>(store: &S, changes: &[Change]) -> Result<(), ProviderError> {
    store
        .with(|conn| {
            let mut threads: Vec<String> = Vec::new();
            for change in changes {
                let touched = match change {
                    Change::Added(reference) => {
                        write::note_listed(conn, &reference.id, &reference.thread_id)?;
                        Some(reference.thread_id.clone())
                    }
                    Change::Deleted(id) => write::delete_message(conn, id)?,
                    Change::LabelsChanged { id, labels } => write::set_labels(conn, id, labels)?,
                };
                if let Some(thread_id) = touched {
                    if !threads.contains(&thread_id) {
                        threads.push(thread_id);
                    }
                }
            }
            for thread_id in &threads {
                write::refresh_thread(conn, thread_id)?;
            }
            Ok(())
        })
        .map_err(oops)
}

/// The recovery path: the change log is too old to be used, so the window is listed again and the
/// difference is worked out locally. Nothing in the state database is touched and nothing in the
/// outbox is dropped, because neither of them came from the provider in the first place.
pub async fn reconcile<S: Store>(
    store: &S,
    remote: &dyn Remote,
    sink: &dyn Sink,
    status: &mut SyncStatus,
) -> Result<Outcome, ProviderError> {
    let cursor = remote.cursor_now().await?;
    let after = store
        .with(|conn| write::window_start(conn, write::now_ms()))
        .map_err(oops)?;

    status.phase = "syncing".to_string();
    status.message = Some("Rebuilding from the mailbox".to_string());
    sink.status(status);

    store
        .with(|conn| {
            conn.execute_batch(
                "CREATE TEMP TABLE IF NOT EXISTS listed (id TEXT PRIMARY KEY);
                 DELETE FROM listed;",
            )
            .map_err(|e| e.to_string())
        })
        .map_err(oops)?;

    let mut page: Option<String> = None;
    loop {
        let listed = remote.list(after, page.as_deref()).await?;
        store
            .with(|conn| {
                for message in &listed.messages {
                    conn.execute(
                        "INSERT OR IGNORE INTO listed (id) VALUES (?1)",
                        [&message.id],
                    )
                    .map_err(|e| e.to_string())?;
                    write::note_listed(conn, &message.id, &message.thread_id)?;
                }
                Ok(())
            })
            .map_err(oops)?;
        page = listed.next_page;
        if page.is_none() {
            break;
        }
    }

    // A row inside the window that the provider no longer lists was deleted upstream. Transient
    // rows came from a search rather than from the window and are not the listing's to judge.
    let gone = store
        .with(|conn| {
            let cutoff = after.unwrap_or(0);
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM messages
                     WHERE hydrated = 1 AND transient = 0 AND date_ms >= ?1
                       AND id NOT IN (SELECT id FROM listed)",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([cutoff], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
        })
        .map_err(oops)?;

    let changed = !gone.is_empty();
    store
        .with(|conn| {
            for id in &gone {
                write::delete_message(conn, id)?;
            }
            conn.execute_batch("DELETE FROM listed;")
                .map_err(|e| e.to_string())
        })
        .map_err(oops)?;

    let hydrated = hydrate::drain_unhydrated(store, remote, sink, status).await?;
    store
        .with(|conn| write::meta_set(conn, write::CURSOR_KEY, &cursor))
        .map_err(oops)?;
    sink.changed("threads");

    Ok(Outcome {
        changed: changed || hydrated > 0,
        error: None,
    })
}
