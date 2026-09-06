// Which of the three paths a sender's headers offer, over a pair of in-memory databases.
//
// No network. What is under test is the choice, because the choice is the part with a rule in it:
// a POST is a confirmation, a message is a confirmation once it goes, and a page is neither.

use rusqlite::Connection;

use crate::db;

fn open() -> Connection {
    db::memory().expect("a pair of in-memory databases")
}

fn from(
    conn: &Connection,
    id: &str,
    address: &str,
    at: i64,
    list_unsubscribe: Option<&str>,
    list_unsub_post: Option<&str>,
) {
    let key = format!("<{id}@example>");
    let tid = format!("t-{id}");
    conn.execute(
        "INSERT INTO threads (provider_thread_id, thread_key, latest_ms, message_count, unseen,
                              subject, snippet, from_name, from_address, in_inbox)
         VALUES (?1, ?2, ?3, 1, 1, 'The week in review', 'snippet', 'A List', ?4, 1)",
        rusqlite::params![tid, key, at, address],
    )
    .expect("a thread");
    conn.execute(
        "INSERT INTO messages (id, provider_thread_id, thread_key, message_id, date_ms,
                               from_address, subject, snippet, hydrated, labels,
                               list_unsubscribe, list_unsub_post)
         VALUES (?1, ?2, ?3, ?3, ?4, ?5, 'The week in review', 'snippet', 1, '[\"INBOX\"]',
                 ?6, ?7)",
        rusqlite::params![
            format!("m-{id}"),
            tid,
            key,
            at,
            address,
            list_unsubscribe,
            list_unsub_post
        ],
    )
    .expect("a message");
}

#[test]
fn one_click_wins_when_the_headers_offer_it() {
    let conn = open();
    from(
        &conn,
        "one",
        "news@brand.example",
        100,
        Some("<https://brand.example/u/abc>, <mailto:unsub@brand.example>"),
        Some("List-Unsubscribe=One-Click"),
    );

    assert_eq!(
        super::path_for(&conn, "news@brand.example").expect("a path"),
        super::Path::OneClick {
            url: "https://brand.example/u/abc".to_string()
        }
    );
}

#[test]
fn the_mailto_is_next_when_there_is_no_one_click_header() {
    let conn = open();
    from(
        &conn,
        "one",
        "news@brand.example",
        100,
        Some("<https://brand.example/u/abc>, <mailto:unsub@brand.example?subject=stop>"),
        None,
    );

    assert_eq!(
        super::path_for(&conn, "news@brand.example").expect("a path"),
        super::Path::Mailto {
            mailto: "mailto:unsub@brand.example?subject=stop".to_string()
        },
        "a URL without the POST header is a page, and a message beats a page"
    );
}

#[test]
fn the_link_is_what_is_left_when_neither_is_offered() {
    let conn = open();
    from(
        &conn,
        "one",
        "news@brand.example",
        100,
        Some("<https://brand.example/preferences>"),
        None,
    );

    assert_eq!(
        super::path_for(&conn, "news@brand.example").expect("a path"),
        super::Path::Link {
            url: "https://brand.example/preferences".to_string()
        }
    );
}

#[test]
fn a_one_click_url_that_is_not_https_is_not_a_one_click() {
    let conn = open();
    from(
        &conn,
        "one",
        "news@brand.example",
        100,
        Some("<http://brand.example/u/abc>"),
        Some("List-Unsubscribe=One-Click"),
    );

    assert_eq!(
        super::path_for(&conn, "news@brand.example").expect("a path"),
        super::Path::Link {
            url: "http://brand.example/u/abc".to_string()
        },
        "RFC 8058 is an https POST, and sending it in the clear is not what the header promised"
    );
}

#[test]
fn a_sender_who_offers_nothing_says_so() {
    let conn = open();
    from(&conn, "one", "maya@example.org", 100, None, None);

    let refused = super::path_for(&conn, "maya@example.org");
    assert!(refused.is_err());
}

#[test]
fn the_newest_message_decides() {
    let conn = open();
    from(
        &conn,
        "old",
        "news@brand.example",
        100,
        Some("<mailto:unsub@brand.example>"),
        None,
    );
    from(
        &conn,
        "new",
        "news@brand.example",
        200,
        Some("<https://brand.example/u/abc>"),
        Some("List-Unsubscribe=One-Click"),
    );

    assert_eq!(
        super::path_for(&conn, "news@brand.example").expect("a path"),
        super::Path::OneClick {
            url: "https://brand.example/u/abc".to_string()
        },
        "a list that moved to one click last month is not unsubscribed from through last year"
    );
}

#[test]
fn a_mailto_becomes_the_message_it_asks_for() {
    let draft = super::mailto_draft("acct", "mailto:unsub@brand.example?subject=unsubscribe%20me")
        .expect("a draft");
    assert_eq!(draft.account_id, "acct");
    assert_eq!(draft.to.len(), 1);
    assert_eq!(draft.to[0].address, "unsub@brand.example");
    assert_eq!(draft.subject, "unsubscribe me");
}
