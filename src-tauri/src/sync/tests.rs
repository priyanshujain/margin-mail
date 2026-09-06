// The engine as a state machine, driven against the fake mailbox over a pair of in-memory
// databases. Nothing here touches Google and nothing here can: the engine only ever speaks through
// `Remote`, and the fake is a real little mailbox with paging, a history log and scriptable
// failures rather than a script of canned answers.
//
// Every test asserts on two things: the calls that were made, and the rows that were left behind.
// A sync engine's bugs are about ordering and recovery, and half of them are invisible in the rows
// alone.

use std::sync::Mutex;

use rusqlite::Connection;
use tauri::async_runtime::block_on;

use crate::badge;
use crate::db;
use crate::dto::{Destination, FlagPatch, Place, SyncStatus, ThreadPage, ThreadQuery};
use crate::mirror::{evict, fts, read, write};
use crate::provider::fake::{Call, FakeProvider};
use crate::provider::{Change, Changes, ListPage, Provider, ProviderError, ProviderLabel, RawHeaders, SentIds};
use crate::state::write::{clear_rule, set_rule};

use super::engine::Engine;
use super::{changes, hydrate, outbox, Boxed, Remote, Sink, Store};

// -- the harness -------------------------------------------------------------------------------

struct Memory(Mutex<Connection>);

impl Store for Memory {
    fn with<T, F: FnOnce(&Connection) -> Result<T, String>>(&self, f: F) -> Result<T, String> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        f(&conn)
    }
}

fn store() -> Memory {
    Memory(Mutex::new(db::memory().expect("a pair of in-memory databases")))
}

#[derive(Default)]
struct Recorder {
    statuses: Mutex<Vec<SyncStatus>>,
    reasons: Mutex<Vec<String>>,
}

impl Sink for Recorder {
    fn status(&self, status: &SyncStatus) {
        self.statuses.lock().expect("statuses").push(status.clone());
    }

    fn changed(&self, reason: &str) {
        self.reasons.lock().expect("reasons").push(reason.to_string());
    }
}

impl Recorder {
    fn phases(&self) -> Vec<String> {
        self.statuses
            .lock()
            .expect("statuses")
            .iter()
            .map(|status| status.phase.clone())
            .collect()
    }

    fn totals(&self) -> Vec<u32> {
        self.statuses
            .lock()
            .expect("statuses")
            .iter()
            .map(|status| status.total)
            .collect()
    }

    fn reasons(&self) -> Vec<String> {
        self.reasons.lock().expect("reasons").clone()
    }
}

/// The fake has no way to say a message was deleted, because nothing it offers deletes one. This
/// forwards every call to it and appends the records a test asked for to the next change log, so a
/// deletion arrives through the same path a real one would.
struct Scripted {
    inner: FakeProvider,
    extra: Mutex<Vec<Change>>,
}

impl Scripted {
    fn new(inner: FakeProvider) -> Scripted {
        Scripted {
            inner,
            extra: Mutex::new(Vec::new()),
        }
    }

    fn deletes(&self, id: &str) {
        self.extra
            .lock()
            .expect("extra")
            .push(Change::Deleted(id.to_string()));
    }
}

impl Remote for Scripted {
    fn list<'a>(
        &'a self,
        after_ms: Option<i64>,
        page: Option<&'a str>,
    ) -> Boxed<'a, Result<ListPage, ProviderError>> {
        Remote::list(&self.inner, after_ms, page)
    }

    fn changes_since<'a>(
        &'a self,
        cursor: &'a str,
        page: Option<&'a str>,
    ) -> Boxed<'a, Result<Changes, ProviderError>> {
        Box::pin(async move {
            let mut changes = Remote::changes_since(&self.inner, cursor, page).await?;
            changes
                .changes
                .extend(self.extra.lock().expect("extra").drain(..));
            Ok(changes)
        })
    }

    fn cursor_now(&self) -> Boxed<'_, Result<String, ProviderError>> {
        Remote::cursor_now(&self.inner)
    }

    fn fetch_headers<'a>(
        &'a self,
        ids: &'a [String],
    ) -> Boxed<'a, Result<Vec<RawHeaders>, ProviderError>> {
        Remote::fetch_headers(&self.inner, ids)
    }

    fn fetch_body<'a>(&'a self, id: &'a str) -> Boxed<'a, Result<Vec<u8>, ProviderError>> {
        Remote::fetch_body(&self.inner, id)
    }

    fn fetch_attachment<'a>(
        &'a self,
        message_id: &'a str,
        attachment_id: &'a str,
    ) -> Boxed<'a, Result<Vec<u8>, ProviderError>> {
        Remote::fetch_attachment(&self.inner, message_id, attachment_id)
    }

    fn set_flags<'a>(
        &'a self,
        ids: &'a [String],
        patch: &'a FlagPatch,
    ) -> Boxed<'a, Result<(), ProviderError>> {
        Remote::set_flags(&self.inner, ids, patch)
    }

    fn labels(&self) -> Boxed<'_, Result<Vec<ProviderLabel>, ProviderError>> {
        Remote::labels(&self.inner)
    }

    fn set_labels<'a>(
        &'a self,
        ids: &'a [String],
        add: &'a [String],
        remove: &'a [String],
    ) -> Boxed<'a, Result<(), ProviderError>> {
        Remote::set_labels(&self.inner, ids, add, remove)
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        page: Option<&'a str>,
    ) -> Boxed<'a, Result<ListPage, ProviderError>> {
        Remote::search(&self.inner, query, page)
    }

    fn send<'a>(
        &'a self,
        raw: &'a [u8],
        thread_hint: Option<&'a str>,
    ) -> Boxed<'a, Result<SentIds, ProviderError>> {
        Remote::send(&self.inner, raw, thread_hint)
    }

    fn draft_put<'a>(
        &'a self,
        provider_draft_id: Option<&'a str>,
        raw: &'a [u8],
        thread_hint: Option<&'a str>,
    ) -> Boxed<'a, Result<String, ProviderError>> {
        Remote::draft_put(&self.inner, provider_draft_id, raw, thread_hint)
    }

    fn draft_delete<'a>(
        &'a self,
        provider_draft_id: &'a str,
    ) -> Boxed<'a, Result<(), ProviderError>> {
        Remote::draft_delete(&self.inner, provider_draft_id)
    }
}

// -- the mail ----------------------------------------------------------------------------------

fn days_ago(days: i64) -> i64 {
    write::now_ms() - days * write::DAY_MS
}

fn rfc2822(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .expect("a moment inside the epoch")
        .to_rfc2822()
}

/// A whole RFC 2822 message, built here rather than read from a corpus: the fixture directory
/// belongs to another package and a test that needs a folded `References` should say so in its own
/// source.
fn eml(
    message_id: &str,
    from: &str,
    subject: &str,
    at_ms: i64,
    extra: &[(&str, &str)],
    body: &str,
) -> Vec<u8> {
    let mut out = String::new();
    out.push_str(&format!("Message-ID: <{message_id}>\r\n"));
    out.push_str(&format!("From: {from}\r\n"));
    out.push_str("To: You <you@example.com>\r\n");
    out.push_str(&format!("Subject: {subject}\r\n"));
    out.push_str(&format!("Date: {}\r\n", rfc2822(at_ms)));
    for (name, value) in extra {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(body);
    out.into_bytes()
}

/// A bare date is midnight UTC of that day, whichever way it was punctuated.
fn midnight_utc(year: i32, month: u32, day: u32) -> i64 {
    chrono::NaiveDate::from_ymd_opt(year, month, day)
        .expect("a real date")
        .and_hms_opt(0, 0, 0)
        .expect("midnight")
        .and_utc()
        .timestamp_millis()
}

fn query(place: Place) -> ThreadQuery {
    ThreadQuery {
        account_id: None,
        place,
        label_id: None,
        query: None,
        limit: 50,
        cursor: None,
    }
}

fn list(store: &Memory, query: &ThreadQuery) -> ThreadPage {
    store
        .with(|conn| read::threads_list(conn, "acct", "hue-1", query, write::now_ms()))
        .expect("a list page")
}

fn count(store: &Memory, sql: &str) -> u32 {
    store.with(|conn| write::count(conn, sql)).expect("a count")
}

fn meta(store: &Memory, key: &str) -> Option<String> {
    store.with(|conn| write::meta_get(conn, key)).expect("meta")
}

fn set_window(store: &Memory, days: i64) {
    store
        .with(|conn| write::meta_set(conn, write::WINDOW_KEY, &days.to_string()))
        .expect("the window");
}

fn pass(store: &Memory, engine: &Engine, remote: &dyn Remote, sink: &Recorder) -> SyncStatus {
    block_on(engine.run_pass(store, remote, sink, false))
}

/// Sixty ordinary messages inside the window, which is enough for the listing to page and for
/// hydration to run more than one batch.
fn a_mailbox(fake: &FakeProvider, count: usize) {
    for index in 0..count {
        let id = format!("m{index}");
        fake.add_eml(
            &id,
            &format!("t{index}"),
            &["INBOX", "UNREAD"],
            &eml(
                &format!("{id}@example.test"),
                "Ana <ana@example.test>",
                &format!("Message {index}"),
                days_ago((index % 20) as i64 + 1),
                &[],
                "Hello there.",
            ),
        );
    }
}

// -- the first sync ----------------------------------------------------------------------------

#[test]
fn a_first_sync_lists_once_hydrates_in_batches_and_commits_the_cursor_at_the_end() {
    let store = store();
    let fake = FakeProvider::new();
    fake.set_page_size(25);
    a_mailbox(&fake, 60);
    let engine = Engine::new("acct");
    let sink = Recorder::default();

    let status = pass(&store, &engine, &fake, &sink);

    assert_eq!(status.phase, "idle", "{:?}", status.error);
    let calls = fake.calls();
    let lists: Vec<&String> = calls.iter().filter(|c| c.starts_with("list ")).collect();
    assert_eq!(lists.len(), 3, "one pass through the ids, paged: {calls:?}");
    let batches: Vec<&String> = calls
        .iter()
        .filter(|c| c.starts_with("fetch_headers"))
        .collect();
    assert_eq!(
        batches,
        vec!["fetch_headers 50", "fetch_headers 10"],
        "hydration runs in batches of fifty"
    );

    assert_eq!(count(&store, "SELECT COUNT(*) FROM messages"), 60);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM threads"), 60);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM messages WHERE hydrated = 0"), 0);

    // The cursor is taken before the crawl starts, so anything that arrives during it is picked up
    // by the first incremental pass rather than missed.
    assert_eq!(meta(&store, write::CURSOR_KEY), Some("60".to_string()));
    assert_eq!(meta(&store, write::FIRST_SYNC_KEY), Some("1".to_string()));

    assert!(sink.phases().contains(&"hydrating".to_string()));
    assert!(sink.totals().contains(&60), "the progress bar has a total");
    assert_eq!(sink.phases().last(), Some(&"idle".to_string()));
}

#[test]
fn a_first_sync_whose_crawl_fails_keeps_its_cursor_and_finishes_on_the_next_pass() {
    // The bug this pins down, because it shipped and it was the worst kind: silent and permanent.
    //
    // The cursor used to be written after the metadata crawl. One failed batch out of a thousand
    // therefore discarded the whole pass, the next pass saw no cursor, re-listed and re-hydrated
    // the entire mailbox, and on a real account with an ordinary flaky connection the first sync
    // never finished at all. The account sat re-crawling itself every twelve seconds for ever,
    // which is what a person sees as an app that is slow for no reason, and the screener seed sits
    // behind that line so it never ran either: every sender waited in the Screener and the Inbox
    // showed a handful of threads out of a thousand.
    let store = store();
    let fake = FakeProvider::new();
    a_mailbox(&fake, 60);
    fake.fail_next(
        Call::Headers,
        ProviderError::Network("a train tunnel".to_string()),
    );
    let engine = Engine::new("acct");
    let sink = Recorder::default();

    // The crawl failed, so the pass did; a first network failure is kept quiet, so the status
    // does not say so, and what does is the mailbox below: listed, and not yet hydrated.
    let first = pass(&store, &engine, &fake, &sink);
    assert!(first.error.is_none(), "one failure is the account's own business: {first:?}");
    // The listing did succeed, so what the mailbox contains is known and the cursor is committed.
    assert!(
        meta(&store, write::CURSOR_KEY).is_some(),
        "the cursor survives a crawl that did not finish"
    );
    assert!(count(&store, "SELECT COUNT(*) FROM messages WHERE hydrated = 0") > 0);

    // A failure rests the account, and a person pressing sync is what closes that. Without this the
    // second pass would return early and prove nothing.
    engine.resume();
    let second = pass(&store, &engine, &fake, &sink);

    assert_eq!(second.phase, "idle", "{:?}", second.error);
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM messages WHERE hydrated = 0"),
        0,
        "the backlog drains on the next pass"
    );
    let lists = fake
        .calls()
        .iter()
        .filter(|call| call.starts_with("list "))
        .count();
    assert_eq!(lists, 1, "the mailbox is listed once, not once per pass");
}

#[test]
fn the_screener_seed_runs_on_a_later_pass_when_the_first_one_did_not_finish() {
    // The seed used to sit immediately after the first sync returned, so a first sync that failed
    // halfway meant it never ran at all, on an account where it is the whole difference between a
    // usable Inbox and a screening queue with every correspondent in it.
    let store = store();
    let fake = FakeProvider::new();
    a_mailbox(&fake, 60);
    fake.fail_next(
        Call::Headers,
        ProviderError::Network("a train tunnel".to_string()),
    );
    let engine = Engine::new("acct");
    let sink = Recorder::default();

    pass(&store, &engine, &fake, &sink);
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM state.sender_rules"),
        0,
        "nothing is screened in while the mailbox is still half unknown"
    );

    engine.resume();
    pass(&store, &engine, &fake, &sink);

    assert!(
        count(&store, "SELECT COUNT(*) FROM state.sender_rules") > 0,
        "everyone the account already knows is screened in once the crawl catches up"
    );
}

#[test]
fn a_crawl_picked_up_on_a_later_pass_says_it_is_hydrating_and_how_far_it_has_got() {
    // The pass that finishes an interrupted first sync used to drain the backlog under the plain
    // "syncing" of an ordinary poll, with no sentence and no count. An account still bringing in
    // a thousand messages therefore looked idle, and its empty Inbox looked empty on purpose.
    let store = store();
    let fake = FakeProvider::new();
    a_mailbox(&fake, 60);
    fake.fail_next(
        Call::Headers,
        ProviderError::Network("a train tunnel".to_string()),
    );
    let engine = Engine::new("acct");

    pass(&store, &engine, &fake, &Recorder::default());
    assert_eq!(count(&store, "SELECT COUNT(*) FROM messages WHERE hydrated = 0"), 60);

    engine.resume();
    let sink = Recorder::default();
    let status = pass(&store, &engine, &fake, &sink);

    assert_eq!(status.phase, "idle", "{:?}", status.error);
    let reported = sink.statuses.lock().expect("statuses").clone();
    let crawl: Vec<&SyncStatus> = reported.iter().filter(|s| s.phase == "hydrating").collect();
    assert!(!crawl.is_empty(), "the resumed crawl reports as hydrating: {:?}", sink.phases());
    assert!(
        crawl.iter().all(|s| s.total == 60 && s.message.is_some()),
        "with the whole mailbox as its total and a sentence to print: {crawl:?}"
    );
    assert_eq!(
        crawl.last().map(|s| s.hydrated),
        Some(60),
        "and the count reaches the total"
    );
    assert_eq!(status.total, 0, "the counts are put down again once the crawl is over");
    assert_eq!(count(&store, "SELECT COUNT(*) FROM messages WHERE hydrated = 0"), 0);
}

#[test]
fn a_pass_with_new_mail_does_not_call_one_batch_of_it_a_crawl() {
    let store = store();
    let fake = FakeProvider::new();
    a_mailbox(&fake, 10);
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());

    for index in 10..13 {
        let id = format!("m{index}");
        fake.add_eml(
            &id,
            &format!("t{index}"),
            &["INBOX", "UNREAD"],
            &eml(&format!("{id}@example.test"), "Ana <ana@example.test>", "New", days_ago(0), &[], "Hi"),
        );
    }
    let sink = Recorder::default();
    pass(&store, &engine, &fake, &sink);
    assert!(
        !sink.phases().contains(&"hydrating".to_string()),
        "three new messages are one fetch, not a progress bar: {:?}",
        sink.phases()
    );
}

#[test]
fn the_screener_is_not_seeded_before_the_crawl_has_finished_however_often_it_is_asked() {
    // The command behind the first-run panel used to seed whatever the mirror held the moment it
    // was called, and it was called the moment consent came back. On an empty mirror that marked
    // the seed done with a count of nobody, and every sender of the account waited in the Screener
    // for ever.
    use crate::routing::{seed_if_ready, Seeded};

    let store = store();
    let fake = FakeProvider::new();
    a_mailbox(&fake, 60);
    fake.fail_next(
        Call::Headers,
        ProviderError::Network("a train tunnel".to_string()),
    );
    let engine = Engine::new("acct");

    assert_eq!(
        store.with(seed_if_ready).expect("asked on an empty mirror"),
        Seeded::NotYet
    );
    pass(&store, &engine, &fake, &Recorder::default());
    assert_eq!(
        store.with(seed_if_ready).expect("asked mid-crawl"),
        Seeded::NotYet,
        "listed but not hydrated is not ready either"
    );
    assert_eq!(
        meta(&store, "screener-seeded"),
        None,
        "a refusal leaves no mark, so the seed can still run"
    );

    engine.resume();
    let sink = Recorder::default();
    pass(&store, &engine, &fake, &sink);
    match store.with(seed_if_ready).expect("asked after the crawl") {
        Seeded::Already(screened) => assert!(screened > 0, "the engine seeded once the crawl was done"),
        other => panic!("expected the seed to have run, got {other:?}"),
    }
    assert!(
        sink.reasons().iter().any(|reason| reason.contains("screener")),
        "and the pass said so, or the Inbox would not refetch: {:?}",
        sink.reasons()
    );
}

#[test]
fn a_seed_that_found_nobody_on_an_empty_mirror_is_run_once_more_when_the_mail_is_in() {
    // The state the shipped bug left behind: seeded, count of nobody, on a mirror that had nothing
    // in it at the time. Every sender waited in the Screener for ever. The repair is not a step
    // anybody takes; it is the next pass after the crawl finishes.
    use crate::routing::{seed_if_ready, seed_once, Seeded};

    let store = store();
    let fake = FakeProvider::new();
    a_mailbox(&fake, 20);
    assert_eq!(store.with(seed_once).expect("the early seed"), 0);
    assert_eq!(meta(&store, "screener-seeded"), Some("1".to_string()));

    let engine = Engine::new("acct");
    let sink = Recorder::default();
    pass(&store, &engine, &fake, &sink);

    assert!(
        count(&store, "SELECT COUNT(*) FROM state.sender_rules") > 0,
        "the pass that finished the crawl ran the seed again"
    );
    assert!(
        sink.reasons().iter().any(|reason| reason.contains("screener")),
        "and said so: {:?}",
        sink.reasons()
    );
    match store.with(seed_if_ready).expect("asked afterwards") {
        Seeded::Already(screened) => assert!(screened > 0),
        other => panic!("expected the repaired count, got {other:?}"),
    }

    // A second nobody would be a mailbox where everyone has a rule, and that is not run again.
    let bare = super::tests::store();
    bare.with(|conn| {
        write::meta_set(conn, write::FIRST_SYNC_KEY, "1")?;
        Ok(())
    })
    .expect("a ready, empty mirror");
    assert_eq!(bare.with(seed_once).expect("seed"), 0);
    assert_eq!(bare.with(seed_if_ready).expect("once more"), Seeded::Ran(0));
    assert_eq!(bare.with(seed_if_ready).expect("and no more"), Seeded::Already(0));
}

#[test]
fn a_window_chosen_before_the_first_sync_is_covered_by_it_and_queues_no_second_listing() {
    // The window is picked on the panel before the first pass, and written through the path a
    // change from Settings takes, which queues a backfill to the new edge. The first sync lists
    // that whole window itself, so the queued backfill would list it again for nothing.
    let store = store();
    let fake = FakeProvider::new();
    a_mailbox(&fake, 20);
    let change = store
        .with(|conn| evict::set_window(conn, 365, write::now_ms()))
        .expect("the window");
    assert_eq!(change, evict::WindowChange::Widened);
    assert!(store.with(evict::backfill_target).expect("target").is_some());

    let engine = Engine::new("acct");
    let sink = Recorder::default();
    let status = pass(&store, &engine, &fake, &sink);

    assert_eq!(status.phase, "idle", "{:?}", status.error);
    assert_eq!(store.with(evict::backfill_target).expect("target"), None);
    assert!(
        !sink.phases().contains(&"backfilling".to_string()),
        "the window was listed once, by the first sync: {:?}",
        sink.phases()
    );
    assert_eq!(meta(&store, write::WINDOW_KEY), Some("365".to_string()));
}

#[test]
fn a_body_whose_render_is_stale_still_opens_and_renders_itself_again() {
    let store = store();
    let fake = FakeProvider::new();
    fake.add_eml(
        "m1",
        "t1",
        &["INBOX"],
        &eml("m1@example.test", "Ana <ana@example.test>", "Hello", days_ago(1), &[], "Body"),
    );
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());
    block_on(hydrate::body(&store, &fake, "m1")).expect("a body");

    // What a body looks like when the pipeline could not read it: the bytes are kept, the render
    // is empty, and the stamp is not the current one. That is the state every cached body is left
    // in by a change to the sanitiser, and the recovery is the same either way.
    store
        .with(|conn| {
            conn.execute(
                "UPDATE bodies SET html = '', quoted_html = NULL, render_version = 0
                 WHERE message_id = 'm1'",
                [],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .expect("an unrendered body");

    let view = store
        .with(|conn| read::thread_view(conn, "acct", "m1@example.test", &[]))
        .expect("the thread still opens");
    assert_eq!(view.messages.len(), 1, "headers are enough to open a thread");
    assert_eq!(view.messages[0].html, "");
    assert_eq!(view.subject, "Hello");

    let stale = store
        .with(|conn| read::stale_renders(conn, "m1@example.test"))
        .expect("stale renders");
    assert_eq!(stale, vec!["m1".to_string()], "and it knows it is out of date");

    let done = store
        .with(|conn| {
            let options = hydrate::render_options(conn)?;
            write::rerender(conn, "m1", &options)
        })
        .expect("a second render");
    assert!(done, "the raw bytes it kept are what it renders again from");
    let stamp: i64 = store
        .with(|conn| {
            conn.query_row(
                "SELECT render_version FROM bodies WHERE message_id = 'm1'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
        })
        .expect("the render stamp");
    assert_eq!(stamp, crate::mime::RENDER_VERSION as i64);
    assert!(store
        .with(|conn| read::stale_renders(conn, "m1@example.test"))
        .expect("stale renders")
        .is_empty());
}

#[test]
fn the_surface_a_body_was_decided_onto_is_what_the_pane_gets_back() {
    // The whole point of the column. The decision is a DOM walk and an open is a local read, so it
    // is made once when the body is stored and read back with the row rather than worked out again
    // every time somebody opens the thread.
    let store = store();
    let fake = FakeProvider::new();
    let html = [("Content-Type", "text/html; charset=utf-8")];
    fake.add_eml(
        "painted",
        "t1",
        &["INBOX"],
        &eml(
            "painted@example.test",
            "The Long Read <hello@example.test>",
            "The weekend edition",
            days_ago(1),
            &html,
            "<table width=\"100%\" bgcolor=\"#f4f1ea\"><tr><td>Three pieces</td></tr></table>",
        ),
    );
    fake.add_eml(
        "bare",
        "t2",
        &["INBOX"],
        &eml(
            "bare@example.test",
            "Arun <arun@example.test>",
            "Three things",
            days_ago(1),
            &html,
            "<div><p>Morning,</p><p>Three things <b>before</b> Thursday.</p></div>",
        ),
    );
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());
    block_on(hydrate::body(&store, &fake, "painted")).expect("a body");
    block_on(hydrate::body(&store, &fake, "bare")).expect("a body");

    let painted = store
        .with(|conn| read::thread_view(conn, "acct", "painted@example.test", &[]))
        .expect("the painted thread opens");
    assert_eq!(painted.messages[0].surface, crate::dto::Surface::Paper);

    let bare = store
        .with(|conn| read::thread_view(conn, "acct", "bare@example.test", &[]))
        .expect("the bare thread opens");
    assert_eq!(bare.messages[0].surface, crate::dto::Surface::Theme);
    // Both arrived as HTML. That is the distinction the old rule could not draw.
    assert!(painted.messages[0].is_html && bare.messages[0].is_html);
}

// -- opening a thread and caching bodies -------------------------------------------------------

/// One conversation of several messages, all replying to the first, with nothing cached. That is
/// what every thread on a mailbox that has only been listed looks like, and it is the shape the
/// reading pane has to open without waiting.
fn a_conversation(fake: &FakeProvider, messages: usize) {
    for index in 0..messages {
        let id = format!("c{index}");
        let reply = [
            ("References", "<c0@example.test>"),
            ("In-Reply-To", "<c0@example.test>"),
        ];
        fake.add_eml(
            &id,
            "conversation",
            &["INBOX"],
            &eml(
                &format!("{id}@example.test"),
                "Ana <ana@example.test>",
                "Lunch",
                days_ago((messages - index) as i64),
                if index == 0 { &[] } else { &reply[..] },
                "Body of the message.",
            ),
        );
    }
}

/// The key of the only thread in the Inbox, read back rather than spelled out, because which
/// message a conversation is keyed on is the threader's business and not this test's.
fn only_key(store: &Memory) -> String {
    let page = list(store, &query(Place::Inbox));
    assert_eq!(page.threads.len(), 1, "one conversation");
    page.threads[0].key.clone()
}

fn opened(store: &Memory, key: &str) -> crate::dto::ThreadView {
    store
        .with(|conn| read::thread_view(conn, "acct", key, &[]))
        .expect("the thread opens")
}

#[test]
fn opening_a_thread_reads_the_mirror_and_asks_the_provider_for_nothing() {
    // The regression this pins down cost seconds on every open. The command used to await each
    // missing body before it read a single row, one round trip after another, so a five message
    // thread that had not been cached was five serial fetches before anything could be drawn.
    // Nothing on the path that returns a thread may touch the network.
    let store = store();
    let fake = FakeProvider::new();
    a_conversation(&fake, 5);
    // What anything reaching for a body on this path would get, so a fetch here fails loudly
    // rather than passing quietly.
    fake.fail_next(Call::Body, ProviderError::Network("a train tunnel".to_string()));
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());

    let view = opened(&store, &only_key(&store));

    assert_eq!(view.messages.len(), 5, "every message, without its body");
    assert!(view.messages.iter().all(|m| m.body_pending), "and each says so");
    assert!(view.messages.iter().all(|m| m.html.is_empty()));
    assert_eq!(view.subject, "Lunch", "the thread is a thread already");
    assert_eq!(view.messages[0].from.address, "ana@example.test");
    assert!(view.messages[0].date_ms > 0);
    assert!(
        !fake.calls().iter().any(|call| call.starts_with("fetch_body")),
        "opening a thread fetched a body: {:?}",
        fake.calls()
    );
}

#[test]
fn hydrating_a_thread_fills_in_the_bodies_the_open_did_not_wait_for() {
    let store = store();
    let fake = FakeProvider::new();
    a_conversation(&fake, 5);
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());
    let key = only_key(&store);

    let fetched = block_on(hydrate::thread_bodies(&store, &fake, &key)).expect("the bodies");
    assert_eq!(fetched, 5);

    let view = opened(&store, &key);
    assert!(view.messages.iter().all(|m| !m.body_pending), "nothing is waiting now");
    assert!(view.messages.iter().all(|m| !m.html.is_empty()));
}

#[test]
fn a_body_that_will_not_come_leaves_the_rest_of_the_thread_in_place() {
    let store = store();
    let fake = FakeProvider::new();
    a_conversation(&fake, 5);
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());
    let key = only_key(&store);

    fake.fail_next(Call::Body, ProviderError::NotFound);
    let fetched = block_on(hydrate::thread_bodies(&store, &fake, &key))
        .expect("four of five is a thread worth reading, not an error");
    assert_eq!(fetched, 4);

    let view = opened(&store, &key);
    assert_eq!(
        view.messages.iter().filter(|m| m.body_pending).count(),
        1,
        "the one that failed is still waiting, so the next ask asks for it again"
    );
    assert_eq!(view.messages.iter().filter(|m| !m.html.is_empty()).count(), 4);
}

#[test]
fn the_body_cache_drains_its_backlog_over_several_passes_and_says_so_while_it_does() {
    // Five bodies a pass is 25 a minute, which never caught up with a window of a few hundred
    // messages, so in practice every thread anybody opened was a cold fetch, for ever. The cache
    // has to finish, and it has to be visible while it runs: an account that is busy for several
    // minutes and says nothing is an account that looks broken.
    let store = store();
    let fake = FakeProvider::new();
    a_mailbox(&fake, 100);
    let engine = Engine::new("acct");
    let sink = Recorder::default();

    let first = block_on(engine.run_pass(&store, &fake, &sink, true));
    assert_eq!(first.phase, "caching", "{:?}", first.error);
    assert_eq!(first.message.as_deref(), Some("Caching recent mail"));
    assert_eq!((first.hydrated, first.total), (40, 100));
    assert_eq!(count(&store, "SELECT COUNT(*) FROM bodies"), 40, "a batch, not five");

    let second = block_on(engine.run_pass(&store, &fake, &sink, true));
    assert_eq!(second.phase, "caching");
    assert_eq!((second.hydrated, second.total), (80, 100));
    assert_eq!(count(&store, "SELECT COUNT(*) FROM bodies"), 80);

    let third = block_on(engine.run_pass(&store, &fake, &sink, true));
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM bodies"),
        100,
        "the backlog drains rather than running for ever"
    );
    assert_eq!(third.phase, "idle", "and the account goes quiet when it has");
    assert_eq!((third.hydrated, third.total), (0, 0));
    assert!(
        sink.reasons().iter().any(|reason| reason == "thread"),
        "a body landing is a thread that changed, not a list that did: {:?}",
        sink.reasons()
    );
}

/// A mailbox whose messages are strictly newest first, an hour apart, so "the head of the queue"
/// means something exact: `p0` is the newest and the body cache asks for it first. The mailbox the
/// other tests use has five messages sharing each date, which is fine for counting and no use at
/// all for saying which one gets asked for.
fn a_queue(fake: &FakeProvider, count: usize) {
    for index in 0..count {
        let id = format!("p{index}");
        fake.add_eml(
            &id,
            &format!("q{index}"),
            &["INBOX"],
            &eml(
                &format!("{id}@example.test"),
                "Ana <ana@example.test>",
                &format!("Message {index}"),
                write::now_ms() - (index as i64 + 1) * 3_600_000,
                &[],
                "Hello there.",
            ),
        );
    }
}

fn asked_for(fake: &FakeProvider, id: &str) -> usize {
    let wanted = format!("fetch_body {id}");
    fake.calls().iter().filter(|call| **call == wanted).count()
}

#[test]
fn a_body_the_provider_will_never_give_up_does_not_stall_the_ones_behind_it() {
    // The sibling of the cursor bug above, and the reason that one is worth remembering. A message
    // deleted upstream between the listing and the fetch answers 404 for ever and sorts newest
    // first like anything else, so the cache asked for the same head of the queue every pass and
    // never reached the mail behind it. At five bodies a pass that was a slow trickle. At forty it
    // is a hard stall, which is the difference that matters.
    let store = store();
    let fake = FakeProvider::new();
    a_queue(&fake, 50);
    for index in 0..5 {
        fake.withhold_body(&format!("p{index}"));
    }
    let engine = Engine::new("acct");
    let sink = Recorder::default();

    // Forty asked for, five of them refused.
    let first = block_on(engine.run_pass(&store, &fake, &sink, true));
    assert_eq!(first.phase, "caching", "{:?}", first.error);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM bodies"), 35);

    // The ten behind them, which is the whole point: the refused five are not asked for again and
    // they are not counted as outstanding either, so the pass that clears the last of the real
    // backlog is the pass that goes quiet.
    let second = block_on(engine.run_pass(&store, &fake, &sink, true));
    assert_eq!(count(&store, "SELECT COUNT(*) FROM bodies"), 45);
    assert_eq!(second.phase, "idle");
    assert_eq!((second.hydrated, second.total), (0, 0));

    block_on(engine.run_pass(&store, &fake, &sink, true));
    assert_eq!(asked_for(&fake, "p0"), 1, "asked once, not once a pass for ever");
    assert_eq!(count(&store, "SELECT COUNT(*) FROM bodies"), 45);

    // Pressing sync is "try anyway", and it means all of it. Without this the only way back for a
    // message refused on a train is to restart the app.
    engine.resume();
    block_on(engine.run_pass(&store, &fake, &sink, true));
    assert_eq!(asked_for(&fake, "p0"), 2, "and asked again when somebody asks for it");
}

#[test]
fn a_search_hit_on_its_way_back_out_is_not_worth_a_body() {
    // Rows a provider search pulled in are transient: the next eviction pass takes them away again
    // unless they gained a pile or a note in the meantime. A body fetched for one is 20 units spent
    // on a message already on its way out, and a hundred of them at the head of the queue after one
    // search is the head of line stall again by another route.
    let store = store();
    let fake = FakeProvider::new();
    a_queue(&fake, 50);
    let engine = Engine::new("acct");
    let sink = Recorder::default();

    // A background pass, so the metadata lands and the cache does not run yet.
    pass(&store, &engine, &fake, &sink);

    // The five newest, as `hydrate::from_search` would have left them: transient on the message
    // and, since each is the only message in its thread, transient on the thread as well.
    store
        .with(|conn| {
            for index in 0..5 {
                conn.execute(
                    "UPDATE messages SET transient = 1 WHERE id = ?1",
                    [format!("p{index}")],
                )
                .map_err(|e| e.to_string())?;
                write::refresh_thread(conn, &format!("q{index}"))?;
            }
            Ok(())
        })
        .expect("five rows from a search");

    let first = block_on(engine.run_pass(&store, &fake, &sink, true));
    assert_eq!(
        (first.hydrated, first.total),
        (40, 45),
        "the five are not outstanding work, so they are not in the total either"
    );

    let second = block_on(engine.run_pass(&store, &fake, &sink, true));
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM bodies"),
        45,
        "the durable mail behind them is cached to the end"
    );
    assert_eq!(second.phase, "idle");
    for index in 0..5 {
        assert_eq!(
            asked_for(&fake, &format!("p{index}")),
            0,
            "a body was fetched for a row that is about to be evicted"
        );
    }
}

#[test]
fn the_cache_forgets_a_message_it_could_not_fetch_once_the_body_arrives() {
    // What the skip list is allowed to hold: only what is actually still missing. Opening a thread
    // asks for its bodies whatever the cache gave up on, so a message somebody has since read has
    // to leave the list rather than sitting in it for the rest of the session and taking a slot
    // from one that is genuinely unfetchable.
    let store = store();
    let fake = FakeProvider::new();
    a_queue(&fake, 3);
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());

    let asked: Vec<String> = (0..3).map(|index| format!("p{index}")).collect();
    let missing = |ids: &[String]| {
        store
            .with(|conn| read::still_bodiless(conn, ids))
            .expect("what is still missing")
    };
    assert_eq!(missing(&asked), asked, "nothing is cached, so nothing is forgotten");

    block_on(hydrate::body(&store, &fake, "p1")).expect("a body by another route");
    assert_eq!(
        missing(&asked),
        vec!["p0".to_string(), "p2".to_string()],
        "the one that arrived is dropped and the rest are kept"
    );
}

// -- the incremental pass ----------------------------------------------------------------------

#[test]
fn a_reply_lands_in_its_thread_and_moves_it_to_the_new_group() {
    let store = store();
    let fake = FakeProvider::new();
    fake.add_eml(
        "a1",
        "t1",
        &["INBOX"],
        &eml("a1@example.test", "Ana <ana@example.test>", "Lunch", days_ago(3), &[], "Free?"),
    );
    let engine = Engine::new("acct");
    let sink = Recorder::default();
    pass(&store, &engine, &fake, &sink);

    let page = list(&store, &query(Place::Inbox));
    assert_eq!(page.threads.len(), 1);
    assert_eq!(page.threads[0].group, "seen");
    assert_eq!(page.threads[0].key, "a1@example.test");
    assert_eq!(page.threads[0].message_count, 1);

    fake.add_eml(
        "a2",
        "t1",
        &["INBOX", "UNREAD"],
        &eml(
            "a2@example.test",
            "Ana <ana@example.test>",
            "Re: Lunch",
            days_ago(1),
            &[("References", "<a1@example.test>"), ("In-Reply-To", "<a1@example.test>")],
            "Yes.",
        ),
    );

    let status = pass(&store, &engine, &fake, &sink);
    assert_eq!(status.phase, "idle", "{:?}", status.error);

    let page = list(&store, &query(Place::Inbox));
    assert_eq!(page.threads.len(), 1, "still one thread");
    assert_eq!(page.threads[0].group, "new", "an unseen message moves it up");
    assert_eq!(page.threads[0].message_count, 2);
    assert_eq!(
        page.threads[0].key, "a1@example.test",
        "the key is the head of the conversation, not the newest message"
    );
}

#[test]
fn a_deletion_from_the_change_log_takes_the_message_and_its_thread() {
    let store = store();
    let fake = FakeProvider::new();
    fake.add_eml(
        "a1",
        "t1",
        &["INBOX"],
        &eml("a1@example.test", "Ana <ana@example.test>", "One", days_ago(2), &[], "One"),
    );
    fake.add_eml(
        "b1",
        "t2",
        &["INBOX"],
        &eml("b1@example.test", "Bo <bo@example.test>", "Two", days_ago(2), &[], "Two"),
    );
    let remote = Scripted::new(fake);
    let engine = Engine::new("acct");
    let sink = Recorder::default();
    pass(&store, &engine, &remote, &sink);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM threads"), 2);

    remote.deletes("a1");
    pass(&store, &engine, &remote, &sink);

    assert_eq!(count(&store, "SELECT COUNT(*) FROM messages"), 1);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM threads"), 1);
    let page = list(&store, &query(Place::Inbox));
    assert_eq!(page.threads.len(), 1);
    assert_eq!(page.threads[0].key, "b1@example.test");
}

#[test]
fn a_label_change_from_the_provider_reaches_the_view() {
    let store = store();
    let fake = FakeProvider::new();
    fake.add_eml(
        "a1",
        "t1",
        &["INBOX"],
        &eml("a1@example.test", "Ana <ana@example.test>", "One", days_ago(2), &[], "One"),
    );
    let engine = Engine::new("acct");
    let sink = Recorder::default();
    pass(&store, &engine, &fake, &sink);
    assert!(list(&store, &query(Place::Starred)).threads.is_empty());

    block_on(Provider::set_flags(
        &fake,
        &["a1".to_string()],
        &FlagPatch {
            starred: Some(true),
            archived: Some(true),
            ..FlagPatch::default()
        },
    ))
    .expect("the provider stars it");

    pass(&store, &engine, &fake, &sink);

    let starred = list(&store, &query(Place::Starred));
    assert_eq!(starred.threads.len(), 1, "the star arrived");
    assert!(
        list(&store, &query(Place::Inbox)).threads.is_empty(),
        "and so did the archive"
    );
    assert_eq!(list(&store, &query(Place::Everything)).threads.len(), 1);
}

#[test]
fn an_expired_change_log_recovers_by_listing_again_and_keeps_what_was_decided_locally() {
    let store = store();
    let fake = FakeProvider::new();
    fake.add_eml(
        "a1",
        "t1",
        &["INBOX"],
        &eml("a1@example.test", "Ana <ana@example.test>", "One", days_ago(2), &[], "One"),
    );
    let engine = Engine::new("acct");
    let sink = Recorder::default();
    pass(&store, &engine, &fake, &sink);

    store
        .with(|conn| {
            conn.execute(
                "INSERT INTO state.piles (thread_key, pile) VALUES ('a1@example.test', 'set-aside')",
                [],
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO state.notes (id, thread_key, body, created_at)
                 VALUES ('n1', 'a1@example.test', 'ring back', 1)",
                [],
            )
            .map_err(|e| e.to_string())
        })
        .expect("two decisions");

    fake.add_eml(
        "b1",
        "t2",
        &["INBOX"],
        &eml("b1@example.test", "Bo <bo@example.test>", "Two", days_ago(1), &[], "Two"),
    );
    fake.expire_history_below(1_000);

    let before = fake.calls().len();
    let status = pass(&store, &engine, &fake, &sink);
    assert_eq!(status.phase, "idle", "{:?}", status.error);

    let after: Vec<String> = fake.calls().split_off(before);
    assert!(
        after.iter().any(|call| call.starts_with("changes_since")),
        "it tried the cheap path first: {after:?}"
    );
    assert!(
        after.iter().any(|call| call.starts_with("list ")),
        "and fell back to a full list: {after:?}"
    );

    assert_eq!(count(&store, "SELECT COUNT(*) FROM messages"), 2, "the new one landed");
    assert_eq!(count(&store, "SELECT COUNT(*) FROM state.piles"), 1, "the pile survived");
    assert_eq!(count(&store, "SELECT COUNT(*) FROM state.notes"), 1, "the note survived");
}

#[test]
fn a_message_the_provider_no_longer_lists_is_removed_by_the_recovery_pass() {
    let store = store();
    let fake = FakeProvider::new();
    fake.add_eml(
        "a1",
        "t1",
        &["INBOX"],
        &eml("a1@example.test", "Ana <ana@example.test>", "One", days_ago(2), &[], "One"),
    );
    let engine = Engine::new("acct");
    let sink = Recorder::default();
    pass(&store, &engine, &fake, &sink);

    // A second mailbox with the message gone, which is what a full list finds after a delete the
    // change log was too old to report.
    let empty = FakeProvider::new();
    empty.add_eml(
        "b1",
        "t2",
        &["INBOX"],
        &eml("b1@example.test", "Bo <bo@example.test>", "Two", days_ago(1), &[], "Two"),
    );
    let mut status = SyncStatus::idle("acct");
    block_on(changes::reconcile(&store, &empty, &sink, &mut status)).expect("a reconcile");

    assert_eq!(count(&store, "SELECT COUNT(*) FROM messages"), 1);
    // Everything rather than the Inbox: Bo arrived through a reconcile rather than a first sync,
    // so nobody has decided about them yet and they are waiting in the Screener, which is the
    // Screener working. What this test is about is that the mirror now holds Bo's message and not
    // Ana's, and Everything is the place that answers that without a routing decision in the way.
    assert_eq!(
        list(&store, &query(Place::Everything)).threads[0].key,
        "b1@example.test"
    );
    assert_eq!(list(&store, &query(Place::Screener)).threads.len(), 1);
}

// -- failure -----------------------------------------------------------------------------------

#[test]
fn a_rate_limit_backs_off_within_the_pass_rather_than_spinning() {
    let store = store();
    let fake = FakeProvider::new();
    fake.add_eml(
        "a1",
        "t1",
        &["INBOX"],
        &eml("a1@example.test", "Ana <ana@example.test>", "One", days_ago(2), &[], "One"),
    );
    let engine = Engine::new("acct");
    let sink = Recorder::default();
    pass(&store, &engine, &fake, &sink);

    fake.fail_next(Call::Changes, ProviderError::RateLimited { retry_after_ms: 60_000 });
    let before = fake.calls().len();
    let status = pass(&store, &engine, &fake, &sink);

    let after: Vec<String> = fake.calls().split_off(before);
    assert_eq!(after.len(), 1, "one attempt, not a loop: {after:?}");
    assert_eq!(status.phase, "error");
    assert!(status.error.unwrap().contains("rate limited"));

    // And the rest it was asked for is honoured: the next tick makes no call at all.
    let before = fake.calls().len();
    pass(&store, &engine, &fake, &sink);
    assert_eq!(fake.calls().len(), before, "it waits out the retry it was given");
}

#[test]
fn repeated_failures_pause_the_account_rather_than_hammering_it() {
    let store = store();
    let fake = FakeProvider::new();
    fake.add_eml(
        "a1",
        "t1",
        &["INBOX"],
        &eml("a1@example.test", "Ana <ana@example.test>", "One", days_ago(2), &[], "One"),
    );
    let engine = Engine::new("acct");
    let sink = Recorder::default();
    pass(&store, &engine, &fake, &sink);

    // A retry of zero so the rest between attempts does not stand in for the breaker.
    for _ in 0..super::engine::BREAKER_TRIPS {
        fake.fail_next(Call::Changes, ProviderError::RateLimited { retry_after_ms: 0 });
    }
    let mut status = SyncStatus::idle("acct");
    for _ in 0..super::engine::BREAKER_TRIPS {
        status = pass(&store, &engine, &fake, &sink);
    }

    assert_eq!(status.phase, "paused");
    assert!(engine.paused());
    assert!(status.message.unwrap().contains("Paused after repeated failures"));

    let before = fake.calls().len();
    pass(&store, &engine, &fake, &sink);
    assert_eq!(fake.calls().len(), before, "a paused account makes no calls");

    // Somebody pressing sync is telling it to try anyway.
    engine.resume();
    let status = pass(&store, &engine, &fake, &sink);
    assert_eq!(status.phase, "idle", "{:?}", status.error);
    assert!(fake.calls().len() > before);
}

// -- the window --------------------------------------------------------------------------------

/// Three threads of two ages: one inside the window, and two outside it of which one carries a
/// decision.
fn a_mailbox_across_the_window(fake: &FakeProvider) {
    for (id, thread, days) in [("recent", "t1", 2), ("old", "t2", 60), ("kept", "t3", 60)] {
        fake.add_eml(
            id,
            thread,
            &["INBOX"],
            &eml(
                &format!("{id}@example.test"),
                "Ana <ana@example.test>",
                id,
                days_ago(days),
                &[],
                "Body",
            ),
        );
    }
}

#[test]
fn shrinking_the_window_evicts_by_age_and_keeps_anything_decided_at_the_same_age() {
    let store = store();
    set_window(&store, 90);
    let fake = FakeProvider::new();
    a_mailbox_across_the_window(&fake);
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());
    assert_eq!(count(&store, "SELECT COUNT(*) FROM threads"), 3);

    store
        .with(|conn| {
            conn.execute(
                "INSERT INTO state.piles (thread_key, pile) VALUES ('kept@example.test', 'reply-later')",
                [],
            )
            .map_err(|e| e.to_string())
        })
        .expect("a pile");

    let change = store
        .with(|conn| evict::set_window(conn, 30, write::now_ms()))
        .expect("the new window");
    assert_eq!(change, evict::WindowChange::Shrunk);
    let report = store
        .with(|conn| evict::run(conn, write::now_ms()))
        .expect("an eviction pass");

    assert_eq!(report.threads, 1, "only the undecided old one goes");
    let left: Vec<String> = list(&store, &query(Place::Everything))
        .threads
        .into_iter()
        .map(|thread| thread.key)
        .collect();
    assert_eq!(left, vec!["recent@example.test", "kept@example.test"]);
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM state.piles"),
        1,
        "eviction never touches a decision"
    );
}

#[test]
fn widening_the_window_queues_a_backfill_that_fills_newest_first() {
    let store = store();
    set_window(&store, 30);
    let fake = FakeProvider::new();
    for index in 0..60 {
        let id = format!("old{index}");
        fake.add_eml(
            &id,
            &format!("t{index}"),
            &["INBOX"],
            &eml(
                &format!("{id}@example.test"),
                "Ana <ana@example.test>",
                &id,
                days_ago(40 + index as i64),
                &[],
                "Body",
            ),
        );
    }
    fake.add_eml(
        "recent",
        "tr",
        &["INBOX"],
        &eml("recent@example.test", "Ana <ana@example.test>", "Recent", days_ago(1), &[], "Body"),
    );

    let engine = Engine::new("acct");
    let sink = Recorder::default();
    pass(&store, &engine, &fake, &sink);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM messages"), 1, "the window held");

    let change = store
        .with(|conn| evict::set_window(conn, 365, write::now_ms()))
        .expect("a wider window");
    assert_eq!(change, evict::WindowChange::Widened);
    let target = store
        .with(evict::backfill_target)
        .expect("a target")
        .expect("a backfill was queued");

    // Newest first: the listing arrives that way and the queue is drained in the order it was
    // filled, so the oldest mail is the last thing anybody waits for.
    block_on(hydrate::list_into(&store, &fake, Some(target), 10_000)).expect("the ids");
    let next = store.with(|conn| write::unhydrated(conn, 3)).expect("the queue");
    assert_eq!(
        next,
        vec!["old0".to_string(), "old1".to_string(), "old2".to_string()]
    );

    block_on(engine.backfill(&store, &fake, &sink)).expect("the backfill");
    assert_eq!(count(&store, "SELECT COUNT(*) FROM messages"), 61);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM messages WHERE hydrated = 0"), 0);
    assert_eq!(
        store.with(evict::backfill_target).expect("target"),
        None,
        "the backfill is done and says so"
    );
    assert!(sink.phases().contains(&"backfilling".to_string()));
}

// -- the outbox --------------------------------------------------------------------------------

#[test]
fn a_flag_change_lands_locally_coalesces_with_the_next_and_reaches_the_provider_once() {
    let store = store();
    let fake = FakeProvider::new();
    fake.add_eml(
        "a1",
        "t1",
        &["INBOX", "UNREAD"],
        &eml("a1@example.test", "Ana <ana@example.test>", "One", days_ago(1), &[], "One"),
    );
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());

    let ids = vec!["a1".to_string()];
    let star = FlagPatch {
        starred: Some(true),
        ..FlagPatch::default()
    };
    let seen = FlagPatch {
        seen: Some(true),
        ..FlagPatch::default()
    };
    store
        .with(|conn| {
            write::apply_flags(conn, &ids, &star)?;
            outbox::queue_flags(conn, &ids, &star, Some("a1@example.test"))?;
            write::apply_flags(conn, &ids, &seen)?;
            outbox::queue_flags(conn, &ids, &seen, Some("a1@example.test"))
        })
        .expect("two optimistic writes");

    // It shows before it goes.
    let page = list(&store, &query(Place::Inbox));
    assert!(page.threads[0].starred);
    assert!(!page.threads[0].unseen);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM outbox"), 1, "one row, not two");

    let before = fake.calls().len();
    let outcome = block_on(outbox::drain(&store, &fake, write::now_ms() + 10_000));
    assert!(outcome.error.is_none(), "{:?}", outcome.error);

    let pushes: Vec<String> = fake
        .calls()
        .split_off(before)
        .into_iter()
        .filter(|call| call.starts_with("set_flags"))
        .collect();
    assert_eq!(pushes.len(), 1, "one call carrying both changes: {pushes:?}");
    let labels = fake.message_labels("a1").expect("the message");
    assert!(labels.contains(&"STARRED".to_string()));
    assert!(!labels.contains(&"UNREAD".to_string()));
    assert_eq!(count(&store, "SELECT COUNT(*) FROM outbox"), 0);
}

#[test]
fn a_push_that_fails_is_deferred_with_the_reason_on_it_and_goes_on_the_retry() {
    let store = store();
    let fake = FakeProvider::new();
    fake.add_eml(
        "a1",
        "t1",
        &["INBOX"],
        &eml("a1@example.test", "Ana <ana@example.test>", "One", days_ago(1), &[], "One"),
    );
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());

    let ids = vec!["a1".to_string()];
    store
        .with(|conn| {
            outbox::queue_flags(
                conn,
                &ids,
                &FlagPatch {
                    archived: Some(true),
                    ..FlagPatch::default()
                },
                Some("a1@example.test"),
            )
        })
        .expect("a queued write");

    fake.fail_next(Call::Flags, ProviderError::Network("the train went into a tunnel".into()));
    let outcome = block_on(outbox::drain(&store, &fake, write::now_ms() + 10_000));
    assert!(outcome.error.is_some());
    assert!(!outcome.changed);

    let row = store
        .with(|conn| write::outbox_rows(conn, 10))
        .expect("the queue")
        .pop()
        .expect("the row is still there");
    assert_eq!(row.attempts, 1);
    assert!(row.last_error.expect("a reason").contains("network"));
    assert!(row.hold_until > write::now_ms(), "and it waits before trying again");

    // The backoff expiring is the only thing standing between it and the provider.
    store
        .with(|conn| {
            conn.execute("UPDATE outbox SET hold_until = 0", [])
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .expect("the wait is over");
    let outcome = block_on(outbox::drain(&store, &fake, write::now_ms() + 10_000));
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert!(outcome.changed);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM outbox"), 0);
    assert!(!fake
        .message_labels("a1")
        .expect("the message")
        .contains(&"INBOX".to_string()));
}

// -- what the queue does with a refusal --------------------------------------------------------

fn two_in_the_inbox(fake: &FakeProvider) {
    for id in ["a1", "a2"] {
        fake.add_eml(
            id,
            &format!("t-{id}"),
            &["INBOX"],
            &eml(
                &format!("{id}@example.test"),
                "Ana <ana@example.test>",
                id,
                days_ago(1),
                &[],
                "One",
            ),
        );
    }
}

fn queue_archive(store: &Memory, id: &str) {
    store
        .with(|conn| {
            outbox::queue_flags(
                conn,
                &[id.to_string()],
                &FlagPatch {
                    archived: Some(true),
                    ..FlagPatch::default()
                },
                Some(&format!("{id}@example.test")),
            )
        })
        .expect("a queued write");
}

fn release_holds(store: &Memory) {
    store
        .with(|conn| {
            conn.execute("UPDATE outbox SET hold_until = 0", [])
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .expect("the waits are over");
}

fn in_inbox(fake: &FakeProvider, id: &str) -> bool {
    fake.message_labels(id)
        .expect("the message")
        .contains(&"INBOX".to_string())
}

fn refused() -> ProviderError {
    ProviderError::Other("Gmail label change failed (400): Invalid id value".to_string())
}

#[test]
fn a_write_refused_for_good_is_dropped_and_the_row_behind_it_goes() {
    // The shape of the bug: a row Gmail answered 400 to was deferred, retried, deferred again, for
    // ever, and every row queued after it waited behind it for ever, and every pass reported the
    // same error. The queue never moved and the person never stopped hearing about it.
    let store = store();
    let fake = FakeProvider::new();
    two_in_the_inbox(&fake);
    let engine = Engine::new("acct");
    let sink = Recorder::default();
    pass(&store, &engine, &fake, &sink);

    queue_archive(&store, "a1");
    queue_archive(&store, "a2");
    assert_eq!(count(&store, "SELECT COUNT(*) FROM outbox"), 2, "two rows, different messages");

    // The first refusal is given the benefit of the doubt, and the row behind it is not held up.
    fake.fail_next(Call::Flags, refused());
    let drained = block_on(outbox::drain_reporting(&store, &fake, write::now_ms() + 10_000));
    assert!(drained.outcome.error.is_none(), "{:?}", drained.outcome.error);
    assert!(drained.dropped.is_empty());
    assert!(in_inbox(&fake, "a1"), "the refused change did not take");
    assert!(!in_inbox(&fake, "a2"), "the row behind it went anyway");
    let rows = store
        .with(|conn| write::outbox_rows(conn, 10))
        .expect("the queue");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].attempts, 1);
    assert!(rows[0].last_error.as_deref().unwrap_or("").contains("(400)"));

    // The second refusal, through a whole pass: dropped, said once, and the pass ends clean.
    release_holds(&store);
    fake.fail_next(Call::Flags, refused());
    let before = sink.statuses.lock().expect("statuses").len();
    let status = pass(&store, &engine, &fake, &sink);
    assert_eq!(status.phase, "idle", "{:?}", status.error);
    assert!(status.error.is_none());
    assert_eq!(count(&store, "SELECT COUNT(*) FROM outbox"), 0, "the dead row is gone");

    let said: Vec<String> = sink.statuses.lock().expect("statuses")[before..]
        .iter()
        .filter_map(|status| status.error.clone())
        .collect();
    assert_eq!(said.len(), 1, "said once: {said:?}");
    assert!(
        said[0].starts_with("Archiving 1 message was refused and dropped: "),
        "{}",
        said[0]
    );
    assert!(said[0].contains("(400)"), "{}", said[0]);
    assert!(!said[0].contains("http"), "{}", said[0]);

    // And it was not a failure of the account: the next pass runs rather than resting.
    let calls = fake.calls().len();
    pass(&store, &engine, &fake, &sink);
    assert!(fake.calls().len() > calls, "no rest was taken for a dropped row");
    assert!(!engine.paused());
}

#[test]
fn a_change_to_a_message_that_is_gone_is_already_done() {
    let store = store();
    let fake = FakeProvider::new();
    two_in_the_inbox(&fake);
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());

    queue_archive(&store, "a1");
    fake.fail_next(Call::Flags, ProviderError::NotFound);
    let drained = block_on(outbox::drain_reporting(&store, &fake, write::now_ms() + 10_000));
    assert!(drained.outcome.error.is_none(), "{:?}", drained.outcome.error);
    assert!(drained.dropped.is_empty(), "nothing to tell: there is no message to archive");
    assert!(drained.outcome.changed);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM outbox"), 0);
}

#[test]
fn a_network_failure_keeps_the_row_and_holds_the_rows_behind_it() {
    let store = store();
    let fake = FakeProvider::new();
    two_in_the_inbox(&fake);
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());

    queue_archive(&store, "a1");
    queue_archive(&store, "a2");
    fake.fail_next(
        Call::Flags,
        ProviderError::Network("the train went into a tunnel".into()),
    );
    let drained = block_on(outbox::drain_reporting(&store, &fake, write::now_ms() + 10_000));
    assert!(matches!(drained.outcome.error, Some(ProviderError::Network(_))));
    assert!(drained.dropped.is_empty());
    assert_eq!(count(&store, "SELECT COUNT(*) FROM outbox"), 2, "both rows wait");
    assert!(in_inbox(&fake, "a2"), "the second row was not tried: same tunnel");

    // Once the connection is back both go, in order, in one drain.
    release_holds(&store);
    let drained = block_on(outbox::drain_reporting(&store, &fake, write::now_ms() + 10_000));
    assert!(drained.outcome.error.is_none(), "{:?}", drained.outcome.error);
    assert!(!in_inbox(&fake, "a1") && !in_inbox(&fake, "a2"));
    assert_eq!(count(&store, "SELECT COUNT(*) FROM outbox"), 0);
}

#[test]
fn the_chip_says_signed_out_for_a_refused_token_and_nothing_else() {
    // A 400 from the provider used to come out as phase "error", which the header printed as
    // "Signed out", on an account that was signed in perfectly well.
    let store = store();
    let fake = FakeProvider::new();
    two_in_the_inbox(&fake);
    let engine = Engine::new("acct");
    let sink = Recorder::default();
    pass(&store, &engine, &fake, &sink);

    // The last field is whether the first failure of the kind is kept quiet. A network failure
    // and a 5xx are answered by the next poll and the chip does not move for one of them; a
    // refused token, a missing scope and a rate limit are true on the first answer.
    let cases = [
        (
            ProviderError::Other("Gmail history failed (400): Invalid startHistoryId".into()),
            "error",
            "Sync trouble",
            true,
        ),
        (
            ProviderError::Auth("Token has been expired or revoked.".into()),
            "error",
            "Signed out",
            false,
        ),
        (
            ProviderError::Network("error sending request: connection reset".into()),
            "offline",
            "Offline",
            true,
        ),
        (
            ProviderError::RateLimited { retry_after_ms: 0 },
            "error",
            "Rate limited",
            false,
        ),
        (
            ProviderError::Scope("https://www.googleapis.com/auth/gmail.modify".into()),
            "error",
            "Needs permission",
            false,
        ),
    ];
    for (error, phase, word, quiet) in cases {
        engine.resume();
        if quiet {
            fake.fail_next(Call::Changes, error.clone());
            let first = pass(&store, &engine, &fake, &sink);
            assert_eq!(first.phase, "idle", "one failure is quiet: {error:?}");
            assert!(first.error.is_none(), "{error:?}");
            assert!(first.message.is_none(), "{error:?}");
            engine.unrest();
        }
        fake.fail_next(Call::Changes, error.clone());
        let status = pass(&store, &engine, &fake, &sink);
        assert_eq!(status.phase, phase, "{error:?}");
        assert_eq!(status.message.as_deref(), Some(word), "{error:?}");
        let detail = status.error.expect("the detail is on the status");
        assert!(!detail.contains("http"), "no link on screen: {detail}");
    }
}

#[test]
fn a_message_deleted_between_the_listing_and_the_fetch_does_not_fail_the_pass() {
    let store = store();
    let fake = FakeProvider::new();
    a_mailbox(&fake, 10);
    fake.withhold_headers("m3");
    let engine = Engine::new("acct");
    let sink = Recorder::default();

    let status = pass(&store, &engine, &fake, &sink);
    assert_eq!(status.phase, "idle", "{:?}", status.error);
    assert_eq!(count(&store, "SELECT COUNT(*) FROM messages"), 9);
    assert_eq!(
        count(&store, "SELECT COUNT(*) FROM messages WHERE hydrated = 0"),
        0,
        "the placeholder for the one that is gone does not wait for ever"
    );
    assert_eq!(count(&store, "SELECT COUNT(*) FROM messages WHERE id = 'm3'"), 0);

    // The cursor was kept, so the next pass polls rather than listing the mailbox again.
    let lists = |fake: &FakeProvider| fake.calls().iter().filter(|c| c.starts_with("list ")).count();
    let listed = lists(&fake);
    let status = pass(&store, &engine, &fake, &sink);
    assert_eq!(status.phase, "idle", "{:?}", status.error);
    assert_eq!(lists(&fake), listed);
}

// -- the places --------------------------------------------------------------------------------

#[test]
fn every_place_this_milestone_serves_is_one_query_over_the_same_rows() {
    let store = store();
    let fake = FakeProvider::new();
    for (id, thread, labels) in [
        ("inboxed", "t1", vec!["INBOX", "UNREAD"]),
        ("sent", "t2", vec!["SENT"]),
        ("drafted", "t3", vec!["DRAFT"]),
        ("starred", "t4", vec!["INBOX", "STARRED"]),
        ("spammed", "t5", vec!["SPAM"]),
        ("trashed", "t6", vec!["TRASH"]),
        ("labelled", "t7", vec!["INBOX", "Label_7"]),
    ] {
        fake.add_eml(
            id,
            thread,
            &labels,
            &eml(
                &format!("{id}@example.test"),
                "Ana <ana@example.test>",
                id,
                days_ago(1),
                &[],
                "Body",
            ),
        );
    }
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());

    let keys = |place: Place| -> Vec<String> {
        list(&store, &query(place))
            .threads
            .into_iter()
            .map(|thread| thread.key)
            .collect()
    };

    assert_eq!(
        keys(Place::Inbox),
        vec![
            "inboxed@example.test".to_string(),
            "starred@example.test".to_string(),
            "labelled@example.test".to_string()
        ]
    );
    assert_eq!(keys(Place::Sent), vec!["sent@example.test".to_string()]);
    assert_eq!(keys(Place::Drafts), vec!["drafted@example.test".to_string()]);
    assert_eq!(keys(Place::Starred), vec!["starred@example.test".to_string()]);
    assert_eq!(keys(Place::Spam), vec!["spammed@example.test".to_string()]);
    assert_eq!(keys(Place::Trash), vec!["trashed@example.test".to_string()]);

    let mut labelled = query(Place::Label);
    labelled.label_id = Some("Label_7".to_string());
    assert_eq!(
        list(&store, &labelled)
            .threads
            .into_iter()
            .map(|thread| thread.key)
            .collect::<Vec<_>>(),
        vec!["labelled@example.test".to_string()]
    );

    // The Inbox's first group is the unseen one and the row says which group it is in.
    let inbox = list(&store, &query(Place::Inbox));
    assert_eq!(inbox.threads[0].group, "new");
    assert_eq!(inbox.threads[1].group, "seen");

    let everything = list(&store, &query(Place::Everything));
    assert_eq!(
        everything.footer,
        Some("Showing the last month. Older mail is on Gmail.".to_string())
    );
}

#[test]
fn a_page_ends_where_the_next_one_begins() {
    let store = store();
    let fake = FakeProvider::new();
    a_mailbox(&fake, 12);
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());

    let mut first = query(Place::Inbox);
    first.limit = 5;
    let page = list(&store, &first);
    assert_eq!(page.threads.len(), 5);

    let mut second = first.clone();
    second.cursor = page.next_cursor.clone();
    let next = list(&store, &second);
    assert_eq!(next.threads.len(), 5);
    for thread in &next.threads {
        assert!(
            !page.threads.iter().any(|seen| seen.key == thread.key),
            "a cursor never repeats a row"
        );
    }
}

// -- search ------------------------------------------------------------------------------------

#[test]
fn the_query_parser_reads_every_operator() {
    struct Case {
        input: &'static str,
        check: fn(&fts::Query),
    }

    let cases = [
        Case {
            input: "quarterly report",
            check: |q| assert_eq!(q.words, vec!["quarterly", "report"]),
        },
        Case {
            input: "from:Ana@Example.test",
            check: |q| assert_eq!(q.from, vec!["ana@example.test"]),
        },
        Case {
            input: "to:bo@example.test",
            check: |q| assert_eq!(q.to, vec!["bo@example.test"]),
        },
        Case {
            input: "subject:\"end of quarter\"",
            check: |q| assert_eq!(q.subject, vec!["end of quarter"]),
        },
        Case {
            input: "has:attachment",
            check: |q| assert!(q.has_attachment),
        },
        Case {
            input: "filename:budget.xlsx",
            check: |q| assert_eq!(q.filename, vec!["budget.xlsx"]),
        },
        Case {
            input: "in:sent",
            check: |q| assert_eq!(q.place.as_deref(), Some("sent")),
        },
        Case {
            input: "before:2026-09-01",
            check: |q| assert_eq!(q.before_ms, Some(midnight_utc(2026, 9, 1))),
        },
        Case {
            input: "after:2026/09/01",
            check: |q| assert_eq!(q.after_ms, Some(midnight_utc(2026, 9, 1))),
        },
        Case {
            input: "label:Work",
            check: |q| assert_eq!(q.label, vec!["Work"]),
        },
        Case {
            input: "before:never",
            check: |q| assert_eq!(q.before_ms, None),
        },
        Case {
            input: "ratio:1:2",
            check: |q| assert_eq!(q.words, vec!["ratio:1:2"]),
        },
        Case {
            input: "from:",
            check: |q| assert_eq!(q.words, vec!["from:"]),
        },
    ];

    for case in cases {
        let parsed = fts::parse(case.input);
        (case.check)(&parsed);
    }

    let everything = fts::parse("budget from:ana@example.test subject:report has:attachment");
    assert_eq!(
        everything.match_expr().expect("a match expression"),
        "\"budget\" sender : \"ana@example.test\" subject : \"report\""
    );
    assert!(everything.has_attachment);

    // A query of nothing but predicates never reaches the index, because there is nothing to match.
    assert_eq!(fts::parse("has:attachment").match_expr(), None);
    // And a stray quote is a search, not a syntax error.
    assert_eq!(fts::parse("say \"no").match_expr(), Some("\"say\" \"no\"".to_string()));
}

#[test]
fn a_query_reaching_past_the_window_says_so() {
    let start = days_ago(30);
    assert!(fts::parse("before:2001-01-01").reaches_before(Some(start)));
    assert!(!fts::parse("before:2001-01-01").reaches_before(None));
    assert!(!fts::parse("budget").reaches_before(Some(start)));
}

#[test]
fn search_finds_a_message_by_its_words_and_by_its_operators() {
    let store = store();
    let fake = FakeProvider::new();
    fake.add_eml(
        "a1",
        "t1",
        &["INBOX"],
        &eml(
            "a1@example.test",
            "Ana <ana@example.test>",
            "Quarterly budget",
            days_ago(2),
            &[],
            "The numbers are attached.",
        ),
    );
    fake.add_eml(
        "b1",
        "t2",
        &["INBOX"],
        &eml("b1@example.test", "Bo <bo@example.test>", "Lunch", days_ago(2), &[], "Free?"),
    );
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());
    block_on(hydrate::body(&store, &fake, "a1")).expect("a body");
    block_on(hydrate::body(&store, &fake, "b1")).expect("a body");

    let find = |text: &str| -> Vec<String> {
        let mut wanted = query(Place::Search);
        wanted.query = Some(text.to_string());
        list(&store, &wanted)
            .threads
            .into_iter()
            .map(|thread| thread.key)
            .collect()
    };

    assert_eq!(find("budget"), vec!["a1@example.test".to_string()]);
    assert_eq!(find("subject:lunch"), vec!["b1@example.test".to_string()]);
    assert_eq!(find("from:ana@example.test"), vec!["a1@example.test".to_string()]);
    assert_eq!(find("in:inbox budget"), vec!["a1@example.test".to_string()]);
    assert!(find("has:attachment").is_empty());
    assert!(find("nothingatall").is_empty());
}

// -- the thread key ----------------------------------------------------------------------------

#[test]
fn the_thread_key_is_the_head_of_the_conversation() {
    let cases: [(Option<&str>, Option<&str>, Option<&str>, &str); 6] = [
        // The first entry of References, however many there are.
        (Some("<r1@x> <r2@x> <r3@x>"), Some("<r3@x>"), Some("<m@x>"), "r1@x"),
        // Folded across lines, which is how it arrives on most real mail.
        (Some("<r1@x>\r\n\t<r2@x>"), None, Some("<m@x>"), "r1@x"),
        // In-Reply-To when there is no References.
        (None, Some("<p@x>"), Some("<m@x>"), "p@x"),
        // Its own Message-ID when it starts the conversation.
        (None, None, Some("<m@x>"), "m@x"),
        // A message with none of the three cannot have a portable key, so it borrows the
        // provider's thread id and stops roaming, which is the honest failure.
        (None, None, None, "provider:t1"),
        // Whitespace and empty brackets are not a reference.
        (Some("  "), Some("<>"), Some("<m@x>"), "m@x"),
    ];

    for (references, in_reply_to, message_id, expected) in cases {
        assert_eq!(
            write::thread_key(references, in_reply_to, message_id, "t1"),
            expected,
            "references={references:?} in-reply-to={in_reply_to:?} message-id={message_id:?}"
        );
    }
}

#[test]
fn a_folded_references_header_survives_the_whole_path_into_the_mirror() {
    let store = store();
    let fake = FakeProvider::new();
    fake.add_eml(
        "a1",
        "t1",
        &["INBOX"],
        &eml("head@example.test", "Ana <ana@example.test>", "Head", days_ago(3), &[], "One"),
    );
    // The header is folded in the bytes, which is what the provider unfolds on the way out.
    fake.add_eml(
        "a2",
        "t1",
        &["INBOX"],
        &eml(
            "reply@example.test",
            "Bo <bo@example.test>",
            "Re: Head",
            days_ago(1),
            &[("References", "<head@example.test>\r\n\t<middle@example.test>")],
            "Two",
        ),
    );
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());

    let keys: Vec<String> = store
        .with(|conn| {
            let mut stmt = conn
                .prepare("SELECT thread_key FROM messages ORDER BY date_ms")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
        })
        .expect("the keys");
    assert_eq!(
        keys,
        vec!["head@example.test".to_string(), "head@example.test".to_string()],
        "both messages carry the head of the conversation"
    );
    assert_eq!(list(&store, &query(Place::Inbox)).threads.len(), 1);
}

#[test]
fn an_address_list_survives_a_comma_inside_a_display_name() {
    let people = write::addresses("\"Doe, Jane\" <JANE@Example.test>, bo@example.test");
    assert_eq!(people.len(), 2);
    assert_eq!(people[0].name.as_deref(), Some("Doe, Jane"));
    assert_eq!(people[0].address, "jane@example.test");
    assert_eq!(people[1].name, None);
    assert_eq!(people[1].address, "bo@example.test");
}

// -- the dock badge ----------------------------------------------------------------------------

/// A mailbox with somewhere for everything: two threads waiting in the Inbox, one already seen, one
/// in a pile, one snoozed, one being ignored, a newsletter in the Feed, a receipt in the Paper
/// Trail and a stranger still waiting in the Screener.
///
/// Every one of them but the seen thread is unread, which is the whole point of the fixture: the
/// mailbox's unread count and the number of things actually waiting for you are different numbers,
/// and only one of them belongs on a dock icon.
fn a_mailbox_with_somewhere_for_everything(fake: &FakeProvider) {
    let mail = [
        ("ana-one", "Ana <ana@example.test>", true),
        ("ana-two", "Ana <ana@example.test>", true),
        ("ana-seen", "Ana <ana@example.test>", false),
        ("piled", "Ana <ana@example.test>", true),
        ("snoozed", "Ana <ana@example.test>", true),
        ("ignored", "Ana <ana@example.test>", true),
        ("weekly", "The Weekly <news@weekly.test>", true),
        ("receipt", "Orders <orders@shop.test>", true),
        ("stranger", "Cai <cai@stranger.test>", true),
    ];
    for (index, (id, from, unread)) in mail.iter().enumerate() {
        let labels: &[&str] = if *unread {
            &["INBOX", "UNREAD"]
        } else {
            &["INBOX"]
        };
        fake.add_eml(
            id,
            &format!("t-{id}"),
            labels,
            &eml(
                &format!("{id}@example.test"),
                from,
                id,
                days_ago(index as i64 + 1),
                &[],
                "Body",
            ),
        );
    }
}

/// The rules the fixture's senders end up with. Set explicitly rather than left to the first run
/// seed, because the seed screens everybody in and this test is about the four destinations being
/// four different places.
fn route_the_fixture(store: &Memory) {
    store
        .with(|conn| {
            set_rule(conn, "ana@example.test", false, Destination::Inbox, None)?;
            set_rule(conn, "news@weekly.test", false, Destination::Feed, None)?;
            set_rule(conn, "orders@shop.test", false, Destination::PaperTrail, None)?;
            // Nobody has decided about Cai, which is what puts them in the Screener.
            clear_rule(conn, "cai@stranger.test")?;

            conn.execute(
                "INSERT INTO state.piles (thread_key, pile) VALUES ('piled@example.test', 'reply-later')",
                [],
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO state.snoozes (thread_key, return_at, kind)
                 VALUES ('snoozed@example.test', ?1, 'tomorrow')",
                [write::now_ms() + write::DAY_MS],
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO state.thread_flags (thread_key, ignored) VALUES ('ignored@example.test', 1)",
                [],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .expect("the fixture's routing");
}

fn new_for_you(store: &Memory) -> Vec<String> {
    list(store, &query(Place::Inbox))
        .threads
        .into_iter()
        .filter(|thread| thread.group == "new")
        .map(|thread| thread.key)
        .collect()
}

fn badge_count(store: &Memory) -> i64 {
    store.with(read::inbox_unseen).expect("the badge count")
}

#[test]
fn the_badge_counts_exactly_what_new_for_you_shows() {
    let store = store();
    let fake = FakeProvider::new();
    a_mailbox_with_somewhere_for_everything(&fake);
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());
    route_the_fixture(&store);

    // The fixture is only worth anything if every other place really did take its thread.
    let keys = |place: Place| -> Vec<String> {
        list(&store, &query(place))
            .threads
            .into_iter()
            .map(|thread| thread.key)
            .collect()
    };
    assert_eq!(keys(Place::Feed), vec!["weekly@example.test".to_string()]);
    assert_eq!(keys(Place::PaperTrail), vec!["receipt@example.test".to_string()]);
    assert_eq!(keys(Place::Screener), vec!["stranger@example.test".to_string()]);
    assert_eq!(keys(Place::ReplyLater), vec!["piled@example.test".to_string()]);
    assert_eq!(keys(Place::Snoozed), vec!["snoozed@example.test".to_string()]);
    assert_eq!(
        keys(Place::Inbox),
        vec![
            "ana-one@example.test".to_string(),
            "ana-two@example.test".to_string(),
            "ana-seen@example.test".to_string(),
            "ignored@example.test".to_string(),
        ],
        "the Inbox holds four threads and only two of them are waiting"
    );

    assert_eq!(
        new_for_you(&store),
        vec!["ana-one@example.test".to_string(), "ana-two@example.test".to_string()],
        "the Screener, the Feed, the Paper Trail, the pile, the snooze and the ignored thread are \
         none of them waiting for you"
    );
    assert_eq!(
        badge_count(&store),
        new_for_you(&store).len() as i64,
        "the badge and the list are the same query or they are a bug"
    );

    // And the number it is not. Eight threads on this device are unread, which is the count
    // Mailspring would put on the dock and the noise this app exists to take off it.
    assert_eq!(count(&store, "SELECT COUNT(*) FROM threads WHERE unseen = 1"), 8);
    assert_eq!(badge_count(&store), 2);
}

#[test]
fn nothing_waiting_is_no_badge_rather_than_a_zero() {
    let store = store();
    let fake = FakeProvider::new();
    a_mailbox_with_somewhere_for_everything(&fake);
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());
    route_the_fixture(&store);

    // Reading the two that were waiting empties the group.
    store
        .with(|conn| {
            write::apply_flags(
                conn,
                &["ana-one".to_string(), "ana-two".to_string()],
                &FlagPatch {
                    seen: Some(true),
                    ..FlagPatch::default()
                },
            )
        })
        .expect("both read");

    assert!(new_for_you(&store).is_empty());
    assert_eq!(badge_count(&store), 0);
    assert_eq!(
        badge::to_show(true, badge_count(&store)),
        None,
        "an empty Inbox takes the badge off rather than putting a 0 on it"
    );
}

#[test]
fn several_accounts_are_one_number() {
    let mut totals = Vec::new();
    for extra in [0usize, 3usize] {
        let store = store();
        let fake = FakeProvider::new();
        a_mailbox_with_somewhere_for_everything(&fake);
        for index in 0..extra {
            fake.add_eml(
                &format!("more-{index}"),
                &format!("t-more-{index}"),
                &["INBOX", "UNREAD"],
                &eml(
                    &format!("more-{index}@example.test"),
                    "Ana <ana@example.test>",
                    "More",
                    days_ago(index as i64 + 20),
                    &[],
                    "Body",
                ),
            );
        }
        let engine = Engine::new("acct");
        pass(&store, &engine, &fake, &Recorder::default());
        route_the_fixture(&store);
        totals.push(badge_count(&store));
    }

    assert_eq!(totals, vec![2, 5]);
    assert_eq!(
        totals.iter().sum::<i64>(),
        7,
        "the badge is one number on one dock icon, so the accounts add up"
    );
}

#[test]
fn turning_the_setting_off_clears_the_badge() {
    let store = store();
    let fake = FakeProvider::new();
    a_mailbox_with_somewhere_for_everything(&fake);
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());
    route_the_fixture(&store);

    let waiting = badge_count(&store);
    assert_eq!(badge::to_show(true, waiting), Some(2));
    assert_eq!(badge::to_show(false, waiting), None);
}

#[test]
fn only_a_scope_that_can_move_the_count_recounts() {
    for reason in [
        "threads",
        "threads state",
        "threads thread:k1 state",
        "threads screener state",
        "accounts",
        "settings accounts",
    ] {
        assert!(badge::moves_the_count(reason), "{reason} moves it");
    }
    // A body landing, a clip and a drained outbox change nothing about what is waiting, and a
    // recount for each of them is the query per event this is written to avoid.
    for reason in ["thread", "thread:k1", "state", "outbox"] {
        assert!(!badge::moves_the_count(reason), "{reason} does not move it");
    }
}

#[test]
fn a_snooze_that_came_back_and_has_not_been_read_is_on_the_badge() {
    let store = store();
    let fake = FakeProvider::new();
    a_mailbox_with_somewhere_for_everything(&fake);
    let engine = Engine::new("acct");
    pass(&store, &engine, &fake, &Recorder::default());
    route_the_fixture(&store);

    assert_eq!(badge_count(&store), 2, "before tomorrow arrives");

    // Tomorrow arrives. The thread leaves the Snoozed place and lands in Back, above New for you,
    // which is a different group and the same fact: it is waiting to be dealt with.
    store
        .with(|conn| {
            conn.execute("DELETE FROM state.snoozes WHERE thread_key = ?1", ["snoozed@example.test"])
                .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO state.returned (thread_key, at_ms) VALUES (?1, ?2)",
                rusqlite::params!["snoozed@example.test", write::now_ms()],
            )
            .map_err(|e| e.to_string())
        })
        .expect("the snooze returns");

    let back: Vec<String> = list(&store, &query(Place::Inbox))
        .threads
        .into_iter()
        .filter(|thread| thread.group == "back")
        .map(|thread| thread.key)
        .collect();
    assert_eq!(back, vec!["snoozed@example.test".to_string()]);

    // Three, not two. The group is Back rather than New for you, and counting the group would mean
    // the badge did not move at the one moment the person asked to be reminded.
    assert_eq!(badge_count(&store), 3);

    // And reading it still takes it off, which is what a badge counting the whole of Back could
    // never promise.
    store
        .with(|conn| {
            write::apply_flags(
                conn,
                &["snoozed".to_string()],
                &FlagPatch {
                    seen: Some(true),
                    ..FlagPatch::default()
                },
            )
        })
        .expect("read it");
    assert_eq!(badge_count(&store), 2);
}
