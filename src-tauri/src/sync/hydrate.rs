// Filling the mirror: the ids, then the metadata, then the bodies.
//
// One path fills the mirror and one path only. A first sync, a backfill after a widened window and
// the hits of a provider search all end up here, because two ways of writing a message into the
// mirror is two sets of bugs about what a message row means.
//
// Bodies are never fetched during a first sync. They are fetched when a thread is opened and
// prefetched when the app is idle, which is what keeps the first sync to metadata and inside the
// quota.

use futures::stream::StreamExt;

use crate::mime::RenderOptions;
use crate::mirror::{evict, read, write};
use crate::provider::ProviderError;

use super::{Remote, Sink, Store};
use crate::dto::SyncStatus;

/// Gmail's batch guidance is 100 at most and "larger than 50 is not recommended", and at 20 units
/// a message a batch of 50 is 1,000 units, so six batches a minute sits inside the budget.
pub const HYDRATE_BATCH: usize = 50;
/// One page of ids is cheap; the pages are what the progress bar counts.
const LIST_CAP: usize = 100_000;
/// How many hits of a provider search are pulled in as transient rows.
const SEARCH_CAP: usize = 100;
/// What the account says while the crawl fills in metadata, on a first sync and on the pass that
/// picks up a first sync a quit or a tunnel cut short.
pub const FETCHING_MESSAGE: &str = "Fetching the newest mail first";

fn oops(e: String) -> ProviderError {
    ProviderError::Other(e)
}

/// What the sanitiser is allowed to do on this account's behalf. Remote images are off until
/// somebody asks for them, and the account's own addresses are what "sent by me" is decided from.
pub fn render_options(conn: &rusqlite::Connection) -> Result<RenderOptions, String> {
    Ok(RenderOptions {
        allow_remote_images: false,
        link_cleaning: true,
        remote_images: Default::default(),
        own_addresses: write::meta_get(conn, write::OWN_ADDRESS_KEY)?
            .into_iter()
            .collect(),
    })
}

/// Metadata for a set of ids, in batches. Ids the provider does not return were deleted between
/// the listing and the fetch, which is normal rather than an error, and their placeholder rows go
/// so that hydration terminates.
pub async fn headers<S: Store>(
    store: &S,
    remote: &dyn Remote,
    ids: &[String],
    transient: bool,
) -> Result<u32, ProviderError> {
    let mut done = 0u32;
    for chunk in ids.chunks(HYDRATE_BATCH) {
        let fetched = remote.fetch_headers(chunk).await?;
        let count = store
            .with(|conn| {
                let mut threads: Vec<String> = Vec::new();
                for message in &fetched {
                    write::upsert_message(conn, message, transient)?;
                    if !threads.contains(&message.thread_id) {
                        threads.push(message.thread_id.clone());
                    }
                }
                for thread_id in &threads {
                    write::refresh_thread(conn, thread_id)?;
                }
                for id in chunk {
                    if !fetched.iter().any(|message| &message.id == id) {
                        write::delete_message(conn, id)?;
                    }
                }
                Ok(fetched.len() as u32)
            })
            .map_err(oops)?;
        done += count;
    }
    Ok(done)
}

/// Everything waiting for metadata, newest first, reporting as it goes.
pub async fn drain_unhydrated<S: Store>(
    store: &S,
    remote: &dyn Remote,
    sink: &dyn Sink,
    status: &mut SyncStatus,
) -> Result<u32, ProviderError> {
    let mut done = 0u32;
    loop {
        let batch = store
            .with(|conn| write::unhydrated(conn, HYDRATE_BATCH))
            .map_err(oops)?;
        if batch.is_empty() {
            break;
        }
        done += headers(store, remote, &batch, false).await?;
        status.hydrated += batch.len() as u32;
        sink.status(status);
        sink.changed("threads");
    }
    Ok(done)
}

/// The first fill of a window. The cursor is taken before anything is listed, so a message that
/// arrives during the crawl is picked up by the first incremental pass rather than missed.
pub async fn first_sync<S: Store>(
    store: &S,
    remote: &dyn Remote,
    sink: &dyn Sink,
    status: &mut SyncStatus,
) -> Result<(), ProviderError> {
    let cursor = remote.cursor_now().await?;
    let after = store
        .with(|conn| write::window_start(conn, write::now_ms()))
        .map_err(oops)?;

    status.phase = "syncing".to_string();
    status.message = Some("Listing your mail".to_string());
    sink.status(status);

    let listed = list_into(store, remote, after, LIST_CAP).await?;
    status.total = listed;
    status.phase = "hydrating".to_string();
    status.message = Some(FETCHING_MESSAGE.to_string());
    sink.status(status);

    if let Ok(labels) = remote.labels().await {
        store
            .with(|conn| write::upsert_labels(conn, &labels))
            .map_err(oops)?;
    }

    // The cursor is stored here, before a single body is fetched, and this line is the difference
    // between a first sync that finishes and one that starts again every twelve seconds.
    //
    // `cursor_now` was taken before the listing, so nothing is lost by committing it early: the
    // next pass reads the change log from that instant forward. What hydration has not filled in
    // yet is already on disk as an unhydrated row, and the incremental path drains those on every
    // pass. Storing it after the crawl instead meant that one failed batch out of a thousand
    // discarded the whole pass, and on a real mailbox with an ordinary flaky connection the first
    // sync never completed at all: the cursor was never written, so the next pass saw no cursor,
    // re-listed and re-hydrated everything, and the account hung there re-crawling itself forever.
    store
        .with(|conn| {
            write::meta_set(conn, write::CURSOR_KEY, &cursor)?;
            write::meta_set(conn, write::FIRST_SYNC_KEY, "1")?;
            // A window chosen before this ran was written through the same path a change from
            // Settings takes, which queues a backfill to the new edge. The listing above just
            // covered the whole window, so that backfill would list it a second time for nothing.
            evict::backfill_done(conn)
        })
        .map_err(oops)?;

    drain_unhydrated(store, remote, sink, status).await?;

    status.phase = "idle".to_string();
    status.message = None;
    status.hydrated = 0;
    status.total = 0;
    sink.status(status);
    sink.changed("threads");
    Ok(())
}

/// Walks `list` to exhaustion, noting every id as waiting for metadata. Returns how many are now
/// waiting, which is what the progress bar divides by.
pub async fn list_into<S: Store>(
    store: &S,
    remote: &dyn Remote,
    after_ms: Option<i64>,
    cap: usize,
) -> Result<u32, ProviderError> {
    let mut page: Option<String> = None;
    let mut seen = 0usize;
    loop {
        let listed = remote.list(after_ms, page.as_deref()).await?;
        let count = listed.messages.len();
        store
            .with(|conn| {
                for message in &listed.messages {
                    write::note_listed(conn, &message.id, &message.thread_id)?;
                }
                Ok(())
            })
            .map_err(oops)?;
        seen += count;
        page = listed.next_page;
        if page.is_none() || seen >= cap {
            break;
        }
    }
    store
        .with(|conn| write::count(conn, "SELECT COUNT(*) FROM messages WHERE hydrated = 0"))
        .map_err(oops)
}

/// The range a widened window newly covers, filled through the same path as a first sync.
pub async fn backfill<S: Store>(
    store: &S,
    remote: &dyn Remote,
    sink: &dyn Sink,
    status: &mut SyncStatus,
    after_ms: Option<i64>,
) -> Result<(), ProviderError> {
    status.phase = "backfilling".to_string();
    status.message = Some("Fetching older mail".to_string());
    status.total = list_into(store, remote, after_ms, LIST_CAP).await?;
    status.hydrated = 0;
    sink.status(status);

    drain_unhydrated(store, remote, sink, status).await?;

    status.phase = "idle".to_string();
    status.message = None;
    status.hydrated = 0;
    status.total = 0;
    sink.status(status);
    Ok(())
}

/// One body, parsed, sanitised and cached. A body that will not render is still stored, so the
/// thread opens with its headers and renders itself again once the pipeline can read it.
pub async fn body<S: Store>(
    store: &S,
    remote: &dyn Remote,
    message_id: &str,
) -> Result<(), ProviderError> {
    let raw = remote.fetch_body(message_id).await?;
    store
        .with(|conn| {
            let options = render_options(conn)?;
            write::store_body(conn, message_id, &raw, &options)
        })
        .map_err(oops)
}

/// How many bodies are asked for at once.
///
/// One call each rather than the batch endpoint the metadata crawl goes through. Fifty raw
/// messages in one batch is a single response of several megabytes that fails as a unit, where
/// fifty separate calls fail one at a time and cost the same 20 units apiece; a body is worth
/// having on its own, which a header is not. So the way a five message thread costs one round trip
/// of wall clock rather than five is to have them in flight together, and eight is enough to hide
/// the latency without asking a hotel connection to carry the whole set at once. It is not what
/// limits the rate either: the quota accountant is.
pub const BODY_CONCURRENCY: usize = 8;

/// What a set of bodies did: how many landed, and which ones would not come.
#[derive(Debug, Default)]
pub struct Cached {
    pub stored: u32,
    /// The ids the provider refused, so a caller working through a backlog can stop asking for the
    /// same ones. A thread open ignores this and asks anyway: its set is the length of one thread,
    /// so it cannot stall anything, and somebody waiting on a message is a reason to try again.
    pub failed: Vec<String>,
}

/// A set of bodies, fetched together and stored as each one lands.
///
/// A body that failed leaves no row rather than failing the set. Four of five bodies is a thread
/// that mostly reads and a fifth message that says so, which is better than an error over a thread
/// the mirror could have shown; the one that failed is simply still missing, so the next thing to
/// ask for it asks again.
async fn bodies<S: Store>(store: &S, remote: &dyn Remote, ids: Vec<String>) -> Cached {
    futures::stream::iter(ids)
        .map(|id| async move {
            match body(store, remote, &id).await {
                Ok(()) => None,
                Err(e) => {
                    // Written down, because the only other trace of it is a message that opens
                    // with three grey bars and nothing under them.
                    crate::log::note("bodies", &format!("{id}: {}: {e}", e.kind()));
                    Some(id)
                }
            }
        })
        .buffer_unordered(BODY_CONCURRENCY)
        .fold(Cached::default(), |mut done, refused| async move {
            match refused {
                Some(id) => done.failed.push(id),
                None => done.stored += 1,
            }
            done
        })
        .await
}

/// Every body a thread is missing, which is what opening one asks for in the background.
///
/// Never on the path that returns the thread. The mirror has the headers, the dates and the
/// attachment list already, and waiting here for five serial round trips is what made opening a
/// thread take seconds.
pub async fn thread_bodies<S: Store>(
    store: &S,
    remote: &dyn Remote,
    key: &str,
) -> Result<u32, ProviderError> {
    let missing = store.with(|conn| read::bodiless(conn, key)).map_err(oops)?;
    Ok(bodies(store, remote, missing).await.stored)
}

/// The body cache: the newest messages that have none, in one batch, leaving out whatever the
/// caller has already been refused. Never during a first sync:
/// the metadata crawl owns the budget until it is finished, and a body fetched early is a body
/// fetched instead of the list somebody is waiting on.
///
/// This is the difference between a mailbox where every thread opens instantly and one where every
/// thread is a cold fetch forever. At five bodies a pass it never caught up with an ordinary
/// account and the cache was decoration; the limit the engine passes now is sized against the
/// minute's quota instead.
pub async fn prefetch<S: Store>(
    store: &S,
    remote: &dyn Remote,
    limit: usize,
    skip: &[String],
) -> Result<Cached, ProviderError> {
    let wanted = store
        .with(|conn| read::prefetchable(conn, limit, skip))
        .map_err(oops)?;
    Ok(bodies(store, remote, wanted).await)
}

/// Hits from the provider's own search, pulled in as transient rows. The next eviction pass takes
/// them away again unless they gained state in the meantime.
pub async fn from_search<S: Store>(
    store: &S,
    remote: &dyn Remote,
    query: &str,
) -> Result<u32, ProviderError> {
    let found = remote.search(query, None).await?;
    let ids: Vec<String> = found
        .messages
        .into_iter()
        .take(SEARCH_CAP)
        .map(|message| message.id)
        .collect();
    headers(store, remote, &ids, true).await
}
