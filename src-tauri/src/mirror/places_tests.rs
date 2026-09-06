// Trash, spam and screened out, as the view answers for them.
//
// Over a pair of in-memory databases with the real `upsert_message` and `refresh_thread` behind
// them, so what is under test is the mirror's own reading of the `TRASH` and `SPAM` labels rather
// than a row written by hand to agree with the assertion. Every list call the app makes sets
// `includeSpamTrash`, so a junked message is already here with its body indexed; these are the
// queries that decide whether anything shows it.

use rusqlite::Connection;

use crate::db;
use crate::dto::{Destination, Place, ThreadPage, ThreadQuery};
use crate::mirror::{fts, read, write};
use crate::provider::{ProviderLabel, RawHeaders};

const INBOXED: &str = "inboxed@example.test";
const SPAMMED: &str = "spammed@example.test";
const TRASHED: &str = "trashed@example.test";
const SCREENED: &str = "screened@example.test";

fn headers(message_id: &str, from: &str, subject: &str, labels: &[&str]) -> RawHeaders {
    RawHeaders {
        id: format!("id-{message_id}"),
        thread_id: format!("t-{message_id}"),
        label_ids: labels.iter().map(|l| l.to_string()).collect(),
        internal_date_ms: 1_000,
        size: 1_000,
        snippet: String::new(),
        headers: vec![
            ("Message-ID".to_string(), format!("<{message_id}>")),
            ("From".to_string(), from.to_string()),
            ("To".to_string(), "PJ <pj@example.test>".to_string()),
            ("Subject".to_string(), subject.to_string()),
        ],
    }
}

/// Four threads, one per corner: routed to the Inbox, junked by Gmail, thrown away, and from a
/// sender who was screened out. Every subject carries the same word, so one query reaches them all
/// and what comes back is the place rule and not the query.
fn mailbox() -> Connection {
    let conn = db::memory().expect("a pair of in-memory databases");
    for (address, destination) in [
        ("ana@example.test", Destination::Inbox),
        ("verify@example.test", Destination::Inbox),
        ("parcels@example.test", Destination::PaperTrail),
        ("recruiter@example.test", Destination::ScreenedOut),
    ] {
        crate::state::write::set_rule(&conn, address, false, destination, None).expect("rule");
    }

    for (message_id, from, subject, labels) in [
        (INBOXED, "Ana <ana@example.test>", "Notice: the lease", vec!["INBOX"]),
        (SPAMMED, "Verify <verify@example.test>", "Notice: verify your account", vec!["SPAM"]),
        (TRASHED, "Parcels <parcels@example.test>", "Notice: your parcel", vec!["TRASH"]),
        (
            SCREENED,
            "Talent <recruiter@example.test>",
            "Notice: an opportunity",
            vec!["INBOX"],
        ),
    ] {
        let raw = headers(message_id, from, subject, &labels);
        let id = write::upsert_message(&conn, &raw, false).expect("message");
        fts::index(&conn, &id).expect("index");
        write::refresh_thread(&conn, &raw.thread_id).expect("thread");
    }
    conn
}

fn page(conn: &Connection, place: Place, query: Option<&str>) -> ThreadPage {
    read::threads_list(
        conn,
        "acct",
        "hue-1",
        &ThreadQuery {
            account_id: None,
            place,
            label_id: None,
            query: query.map(str::to_string),
            limit: 50,
            cursor: None,
        },
        write::now_ms(),
    )
    .expect("a page")
}

fn keys(page: &ThreadPage) -> Vec<String> {
    let mut keys: Vec<String> = page.threads.iter().map(|t| t.key.clone()).collect();
    keys.sort();
    keys
}

/// Everything is the one place with no filter in it, so a message Gmail junked is in it. Trash is
/// the exception: a thread you threw away is not one you are looking through.
#[test]
fn everything_holds_the_spam_thread_and_not_the_trashed_one() {
    let conn = mailbox();
    assert_eq!(
        keys(&page(&conn, Place::Everything, None)),
        vec![INBOXED.to_string(), SCREENED.to_string(), SPAMMED.to_string()]
    );
    assert_eq!(keys(&page(&conn, Place::Trash, None)), vec![TRASHED.to_string()]);
    assert_eq!(keys(&page(&conn, Place::Spam, None)), vec![SPAMMED.to_string()]);
    assert_eq!(keys(&page(&conn, Place::ScreenedOut, None)), vec![SCREENED.to_string()]);
    // And the daily places are unchanged by any of it.
    assert_eq!(keys(&page(&conn, Place::Inbox, None)), vec![INBOXED.to_string()]);
}

/// The message you most need to find is the one something else decided you should not see, so a
/// search with no `in:` in it reaches all three and the rows say which of them they came from.
#[test]
fn a_bare_search_reaches_trash_and_spam_and_the_rows_carry_the_flags() {
    let conn = mailbox();
    let found = page(&conn, Place::Search, Some("notice"));
    assert_eq!(
        keys(&found),
        vec![
            INBOXED.to_string(),
            SCREENED.to_string(),
            SPAMMED.to_string(),
            TRASHED.to_string()
        ]
    );
    let flags = |key: &str| {
        let row = found.threads.iter().find(|t| t.key == key).expect("a row");
        (row.trashed, row.spam)
    };
    assert_eq!(flags(TRASHED), (true, false));
    assert_eq!(flags(SPAMMED), (false, true));
    assert_eq!(flags(INBOXED), (false, false));
}

#[test]
fn in_names_one_of_the_three_and_narrows_to_it() {
    let conn = mailbox();
    assert_eq!(read::place_named("screened-out"), Some(Place::ScreenedOut));
    assert_eq!(read::place_named("screenedout"), Some(Place::ScreenedOut));
    assert_eq!(
        keys(&page(&conn, Place::Search, Some("notice in:screened-out"))),
        vec![SCREENED.to_string()]
    );
    assert_eq!(
        keys(&page(&conn, Place::Search, Some("notice in:trash"))),
        vec![TRASHED.to_string()]
    );
    assert_eq!(
        keys(&page(&conn, Place::Search, Some("notice in:spam"))),
        vec![SPAMMED.to_string()]
    );
}

/// There is no Empty button, because deleting for good needs the scope that is the whole mailbox.
/// The line at the foot states the rule Gmail applies anyway. Screened out carries none: it falls
/// off with the storage window like everything else of its age.
#[test]
fn trash_and_spam_say_what_gmail_will_do_after_thirty_days() {
    let conn = mailbox();
    let rule = Some("Gmail empties this after 30 days.".to_string());
    assert_eq!(page(&conn, Place::Trash, None).footer, rule);
    assert_eq!(page(&conn, Place::Spam, None).footer, rule);
    assert_eq!(page(&conn, Place::ScreenedOut, None).footer, None);
    assert_eq!(page(&conn, Place::Inbox, None).footer, None);
}

/// The pane reads the thread rather than the row it was opened from, so the two flags have to be
/// on the view as well or the banner offering the way back out has nothing to go on.
#[test]
fn the_view_carries_the_same_two_flags_as_the_row() {
    let conn = mailbox();
    let view = |key: &str| read::thread_view(&conn, "acct", key, &[]).expect("the thread");
    assert_eq!((view(TRASHED).trashed, view(TRASHED).spam), (true, false));
    assert_eq!((view(SPAMMED).trashed, view(SPAMMED).spam), (false, true));
    assert_eq!((view(INBOXED).trashed, view(INBOXED).spam), (false, false));
}

/// The provider's own labels are places this app already has under names of its own, so they are
/// stored and never offered. Handing them over means a palette that answers "spam" with a Labels
/// row and an Other row, and a picker that will put `SPAM` on a thread as though it were a choice.
#[test]
fn the_providers_system_labels_are_kept_and_not_offered() {
    let conn = mailbox();
    let label = |id: &str, name: &str, kind: &str| ProviderLabel {
        id: id.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
    };
    write::upsert_labels(
        &conn,
        &[
            label("SPAM", "SPAM", "system"),
            label("IMPORTANT", "IMPORTANT", "system"),
            label("Label_7", "The flat", "user"),
        ],
    )
    .expect("labels");

    let offered = read::labels(&conn, "acct").expect("the labels");
    assert_eq!(
        offered.iter().map(|l| l.name.clone()).collect::<Vec<_>>(),
        vec!["The flat".to_string()]
    );

    // Stored all the same: `in:` and `label:` resolve a name against this table, and the mirror
    // writes `SPAM` and `TRASH` onto messages whether or not anything lists them.
    let stored: i64 = conn
        .query_row("SELECT COUNT(*) FROM labels", [], |row| row.get(0))
        .expect("a count");
    assert_eq!(stored, 3);
}
