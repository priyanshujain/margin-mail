// Answering an invitation, and the refusal that comes first.
//
// The scope check is the whole of what is worth testing without a Google account behind it: the
// three calls after it are `google::calendar`'s and are tested there. What matters here is that an
// account without the permission is told which one, and that nothing is asked of anybody before it
// is checked.

use rusqlite::Connection;

use crate::db;
use crate::fixtures;
use crate::google::calendar;
use crate::mirror::write;
use crate::provider::fake::FakeProvider;

use super::{check_scope, plan};

fn open() -> Connection {
    let conn = db::memory().expect("a pair of in-memory databases");
    write::meta_set(&conn, write::OWN_ADDRESS_KEY, "pj@73ai.org").expect("an own address");
    conn.execute(
        "INSERT INTO messages (id, provider_thread_id, thread_key, message_id, date_ms,
                               from_address, subject, hydrated, labels)
         VALUES ('m-invite', 't-invite', 'k-invite', '000000000000a1b2c3@google.com', 1000,
                 'sam@sunnydaymusic.example', 'Invitation: Piano lesson', 1, '[\"INBOX\"]')",
        [],
    )
    .expect("a message");
    conn.execute(
        "INSERT INTO bodies (message_id, raw, fetched_at) VALUES ('m-invite', ?1, 1000)",
        [fixtures::CALENDAR_INVITE],
    )
    .expect("its body");
    conn
}

#[test]
fn answering_without_the_calendar_scope_names_the_scope_and_asks_nobody_anything() {
    let conn = open();
    let fake = FakeProvider::new();

    let refusal = plan(&conn, "m-invite", &["https://www.googleapis.com/auth/gmail.modify".into()])
        .expect_err("no calendar scope, no answer");
    assert!(
        refusal.contains(calendar::SCOPE),
        "the refusal has to name the scope so the button can ask for it: {refusal}"
    );
    assert!(
        fake.calls().is_empty(),
        "the check happens before anything is fetched"
    );
    assert!(check_scope(&[]).is_err());
}

#[test]
fn with_the_scope_the_invitation_is_read_off_the_message() {
    let conn = open();
    let plan = plan(&conn, "m-invite", &[calendar::SCOPE.to_string()]).expect("a plan");

    assert_eq!(plan.invite.uid, "4c9b2f7a-piano-cooper@sunnydaymusic.example");
    assert_eq!(plan.invite.summary, "Piano lesson: Cooper");
    assert_eq!(
        plan.self_email, "pj@73ai.org",
        "the answer goes as this account and not as the organiser"
    );
}

#[test]
fn a_message_with_no_invitation_says_so_rather_than_answering_something_else() {
    let conn = open();
    conn.execute(
        "INSERT INTO messages (id, provider_thread_id, thread_key, message_id, date_ms,
                               from_address, subject, hydrated, labels)
         VALUES ('m-plain', 't-plain', 'k-plain', 'plain@example.test', 1000,
                 'ana@example.test', 'Lunch', 1, '[\"INBOX\"]')",
        [],
    )
    .expect("a message");
    conn.execute(
        "INSERT INTO bodies (message_id, raw, fetched_at) VALUES ('m-plain', ?1, 1000)",
        [fixtures::PLAIN_TEXT],
    )
    .expect("its body");

    let refusal = plan(&conn, "m-plain", &[calendar::SCOPE.to_string()])
        .expect_err("there is nothing to answer");
    assert!(refusal.contains("no invitation"), "{refusal}");
}
