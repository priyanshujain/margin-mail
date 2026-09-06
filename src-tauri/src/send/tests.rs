// The send pipeline over the fake mailbox and a pair of in-memory databases.
//
// Two things are asserted everywhere here: what the provider was handed, and what the outbox has
// left. A send is the one operation in the app where doing it twice is worse than not doing it at
// all, so the tests that matter most are the ones about the second attempt.

use std::sync::Mutex;

use rusqlite::Connection;
use tauri::async_runtime::block_on;

use crate::db;
use crate::dto::{Draft, Person, Place, SnoozeKind, ThreadQuery};
use crate::mirror::{read, write};
use crate::mirror::write::OutboxRow;
use crate::provider::fake::{Call, FakeProvider};
use crate::provider::{Provider, ProviderError};
use crate::state;
use crate::sync::{outbox, Store};

use super::{cancel, queue, release, threading};

struct Memory(Mutex<Connection>);

impl Store for Memory {
    fn with<T, F: FnOnce(&Connection) -> Result<T, String>>(&self, f: F) -> Result<T, String> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        f(&conn)
    }
}

fn store() -> Memory {
    let store = Memory(Mutex::new(db::memory().expect("a pair of in-memory databases")));
    store
        .with(|conn| write::meta_set(conn, write::OWN_ADDRESS_KEY, "you@example.com"))
        .expect("the account's own address");
    store
}

fn me() -> Person {
    Person {
        name: Some("You".to_string()),
        address: "you@example.com".to_string(),
    }
}

fn ana() -> Person {
    Person {
        name: Some("Ana Ruiz".to_string()),
        address: "ana@example.test".to_string(),
    }
}

fn draft() -> Draft {
    Draft {
        id: None,
        account_id: "acct".to_string(),
        thread_key: None,
        in_reply_to: None,
        from_alias: None,
        to: vec![ana()],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: "About the lease".to_string(),
        body_html: "<p>Thursday works.</p>".to_string(),
        attachments: Vec::new(),
        remind_at_ms: None,
    }
}

/// One thread of two messages from Ana, as the mirror holds it.
fn conversation(conn: &Connection) -> String {
    let key = "lease-01@example.test".to_string();
    conn.execute(
        "INSERT INTO threads (provider_thread_id, thread_key, latest_ms, message_count, unseen,
                              subject, snippet, from_name, from_address, in_inbox)
         VALUES ('t-lease', ?1, 2000, 2, 1, 'About the lease', 'snippet', 'Ana Ruiz',
                 'ana@example.test', 1)",
        [&key],
    )
    .expect("a thread");
    for (id, message_id, at) in [
        ("m-1", "lease-01@example.test", 1000),
        ("m-2", "lease-02@example.test", 2000),
    ] {
        conn.execute(
            "INSERT INTO messages (id, provider_thread_id, thread_key, message_id, date_ms,
                                   from_address, subject, snippet, hydrated, labels)
             VALUES (?1, 't-lease', ?2, ?3, ?4, 'ana@example.test', 'About the lease', 'snippet',
                     1, '[\"INBOX\"]')",
            rusqlite::params![id, key, message_id, at],
        )
        .expect("a message");
    }
    key
}

fn reply(key: &str) -> Draft {
    Draft {
        thread_key: Some(key.to_string()),
        in_reply_to: Some("m-2".to_string()),
        subject: "Re: About the lease".to_string(),
        ..draft()
    }
}

fn drain(store: &Memory, fake: &FakeProvider) -> crate::sync::Outcome {
    block_on(outbox::drain(store, fake, write::now_ms() + 10_000))
}

fn sends(fake: &FakeProvider) -> Vec<String> {
    fake.calls()
        .into_iter()
        .filter(|call| call.starts_with("send "))
        .collect()
}

/// The bytes the provider was handed. The fake keeps a sent message like any other, so this is the
/// message as it left rather than a record of the call.
fn sent_bytes(fake: &FakeProvider) -> Vec<u8> {
    for n in 1..20 {
        if let Ok(raw) = block_on(Provider::fetch_body(fake, &format!("sent-{n}"))) {
            return raw;
        }
    }
    panic!("nothing was sent");
}

fn queued(store: &Memory) -> Vec<OutboxRow> {
    store.with(|conn| write::outbox_rows(conn, 10)).expect("the queue")
}

fn header(raw: &[u8], name: &str) -> String {
    let text = String::from_utf8_lossy(raw);
    let head = text
        .split_once("\r\n\r\n")
        .map(|(head, _)| head.to_string())
        .unwrap_or_else(|| text.to_string());
    let mut out = String::new();
    let mut inside = false;
    for line in head.split("\r\n") {
        if inside {
            if line.starts_with(' ') || line.starts_with('\t') {
                out.push(' ');
                out.push_str(line.trim());
                continue;
            }
            break;
        }
        if let Some(rest) = line.strip_prefix(&format!("{name}: ")) {
            out.push_str(rest);
            inside = true;
        }
    }
    out
}

// -- the hold ----------------------------------------------------------------------------------

#[test]
fn a_send_waits_out_its_delay_and_the_provider_sees_nothing_until_it_passes() {
    let store = store();
    let fake = FakeProvider::new();
    let hold_until = write::now_ms() + 10_000;
    store
        .with(|conn| queue(conn, &draft(), &me(), hold_until))
        .expect("a queued send");

    drain(&store, &fake);
    assert!(sends(&fake).is_empty(), "nothing leaves inside the delay");
    assert_eq!(queued(&store).len(), 1, "and the row is still waiting");

    store
        .with(|conn| {
            conn.execute("UPDATE outbox SET hold_until = 0", [])
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .expect("the delay passes");
    let outcome = drain(&store, &fake);
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(sends(&fake).len(), 1, "once the delay is up, once");
    assert!(queued(&store).is_empty(), "and the row has gone with it");
}

#[test]
fn cancelling_inside_the_delay_puts_the_draft_back_and_the_provider_never_sees_it() {
    let store = store();
    let fake = FakeProvider::new();
    let mut draft = draft();
    draft.id = Some("draft-7".to_string());

    let id = store
        .with(|conn| {
            crate::drafts::save(conn, &draft)?;
            crate::drafts::delete(conn, "draft-7")?;
            queue(conn, &draft, &me(), write::now_ms() + 10_000)
        })
        .expect("a queued send");

    store
        .with(|conn| cancel(conn, &id, &draft))
        .expect("the cancel");
    drain(&store, &fake);

    assert!(sends(&fake).is_empty(), "a cancelled send never happened");
    assert!(queued(&store).is_empty());
    let back = store
        .with(|conn| crate::drafts::get(conn, "draft-7"))
        .expect("the draft is back");
    assert_eq!(back.body_html, draft.body_html);
    assert_eq!(back.id.as_deref(), Some("draft-7"), "and under its own id");
}

#[test]
fn send_now_skips_the_rest_of_the_hold() {
    let store = store();
    let fake = FakeProvider::new();
    let id = store
        .with(|conn| queue(conn, &draft(), &me(), write::now_ms() + 30_000))
        .expect("a queued send");

    drain(&store, &fake);
    assert!(sends(&fake).is_empty());

    assert!(store.with(|conn| release(conn, &id)).expect("released"));
    drain(&store, &fake);
    assert_eq!(sends(&fake).len(), 1);
    assert!(queued(&store).is_empty());
}

// -- failing and retrying ----------------------------------------------------------------------

#[test]
fn a_send_that_fails_waits_with_the_reason_on_it_and_the_thread_reads_as_sending() {
    let store = store();
    let fake = FakeProvider::new();
    let key = store.with(|conn| Ok(conversation(conn))).expect("a thread");
    store
        .with(|conn| queue(conn, &reply(&key), &me(), 0))
        .expect("a queued send");

    fake.fail_next(
        Call::Send,
        ProviderError::Network("the train went into a tunnel".into()),
    );
    let outcome = drain(&store, &fake);
    assert!(outcome.error.is_some());
    assert!(sends(&fake).len() == 1, "it was tried");

    let row = queued(&store).pop().expect("the row is still there");
    assert_eq!(row.attempts, 1, "counted once, at the lease");
    assert!(row.last_error.expect("a reason").contains("network"));
    assert!(row.hold_until > write::now_ms(), "and it waits before trying again");

    let page = store
        .with(|conn| {
            read::threads_list(
                conn,
                "acct",
                "hue-1",
                &ThreadQuery {
                    account_id: None,
                    place: Place::Everything,
                    label_id: None,
                    query: None,
                    limit: 50,
                    cursor: None,
                },
                write::now_ms(),
            )
        })
        .expect("a list page");
    assert!(page.threads[0].sending, "the thread says it is waiting to send");

    store
        .with(|conn| {
            conn.execute("UPDATE outbox SET hold_until = 0", [])
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .expect("the wait is over");
    let outcome = drain(&store, &fake);
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(sends(&fake).len(), 2, "the retry is the second attempt");
    assert!(queued(&store).is_empty());
}

#[test]
fn a_second_drain_does_not_send_the_same_message_twice() {
    let store = store();
    let fake = FakeProvider::new();
    store
        .with(|conn| queue(conn, &draft(), &me(), 0))
        .expect("a queued send");

    drain(&store, &fake);
    drain(&store, &fake);
    assert_eq!(sends(&fake).len(), 1);
    assert!(queued(&store).is_empty());
}

/// The process dying between the provider accepting the message and the row being deleted is the
/// one failure that could send a message twice. The row survives with an attempt counted against
/// it, and the next drain finds the message in the mirror under the `Message-ID` this app stamped
/// and drops the row instead of sending it again.
#[test]
fn a_send_interrupted_after_the_provider_took_it_is_not_sent_again() {
    let store = store();
    let fake = FakeProvider::new();
    let id = store
        .with(|conn| queue(conn, &draft(), &me(), 0))
        .expect("a queued send");

    let message_id = store
        .with(|conn| {
            let payload: String = conn
                .query_row("SELECT payload FROM outbox WHERE id = ?1", [&id], |row| {
                    row.get(0)
                })
                .map_err(|e| e.to_string())?;
            let payload: super::SendPayload =
                serde_json::from_str(&payload).map_err(|e| e.to_string())?;
            payload.message_id.ok_or_else(|| "no Message-ID".to_string())
        })
        .expect("the stamped Message-ID");

    // What the process would have left behind: the attempt counted, the row still there, and the
    // message itself in the mailbox and back through a sync.
    store
        .with(|conn| {
            conn.execute("UPDATE outbox SET attempts = 1, hold_until = 0", [])
                .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO messages (id, provider_thread_id, thread_key, message_id, date_ms,
                                       from_address, subject, hydrated, sent, labels)
                 VALUES ('sent-1', 't-sent', 'k-sent', ?1, 3000, 'you@example.com',
                         'About the lease', 1, 1, '[\"SENT\"]')",
                [write::bare_id(&message_id).expect("a bare id")],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .expect("the interrupted state");

    let outcome = drain(&store, &fake);
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert!(
        sends(&fake).is_empty(),
        "the message was already gone, so it does not go again"
    );
    assert!(queued(&store).is_empty(), "and the row is dropped");
}

// -- threading ---------------------------------------------------------------------------------

#[test]
fn a_reply_carries_the_threading_headers_into_the_bytes() {
    let store = store();
    let fake = FakeProvider::new();
    let key = store.with(|conn| Ok(conversation(conn))).expect("a thread");

    let facts = store
        .with(|conn| threading(conn, &reply(&key)))
        .expect("the facts");
    assert_eq!(facts.in_reply_to.as_deref(), Some("<lease-02@example.test>"));
    assert_eq!(facts.references.len(), 2, "the thread, oldest first");
    assert_eq!(facts.thread_hint.as_deref(), Some("t-lease"));

    store
        .with(|conn| queue(conn, &reply(&key), &me(), 0))
        .expect("a queued send");
    drain(&store, &fake);

    let raw = sent_bytes(&fake);
    assert_eq!(header(&raw, "In-Reply-To"), "<lease-02@example.test>");
    assert_eq!(
        header(&raw, "References"),
        "<lease-01@example.test> <lease-02@example.test>"
    );
    assert_eq!(header(&raw, "Subject"), "Re: About the lease");
    assert!(
        fake.calls().iter().any(|call| call.starts_with("send ")),
        "and it went as one send"
    );

    // The provider's own thread id goes with it, which is the half of threading that is theirs.
    let thread_id = block_on(Provider::fetch_headers(&fake, &["sent-1".to_string()]))
        .ok()
        .and_then(|held| held.first().map(|held| held.thread_id.clone()));
    assert_eq!(thread_id.as_deref(), Some("t-lease"));
}

// -- the reminder ------------------------------------------------------------------------------

#[test]
fn remind_me_if_no_reply_lands_only_after_the_message_has_gone() {
    let store = store();
    let fake = FakeProvider::new();
    let key = store.with(|conn| Ok(conversation(conn))).expect("a thread");
    let mut reply = reply(&key);
    reply.remind_at_ms = Some(write::now_ms() + write::DAY_MS);
    store
        .with(|conn| queue(conn, &reply, &me(), 0))
        .expect("a queued send");

    fake.fail_next(Call::Send, ProviderError::Network("no signal".into()));
    drain(&store, &fake);
    assert!(
        store
            .with(|conn| state::read::snooze_of(conn, &key))
            .expect("a lookup")
            .is_none(),
        "a reminder about a message that never left is a reminder about nothing"
    );

    store
        .with(|conn| {
            conn.execute("UPDATE outbox SET hold_until = 0", [])
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .expect("the wait is over");
    drain(&store, &fake);

    let snooze = store
        .with(|conn| state::read::snooze_of(conn, &key))
        .expect("a lookup")
        .expect("the reminder");
    assert_eq!(snooze.kind, SnoozeKind::IfNoReply);
    assert_eq!(snooze.return_at, reply.remind_at_ms.expect("a moment"));
}

// -- the row somebody else queued --------------------------------------------------------------

/// `unsubscribe` queues a `mailto:` unsubscribe as a bare draft on an `OP_SEND` row, which was
/// refused until the pipeline existed. It carries no built message, so the drain completes it on
/// the first pass and freezes it from then on.
#[test]
fn a_mailto_unsubscribe_row_drains_through_the_send_pipeline() {
    let store = store();
    let fake = FakeProvider::new();
    let draft = crate::unsubscribe::mailto_draft(
        "acct",
        "mailto:unsub@brand.example?subject=unsubscribe%20me",
    )
    .expect("a draft");
    store
        .with(|conn| {
            write::enqueue(
                conn,
                &OutboxRow {
                    op: write::OP_SEND.to_string(),
                    payload: serde_json::to_string(&draft).map_err(|e| e.to_string())?,
                    created_at: write::now_ms(),
                    ..OutboxRow::default()
                },
            )
            .map(|_| ())
        })
        .expect("the row unsubscribe queues");

    let outcome = drain(&store, &fake);
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(sends(&fake).len(), 1);
    assert!(queued(&store).is_empty());

    let raw = sent_bytes(&fake);
    assert!(header(&raw, "To").contains("unsub@brand.example"), "{}", header(&raw, "To"));
    assert_eq!(header(&raw, "Subject"), "unsubscribe me");
    assert!(
        header(&raw, "From").contains("you@example.com"),
        "from the address the mirror knows this account by"
    );
}
