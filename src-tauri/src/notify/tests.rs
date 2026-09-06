// The engine driven against the fake mailbox with a sink that records what it was asked to
// announce. Nothing here touches the notification plugin: the recorder is the app's half, and what
// the app would have said is asserted through the pure `wanted` and `text` rather than posted.
//
// New mail in these tests is dated a minute into the future. The mark is never allowed to fall
// behind the moment the test process started, and the fake reads a message's date back out of its
// `Date` header at second precision, so "now" could land a few hundred milliseconds before the
// process did and be taken for backlog.

use std::sync::Mutex;

use rusqlite::Connection;
use tauri::async_runtime::block_on;

use crate::db;
use crate::dto::{ContactPatch, Destination, NotifyTarget, Place, SyncStatus};
use crate::mirror::write;
use crate::provider::fake::FakeProvider;
use crate::state;
use crate::sync::engine::Engine;
use crate::sync::{Remote, Sink, Store};

use super::{text, wanted, Arrival, MARK_KEY};

// -- the harness -------------------------------------------------------------------------------

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

/// Every `arrived` call, one entry per call, so a test can tell one grouped call from two.
#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<Vec<Arrival>>>,
}

impl Sink for Recorder {
    fn status(&self, _status: &SyncStatus) {}
    fn changed(&self, _reason: &str) {}
    fn arrived(&self, _account_id: &str, arrivals: &[Arrival]) {
        self.calls.lock().expect("calls").push(arrivals.to_vec());
    }
}

impl Recorder {
    fn calls(&self) -> Vec<Vec<Arrival>> {
        self.calls.lock().expect("calls").clone()
    }

    fn all(&self) -> Vec<Arrival> {
        self.calls().into_iter().flatten().collect()
    }
}

fn rfc2822(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .expect("a moment inside the epoch")
        .to_rfc2822()
}

fn eml(
    message_id: &str,
    from: &str,
    subject: &str,
    at_ms: i64,
    extra: &[(&str, &str)],
    body: &str,
) -> Vec<u8> {
    let mut out = String::new();
    out.push_str(&format!("Message-ID: <{message_id}>\r\n"));
    out.push_str(&format!("From: {from}\r\n"));
    out.push_str("To: You <you@example.com>\r\n");
    out.push_str(&format!("Subject: {subject}\r\n"));
    out.push_str(&format!("Date: {}\r\n", rfc2822(at_ms)));
    for (name, value) in extra {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(body);
    out.into_bytes()
}

fn days_ago(days: i64) -> i64 {
    write::now_ms() - days * write::DAY_MS
}

/// A minute from now: after the process came up, whatever second the header rounds to.
fn soon() -> i64 {
    write::now_ms() + 60_000
}

fn pass(store: &Memory, engine: &Engine, remote: &dyn Remote, sink: &Recorder) -> SyncStatus {
    let status = block_on(engine.run_pass(store, remote, sink, false));
    assert_eq!(status.phase, "idle", "{:?}", status.error);
    status
}

fn rule(store: &Memory, address: &str, destination: Destination) {
    store
        .with(|conn| state::write::set_rule(conn, address, false, destination, None))
        .expect("a sender rule");
}

fn places(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

/// A mailbox with one old message from Ana, synced once, with Ana screened into the Inbox.
fn settled() -> (Memory, FakeProvider, Engine, Recorder) {
    let store = store();
    let fake = FakeProvider::new();
    fake.add_eml(
        "a1",
        "t1",
        &["INBOX"],
        &eml(
            "a1@example.test",
            "Ana <ana@example.test>",
            "Before",
            days_ago(2),
            &[],
            "An old one.",
        ),
    );
    let engine = Engine::new("acct");
    let sink = Recorder::default();
    pass(&store, &engine, &fake, &sink);
    rule(&store, "ana@example.test", Destination::Inbox);
    (store, fake, engine, sink)
}

// -- the rules ---------------------------------------------------------------------------------

#[test]
fn a_first_sync_announces_nothing_and_neither_does_the_quiet_pass_after_it() {
    let store = store();
    let fake = FakeProvider::new();
    for index in 0..12 {
        fake.add_eml(
            &format!("m{index}"),
            &format!("t{index}"),
            &["INBOX", "UNREAD"],
            &eml(
                &format!("m{index}@example.test"),
                "Ana <ana@example.test>",
                &format!("Message {index}"),
                days_ago(index + 1),
                &[],
                "Hello.",
            ),
        );
    }
    let engine = Engine::new("acct");
    let sink = Recorder::default();

    pass(&store, &engine, &fake, &sink);
    assert!(sink.calls().is_empty(), "the first sync is a backlog, not news");

    pass(&store, &engine, &fake, &sink);
    assert!(sink.calls().is_empty(), "and a pass with nothing new has nothing to say");
    assert!(
        store.with(|conn| write::meta_i64(conn, MARK_KEY)).expect("the mark").is_some(),
        "the mark was started at the first read"
    );
}

#[test]
fn a_new_unread_inbox_message_is_announced_once_and_not_again() {
    let (store, fake, engine, sink) = settled();
    fake.add_eml(
        "a2",
        "t2",
        &["INBOX", "UNREAD"],
        &eml(
            "a2@example.test",
            "Ana <ana@example.test>",
            "Lunch?",
            soon(),
            &[],
            "Are you free on Thursday?",
        ),
    );

    pass(&store, &engine, &fake, &sink);
    let all = sink.all();
    assert_eq!(all.len(), 1, "{all:?}");
    let arrival = &all[0];
    assert_eq!(arrival.account_id, "acct");
    assert_eq!(arrival.thread_key, "a2@example.test");
    assert_eq!(arrival.sender, "Ana");
    assert_eq!(arrival.sender_address, "ana@example.test");
    assert_eq!(arrival.subject, "Lunch?");
    assert_eq!(arrival.place, Some(Place::Inbox));
    assert!(!arrival.thread_notify && !arrival.sender_notify);

    assert_eq!(wanted(&all, &places(&["inbox"])).len(), 1);
    assert!(wanted(&all, &places(&["feed", "paper-trail"])).is_empty());
    assert!(wanted(&all, &[]).is_empty(), "off everywhere is off");

    let said = text(&wanted(&all, &places(&["inbox"]))).expect("something to say");
    assert_eq!(said.subtitle, "Ana");
    assert_eq!(said.body, "Lunch?");
    let target = said.target.expect("a click has somewhere to go");
    assert_eq!(target.thread_key.as_deref(), Some(arrival.thread_key.as_str()));
    assert_eq!(target.place, Place::Inbox);

    pass(&store, &engine, &fake, &sink);
    pass(&store, &engine, &fake, &sink);
    assert_eq!(sink.calls().len(), 1, "announced once, whatever the next passes find");
}

#[test]
fn mail_the_account_sent_itself_is_not_announced() {
    let (store, fake, engine, sink) = settled();
    store
        .with(|conn| write::meta_set(conn, write::OWN_ADDRESS_KEY, "you@example.com"))
        .expect("an own address");
    rule(&store, "you@example.com", Destination::Inbox);

    // Unread and in the Inbox, which is what a message to yourself looks like on Gmail.
    fake.add_eml(
        "s1",
        "t9",
        &["INBOX", "UNREAD"],
        &eml(
            "s1@example.test",
            "You <you@example.com>",
            "Note to self",
            soon(),
            &[],
            "Buy milk.",
        ),
    );
    // And a reply from somewhere else that the provider filed as sent, as a copy of an outgoing
    // message arriving through another client does.
    fake.add_eml(
        "s2",
        "t1",
        &["SENT", "INBOX", "UNREAD"],
        &eml(
            "s2@example.test",
            "Ana <ana@example.test>",
            "Re: Before",
            soon(),
            &[("In-Reply-To", "<a1@example.test>"), ("References", "<a1@example.test>")],
            "Copy.",
        ),
    );

    pass(&store, &engine, &fake, &sink);
    assert!(sink.calls().is_empty(), "{:?}", sink.all());
}

#[test]
fn a_seen_message_a_draft_and_mail_in_spam_or_trash_are_quiet() {
    let (store, fake, engine, sink) = settled();
    fake.add_eml(
        "r1",
        "t3",
        &["INBOX"],
        &eml("r1@example.test", "Ana <ana@example.test>", "Read elsewhere", soon(), &[], "."),
    );
    fake.add_eml(
        "d1",
        "t4",
        &["DRAFT", "UNREAD"],
        &eml("d1@example.test", "Ana <ana@example.test>", "Draft", soon(), &[], "."),
    );
    fake.add_eml(
        "j1",
        "t5",
        &["SPAM", "UNREAD"],
        &eml("j1@example.test", "Ana <ana@example.test>", "Spam", soon(), &[], "."),
    );
    fake.add_eml(
        "x1",
        "t6",
        &["TRASH", "UNREAD"],
        &eml("x1@example.test", "Ana <ana@example.test>", "Trash", soon(), &[], "."),
    );

    pass(&store, &engine, &fake, &sink);
    assert!(sink.calls().is_empty(), "{:?}", sink.all());
}

#[test]
fn a_place_not_opted_in_is_silent_unless_the_thread_or_the_sender_says_otherwise() {
    let (store, fake, engine, sink) = settled();
    rule(&store, "news@thelongread.example", Destination::Feed);
    fake.add_eml(
        "n1",
        "t7",
        &["INBOX", "UNREAD"],
        &eml(
            "n1@thelongread.example",
            "The Long Read <news@thelongread.example>",
            "This week",
            soon(),
            &[("List-Unsubscribe", "<https://thelongread.example/u>")],
            "Five pieces.",
        ),
    );
    pass(&store, &engine, &fake, &sink);

    let first = sink.all();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].place, Some(Place::Feed));
    assert!(wanted(&first, &places(&["inbox"])).is_empty(), "the Feed was not asked for");
    assert_eq!(wanted(&first, &places(&["feed"])).len(), 1);

    // The thread's own switch, on the key the newsletter's thread carries.
    store
        .with(|conn| {
            state::write::set_thread_flags(conn, "n1@thelongread.example", None, Some(true))
        })
        .expect("notify on the thread");
    fake.add_eml(
        "n2",
        "t7",
        &["INBOX", "UNREAD"],
        &eml(
            "n2@thelongread.example",
            "The Long Read <news@thelongread.example>",
            "Re: This week",
            soon() + 1_000,
            &[
                ("In-Reply-To", "<n1@thelongread.example>"),
                ("References", "<n1@thelongread.example>"),
            ],
            "A correction.",
        ),
    );
    pass(&store, &engine, &fake, &sink);
    let second = &sink.calls()[1];
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].thread_key, "n1@thelongread.example");
    assert!(second[0].thread_notify);
    assert_eq!(wanted(second, &[]).len(), 1, "the thread asked, whatever the places say");

    // The sender's switch, from the contact card.
    rule(&store, "bo@example.test", Destination::PaperTrail);
    store
        .with(|conn| {
            state::write::set_contact(
                conn,
                "bo@example.test",
                &ContactPatch {
                    notify: Some(true),
                    ..ContactPatch::default()
                },
            )
        })
        .expect("notify on the sender");
    fake.add_eml(
        "b1",
        "t8",
        &["INBOX", "UNREAD"],
        &eml(
            "b1@example.test",
            "Bo <bo@example.test>",
            "Your statement",
            soon() + 2_000,
            &[],
            "Attached.",
        ),
    );
    pass(&store, &engine, &fake, &sink);
    let third = &sink.calls()[2];
    assert_eq!(third.len(), 1);
    assert_eq!(third[0].place, Some(Place::PaperTrail));
    assert!(third[0].sender_notify);
    assert_eq!(wanted(third, &[]).len(), 1, "the person asked, whatever the places say");
}

#[test]
fn a_sender_still_waiting_in_the_screener_has_no_place_to_notify_from() {
    let (store, fake, engine, sink) = settled();
    fake.add_eml(
        "u1",
        "t10",
        &["INBOX", "UNREAD"],
        &eml(
            "u1@example.test",
            "Unknown <stranger@example.test>",
            "Hello",
            soon(),
            &[],
            "We have not met.",
        ),
    );
    pass(&store, &engine, &fake, &sink);

    let all = sink.all();
    assert_eq!(all.len(), 1, "the fact is recorded");
    assert_eq!(all[0].place, None);
    assert!(
        wanted(&all, &places(&["inbox", "feed", "paper-trail"])).is_empty(),
        "and every place opted in still says nothing about the Screener"
    );
}

#[test]
fn an_ignored_thread_is_silent() {
    let (store, fake, engine, sink) = settled();
    store
        .with(|conn| state::write::set_thread_flags(conn, "a1@example.test", Some(true), None))
        .expect("ignore the thread");
    fake.add_eml(
        "a3",
        "t1",
        &["INBOX", "UNREAD"],
        &eml(
            "a3@example.test",
            "Ana <ana@example.test>",
            "Re: Before",
            soon(),
            &[("In-Reply-To", "<a1@example.test>"), ("References", "<a1@example.test>")],
            "Still going.",
        ),
    );
    pass(&store, &engine, &fake, &sink);
    assert!(sink.calls().is_empty(), "{:?}", sink.all());
}

#[test]
fn two_messages_in_one_pass_are_one_call_and_one_grouped_notification() {
    let (store, fake, engine, sink) = settled();
    rule(&store, "bo@example.test", Destination::Inbox);
    fake.add_eml(
        "a2",
        "t2",
        &["INBOX", "UNREAD"],
        &eml("a2@example.test", "Ana <ana@example.test>", "One", soon(), &[], "."),
    );
    fake.add_eml(
        "b1",
        "t3",
        &["INBOX", "UNREAD"],
        &eml("b1@example.test", "Bo <bo@example.test>", "Two", soon() + 1_000, &[], "."),
    );
    pass(&store, &engine, &fake, &sink);

    let calls = sink.calls();
    assert_eq!(calls.len(), 1, "one call for the pass");
    assert_eq!(calls[0].len(), 2);
    let said = text(&wanted(&calls[0], &places(&["inbox"]))).expect("something to say");
    assert_eq!(said.subtitle, "2 new messages");
    assert_eq!(said.body, "Ana and Bo");
    assert_eq!(said.target.expect("a click has somewhere to go").thread_key, None);

}

#[test]
fn a_backlog_from_before_the_app_came_up_is_not_announced() {
    let (store, fake, engine, sink) = settled();
    // A device that last looked a week ago and was then shut. The mail from the days in between
    // arrives on the first pass after launch and is dated before the process started.
    store
        .with(|conn| write::meta_set(conn, MARK_KEY, &days_ago(7).to_string()))
        .expect("an old mark");
    fake.add_eml(
        "w1",
        "t2",
        &["INBOX", "UNREAD"],
        &eml("w1@example.test", "Ana <ana@example.test>", "While you were away", days_ago(3), &[], "."),
    );
    pass(&store, &engine, &fake, &sink);
    assert!(sink.calls().is_empty(), "{:?}", sink.all());

    // Whereas what arrives after launch is news.
    fake.add_eml(
        "w2",
        "t3",
        &["INBOX", "UNREAD"],
        &eml("w2@example.test", "Ana <ana@example.test>", "Now", soon(), &[], "."),
    );
    pass(&store, &engine, &fake, &sink);
    assert_eq!(sink.all().len(), 1);
}

#[test]
fn a_message_that_missed_its_own_pass_is_still_announced_by_the_one_that_hydrates_it() {
    // The change log names the id and the metadata fetch fails, so the row waits unhydrated. The
    // next pass drains it, and a rule keyed on "the ids this pass added" would have lost it.
    let (store, fake, engine, sink) = settled();
    fake.add_eml(
        "a2",
        "t2",
        &["INBOX", "UNREAD"],
        &eml("a2@example.test", "Ana <ana@example.test>", "Late", soon(), &[], "."),
    );
    fake.fail_next(
        crate::provider::fake::Call::Headers,
        crate::provider::ProviderError::Network("a tunnel".to_string()),
    );
    // A single network failure is kept quiet, so the status reads idle; what shows it failed is
    // that nothing was announced and the row is still waiting.
    let failed = block_on(engine.run_pass(&store, &fake, &sink, false));
    assert!(failed.error.is_none(), "{failed:?}");
    assert!(sink.calls().is_empty());

    engine.resume();
    pass(&store, &engine, &fake, &sink);
    let all = sink.all();
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0].subject, "Late");
}

// -- the words ---------------------------------------------------------------------------------

fn arrival(sender: &str, subject: &str, snippet: &str) -> Arrival {
    Arrival {
        account_id: "acct".to_string(),
        message_id: format!("{sender}-{subject}"),
        thread_key: format!("{sender}@example.test"),
        sender: sender.to_string(),
        sender_address: format!("{}@example.test", sender.to_lowercase()),
        subject: subject.to_string(),
        snippet: snippet.to_string(),
        place: Some(Place::Inbox),
        thread_notify: false,
        sender_notify: false,
    }
}

#[test]
fn one_message_is_the_app_the_sender_and_the_subject_and_a_click_opens_the_thread() {
    let one = text(&[arrival("Ana", "Lunch?", "Thursday works.")]).unwrap();
    assert_eq!(one.title, "Margin Mail");
    assert_eq!(one.subtitle, "Ana");
    assert_eq!(one.body, "Lunch?");
    assert_eq!(
        one.target,
        Some(NotifyTarget {
            account_id: "acct".to_string(),
            place: Place::Inbox,
            thread_key: Some("Ana@example.test".to_string()),
        })
    );

    let bare = text(&[arrival("Ana", "", "Only a snippet.")]).unwrap();
    assert_eq!(bare.body, "Only a snippet.");

    let nothing = text(&[arrival("Ana", "", "")]).unwrap();
    assert_eq!(nothing.body, "(no subject)");

    // A thread that notifies from a pile or a snooze shows in none of the three lists, and the
    // one list it is in is Everything.
    let mut piled = arrival("Ana", "Still on", "");
    piled.place = None;
    piled.thread_notify = true;
    let away = text(&[piled]).unwrap();
    assert_eq!(away.target.unwrap().place, Place::Everything);
}

#[test]
fn several_messages_count_themselves_name_the_senders_once_each_and_open_the_inbox() {
    let two = text(&[arrival("Ana", "One", ""), arrival("Ana", "Two", "")]).unwrap();
    assert_eq!(two.title, "Margin Mail");
    assert_eq!(two.subtitle, "2 new messages");
    assert_eq!(two.body, "Ana");
    assert_eq!(
        two.target,
        Some(NotifyTarget {
            account_id: "acct".to_string(),
            place: Place::Inbox,
            thread_key: None,
        })
    );

    let three = text(&[
        arrival("Ana", "One", ""),
        arrival("Bo", "Two", ""),
        arrival("Cy", "Three", ""),
    ])
    .unwrap();
    assert_eq!(three.body, "Ana, Bo and Cy");

    let five = text(&[
        arrival("Ana", "One", ""),
        arrival("Bo", "Two", ""),
        arrival("Cy", "Three", ""),
        arrival("Di", "Four", ""),
        arrival("Ed", "Five", ""),
    ])
    .unwrap();
    assert_eq!(five.subtitle, "5 new messages");
    assert_eq!(five.body, "Ana, Bo, Cy and 2 others");


    assert!(text(&[]).is_none());
}
