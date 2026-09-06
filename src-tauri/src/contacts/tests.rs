// The contact card, over a pair of in-memory databases.
//
// Threads and messages go in with SQL rather than through the sync engine, the way the Screener's
// tests do it: what is being asserted is what a card says given a mailbox and a set of decisions,
// and building the mailbox through a fake provider would test the engine again and hide the answer.
//
// The one thing that is not raw SQL is `fts::index`. The card's recent threads are a search over
// `from:`, which is the same query the search bar runs, so a message that is in the mirror and not
// in the index is a message the card cannot see. The engine indexes on headers arriving; these
// tests do the same thing by hand.

use rusqlite::Connection;

use crate::db;
use crate::dto::{ContactPatch, Destination, Place, ThreadQuery};
use crate::mirror;
use crate::provider::fake::FakeProvider;
use crate::state;

const NOW: i64 = 2_000_000_000_000;

fn open() -> Connection {
    db::memory().expect("a pair of in-memory databases")
}

struct Mail<'a> {
    address: &'a str,
    name: &'a str,
    subject: &'a str,
    list_unsubscribe: Option<&'a str>,
    attachment: Option<&'a str>,
}

impl<'a> Mail<'a> {
    fn from(address: &'a str, name: &'a str) -> Self {
        Mail {
            address,
            name,
            subject: "Hello",
            list_unsubscribe: None,
            attachment: None,
        }
    }
}

/// One thread with one message from one sender, indexed the way the engine indexes it.
fn arrive(conn: &Connection, mail: &Mail<'_>, at: i64) -> String {
    let key = format!("<{}-{at}@example>", mail.address);
    let tid = format!("t-{}-{at}", mail.address);
    let mid = format!("m-{tid}");
    conn.execute(
        "INSERT INTO threads (provider_thread_id, thread_key, latest_ms, message_count, unseen,
                              subject, snippet, from_name, from_address, in_inbox, has_attachment)
         VALUES (?1, ?2, ?3, 1, 1, ?4, 'snippet', ?5, ?6, 1, ?7)",
        rusqlite::params![
            tid,
            key,
            at,
            mail.subject,
            mail.name,
            mail.address,
            mail.attachment.is_some() as i64
        ],
    )
    .expect("a thread");
    conn.execute(
        "INSERT INTO messages (id, provider_thread_id, thread_key, message_id, date_ms, from_name,
                               from_address, subject, snippet, hydrated, labels, list_unsubscribe,
                               has_attachment)
         VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, ?7, 'snippet', 1, '[\"INBOX\"]', ?8, ?9)",
        rusqlite::params![
            mid,
            tid,
            key,
            at,
            mail.name,
            mail.address,
            mail.subject,
            mail.list_unsubscribe,
            mail.attachment.is_some() as i64,
        ],
    )
    .expect("a message");
    if let Some(filename) = mail.attachment {
        conn.execute(
            "INSERT INTO attachments (id, message_id, filename, mime_type, size, inline)
             VALUES (?1, ?2, ?3, 'application/pdf', 1024, 0)",
            rusqlite::params![format!("a-{tid}"), mid, filename],
        )
        .expect("an attachment");
    }
    conn.execute(
        "INSERT INTO correspondents (address, name, last_ms, seen_count, sent_count, source)
         VALUES (?1, ?2, ?3, 1, 0, 'mirror')
         ON CONFLICT(address) DO UPDATE SET
            last_ms = MAX(correspondents.last_ms, excluded.last_ms),
            seen_count = correspondents.seen_count + 1",
        rusqlite::params![mail.address, mail.name, at],
    )
    .expect("a correspondent");
    mirror::fts::index(conn, &mid).expect("the index");
    key
}

fn card(conn: &Connection, address: &str) -> crate::dto::ContactCard {
    super::card(conn, "acct", "hue-1", address, NOW).expect("a card")
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
    mirror::read::threads_list(conn, "acct", "hue-1", &query, NOW)
        .expect("a page")
        .threads
        .into_iter()
        .map(|thread| thread.key)
        .collect()
}

// -- the card ----------------------------------------------------------------------------------

#[test]
fn a_card_for_somebody_with_no_rule_says_where_they_would_go() {
    let conn = open();
    let mut letter = Mail::from("hello@thelongread.example", "The Long Read");
    letter.subject = "The week in review";
    letter.list_unsubscribe = Some("<https://thelongread.example/u/1>");
    arrive(&conn, &letter, NOW - 1000);

    let card = card(&conn, "hello@thelongread.example");

    assert_eq!(card.person.name.as_deref(), Some("The Long Read"));
    assert_eq!(card.person.address, "hello@thelongread.example");
    // Nobody has decided about them, so the card says what the Screener would suggest rather than
    // nothing at all, and says out loud that it is not a decision by carrying no date.
    assert_eq!(card.destination, Destination::Feed);
    assert_eq!(card.screened_at_ms, None);
    assert!(!card.domain_rule);
    assert!(card.domain_rule_allowed);
    assert!(card.unsubscribe.is_some());
    assert_eq!(card.note, None);
    assert!(!card.notify);
}

#[test]
fn a_card_for_somebody_with_a_rule_carries_the_decision_and_its_date() {
    let conn = open();
    arrive(&conn, &Mail::from("maya@example.org", "Maya"), NOW - 1000);
    super::update(
        &conn,
        "maya@example.org",
        &ContactPatch {
            destination: Some(Destination::PaperTrail),
            ..ContactPatch::default()
        },
    )
    .expect("a move");

    let card = card(&conn, "maya@example.org");

    assert_eq!(card.destination, Destination::PaperTrail);
    let decided = card.screened_at_ms.expect("a screening date");
    assert!(decided > 0, "a decision has a moment");
}

#[test]
fn a_card_carries_the_senders_recent_threads_and_files() {
    let conn = open();
    let mut with_file = Mail::from("sam@sunnydaymusic.example", "Sam Okafor");
    with_file.subject = "Enrolment form";
    with_file.attachment = Some("Enrolment-form.pdf");
    arrive(&conn, &with_file, NOW - 3000);
    let mut later = Mail::from("sam@sunnydaymusic.example", "Sam Okafor");
    later.subject = "Piano on Wednesdays";
    arrive(&conn, &later, NOW - 1000);
    // Somebody else at the same domain, whose mail is not Sam's.
    arrive(
        &conn,
        &Mail::from("enrolments@sunnydaymusic.example", "Sunny Day Music"),
        NOW - 2000,
    );

    let card = card(&conn, "sam@sunnydaymusic.example");

    let subjects: Vec<String> = card
        .recent_threads
        .iter()
        .map(|thread| thread.subject.clone())
        .collect();
    assert_eq!(subjects, vec!["Piano on Wednesdays", "Enrolment form"]);
    // A card is not a place, so its rows sit under the card's own heading and carry no group.
    assert!(card.recent_threads.iter().all(|thread| thread.group.is_empty()));
    assert_eq!(card.files.len(), 1);
    assert_eq!(card.files[0].filename, "Enrolment-form.pdf");
}

#[test]
fn the_domain_toggle_is_not_offered_on_a_consumer_domain() {
    let conn = open();
    arrive(&conn, &Mail::from("someone@gmail.com", "Someone"), NOW - 1000);

    assert!(!card(&conn, "someone@gmail.com").domain_rule_allowed);
    assert!(card(&conn, "ana@northgate.example").domain_rule_allowed);
}

// -- moving somebody -----------------------------------------------------------------------------

#[test]
fn a_destination_change_moves_the_mail_that_is_already_here() {
    // The whole point of the card: a move that only affected future mail reads as a move that did
    // not work.
    let conn = open();
    let mut keys = Vec::new();
    for at in [NOW - 3000, NOW - 2000, NOW - 1000] {
        keys.push(arrive(
            &conn,
            &Mail::from("news@brand.example", "Brand"),
            at,
        ));
    }
    super::update(
        &conn,
        "news@brand.example",
        &ContactPatch {
            destination: Some(Destination::Feed),
            ..ContactPatch::default()
        },
    )
    .expect("a move");

    let feed = keys_in(&conn, Place::Feed);
    assert_eq!(feed.len(), 3);
    for key in keys {
        assert!(feed.contains(&key), "{key} did not move to the Feed");
    }
    assert!(keys_in(&conn, Place::Screener).is_empty());
}

#[test]
fn screening_somebody_out_from_the_card_is_the_block() {
    let conn = open();
    let key = arrive(
        &conn,
        &Mail::from("reach@talentpartners.example", "Talent Partners"),
        NOW - 1000,
    );
    super::update(
        &conn,
        "reach@talentpartners.example",
        &ContactPatch {
            destination: Some(Destination::ScreenedOut),
            ..ContactPatch::default()
        },
    )
    .expect("a block");

    assert_eq!(keys_in(&conn, Place::ScreenedOut), vec![key]);
    assert!(keys_in(&conn, Place::Inbox).is_empty());
}

#[test]
fn turning_the_domain_toggle_on_decides_everyone_at_the_domain() {
    let conn = open();
    let ana = arrive(&conn, &Mail::from("ana@northgate.example", "Ana"), NOW - 2000);
    let ben = arrive(&conn, &Mail::from("ben@northgate.example", "Ben"), NOW - 1000);

    super::update(
        &conn,
        "ana@northgate.example",
        &ContactPatch {
            destination: Some(Destination::PaperTrail),
            ..ContactPatch::default()
        },
    )
    .expect("a move");
    super::update(
        &conn,
        "ana@northgate.example",
        &ContactPatch {
            domain_rule: Some(true),
            ..ContactPatch::default()
        },
    )
    .expect("the toggle");

    let trail = keys_in(&conn, Place::PaperTrail);
    assert!(trail.contains(&ana) && trail.contains(&ben));
    // The address rule would beat the domain rule, so turning the toggle on has to take it away or
    // the card would read as off the moment it was asked again.
    let card = card(&conn, "ana@northgate.example");
    assert!(card.domain_rule);
    assert_eq!(card.destination, Destination::PaperTrail);
}

#[test]
fn a_domain_rule_on_a_consumer_domain_is_refused_and_changes_nothing() {
    let conn = open();
    arrive(&conn, &Mail::from("someone@gmail.com", "Someone"), NOW - 1000);
    super::update(
        &conn,
        "someone@gmail.com",
        &ContactPatch {
            destination: Some(Destination::Inbox),
            ..ContactPatch::default()
        },
    )
    .expect("a move");

    let refused = super::update(
        &conn,
        "someone@gmail.com",
        &ContactPatch {
            domain_rule: Some(true),
            ..ContactPatch::default()
        },
    );

    let message = refused.expect_err("everyone at gmail.com is not one sender");
    assert!(message.contains("gmail.com"), "the message was {message}");
    // Refused before anything was undone: the address rule they already had is still deciding them.
    let card = card(&conn, "someone@gmail.com");
    assert_eq!(card.destination, Destination::Inbox);
    assert!(!card.domain_rule);
}

// -- the rest of the card ------------------------------------------------------------------------

#[test]
fn a_patch_of_one_field_leaves_the_others_alone() {
    let conn = open();
    arrive(&conn, &Mail::from("sam@example.org", "Sam"), NOW - 1000);
    super::update(
        &conn,
        "sam@example.org",
        &ContactPatch {
            destination: Some(Destination::Inbox),
            notify: Some(true),
            note: Some("Cooper's teacher".to_string()),
            allow_remote_images: Some(true),
            ..ContactPatch::default()
        },
    )
    .expect("the first patch");

    super::update(
        &conn,
        "sam@example.org",
        &ContactPatch {
            notify: Some(false),
            ..ContactPatch::default()
        },
    )
    .expect("one field");

    let card = card(&conn, "sam@example.org");
    assert!(!card.notify);
    assert_eq!(card.note.as_deref(), Some("Cooper's teacher"));
    assert!(card.allow_remote_images);
    assert_eq!(card.destination, Destination::Inbox);
    assert!(card.bundle, "the default a contact starts with survives a patch");
}

#[test]
fn the_note_and_the_switches_survive_a_replay_of_the_journal() {
    // The card's decisions roam, which means they are the log rather than the tables: the same
    // events landing on an empty database have to produce the same card.
    let written = open();
    arrive(&written, &Mail::from("sam@example.org", "Sam"), NOW - 1000);
    super::update(
        &written,
        "sam@example.org",
        &ContactPatch {
            destination: Some(Destination::PaperTrail),
            notify: Some(true),
            note: Some("Cooper's teacher".to_string()),
            bundle: Some(false),
            auto_trash_days: Some(Some(30)),
            ..ContactPatch::default()
        },
    )
    .expect("a patch");

    let roamed = open();
    arrive(&roamed, &Mail::from("sam@example.org", "Sam"), NOW - 1000);
    let log = state::journal::records(&written).expect("the log");
    let report = state::merge::absorb(&roamed, &log).expect("absorb");
    assert_eq!(report.applied, log.len());

    let here = card(&written, "sam@example.org");
    let there = card(&roamed, "sam@example.org");
    assert_eq!(there.destination, here.destination);
    assert_eq!(there.screened_at_ms, here.screened_at_ms);
    assert_eq!(there.notify, here.notify);
    assert_eq!(there.note, here.note);
    assert_eq!(there.bundle, here.bundle);
    assert_eq!(there.auto_trash_days, here.auto_trash_days);
    assert_eq!(there.auto_trash_days, Some(30));
}

// -- the Contacts place --------------------------------------------------------------------------

#[test]
fn the_list_holds_everyone_with_a_rule_and_nobody_who_is_waiting() {
    let conn = open();
    arrive(&conn, &Mail::from("maya@example.org", "Maya"), NOW - 3000);
    arrive(&conn, &Mail::from("dev@example.net", "Dev Patel"), NOW - 2000);
    arrive(&conn, &Mail::from("stranger@example.com", "A Stranger"), NOW - 1000);
    for address in ["maya@example.org", "dev@example.net"] {
        super::update(
            &conn,
            address,
            &ContactPatch {
                destination: Some(Destination::Inbox),
                ..ContactPatch::default()
            },
        )
        .expect("a decision");
    }

    let everyone = super::list(&conn, "acct", "").expect("the list");
    let addresses: Vec<String> = everyone
        .iter()
        .map(|card| card.person.address.clone())
        .collect();
    assert_eq!(addresses, vec!["dev@example.net", "maya@example.org"]);
    assert_eq!(everyone[0].person.name.as_deref(), Some("Dev Patel"));
    // A row draws none of these, so the list does not answer three more queries per sender for them.
    assert!(everyone.iter().all(|card| card.recent_threads.is_empty()));
    assert!(everyone.iter().all(|card| card.files.is_empty()));
}

#[test]
fn a_domain_rule_puts_the_people_it_decides_in_the_list_rather_than_the_domain() {
    let conn = open();
    arrive(&conn, &Mail::from("ana@northgate.example", "Ana"), NOW - 2000);
    arrive(&conn, &Mail::from("ben@northgate.example", "Ben"), NOW - 1000);
    super::update(
        &conn,
        "ana@northgate.example",
        &ContactPatch {
            destination: Some(Destination::Feed),
            domain_rule: Some(true),
            ..ContactPatch::default()
        },
    )
    .expect("a domain rule");

    let everyone = super::list(&conn, "acct", "").expect("the list");
    let addresses: Vec<String> = everyone
        .iter()
        .map(|card| card.person.address.clone())
        .collect();
    assert_eq!(
        addresses,
        vec!["ana@northgate.example", "ben@northgate.example"]
    );
    assert!(everyone.iter().all(|card| card.domain_rule));
}

#[test]
fn the_search_narrows_the_list_by_name_and_by_address() {
    let conn = open();
    arrive(&conn, &Mail::from("maya@example.org", "Maya Raghunathan"), NOW - 3000);
    arrive(&conn, &Mail::from("dev@northgate.example", "Dev Patel"), NOW - 2000);
    for address in ["maya@example.org", "dev@northgate.example"] {
        super::update(
            &conn,
            address,
            &ContactPatch {
                destination: Some(Destination::Inbox),
                ..ContactPatch::default()
            },
        )
        .expect("a decision");
    }

    let by_name = super::list(&conn, "acct", "raghu").expect("by name");
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].person.address, "maya@example.org");

    let by_domain = super::list(&conn, "acct", "northgate").expect("by address");
    assert_eq!(by_domain.len(), 1);
    assert_eq!(by_domain[0].person.address, "dev@northgate.example");

    assert!(super::list(&conn, "acct", "nobody").expect("no match").is_empty());
}

// -- autocomplete ---------------------------------------------------------------------------------

fn correspondent(conn: &Connection, address: &str, name: &str, source: &str, seen: i64, last: i64) {
    conn.execute(
        "INSERT INTO correspondents (address, name, last_ms, seen_count, sent_count, source)
         VALUES (?1, ?2, ?3, ?4, 0, ?5)",
        rusqlite::params![address, name, last, seen, source],
    )
    .expect("a correspondent");
}

#[test]
fn autocomplete_ranks_the_mirror_above_the_provider_and_asks_nobody() {
    // The provider is here to prove it is not asked. Everyone this account has actually written to
    // or heard from is already on the device, which is what makes autocomplete local: an address
    // typed into the compose field is never sent anywhere to be looked up.
    let provider = FakeProvider::new();
    provider.set_contacts(vec![crate::dto::Person {
        name: Some("Anita Desai".to_string()),
        address: "anita@northwind.example".to_string(),
    }]);

    let conn = open();
    correspondent(&conn, "ana@example.org", "Ana Ruiz", "mirror", 12, NOW - 5000);
    correspondent(&conn, "andrew@example.net", "Andrew Bell", "mirror", 2, NOW - 1000);
    correspondent(&conn, "anita@northwind.example", "Anita Desai", "people-api", 0, NOW);

    let found = super::suggest(&conn, "an", 8).expect("suggestions");
    let addresses: Vec<String> = found.iter().map(|person| person.address.clone()).collect();

    assert_eq!(
        addresses,
        vec![
            "ana@example.org",
            "andrew@example.net",
            "anita@northwind.example"
        ],
        "the mirror's own people come before the provider's address book"
    );
    assert!(
        provider.calls().is_empty(),
        "the mirror answered, so nothing was asked of anybody: {:?}",
        provider.calls()
    );
}

#[test]
fn autocomplete_matches_a_name_as_well_as_an_address_and_nothing_else() {
    let conn = open();
    correspondent(&conn, "sam@sunnydaymusic.example", "Sam Okafor", "mirror", 3, NOW);
    correspondent(&conn, "billing@citypower.example", "City Power", "mirror", 9, NOW);

    let by_name = super::suggest(&conn, "okaf", 8).expect("by name");
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].address, "sam@sunnydaymusic.example");

    let by_address = super::suggest(&conn, "billing", 8).expect("by address");
    assert_eq!(by_address.len(), 1);

    // A wildcard is a character somebody typed, not a pattern.
    assert!(super::suggest(&conn, "%", 8).expect("a wildcard").is_empty());
    assert!(super::suggest(&conn, "", 8).expect("nothing typed").is_empty());
}
