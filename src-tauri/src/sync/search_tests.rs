// Searching the provider, and what happens to what comes back.
//
// The interesting part is not the search, which the fake answers from the same little mailbox as
// everything else. It is the lifetime of the rows: a hit reaches past the storage window on
// purpose, so it is marked transient on the way in and eviction takes it away again unless
// somebody did something with it in the meantime.

use std::sync::Mutex;

use rusqlite::Connection;
use tauri::async_runtime::block_on;

use crate::db;
use crate::dto::{Place, ThreadQuery};
use crate::mirror::{evict, read, write};
use crate::provider::fake::FakeProvider;

use super::{hydrate, Store};

struct Memory(Mutex<Connection>);

impl Store for Memory {
    fn with<T, F: FnOnce(&Connection) -> Result<T, String>>(&self, f: F) -> Result<T, String> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        f(&conn)
    }
}

impl Memory {
    fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T, String>) -> T {
        self.with(f).expect("the mirror")
    }
}

fn store() -> Memory {
    Memory(Mutex::new(db::memory().expect("a pair of in-memory databases")))
}

fn days_ago(days: i64) -> i64 {
    write::now_ms() - days * write::DAY_MS
}

fn eml(id: &str, subject: &str, at_ms: i64, body: &str) -> Vec<u8> {
    format!(
        "Message-ID: <{id}@example.test>\r\nFrom: Ana <ana@example.test>\r\n\
         To: You <you@example.test>\r\nSubject: {subject}\r\nDate: {}\r\n\r\n{body}\r\n",
        chrono::DateTime::from_timestamp_millis(at_ms)
            .expect("a moment inside the epoch")
            .to_rfc2822()
    )
    .into_bytes()
}

fn found(store: &Memory, query: &str) -> Vec<String> {
    store
        .read(|conn| {
            read::threads_list(
                conn,
                "acct",
                "hue-1",
                &ThreadQuery {
                    account_id: None,
                    place: Place::Search,
                    label_id: None,
                    query: Some(query.to_string()),
                    limit: 50,
                    cursor: None,
                },
                write::now_ms(),
            )
        })
        .threads
        .into_iter()
        .map(|thread| thread.key)
        .collect()
}

#[test]
fn a_provider_search_hydrates_its_hits_and_marks_them_transient() {
    let store = store();
    store.read(|conn| write::meta_set(conn, write::WINDOW_KEY, "30"));

    let fake = FakeProvider::new();
    fake.add_eml(
        "inside",
        "t1",
        &["INBOX"],
        &eml("inside", "Quarterly budget", days_ago(2), "The numbers."),
    );
    fake.add_eml(
        "outside",
        "t2",
        &["INBOX"],
        &eml("outside", "Budget from before", days_ago(400), "The numbers."),
    );

    // Only what the window covers is here to start with, which is why the search is offered.
    block_on(hydrate::headers(&store, &fake, &["inside".to_string()], false)).expect("a sync");
    assert_eq!(found(&store, "budget"), vec!["inside@example.test".to_string()]);

    let hits = block_on(hydrate::from_search(&store, &fake, "budget")).expect("a search");
    assert_eq!(hits, 2);

    let keys = found(&store, "budget");
    assert!(
        keys.contains(&"outside@example.test".to_string()),
        "the older one is findable now: {keys:?}"
    );

    let transient: Vec<(String, i64)> = store.read(|conn| {
        let mut stmt = conn
            .prepare("SELECT id, transient FROM messages ORDER BY id")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    });
    assert_eq!(
        transient,
        vec![("inside".to_string(), 0), ("outside".to_string(), 1)],
        "a row already held stays what it was; the one the search reached for is on loan"
    );
}

#[test]
fn eviction_takes_the_loaned_rows_back_and_keeps_the_ones_that_gained_a_decision() {
    let store = store();
    store.read(|conn| write::meta_set(conn, write::WINDOW_KEY, "30"));

    let fake = FakeProvider::new();
    for (id, days) in [("kept", 400), ("dropped", 500)] {
        fake.add_eml(
            id,
            id,
            &["INBOX"],
            &eml(id, "Budget", days_ago(days), "The numbers."),
        );
    }
    block_on(hydrate::from_search(&store, &fake, "budget")).expect("a search");
    assert_eq!(found(&store, "budget").len(), 2);

    // One of them was worth keeping, which is the only thing that saves a transient row.
    store.read(|conn| {
        conn.execute(
            "INSERT INTO state.piles (thread_key, pile) VALUES ('kept@example.test', 'reply-later')",
            [],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    });

    let report = store.read(|conn| evict::run(conn, write::now_ms()));
    assert_eq!(report.threads, 1);
    assert_eq!(found(&store, "budget"), vec!["kept@example.test".to_string()]);
    assert_eq!(
        store.read(|conn| write::count(conn, "SELECT COUNT(*) FROM search")),
        1,
        "and the index went with it"
    );
}
