// Drafts over a pair of in-memory databases and the fake mailbox.
//
// The two rhythms are what these hold to: a save is a row and nothing else, and the upload is rate
// limited, so the assertions are about how many rows there are and how many calls the provider saw.

use std::sync::Mutex;

use rusqlite::Connection;
use tauri::async_runtime::block_on;

use crate::db;
use crate::dto::{Draft, DraftAttachment, Person};
use crate::mirror::write;
use crate::provider::fake::FakeProvider;
use crate::sync::Store;

use super::{save, upload, MAX_ENCODED_BYTES, UPLOAD_EVERY_MS};

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

fn me() -> Person {
    Person {
        name: Some("You".to_string()),
        address: "you@example.com".to_string(),
    }
}

fn draft(body: &str) -> Draft {
    Draft {
        id: None,
        account_id: "acct".to_string(),
        thread_key: None,
        in_reply_to: None,
        from_alias: None,
        to: vec![Person {
            name: Some("Ana".to_string()),
            address: "ana@example.test".to_string(),
        }],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: "About the lease".to_string(),
        body_html: format!("<p>{body}</p>"),
        attachments: Vec::new(),
        remind_at_ms: None,
    }
}

fn rows(store: &Memory) -> i64 {
    store
        .with(|conn| {
            conn.query_row("SELECT COUNT(*) FROM drafts", [], |row| row.get(0))
                .map_err(|e| e.to_string())
        })
        .expect("a count")
}

fn puts(fake: &FakeProvider) -> Vec<String> {
    fake.calls()
        .into_iter()
        .filter(|call| call.starts_with("draft_put"))
        .collect()
}

#[test]
fn a_second_save_updates_the_draft_rather_than_adding_one() {
    let store = store();
    let first = store
        .with(|conn| save(conn, &draft("Half a sentence")))
        .expect("the first save");
    let mut again = draft("Half a sentence, and the rest of it");
    again.id = Some(first.id.clone());
    let second = store.with(|conn| save(conn, &again)).expect("the second save");

    assert_eq!(second.id, first.id, "the composer's id is the draft's id");
    assert_eq!(rows(&store), 1, "one draft, not two");
    assert_eq!(
        store.with(|conn| super::get(conn, &first.id)).expect("the draft").body_html,
        "<p>Half a sentence, and the rest of it</p>"
    );
}

#[test]
fn the_upload_coalesces_rather_than_going_once_per_keystroke() {
    let store = store();
    let fake = FakeProvider::new();

    let mut id = None;
    for keystroke in 0..5 {
        let mut typing = draft(&format!("Typing {keystroke}"));
        typing.id = id.clone();
        id = Some(
            store
                .with(|conn| save(conn, &typing))
                .expect("a save")
                .id,
        );
    }
    assert_eq!(rows(&store), 1);
    assert!(puts(&fake).is_empty(), "saving is not uploading");

    let now = write::now_ms();
    assert_eq!(
        block_on(upload(&store, &fake, &me(), now)).expect("an upload"),
        1
    );
    assert_eq!(puts(&fake).len(), 1, "five saves, one request");

    // Nothing has changed since, so there is nothing to send.
    assert_eq!(block_on(upload(&store, &fake, &me(), now)).expect("nothing to do"), 0);

    // And a keystroke inside the interval waits for it rather than going straight out.
    let mut typing = draft("Typing again");
    typing.id = id.clone();
    store.with(|conn| save(conn, &typing)).expect("another save");
    assert_eq!(block_on(upload(&store, &fake, &me(), now)).expect("too soon"), 0);
    assert_eq!(puts(&fake).len(), 1);

    assert_eq!(
        block_on(upload(&store, &fake, &me(), now + UPLOAD_EVERY_MS)).expect("the interval passed"),
        1
    );
    let puts = puts(&fake);
    assert_eq!(puts.len(), 2);
    assert!(
        puts[1].contains("draft-1"),
        "the second upload updates the draft the first one created: {puts:?}"
    );
}

#[test]
fn a_draft_over_the_size_limit_says_so_before_a_send_is_attempted() {
    let store = store();
    let fake = FakeProvider::new();

    let mut heavy = draft("The plans are attached");
    heavy.attachments = vec![DraftAttachment {
        path: Some("/tmp/plans.pdf".to_string()),
        attachment_id: None,
        filename: "plans.pdf".to_string(),
        mime_type: "application/pdf".to_string(),
        size: 30 * 1024 * 1024,
    }];

    let saved = store.with(|conn| save(conn, &heavy)).expect("a save");
    assert!(
        saved.encoded_size > 30 * 1024 * 1024,
        "base64 costs a third on top: {}",
        saved.encoded_size
    );
    assert!(
        saved.over_limit,
        "40 MB encoded is over the {} MB one message may be",
        MAX_ENCODED_BYTES / (1024 * 1024)
    );
    assert!(
        fake.calls().is_empty(),
        "the refusal is a fact about the draft and costs no request"
    );

    // The same draft without the attachment is not over anything.
    let mut light = heavy.clone();
    light.id = saved.id.clone().into();
    light.attachments.clear();
    let saved = store.with(|conn| save(conn, &light)).expect("a save");
    assert!(!saved.over_limit);
}

#[test]
fn deleting_a_draft_hands_back_the_copy_the_provider_is_holding() {
    let store = store();
    let fake = FakeProvider::new();
    let id = store
        .with(|conn| save(conn, &draft("A line")))
        .expect("a save")
        .id;
    block_on(upload(&store, &fake, &me(), write::now_ms())).expect("an upload");

    let provider_draft_id = store.with(|conn| super::delete(conn, &id)).expect("the delete");
    assert_eq!(provider_draft_id.as_deref(), Some("draft-1"));
    assert_eq!(rows(&store), 0);
    assert!(store.with(|conn| super::get(conn, &id)).is_err());
}
