// The gate, over a pair of in-memory databases.
//
// Threads and messages are inserted with SQL rather than through the sync engine on purpose: what
// is being tested is which place a thread lands in given a rule, and building that thread through a
// fake provider would test the engine again and hide the clause.

use rusqlite::Connection;

use crate::db;
use crate::dto::{Destination, Place, ThreadQuery};
use crate::mirror;
use crate::state;

fn open() -> Connection {
    db::memory().expect("a pair of in-memory databases")
}

struct Sender<'a> {
    address: &'a str,
    name: &'a str,
    subject: &'a str,
    /// The headers the suggestion reads. Empty is a person writing.
    list_unsubscribe: Option<&'a str>,
    auto_submitted: Option<&'a str>,
    /// This account has written in the thread, which is what "a thread you are in" means.
    replied: bool,
}

impl<'a> Sender<'a> {
    fn person(address: &'a str, name: &'a str) -> Self {
        Sender {
            address,
            name,
            subject: "Hello",
            list_unsubscribe: None,
            auto_submitted: None,
            replied: false,
        }
    }
}

/// One thread from one sender, with one message, at a fixed moment.
fn arrive(conn: &Connection, sender: &Sender<'_>, at: i64) -> String {
    let key = format!("<{}-{at}@example>", sender.address);
    let tid = format!("t-{}-{at}", sender.address);
    conn.execute(
        "INSERT INTO threads (provider_thread_id, thread_key, latest_ms, message_count, unseen,
                              subject, snippet, from_name, from_address, in_inbox)
         VALUES (?1, ?2, ?3, 1, 1, ?4, 'snippet', ?5, ?6, 1)",
        rusqlite::params![tid, key, at, sender.subject, sender.name, sender.address],
    )
    .expect("a thread");
    conn.execute(
        "INSERT INTO messages (id, provider_thread_id, thread_key, message_id, date_ms,
                               from_address, subject, snippet, hydrated, labels,
                               list_unsubscribe, auto_submitted)
         VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, 'snippet', 1, '[\"INBOX\"]', ?7, ?8)",
        rusqlite::params![
            format!("m-{tid}"),
            tid,
            key,
            at,
            sender.address,
            sender.subject,
            sender.list_unsubscribe,
            sender.auto_submitted,
        ],
    )
    .expect("a message");
    if sender.replied {
        conn.execute(
            "INSERT INTO messages (id, provider_thread_id, thread_key, message_id, date_ms,
                                   from_address, subject, hydrated, sent, labels)
             VALUES (?1, ?2, ?3, ?3, ?4, 'you@example.com', ?5, 1, 1, '[\"SENT\"]')",
            rusqlite::params![format!("m-{tid}-reply"), tid, key, at + 1, sender.subject],
        )
        .expect("a reply of our own");
    }
    key
}

fn keys_in(conn: &Connection, place: Place) -> Vec<String> {
    let query = ThreadQuery {
        account_id: None,
        place,
        label_id: None,
        query: None,
        limit: 50,
        cursor: None,
    };
    mirror::read::threads_list(conn, "acct", "hue-1", &query, 2_000_000_000_000)
        .expect("a page")
        .threads
        .into_iter()
        .map(|thread| thread.key)
        .collect()
}

#[test]
fn a_sender_with_no_rule_waits_in_the_screener_and_is_in_no_box() {
    let conn = open();
    arrive(&conn, &Sender::person("stranger@example.org", "A Stranger"), 100);

    assert_eq!(keys_in(&conn, Place::Screener).len(), 1);
    assert!(keys_in(&conn, Place::Inbox).is_empty());
    assert!(keys_in(&conn, Place::Feed).is_empty());
    assert!(keys_in(&conn, Place::PaperTrail).is_empty());
}

#[test]
fn a_decision_puts_the_sender_in_one_box_and_takes_them_out_of_the_screener() {
    let conn = open();
    let key = arrive(&conn, &Sender::person("maya@example.org", "Maya"), 100);

    super::decide(&conn, "maya@example.org", Destination::Inbox, false).expect("decide");

    assert_eq!(keys_in(&conn, Place::Inbox), vec![key]);
    assert!(keys_in(&conn, Place::Screener).is_empty());
}

#[test]
fn a_decision_applies_to_the_mail_that_is_already_here() {
    // The point of the test: a rule set now has to move the three threads that arrived before it,
    // because a decision that only affects future mail reads as a decision that did not work.
    let conn = open();
    let mut keys = Vec::new();
    for at in [100, 200, 300] {
        keys.push(arrive(&conn, &Sender::person("news@brand.example", "Brand"), at));
    }

    super::decide(&conn, "news@brand.example", Destination::Feed, false).expect("decide");

    let feed = keys_in(&conn, Place::Feed);
    assert_eq!(feed.len(), 3);
    for key in keys {
        assert!(feed.contains(&key), "{key} did not move to the Feed");
    }
}

#[test]
fn a_reply_to_a_thread_you_are_in_is_never_held() {
    let conn = open();
    let mut sender = Sender::person("client@example.org", "A Client");
    sender.replied = true;
    let key = arrive(&conn, &sender, 100);

    assert!(
        keys_in(&conn, Place::Screener).is_empty(),
        "a thread this account has written in must not wait at the door"
    );
    assert_eq!(keys_in(&conn, Place::Inbox), vec![key]);
}

#[test]
fn the_setting_can_hold_a_reply_anyway() {
    let conn = open();
    let mut sender = Sender::person("client@example.org", "A Client");
    sender.replied = true;
    arrive(&conn, &sender, 100);

    conn.execute("INSERT INTO meta (key, value) VALUES ('hold-replies', '1')", [])
        .expect("the setting");

    assert_eq!(keys_in(&conn, Place::Screener).len(), 1);
}

#[test]
fn a_domain_rule_decides_everyone_at_the_domain() {
    let conn = open();
    let one = arrive(&conn, &Sender::person("ana@northgate.example", "Ana"), 100);
    let two = arrive(&conn, &Sender::person("ben@northgate.example", "Ben"), 200);

    super::decide(&conn, "ana@northgate.example", Destination::PaperTrail, true).expect("decide");

    let trail = keys_in(&conn, Place::PaperTrail);
    assert!(trail.contains(&one) && trail.contains(&two));
}

#[test]
fn an_address_rule_beats_the_domain_rule_over_it() {
    let conn = open();
    let ana = arrive(&conn, &Sender::person("ana@northgate.example", "Ana"), 100);
    let ben = arrive(&conn, &Sender::person("ben@northgate.example", "Ben"), 200);

    super::decide(&conn, "ana@northgate.example", Destination::PaperTrail, true).expect("domain");
    super::decide(&conn, "ana@northgate.example", Destination::Inbox, false).expect("address");

    assert_eq!(keys_in(&conn, Place::Inbox), vec![ana]);
    assert_eq!(keys_in(&conn, Place::PaperTrail), vec![ben]);
}

#[test]
fn a_domain_rule_on_a_consumer_domain_is_refused() {
    let conn = open();
    arrive(&conn, &Sender::person("someone@gmail.com", "Someone"), 100);

    let refused = super::decide(&conn, "someone@gmail.com", Destination::Feed, true);
    assert!(refused.is_err(), "everyone at gmail.com is not one sender");
    assert_eq!(keys_in(&conn, Place::Screener).len(), 1);
}

#[test]
fn the_card_carries_the_suggestion_and_the_reason_that_decided_it() {
    let conn = open();
    let mut newsletter = Sender::person("hello@thelongread.example", "The Long Read");
    newsletter.subject = "The week in review";
    newsletter.list_unsubscribe = Some("<https://thelongread.example/u/1>");
    arrive(&conn, &newsletter, 200);
    arrive(&conn, &Sender::person("maya@example.org", "Maya"), 100);

    let cards = super::list(&conn, "acct").expect("cards");
    assert_eq!(cards.len(), 2);

    let feed = cards
        .iter()
        .find(|card| card.sender.address == "hello@thelongread.example")
        .expect("the newsletter");
    assert_eq!(feed.suggestion, Destination::Feed);
    assert!(feed.reason.contains("unsubscribe"), "reason was {}", feed.reason);

    let person = cards
        .iter()
        .find(|card| card.sender.address == "maya@example.org")
        .expect("the person");
    assert_eq!(person.suggestion, Destination::Inbox);
}

#[test]
fn a_card_is_one_per_sender_and_counts_what_is_waiting() {
    let conn = open();
    for at in [100, 200, 300] {
        arrive(&conn, &Sender::person("keen@example.org", "Keen"), at);
    }

    let cards = super::list(&conn, "acct").expect("cards");
    assert_eq!(cards.len(), 1, "three messages from one sender is one decision");
    assert_eq!(cards[0].waiting, 3);
    // The card shows the first message, because a rule is about first contact.
    assert_eq!(cards[0].date_ms, 100);
}

#[test]
fn clear_all_screens_out_everyone_waiting_and_nobody_else() {
    let conn = open();
    let kept = arrive(&conn, &Sender::person("maya@example.org", "Maya"), 100);
    arrive(&conn, &Sender::person("one@example.org", "One"), 200);
    arrive(&conn, &Sender::person("two@example.org", "Two"), 300);
    super::decide(&conn, "maya@example.org", Destination::Inbox, false).expect("decide");

    let cleared = super::clear_all(&conn, "acct").expect("clear all");
    assert_eq!(cleared.len(), 2);
    assert!(keys_in(&conn, Place::Screener).is_empty());
    assert_eq!(keys_in(&conn, Place::Inbox), vec![kept]);
    assert_eq!(keys_in(&conn, Place::ScreenedOut).len(), 2);
}

#[test]
fn taking_a_decision_back_puts_the_sender_at_the_door_again() {
    let conn = open();
    arrive(&conn, &Sender::person("maya@example.org", "Maya"), 100);
    super::decide(&conn, "maya@example.org", Destination::Inbox, false).expect("decide");
    assert!(keys_in(&conn, Place::Screener).is_empty());

    state::write::clear_rule(&conn, "maya@example.org").expect("clear");

    assert_eq!(keys_in(&conn, Place::Screener).len(), 1);
    assert!(keys_in(&conn, Place::Inbox).is_empty());
}

#[test]
fn taking_a_decision_back_leaves_no_domain_rule_deciding_them_instead() {
    let conn = open();
    arrive(&conn, &Sender::person("ana@northgate.example", "Ana"), 100);
    super::decide(&conn, "ana@northgate.example", Destination::Feed, true).expect("domain");
    super::decide(&conn, "ana@northgate.example", Destination::Inbox, false).expect("address");

    state::write::clear_rule(&conn, "ana@northgate.example").expect("clear");

    assert_eq!(
        keys_in(&conn, Place::Screener).len(),
        1,
        "undoing a decision has to leave the sender genuinely undecided"
    );
}

#[test]
fn the_seed_screens_in_everyone_already_known_and_counts_them() {
    let conn = open();
    arrive(&conn, &Sender::person("maya@example.org", "Maya"), 100);
    let mut newsletter = Sender::person("hello@thelongread.example", "The Long Read");
    newsletter.list_unsubscribe = Some("<https://thelongread.example/u/1>");
    arrive(&conn, &newsletter, 200);

    for (address, source) in [
        ("maya@example.org", "mirror"),
        ("hello@thelongread.example", "mirror"),
        // Somebody in the provider's address book this device has never heard from.
        ("cousin@example.net", "people-api"),
    ] {
        conn.execute(
            "INSERT INTO correspondents (address, last_ms, source) VALUES (?1, 1, ?2)",
            rusqlite::params![address, source],
        )
        .expect("a correspondent");
    }

    let screened = crate::routing::seed(&conn).expect("seed");
    assert_eq!(screened, 3);

    assert_eq!(
        state::read::destination_for(&conn, "maya@example.org").expect("rule"),
        Some(Destination::Inbox)
    );
    assert_eq!(
        state::read::destination_for(&conn, "hello@thelongread.example").expect("rule"),
        Some(Destination::Feed)
    );
    // A contact with no mail on the device is a person, because that is what a contact is.
    assert_eq!(
        state::read::destination_for(&conn, "cousin@example.net").expect("rule"),
        Some(Destination::Inbox)
    );
    assert!(keys_in(&conn, Place::Screener).is_empty());
}

#[test]
fn running_the_seed_twice_changes_nothing_and_never_overwrites_a_decision() {
    let conn = open();
    let mut newsletter = Sender::person("hello@thelongread.example", "The Long Read");
    newsletter.list_unsubscribe = Some("<https://thelongread.example/u/1>");
    arrive(&conn, &newsletter, 100);
    conn.execute(
        "INSERT INTO correspondents (address, last_ms, source)
         VALUES ('hello@thelongread.example', 1, 'mirror')",
        [],
    )
    .expect("a correspondent");

    // Somebody decided by hand that this newsletter belongs in the Inbox. A later seed must not
    // suggest its way over the top of that.
    super::decide(&conn, "hello@thelongread.example", Destination::Inbox, false).expect("decide");

    assert_eq!(crate::routing::seed(&conn).expect("first seed"), 0);
    assert_eq!(crate::routing::seed(&conn).expect("second seed"), 0);
    assert_eq!(
        state::read::destination_for(&conn, "hello@thelongread.example").expect("rule"),
        Some(Destination::Inbox)
    );
}

#[test]
fn a_thread_merged_twice_over_appears_once() {
    // Two devices can each merge without either meaning to make a chain, so a one level deep
    // resolution shows the chained thread twice. This is that shape: c into b, b into a.
    let conn = open();
    let a = arrive(&conn, &Sender::person("one@example.org", "One"), 300);
    let b = arrive(&conn, &Sender::person("two@example.org", "Two"), 200);
    let c = arrive(&conn, &Sender::person("three@example.org", "Three"), 100);
    for address in ["one@example.org", "two@example.org", "three@example.org"] {
        super::decide(&conn, address, Destination::Inbox, false).expect("decide");
    }

    state::write::merge_threads(&conn, &[c.clone()], &b).expect("c into b");
    state::write::merge_threads(&conn, &[b.clone()], &a).expect("b into a");

    let inbox = keys_in(&conn, Place::Inbox);
    assert_eq!(inbox, vec![a], "a chained merge has to show as one thread");
    assert!(!inbox.contains(&b) && !inbox.contains(&c));
}
