// The sync engine: the poll loop, the first fill, the change log, the outbox and the window.
//
// Everything the engine says to a mailbox goes through `Remote`, which is `provider::Provider`
// with its futures boxed so a handle can be stored and handed round at runtime. The blanket
// implementation means any `Provider` is a `Remote` without being written twice, and the fake
// mailbox in `provider::fake` is what the tests drive. Nothing here knows what a Gmail label id
// looks like except the five system names the mirror has to read a flag out of.
//
// Polling, not push. `history.list` costs two units, so twelve seconds in the foreground and sixty
// in the background is a rounding error against the budget, and the alternative needs a server.

pub mod changes;
pub mod engine;
pub mod hydrate;
pub mod outbox;

#[cfg(test)]
mod search_tests;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rusqlite::Connection;
use tauri::{Emitter, Manager};

use crate::db::Db;
use crate::dto::{SearchResult, SyncStatus, ThreadPage, ThreadQuery};
use crate::mirror::{evict, fts, read, write};
use crate::dto::FlagPatch;
use crate::provider::{
    Changes, ListPage, Provider, ProviderError, ProviderLabel, RawHeaders, SentIds,
};

/// What one half of a pass did. `error` is a provider error rather than a string because the
/// engine branches on the kind: a rate limit is a wait, an expired change log is a full list, and
/// a network failure on a train is not a fault to report at all.
#[derive(Debug, Default)]
pub struct Outcome {
    pub changed: bool,
    pub error: Option<ProviderError>,
}

pub const POLL_FOREGROUND_SECS: u64 = 12;
pub const POLL_BACKGROUND_SECS: u64 = 60;
/// A cold start should show fresh mail rather than wait out a poll interval.
const FIRST_PASS_SECS: u64 = 2;

pub type Boxed<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// `Provider` with its futures boxed. The trait itself returns `impl Future`, which is the right
/// shape for the implementation and the wrong shape for a map of live accounts, because an opaque
/// return type cannot be named twice. This is that trait made storable and nothing else.
pub trait Remote: Send + Sync {
    fn list<'a>(
        &'a self,
        after_ms: Option<i64>,
        page: Option<&'a str>,
    ) -> Boxed<'a, Result<ListPage, ProviderError>>;

    fn changes_since<'a>(
        &'a self,
        cursor: &'a str,
        page: Option<&'a str>,
    ) -> Boxed<'a, Result<Changes, ProviderError>>;

    fn cursor_now(&self) -> Boxed<'_, Result<String, ProviderError>>;

    fn fetch_headers<'a>(
        &'a self,
        ids: &'a [String],
    ) -> Boxed<'a, Result<Vec<RawHeaders>, ProviderError>>;

    fn fetch_body<'a>(&'a self, id: &'a str) -> Boxed<'a, Result<Vec<u8>, ProviderError>>;

    fn fetch_attachment<'a>(
        &'a self,
        message_id: &'a str,
        attachment_id: &'a str,
    ) -> Boxed<'a, Result<Vec<u8>, ProviderError>>;

    fn set_flags<'a>(
        &'a self,
        ids: &'a [String],
        patch: &'a FlagPatch,
    ) -> Boxed<'a, Result<(), ProviderError>>;

    fn labels(&self) -> Boxed<'_, Result<Vec<ProviderLabel>, ProviderError>>;

    fn set_labels<'a>(
        &'a self,
        ids: &'a [String],
        add: &'a [String],
        remove: &'a [String],
    ) -> Boxed<'a, Result<(), ProviderError>>;

    fn search<'a>(
        &'a self,
        query: &'a str,
        page: Option<&'a str>,
    ) -> Boxed<'a, Result<ListPage, ProviderError>>;

    fn send<'a>(
        &'a self,
        raw: &'a [u8],
        thread_hint: Option<&'a str>,
    ) -> Boxed<'a, Result<SentIds, ProviderError>>;

    fn draft_put<'a>(
        &'a self,
        provider_draft_id: Option<&'a str>,
        raw: &'a [u8],
        thread_hint: Option<&'a str>,
    ) -> Boxed<'a, Result<String, ProviderError>>;

    fn draft_delete<'a>(
        &'a self,
        provider_draft_id: &'a str,
    ) -> Boxed<'a, Result<(), ProviderError>>;
}

impl<P: Provider + Send + Sync> Remote for P {
    fn list<'a>(
        &'a self,
        after_ms: Option<i64>,
        page: Option<&'a str>,
    ) -> Boxed<'a, Result<ListPage, ProviderError>> {
        Box::pin(Provider::list(self, after_ms, page))
    }

    fn changes_since<'a>(
        &'a self,
        cursor: &'a str,
        page: Option<&'a str>,
    ) -> Boxed<'a, Result<Changes, ProviderError>> {
        Box::pin(Provider::changes_since(self, cursor, page))
    }

    fn cursor_now(&self) -> Boxed<'_, Result<String, ProviderError>> {
        Box::pin(Provider::cursor_now(self))
    }

    fn fetch_headers<'a>(
        &'a self,
        ids: &'a [String],
    ) -> Boxed<'a, Result<Vec<RawHeaders>, ProviderError>> {
        Box::pin(Provider::fetch_headers(self, ids))
    }

    fn fetch_body<'a>(&'a self, id: &'a str) -> Boxed<'a, Result<Vec<u8>, ProviderError>> {
        Box::pin(Provider::fetch_body(self, id))
    }

    fn fetch_attachment<'a>(
        &'a self,
        message_id: &'a str,
        attachment_id: &'a str,
    ) -> Boxed<'a, Result<Vec<u8>, ProviderError>> {
        Box::pin(Provider::fetch_attachment(self, message_id, attachment_id))
    }

    fn set_flags<'a>(
        &'a self,
        ids: &'a [String],
        patch: &'a FlagPatch,
    ) -> Boxed<'a, Result<(), ProviderError>> {
        Box::pin(Provider::set_flags(self, ids, patch))
    }

    fn labels(&self) -> Boxed<'_, Result<Vec<ProviderLabel>, ProviderError>> {
        Box::pin(Provider::labels(self))
    }

    fn set_labels<'a>(
        &'a self,
        ids: &'a [String],
        add: &'a [String],
        remove: &'a [String],
    ) -> Boxed<'a, Result<(), ProviderError>> {
        Box::pin(Provider::set_labels(self, ids, add, remove))
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        page: Option<&'a str>,
    ) -> Boxed<'a, Result<ListPage, ProviderError>> {
        Box::pin(Provider::search(self, query, page))
    }

    fn send<'a>(
        &'a self,
        raw: &'a [u8],
        thread_hint: Option<&'a str>,
    ) -> Boxed<'a, Result<SentIds, ProviderError>> {
        Box::pin(Provider::send(self, raw, thread_hint))
    }
    fn draft_put<'a>(
        &'a self,
        provider_draft_id: Option<&'a str>,
        raw: &'a [u8],
        thread_hint: Option<&'a str>,
    ) -> Boxed<'a, Result<String, ProviderError>> {
        Box::pin(Provider::draft_put(self, provider_draft_id, raw, thread_hint))
    }

    fn draft_delete<'a>(
        &'a self,
        provider_draft_id: &'a str,
    ) -> Boxed<'a, Result<(), ProviderError>> {
        Box::pin(Provider::draft_delete(self, provider_draft_id))
    }
}

/// One account's connection, taken for one synchronous unit of work. Nothing inside the closure
/// may await: the guard is a std one, and holding it across a suspension point would make the
/// engine's futures not `Send`.
pub trait Store: Sync {
    fn with<T, F: FnOnce(&Connection) -> Result<T, String>>(&self, f: F) -> Result<T, String>;
}

/// The app's store: the shared connection pool, narrowed to one account.
pub struct Scoped<'a> {
    pub db: &'a Db,
    pub account_id: &'a str,
}

impl Store for Scoped<'_> {
    fn with<T, F: FnOnce(&Connection) -> Result<T, String>>(&self, f: F) -> Result<T, String> {
        self.db.with(self.account_id, f)
    }
}

/// Where progress goes. The engine is written against this rather than against `AppHandle` so a
/// whole pass can run in a test with nothing but a recorder behind it.
pub trait Sink: Sync {
    fn status(&self, status: &SyncStatus);
    /// An invalidation signal with a scope in it, so a note landing does not make the list refetch
    /// its bodies.
    fn changed(&self, reason: &str);
    /// New mail since the last pass, with the facts the app decides on. The engine reports what
    /// arrived and nothing about whether to say so, which is the app's half in `notify::announce`.
    /// Empty by default: a pass nobody is watching announces nothing.
    fn arrived(&self, _account_id: &str, _arrivals: &[crate::notify::Arrival]) {}
}

pub struct AppSink {
    pub app: tauri::AppHandle,
}

impl Sink for AppSink {
    fn status(&self, status: &SyncStatus) {
        let _ = self.app.emit("sync-progress", status.clone());
    }

    fn changed(&self, reason: &str) {
        crate::emit_store_changed(&self.app, reason);
    }

    fn arrived(&self, account_id: &str, arrivals: &[crate::notify::Arrival]) {
        crate::notify::announce(&self.app, account_id, arrivals);
    }
}

/// A sink that wants nothing, for a pass nobody is watching.
pub struct Quiet;

impl Sink for Quiet {
    fn status(&self, _status: &SyncStatus) {}
    fn changed(&self, _reason: &str) {}
}

// ---------------------------------------------------------------------------------------------
// The live accounts
// ---------------------------------------------------------------------------------------------

type Remotes = Mutex<HashMap<String, Arc<dyn Remote>>>;

fn remotes() -> &'static Remotes {
    static REMOTES: OnceLock<Remotes> = OnceLock::new();
    REMOTES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Hands the engine the mailbox for an account. Called by whoever built the provider, which is the
/// Gmail client today and an IMAP one later. This registry is process global rather than Tauri
/// managed state because `lib.rs` belongs to the integrator and a work package that has to edit
/// somebody else's file was cut in the wrong place.
pub fn attach<P: Provider + Send + Sync + 'static>(account_id: &str, provider: P) {
    if let Ok(mut held) = remotes().lock() {
        held.insert(account_id.to_string(), Arc::new(provider));
    }
}

/// Takes the mailbox away again, and the engine with it. A removed account's breaker and pass
/// state would otherwise sit waiting under the same id if it were ever added back, and a remote
/// left attached would run a pass on the next tick and write a fresh, empty mirror where the one
/// the person just deleted was.
pub fn detach(account_id: &str) {
    if let Ok(mut held) = remotes().lock() {
        held.remove(account_id);
    }
    if let Ok(mut held) = engines().lock() {
        held.remove(account_id);
    }
}

pub fn remote_for(account_id: &str) -> Option<Arc<dyn Remote>> {
    remotes().lock().ok()?.get(account_id).cloned()
}

pub fn attached() -> Vec<String> {
    remotes()
        .lock()
        .map(|held| held.keys().cloned().collect())
        .unwrap_or_default()
}

type Engines = Mutex<HashMap<String, Arc<engine::Engine>>>;

fn engines() -> &'static Engines {
    static ENGINES: OnceLock<Engines> = OnceLock::new();
    ENGINES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn engine_for(account_id: &str) -> Arc<engine::Engine> {
    let mut held = engines().lock().expect("the engine registry");
    held.entry(account_id.to_string())
        .or_insert_with(|| Arc::new(engine::Engine::new(account_id)))
        .clone()
}

// ---------------------------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------------------------

/// The accounts with a mirror on disk, and the hue each one paints its rows with. The colour is in
/// the mirror's own `meta` so a list page needs nothing from the accounts package to render; that
/// package writes it there when an account is connected or recoloured.
pub fn accounts(db: &Db, only: Option<&str>) -> Vec<(String, String)> {
    let ids = match only {
        Some(id) => vec![id.to_string()],
        None => {
            let mut ids = db.on_disk();
            ids.sort();
            ids
        }
    };
    ids.into_iter()
        .map(|id| {
            let color = db
                .with(&id, |conn| write::meta_get(conn, write::ACCOUNT_COLOR_KEY))
                .ok()
                .flatten()
                .unwrap_or_else(|| "hue-1".to_string());
            (id, color)
        })
        .collect()
}

fn db_of(app: &tauri::AppHandle) -> Result<tauri::State<'_, Db>, String> {
    app.try_state::<Db>()
        .ok_or_else(|| "the mirror is not open yet".to_string())
}

// ---------------------------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------------------------

/// Starts the poll loop. The integrator calls this from `setup` once the database is managed.
pub fn setup(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut delay = Duration::from_secs(FIRST_PASS_SECS);
        loop {
            tokio::time::sleep(delay).await;
            tick(&app).await;
            delay = Duration::from_secs(if focused(&app) {
                POLL_FOREGROUND_SECS
            } else {
                POLL_BACKGROUND_SECS
            });
        }
    });
}

fn focused(app: &tauri::AppHandle) -> bool {
    app.webview_windows()
        .values()
        .any(|window| window.is_focused().unwrap_or(false))
}

async fn tick(app: &tauri::AppHandle) {
    let Ok(db) = db_of(app) else { return };
    let sink = AppSink { app: app.clone() };
    for account_id in attached() {
        let Some(remote) = remote_for(&account_id) else {
            continue;
        };
        let engine = engine_for(&account_id);
        let store = Scoped {
            db: db.inner(),
            account_id: &account_id,
        };
        engine
            .run_pass(&store, remote.as_ref(), &sink, focused(app))
            .await;

        // A draft written just before a quit has a local row and no provider copy yet. The pass is
        // where it catches up, so a draft roams the way Gmail drafts always have rather than
        // waiting for the composer to be opened again.
        let _ = crate::drafts::upload_pending(app, &account_id).await;
    }
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

/// One account, or every attached account when the id is omitted.
#[tauri::command]
pub async fn sync_now(
    app: tauri::AppHandle,
    account_id: Option<String>,
) -> Result<Vec<SyncStatus>, String> {
    let db = db_of(&app)?;
    let sink = AppSink { app: app.clone() };
    let wanted: Vec<String> = match account_id {
        Some(id) => vec![id],
        None => attached(),
    };
    let mut out = Vec::new();
    for id in wanted {
        let Some(remote) = remote_for(&id) else {
            out.push(SyncStatus::idle(&id));
            continue;
        };
        let engine = engine_for(&id);
        // A sync somebody asked for closes the breaker: they can see the failure and they are
        // telling us to try anyway.
        engine.resume();
        let store = Scoped {
            db: db.inner(),
            account_id: &id,
        };
        out.push(engine.run_pass(&store, remote.as_ref(), &sink, true).await);
    }
    Ok(out)
}

/// A pass for an account that has just been connected, in its own task, so the mail starts
/// arriving now rather than at the next tick. The tick can be a minute away: the consent happened
/// in a browser, so the window is in the background and the loop is on its slow interval.
pub fn kick(app: &tauri::AppHandle, account_id: &str) {
    let app = app.clone();
    let account_id = account_id.to_string();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = sync_now(app, Some(account_id.clone())).await {
            crate::log::note(&account_id, &format!("first pass: {e}"));
        }
    });
}

/// One status per attached account, whether or not a pass has run on it yet.
///
/// An account the engine has not touched this launch answers idle with its last sync read off the
/// mirror, so the front end can tell a mailbox that finished its first sync last week from one
/// that has never finished it at all: the second is still being brought in and its empty places
/// say so, the first is simply empty.
#[tauri::command]
pub async fn sync_status(app: tauri::AppHandle) -> Result<Vec<SyncStatus>, String> {
    let db = db_of(&app)?;
    let mut out = Vec::new();
    for account_id in attached() {
        let mut status = engine_for(&account_id).status();
        if status.last_sync_ms.is_none() {
            status.last_sync_ms = db
                .with(&account_id, |conn| write::meta_i64(conn, write::LAST_SYNC_KEY))
                .ok()
                .flatten();
        }
        out.push(status);
    }
    out.sort_by(|a, b| a.account_id.cmp(&b.account_id));
    Ok(out)
}

/// Drains the outbox. The close-request hook races this against a timeout, so it returns on a
/// budget however deep the queue is.
#[tauri::command]
pub async fn sync_flush(app: tauri::AppHandle) -> Result<(), String> {
    let db = db_of(&app)?;
    for account_id in attached() {
        let Some(remote) = remote_for(&account_id) else {
            continue;
        };
        let store = Scoped {
            db: db.inner(),
            account_id: &account_id,
        };
        let deadline = write::now_ms() + outbox::FLUSH_BUDGET_MS;
        let outcome = outbox::drain(&store, remote.as_ref(), deadline).await;
        if outcome.changed {
            crate::emit_store_changed(&app, "outbox");
        }
    }
    Ok(())
}

/// Fills in the range a widened storage window newly covers, newest first.
#[tauri::command]
pub async fn sync_backfill(app: tauri::AppHandle, account_id: String) -> Result<(), String> {
    let db = db_of(&app)?;
    let remote = remote_for(&account_id).ok_or("that account is not connected")?;
    let sink = AppSink { app: app.clone() };
    let engine = engine_for(&account_id);
    let store = Scoped {
        db: db.inner(),
        account_id: &account_id,
    };
    engine.backfill(&store, remote.as_ref(), &sink).await
}

/// Local search over the window, with the provider as a second pass when the query reaches past
/// it. A query carrying a `before:` or `after:` that lands outside the window skips the local
/// index entirely, because a local answer to that question would be wrong rather than short.
#[tauri::command]
pub async fn search(
    app: tauri::AppHandle,
    account_id: Option<String>,
    query: String,
    cursor: Option<String>,
) -> Result<SearchResult, String> {
    let db = db_of(&app)?;
    let parsed = fts::parse(&query);
    let mut provider_searched = false;

    for (id, _) in accounts(db.inner(), account_id.as_deref()) {
        let reaches = db.with(&id, |conn| {
            let start = write::window_start(conn, write::now_ms())?;
            Ok(parsed.reaches_before(start))
        })?;
        if !reaches {
            continue;
        }
        let Some(remote) = remote_for(&id) else { continue };
        let store = Scoped {
            db: db.inner(),
            account_id: &id,
        };
        // A provider that will not answer is a shorter answer, not a failed search: the local
        // index still holds what the device has.
        if hydrate::from_search(&store, remote.as_ref(), &query).await.is_ok() {
            provider_searched = true;
        }
    }

    let page = list_pages(
        db.inner(),
        &ThreadQuery {
            account_id: account_id.clone(),
            place: crate::dto::Place::Search,
            label_id: None,
            query: Some(query),
            limit: 50,
            cursor,
        },
    )?;
    Ok(SearchResult {
        note: provider_searched
            .then(|| "Older mail was fetched from Gmail for this search.".to_string()),
        provider_searched,
        page: ThreadPage {
            footer: Some("Search older mail on Gmail".to_string()),
            ..page
        },
    })
}

/// The explicit "Search older mail on Gmail" at the foot of a result list.
///
/// The hits are hydrated through the same path a sync uses and land as transient rows, so the next
/// eviction pass takes them away again unless they picked up a pile, a note or some other decision
/// in the meantime. There is no footer on the way back: the offer has been taken.
#[tauri::command]
pub async fn search_provider(
    app: tauri::AppHandle,
    account_id: String,
    query: String,
) -> Result<SearchResult, String> {
    let db = db_of(&app)?;
    let remote = remote_for(&account_id).ok_or("that account is not connected")?;
    let store = Scoped {
        db: db.inner(),
        account_id: &account_id,
    };
    let found = hydrate::from_search(&store, remote.as_ref(), &query)
        .await
        .map_err(|e| e.to_string())?;

    let page = list_pages(
        db.inner(),
        &ThreadQuery {
            account_id: Some(account_id),
            place: crate::dto::Place::Search,
            label_id: None,
            query: Some(query),
            limit: 50,
            cursor: None,
        },
    )?;
    crate::emit_store_changed(&app, "threads");

    Ok(SearchResult {
        note: Some(fetched_note(found)),
        provider_searched: true,
        page: ThreadPage {
            footer: None,
            ..page
        },
    })
}

/// The sentence under the results. It says what the rows are, because a row that arrived from a
/// search and will leave again at the next tidy up is not the same thing as a row that lives here.
fn fetched_note(found: u32) -> String {
    match found {
        0 => "Gmail had nothing else for this search.".to_string(),
        1 => "One older message came from Gmail. It is not kept unless you do something with it."
            .to_string(),
        many => format!(
            "{many} older messages came from Gmail. They are not kept unless you do something \
             with them."
        ),
    }
}

/// One page, across however many accounts the query names. Each account's own page is one SQL
/// statement; more than one account is a merge of already ordered lists and nothing more.
pub fn list_pages(db: &Db, query: &ThreadQuery) -> Result<ThreadPage, String> {
    let now = write::now_ms();
    let accounts = accounts(db, query.account_id.as_deref());
    if accounts.len() == 1 {
        let (id, color) = &accounts[0];
        return db.with(id, |conn| read::threads_list(conn, id, color, query, now));
    }

    let mut threads = Vec::new();
    let mut footer = None;
    for (id, color) in &accounts {
        let page = db.with(id, |conn| read::threads_list(conn, id, color, query, now))?;
        footer = footer.or(page.footer);
        threads.extend(page.threads);
    }
    threads.sort_by(|a, b| {
        read::group_rank(&a.group)
            .cmp(&read::group_rank(&b.group))
            .then(b.date_ms.cmp(&a.date_ms))
    });
    threads.truncate(query.limit as usize);
    Ok(ThreadPage {
        threads,
        // Paging the unified view needs a cursor per account, which is the accounts package's
        // problem and not this milestone's: one account pages, several show their first page.
        next_cursor: None,
        footer,
    })
}

/// Runs an eviction pass when one is due, and takes one immediately when the window shrinks.
pub fn window_set(app: &tauri::AppHandle, account_id: &str, days: i64) -> Result<(), String> {
    let db = db_of(app)?;
    let change = db.with(account_id, |conn| {
        let change = evict::set_window(conn, days, write::now_ms())?;
        if change == evict::WindowChange::Shrunk {
            evict::run(conn, write::now_ms())?;
        }
        Ok(change)
    })?;
    crate::emit_store_changed(app, "threads");
    if change == evict::WindowChange::Widened {
        let app = app.clone();
        let account_id = account_id.to_string();
        tauri::async_runtime::spawn(async move {
            let _ = sync_backfill(app, account_id).await;
        });
    }
    Ok(())
}
