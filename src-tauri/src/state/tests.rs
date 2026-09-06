// The log and its tables, over a pair of in-memory databases.
//
// Almost everything here asserts the same property from a different angle: the tables are a view of
// the log. A test that only checked a write landed would pass just as happily against a module that
// wrote the row and forgot the event, and that module would lose a year of somebody's decisions the
// first time they opened the app on a second machine. So the shape of these is write, then rebuild
// from the log alone, then compare, and the ones about two devices compare a pair of orders rather
// than a pair of expected values.

use rusqlite::Connection;

use crate::db;
use crate::dto::{ContactPatch, Destination, Person, Pile, SnoozeKind};

use super::journal::{self, Payload, Record};
use super::{device, merge, read, write};

/// The eleven materialised tables, written out rather than asked of `Kind`, so that a kind losing
/// its table is a failing test rather than a table nobody looks at any more.
const TABLES: [&str; 11] = [
    "sender_rules",
    "piles",
    "snoozes",
    "notes",
    "renames",
    "merges",
    "clips",
    "thread_flags",
    "contacts",
    "prefs",
    "markers",
];

fn open() -> Connection {
    db::memory().expect("a pair of in-memory databases")
}

fn ana() -> Person {
    Person {
        name: Some("Ana".to_string()),
        address: "ana@example.com".to_string(),
    }
}

fn dump(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("SELECT * FROM state.{table}"))
        .expect("prepare");
    let columns = stmt.column_count();
    let rows = stmt
        .query_map([], |row| {
            let mut cells = Vec::new();
            for at in 0..columns {
                cells.push(format!("{:?}", row.get::<_, rusqlite::types::Value>(at)?));
            }
            Ok(cells.join("|"))
        })
        .expect("query");
    let mut out: Vec<String> = rows.map(|row| row.expect("row")).collect();
    out.sort();
    out
}

/// Every materialised table, row by row, in an order that does not depend on how they were written.
fn snapshot(conn: &Connection) -> Vec<String> {
    let mut out = Vec::new();
    for table in TABLES {
        for row in dump(conn, table) {
            out.push(format!("{table}: {row}"));
        }
    }
    out
}

/// One of every kind, so that a test about the whole log has the whole log.
fn every_kind(conn: &Connection) {
    write::set_rule(
        conn,
        "Ana@Example.com",
        false,
        Destination::Inbox,
        Some("written by a person"),
    )
    .expect("rule");
    write::set_pile(conn, "key-1", Pile::ReplyLater).expect("pile");
    write::set_snooze(conn, "key-2", 8_000, SnoozeKind::Tomorrow, 5_000).expect("snooze");
    write::add_note(conn, "key-1", "ring them back", Some("mid-1")).expect("note");
    write::set_rename(conn, "key-1", "The lease").expect("rename");
    write::merge_threads(conn, &["key-3".to_string()], "key-1").expect("merge");
    write::add_clip(
        conn,
        "key-1",
        "mid-1",
        "the bit that mattered",
        &ana(),
        "Re: the lease",
    )
    .expect("clip");
    write::set_thread_flags(conn, "key-1", Some(true), Some(false)).expect("flags");
    write::set_contact(
        conn,
        "ana@example.com",
        &ContactPatch {
            note: Some("owes me a call".to_string()),
            notify: Some(true),
            ..ContactPatch::default()
        },
    )
    .expect("contact");
    write::set_pref(conn, "signature", "Ana").expect("pref");
    write::set_marker(conn, "feed", 9_000).expect("marker");
}

/// Moves the log across without moving a single materialised row, which is the only honest way to
/// ask whether the tables can be rebuilt from it.
fn transplant_log(from: &Connection, to: &Connection) {
    for record in journal::records(from).expect("records") {
        journal::insert(to, &record).expect("insert");
    }
}

fn raw(device_id: &str, seq: i64, at_ms: i64, kind: &str, key: &str, payload: &str) -> Record {
    Record {
        device_id: device_id.to_string(),
        seq,
        at_ms,
        kind: kind.to_string(),
        key: key.to_string(),
        payload: payload.to_string(),
    }
}

// -- the write path ----------------------------------------------------------------------------

#[test]
fn every_kind_of_event_is_appended_and_applied() {
    let conn = open();
    every_kind(&conn);

    let records = journal::records(&conn).expect("records");
    let mut kinds: Vec<String> = records.iter().map(|record| record.kind.clone()).collect();
    kinds.sort();
    kinds.dedup();
    assert_eq!(kinds.len(), 11, "one event of every kind: {kinds:?}");

    let rule = read::rule_for(&conn, "acc", "ana@example.com")
        .expect("rule")
        .expect("a rule");
    assert_eq!(rule.destination, Destination::Inbox);
    assert_eq!(rule.subject, "ana@example.com", "the key is lowercased");
    assert_eq!(rule.reason.as_deref(), Some("written by a person"));

    assert_eq!(
        read::pile_of(&conn, "key-1").expect("pile"),
        Some(Pile::ReplyLater)
    );
    let snooze = read::snooze_of(&conn, "key-2")
        .expect("snooze")
        .expect("a snooze");
    assert_eq!(snooze.return_at, 8_000);
    assert_eq!(snooze.kind, SnoozeKind::Tomorrow);
    assert_eq!(snooze.watermark, 5_000);

    let notes = read::notes_on(&conn, "key-1").expect("notes");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].body, "ring them back");
    assert_eq!(notes[0].after_message_id.as_deref(), Some("mid-1"));

    assert_eq!(
        read::rename_of(&conn, "key-1").expect("rename").as_deref(),
        Some("The lease")
    );
    assert_eq!(
        read::effective_key(&conn, "key-3").expect("effective"),
        "key-1"
    );

    let clips = read::clips(&conn, "acc", 10).expect("clips");
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].text, "the bit that mattered");
    assert_eq!(clips[0].sender.address, "ana@example.com");

    let flags = read::flags_of(&conn, "key-1").expect("flags");
    assert!(flags.ignored);
    assert!(!flags.notify);

    let contact = read::contact(&conn, "ana@example.com")
        .expect("contact")
        .expect("a contact");
    assert_eq!(contact.note.as_deref(), Some("owes me a call"));
    assert!(contact.notify);
    assert!(contact.bundle, "bundling is on until it is turned off");

    assert_eq!(
        read::pref(&conn, "signature").expect("pref").as_deref(),
        Some("Ana")
    );
    assert_eq!(read::marker(&conn, "feed").expect("marker"), Some(9_000));
}

#[test]
fn a_patch_writes_the_whole_value_it_becomes() {
    let conn = open();
    write::set_thread_flags(&conn, "key-1", Some(true), None).expect("ignore");
    write::set_thread_flags(&conn, "key-1", None, Some(true)).expect("notify");

    let flags = read::flags_of(&conn, "key-1").expect("flags");
    assert!(flags.ignored, "the second event kept what the first said");
    assert!(flags.notify);

    let last = journal::records(&conn).expect("records").pop().expect("one");
    assert_eq!(
        last.payload,
        serde_json::to_string(&Payload::ThreadFlag {
            ignored: true,
            notify: true
        })
        .expect("json"),
        "the payload is the value, not the difference"
    );
}

#[test]
fn a_pile_is_a_stack_and_leaving_it_removes_the_row() {
    let conn = open();
    write::set_pile(&conn, "key-1", Pile::ReplyLater).expect("one");
    write::set_pile(&conn, "key-2", Pile::ReplyLater).expect("two");
    write::set_pile(&conn, "key-3", Pile::SetAside).expect("three");

    assert_eq!(
        read::pile_keys(&conn, Pile::ReplyLater).expect("keys"),
        vec!["key-2".to_string(), "key-1".to_string()],
        "the last one put on the pile is on top"
    );

    write::clear_pile(&conn, "key-1").expect("clear");
    assert_eq!(read::pile_of(&conn, "key-1").expect("pile"), None);
    assert!(
        dump(&conn, "piles").iter().all(|row| !row.contains("key-1")),
        "a thread out of a pile leaves no row for eviction to trip over"
    );
}

// -- replay ------------------------------------------------------------------------------------

#[test]
fn replaying_a_log_into_an_empty_database_reproduces_every_table() {
    let written = open();
    every_kind(&written);
    let expected = snapshot(&written);
    assert!(!expected.is_empty());

    let empty = open();
    transplant_log(&written, &empty);
    assert_eq!(snapshot(&empty), Vec::<String>::new(), "nothing applied yet");

    let report = journal::replay(&empty).expect("replay");
    assert_eq!(report.skipped, 0);
    assert!(report.unknown.is_empty());
    assert_eq!(
        report.applied,
        journal::records(&written).expect("records").len()
    );
    assert_eq!(snapshot(&empty), expected);
}

#[test]
fn replaying_over_a_full_database_changes_nothing() {
    let conn = open();
    every_kind(&conn);
    let before = snapshot(&conn);
    journal::replay(&conn).expect("replay");
    assert_eq!(snapshot(&conn), before);
}

#[test]
fn an_event_of_an_unknown_kind_is_skipped_with_the_rest_of_the_replay_intact() {
    let conn = open();
    every_kind(&conn);
    let expected = snapshot(&conn);

    journal::insert(
        &conn,
        &raw(
            "a-newer-device",
            1,
            50_000,
            "constellation",
            "key-1",
            "{\"event\":\"constellation\",\"shape\":\"orion\"}",
        ),
    )
    .expect("insert");

    let report = journal::replay(&conn).expect("replay");
    assert_eq!(report.unknown, vec!["constellation".to_string()]);
    assert_eq!(report.skipped, 1);
    assert_eq!(snapshot(&conn), expected, "everything else still landed");

    let held: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM state.journal WHERE kind = 'constellation'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(held, 1, "the event a newer build wrote is kept, not dropped");
}

// -- two devices -------------------------------------------------------------------------------

#[test]
fn two_logs_land_in_the_same_place_in_either_order() {
    let one = open();
    let two = open();

    write::set_rule(&one, "ana@example.com", false, Destination::Inbox, None).expect("rule");
    journal::append_at(&one, 2_000, "key-1", &Payload::Rename { name: Some("Ours".into()) })
        .expect("rename");
    write::add_note(&two, "key-1", "from the other machine", None).expect("note");
    journal::append_at(
        &two,
        3_000,
        "key-1",
        &Payload::Pile {
            pile: Some("set-aside".into()),
            position: 1,
        },
    )
    .expect("pile");

    let first = journal::records(&one).expect("records");
    let second = journal::records(&two).expect("records");

    let forwards = open();
    merge::absorb(&forwards, &first).expect("absorb");
    merge::absorb(&forwards, &second).expect("absorb");

    let backwards = open();
    merge::absorb(&backwards, &second).expect("absorb");
    merge::absorb(&backwards, &first).expect("absorb");

    assert_eq!(snapshot(&forwards), snapshot(&backwards));
    assert_eq!(
        read::rename_of(&forwards, "key-1").expect("rename").as_deref(),
        Some("Ours")
    );
}

#[test]
fn the_device_id_breaks_a_tie_inside_the_same_millisecond() {
    let one = open();
    let two = open();
    let one_id = device::device_id(&one).expect("device");
    let two_id = device::device_id(&two).expect("device");
    assert_ne!(one_id, two_id, "two installations, two ids");

    journal::append_at(&one, 1_000, "key-1", &Payload::Rename { name: Some("from one".into()) })
        .expect("rename");
    journal::append_at(&two, 1_000, "key-1", &Payload::Rename { name: Some("from two".into()) })
        .expect("rename");

    let first = journal::records(&one).expect("records");
    let second = journal::records(&two).expect("records");

    let forwards = open();
    merge::absorb(&forwards, &first).expect("absorb");
    merge::absorb(&forwards, &second).expect("absorb");

    let backwards = open();
    merge::absorb(&backwards, &second).expect("absorb");
    merge::absorb(&backwards, &first).expect("absorb");

    assert_eq!(
        snapshot(&forwards),
        snapshot(&backwards),
        "the same millisecond, and still the same answer"
    );

    let winner = if one_id > two_id { "from one" } else { "from two" };
    assert_eq!(
        read::rename_of(&forwards, "key-1").expect("rename").as_deref(),
        Some(winner),
        "the higher device id wins, whichever arrived first"
    );
}

#[test]
fn a_device_that_missed_a_month_catches_up_to_the_same_place() {
    let away = open();
    let busy = open();

    let day = 86_400_000_i64;
    for step in 0..30_i64 {
        journal::append_at(
            &busy,
            step * day,
            &format!("key-{}", step % 4),
            &Payload::Marker {
                seen_ms: step * day,
            },
        )
        .expect("marker");
        journal::append_at(
            &busy,
            step * day,
            "key-1",
            &Payload::Rename {
                name: Some(format!("day {step}")),
            },
        )
        .expect("rename");
    }

    // The device that was here for the first week and then went away for the rest of the month.
    let month = journal::records(&busy).expect("records");
    let (first_week, rest): (Vec<Record>, Vec<Record>) =
        month.iter().cloned().partition(|record| record.at_ms < 7 * day);
    merge::absorb(&away, &first_week).expect("absorb");
    assert_eq!(
        read::rename_of(&away, "key-1").expect("rename").as_deref(),
        Some("day 6")
    );

    merge::absorb(&away, &rest).expect("absorb");

    let all_along = open();
    merge::absorb(&all_along, &month).expect("absorb");

    assert_eq!(snapshot(&away), snapshot(&all_along));
    assert_eq!(
        read::rename_of(&away, "key-1").expect("rename").as_deref(),
        Some("day 29")
    );
}

#[test]
fn absorbing_the_same_segment_twice_changes_nothing() {
    let conn = open();
    let other = open();
    every_kind(&other);
    let records = journal::records(&other).expect("records");

    let first = merge::absorb(&conn, &records).expect("absorb");
    let before = snapshot(&conn);
    let again = merge::absorb(&conn, &records).expect("absorb");

    assert_eq!(first.applied, records.len());
    assert_eq!(again.applied, 0, "nothing in it was new");
    assert_eq!(snapshot(&conn), before);
}

// -- deletes -----------------------------------------------------------------------------------

#[test]
fn a_delete_that_arrives_before_its_creation_still_wins() {
    let made = open();
    let unmade = open();

    journal::append_at(
        &made,
        1_000,
        "key-1",
        &Payload::Pile {
            pile: Some("reply-later".into()),
            position: 1,
        },
    )
    .expect("pile");
    journal::append_at(
        &unmade,
        2_000,
        "key-1",
        &Payload::Pile {
            pile: None,
            position: 0,
        },
    )
    .expect("out of the pile");

    let conn = open();
    merge::absorb(&conn, &journal::records(&unmade).expect("records")).expect("absorb");
    assert_eq!(read::pile_of(&conn, "key-1").expect("pile"), None);

    merge::absorb(&conn, &journal::records(&made).expect("records")).expect("absorb");
    assert_eq!(
        read::pile_of(&conn, "key-1").expect("pile"),
        None,
        "the tombstone is the event, and the event is still in the log"
    );

    let in_order = open();
    merge::absorb(&in_order, &journal::records(&made).expect("records")).expect("absorb");
    merge::absorb(&in_order, &journal::records(&unmade).expect("records")).expect("absorb");
    assert_eq!(snapshot(&in_order), snapshot(&conn));
}

#[test]
fn a_note_deleted_here_and_edited_there_resolves_the_same_way_in_either_order() {
    let shared = open();
    let id = "note-1";
    journal::append_at(
        &shared,
        1_000,
        id,
        &Payload::Note {
            thread_key: "key-1".into(),
            body: "the first thought".into(),
            after_message_id: None,
            created_at: 1_000,
            deleted: false,
        },
    )
    .expect("note");
    let start = journal::records(&shared).expect("records");

    let deleting = open();
    merge::absorb(&deleting, &start).expect("absorb");
    let editing = open();
    merge::absorb(&editing, &start).expect("absorb");

    journal::append_at(
        &deleting,
        4_000,
        id,
        &Payload::Note {
            thread_key: "key-1".into(),
            body: "the first thought".into(),
            after_message_id: None,
            created_at: 1_000,
            deleted: true,
        },
    )
    .expect("delete");
    journal::append_at(
        &editing,
        5_000,
        id,
        &Payload::Note {
            thread_key: "key-1".into(),
            body: "the second thought".into(),
            after_message_id: None,
            created_at: 1_000,
            deleted: false,
        },
    )
    .expect("edit");

    let deleted = journal::records(&deleting).expect("records");
    let edited = journal::records(&editing).expect("records");

    let forwards = open();
    merge::absorb(&forwards, &deleted).expect("absorb");
    merge::absorb(&forwards, &edited).expect("absorb");

    let backwards = open();
    merge::absorb(&backwards, &edited).expect("absorb");
    merge::absorb(&backwards, &deleted).expect("absorb");

    assert_eq!(snapshot(&forwards), snapshot(&backwards));
    let notes = read::notes_on(&forwards, "key-1").expect("notes");
    assert_eq!(notes.len(), 1, "the later edit brought it back");
    assert_eq!(notes[0].body, "the second thought");
}

#[test]
fn two_devices_writing_different_notes_on_one_thread_keep_both() {
    let one = open();
    let two = open();
    write::add_note(&one, "key-1", "mine", None).expect("note");
    write::add_note(&two, "key-1", "theirs", None).expect("note");

    let conn = open();
    merge::absorb(&conn, &journal::records(&one).expect("records")).expect("absorb");
    merge::absorb(&conn, &journal::records(&two).expect("records")).expect("absorb");

    let mut bodies: Vec<String> = read::notes_on(&conn, "key-1")
        .expect("notes")
        .into_iter()
        .map(|note| note.body)
        .collect();
    bodies.sort();
    assert_eq!(bodies, vec!["mine".to_string(), "theirs".to_string()]);
}

// -- routing -----------------------------------------------------------------------------------

#[test]
fn an_address_rule_beats_a_domain_rule() {
    let conn = open();
    write::set_rule(&conn, "example.com", true, Destination::Feed, None).expect("domain");
    write::set_rule(&conn, "ana@example.com", false, Destination::Inbox, None).expect("address");

    let ana = read::rule_for(&conn, "acc", "Ana@Example.com")
        .expect("rule")
        .expect("a rule");
    assert_eq!(ana.destination, Destination::Inbox);
    assert!(!ana.is_domain);

    let anyone = read::rule_for(&conn, "acc", "bob@example.com")
        .expect("rule")
        .expect("a rule");
    assert_eq!(anyone.destination, Destination::Feed);
    assert!(anyone.is_domain);

    assert_eq!(
        read::destination_for(&conn, "nobody@elsewhere.com").expect("none"),
        None
    );
}

#[test]
fn a_domain_rule_on_a_consumer_domain_is_refused() {
    let conn = open();
    for domain in ["gmail.com", "outlook.com", "yahoo.com", "icloud.com", "proton.me", "hey.com"] {
        assert!(
            write::set_rule(&conn, domain, true, Destination::Feed, None).is_err(),
            "{domain} is not one sender"
        );
    }
    write::set_rule(&conn, "ana@gmail.com", false, Destination::Feed, None)
        .expect("one person at a consumer domain is still one person");
    assert!(
        write::set_rule(&conn, "example.com", true, Destination::Feed, None).is_ok(),
        "a company domain can carry one"
    );
}

// -- merges ------------------------------------------------------------------------------------

#[test]
fn the_effective_key_follows_a_merge_of_a_merge_and_comes_back_after_an_unmerge() {
    let conn = open();
    write::merge_threads(&conn, &["key-b".to_string()], "key-a").expect("merge");
    assert_eq!(read::effective_key(&conn, "key-b").expect("key"), "key-a");

    write::merge_threads(&conn, &["key-a".to_string()], "key-c").expect("merge of a merge");
    assert_eq!(read::effective_key(&conn, "key-b").expect("key"), "key-c");
    assert_eq!(read::effective_key(&conn, "key-a").expect("key"), "key-c");
    assert_eq!(
        read::effective_key(&conn, "key-c").expect("key"),
        "key-c",
        "the thread everything merged into is itself"
    );
    assert_eq!(
        read::merge_sources(&conn, "key-c").expect("sources"),
        vec!["key-a".to_string(), "key-b".to_string()]
    );

    write::unmerge(&conn, "key-c").expect("unmerge");
    assert_eq!(
        read::effective_key(&conn, "key-b").expect("key"),
        "key-a",
        "one layer at a time: b is still part of a"
    );
    write::unmerge(&conn, "key-a").expect("unmerge");
    assert_eq!(read::effective_key(&conn, "key-b").expect("key"), "key-b");
    assert!(read::merge_sources(&conn, "key-a")
        .expect("sources")
        .is_empty());
}

#[test]
fn a_merge_that_points_at_itself_does_not_hang() {
    let conn = open();
    journal::append_at(
        &conn,
        1_000,
        "key-a",
        &Payload::Merge {
            merged_key: Some("key-b".into()),
        },
    )
    .expect("merge");
    journal::append_at(
        &conn,
        1_000,
        "key-b",
        &Payload::Merge {
            merged_key: Some("key-a".into()),
        },
    )
    .expect("merge back");

    assert_eq!(read::effective_key(&conn, "key-a").expect("key"), "key-a");
    assert_eq!(read::effective_key(&conn, "key-b").expect("key"), "key-b");
}

#[test]
fn an_unmerge_that_arrives_before_its_merge_still_wins() {
    let merging = open();
    let unmerging = open();
    journal::append_at(
        &merging,
        1_000,
        "key-b",
        &Payload::Merge {
            merged_key: Some("key-a".into()),
        },
    )
    .expect("merge");
    journal::append_at(&unmerging, 2_000, "key-b", &Payload::Merge { merged_key: None })
        .expect("unmerge");

    let conn = open();
    merge::absorb(&conn, &journal::records(&unmerging).expect("records")).expect("absorb");
    merge::absorb(&conn, &journal::records(&merging).expect("records")).expect("absorb");
    assert_eq!(read::effective_key(&conn, "key-b").expect("key"), "key-b");
}

// -- the log as a thing to move around ----------------------------------------------------------

#[test]
fn a_segment_survives_a_round_trip_through_its_own_format() {
    let conn = open();
    every_kind(&conn);
    let id = device::device_id(&conn).expect("device");

    assert_eq!(merge::devices(&conn).expect("devices"), vec![id.clone()]);
    let high = merge::high_water(&conn).expect("high water");
    assert_eq!(high.len(), 1);
    assert_eq!(high[0].0, id);

    let segment = merge::export(&conn, &id, 0).expect("export");
    assert_eq!(segment.len() as i64, high[0].1);
    assert_eq!(
        merge::export(&conn, &id, high[0].1).expect("export").len(),
        0,
        "nothing above the high water mark"
    );

    let carried = merge::decode(&merge::encode(&segment).expect("encode")).expect("decode");
    assert_eq!(carried, segment);

    let restored = open();
    merge::absorb(&restored, &carried).expect("absorb");
    assert_eq!(snapshot(&restored), snapshot(&conn));
}

#[test]
fn the_device_id_is_made_once_and_kept() {
    let conn = open();
    let first = device::device_id(&conn).expect("device");
    let again = device::device_id(&conn).expect("device");
    assert_eq!(first, again);
    assert_eq!(first.len(), 16);
    assert_eq!(
        device::meta_get(&conn, device::DEVICE_KEY).expect("meta"),
        Some(first)
    );
}

#[test]
fn a_sequence_is_per_device_and_starts_at_one() {
    let conn = open();
    let first = journal::append(&conn, "key-1", &Payload::Marker { seen_ms: 1 }).expect("one");
    let second = journal::append(&conn, "key-2", &Payload::Marker { seen_ms: 2 }).expect("two");
    assert_eq!((first.seq, second.seq), (1, 2));

    merge::absorb(
        &conn,
        &[raw(
            "somebody-else",
            1,
            10,
            "marker",
            "key-3",
            "{\"event\":\"marker\",\"seenMs\":3}",
        )],
    )
    .expect("absorb");

    let third = journal::append(&conn, "key-4", &Payload::Marker { seen_ms: 4 }).expect("three");
    assert_eq!(
        third.seq, 3,
        "another device's sequence is not this device's"
    );
    assert_eq!(read::marker(&conn, "key-3").expect("marker"), Some(3));
}

// -- snoozes -----------------------------------------------------------------------------------

#[test]
fn a_snooze_is_due_when_its_moment_has_passed_and_gone_when_it_is_cleared() {
    let conn = open();
    write::set_snooze(&conn, "key-1", 1_000, SnoozeKind::Tomorrow, 0).expect("snooze");
    write::set_snooze(&conn, "key-2", 9_000, SnoozeKind::IfNoReply, 500).expect("snooze");

    let due = read::due_snoozes(&conn, 5_000).expect("due");
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].thread_key, "key-1");
    assert_eq!(read::snoozes(&conn).expect("all").len(), 2);

    write::clear_snooze(&conn, "key-1").expect("clear");
    assert!(read::due_snoozes(&conn, 5_000).expect("due").is_empty());
    assert_eq!(read::snooze_of(&conn, "key-1").expect("gone"), None);

    write::mark_returned(&conn, "key-1", 1_000).expect("returned");
    assert_eq!(
        read::returned(&conn).expect("returned"),
        vec!["key-1".to_string()]
    );
    write::clear_returned(&conn, "key-1").expect("opened");
    assert!(read::returned(&conn).expect("returned").is_empty());
}
