// The export format, which is the part somebody else's software has to be able to read.

use rusqlite::Connection;

use crate::db;
use crate::dto::Destination;
use crate::state;

use super::{escape_from_lines, mbox_entry, write_mbox};

fn open() -> Connection {
    db::memory().expect("a pair of in-memory databases")
}

fn message(conn: &Connection, id: &str, from: &str, date_ms: i64, raw: &str) {
    conn.execute(
        "INSERT INTO messages (id, provider_thread_id, thread_key, message_id, date_ms,
                               from_address, subject, hydrated)
         VALUES (?1, ?1, ?1, ?1, ?2, ?3, 'Subject', 1)",
        rusqlite::params![id, date_ms, from],
    )
    .expect("a message");
    conn.execute(
        "INSERT INTO bodies (message_id, raw, fetched_at) VALUES (?1, ?2, 1)",
        rusqlite::params![id, raw.as_bytes()],
    )
    .expect("a body");
}

#[test]
fn a_body_line_that_looks_like_a_separator_is_escaped() {
    // The one thing mbox can get wrong. A message quoting an email header would otherwise split
    // into two messages in whatever reads the file.
    let mut out = Vec::new();
    escape_from_lines(b"Hello\nFrom Ana, quoted\nBye\n", &mut out);
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Hello\n>From Ana, quoted\nBye\n"
    );
}

#[test]
fn a_from_in_the_middle_of_a_line_is_left_alone() {
    let mut out = Vec::new();
    escape_from_lines(b"a note From Ana\n", &mut out);
    assert_eq!(String::from_utf8(out).unwrap(), "a note From Ana\n");
}

#[test]
fn an_entry_carries_the_separator_the_format_asks_for() {
    let mut out = Vec::new();
    mbox_entry("ana@example.org", 1_760_000_000_000, b"Subject: Hi\n\nHello\n", &mut out);
    let text = String::from_utf8(out).unwrap();
    assert!(text.starts_with("From ana@example.org "), "got {text}");
    assert!(text.contains("\nSubject: Hi\n"));
    assert!(text.ends_with("\n\n"), "an entry ends with a blank line");
}

#[test]
fn a_message_with_no_body_on_the_device_is_not_an_empty_entry() {
    let conn = open();
    message(&conn, "m1", "ana@example.org", 1_760_000_000_000, "Subject: One\n\nOne\n");
    // Headers with no body, which is what the mirror holds until a thread is opened.
    conn.execute(
        "INSERT INTO messages (id, provider_thread_id, thread_key, message_id, date_ms,
                               from_address, subject, hydrated)
         VALUES ('m2', 'm2', 'm2', 'm2', 1760000001000, 'bo@example.org', 'Two', 1)",
        [],
    )
    .expect("a message with no body");

    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("out.mbox");
    let written = write_mbox(&conn, &path).expect("an mbox");

    assert_eq!(written, 1, "the export is of what is on the device");
    let text = std::fs::read_to_string(&path).expect("the file");
    assert!(text.contains("From ana@example.org "));
    assert!(!text.contains("bo@example.org"));
}

#[test]
fn the_decisions_round_trip_through_the_journal() {
    let from = open();
    state::write::set_rule(&from, "maya@example.org", false, Destination::Inbox, None)
        .expect("a rule");
    state::write::add_note(&from, "<t1@example>", "Ask about the oak finish", None)
        .expect("a note");

    let records: Vec<_> = state::merge::devices(&from)
        .expect("devices")
        .into_iter()
        .flat_map(|device| state::merge::export(&from, &device, 0).expect("export"))
        .collect();
    let encoded = state::merge::encode(&records).expect("encode");

    // A second device with nothing on it, which is what an import into a fresh install is.
    let to = open();
    state::merge::absorb(&to, &state::merge::decode(&encoded).expect("decode")).expect("absorb");

    assert_eq!(
        state::read::destination_for(&to, "maya@example.org").expect("rule"),
        Some(Destination::Inbox)
    );
    assert_eq!(state::read::notes_on(&to, "<t1@example>").expect("notes").len(), 1);
}

#[test]
fn importing_the_same_export_twice_changes_nothing() {
    let from = open();
    state::write::set_rule(&from, "maya@example.org", false, Destination::Feed, None)
        .expect("a rule");
    let records: Vec<_> = state::merge::devices(&from)
        .expect("devices")
        .into_iter()
        .flat_map(|device| state::merge::export(&from, &device, 0).expect("export"))
        .collect();

    let to = open();
    state::merge::absorb(&to, &records).expect("first");
    state::merge::absorb(&to, &records).expect("second");

    let rules = state::read::rules(&to, "acct").expect("rules");
    assert_eq!(rules.len(), 1, "an import is a merge, not an append");
}

#[test]
fn importing_into_an_account_that_has_moved_on_is_a_merge_rather_than_a_loss() {
    let from = open();
    state::write::set_rule(&from, "maya@example.org", false, Destination::Feed, None)
        .expect("the exported rule");
    let records: Vec<_> = state::merge::devices(&from)
        .expect("devices")
        .into_iter()
        .flat_map(|device| state::merge::export(&from, &device, 0).expect("export"))
        .collect();

    let to = open();
    state::write::set_rule(&to, "ben@example.org", false, Destination::Inbox, None)
        .expect("a decision made since");
    state::merge::absorb(&to, &records).expect("absorb");

    assert_eq!(
        state::read::destination_for(&to, "ben@example.org").expect("kept"),
        Some(Destination::Inbox),
        "a decision made here is not lost by importing one made there"
    );
    assert_eq!(
        state::read::destination_for(&to, "maya@example.org").expect("arrived"),
        Some(Destination::Feed)
    );
}
