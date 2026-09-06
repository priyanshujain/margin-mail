// New mail, said out loud.
//
// Two halves with a boundary between them. The engine's half, `arrivals`, is a read over the mirror
// and the state database: which of the messages that landed since the last pass are worth telling
// somebody about, and the facts about each that the answer turns on. The app's half, `announce`,
// has the app handle and therefore the device settings, and it is where the preference is applied
// and the notification posted. The engine cannot read settings.json and should not: it reports
// what arrived and the app decides what to say, which is also what lets a test drive the whole
// detection with nothing but a recording sink behind it.
//
// Only the incremental pass asks. The first sync, the recovery listing and a backfill all bring in
// mail that is old, and a person who connects a mailbox does not want the last month of it read
// back one notification at a time. Mailspring draws the same line: a message is announced when it
// is unread, dated after the app came up, not from the account itself and in the inbox, and five
// or more in one go collapse into a single summary. The rule here is that one with the places and
// the two per-thing switches this app has instead of an inbox folder.

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(test)]
mod tests;

use std::sync::{Mutex, OnceLock};

use rusqlite::{Connection, OptionalExtension};
use tauri::Emitter;

use crate::dto::{Destination, NotifyPermission, NotifyTarget, Place};
use crate::mirror::write;
use crate::routing;
use crate::state;

/// The high-water mark, in the mirror's `meta` beside the sync cursor: the `date_ms` of the newest
/// message a pass has looked at. Per account, because the mirror is.
pub const MARK_KEY: &str = "notify-mark";

/// The bold line of every notification. A macOS banner shows the app's icon and never its name,
/// so the name is the title, and who wrote and what about are the two lines under it.
const APP_NAME: &str = "Margin Mail";
/// The event that tells the front end a notification was clicked; `notify_take` says where to.
pub const OPENED_EVENT: &str = "notification-open";
/// A message with no subject is announced by its snippet, cut to this.
const SNIPPET_CHARS: usize = 90;
/// How many senders a grouped notification names before it says "and N others".
const NAMED_SENDERS: usize = 3;

/// One message worth mentioning, and the facts the app decides on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrival {
    pub account_id: String,
    pub message_id: String,
    pub thread_key: String,
    /// The display name, or the address when there is none.
    pub sender: String,
    pub sender_address: String,
    pub subject: String,
    pub snippet: String,
    /// Inbox, Feed or Paper Trail: the list this thread shows in, by the same rule as the list. None
    /// for a thread none of the three would show, which is the Screener, a pile, a snooze or
    /// Screened out, and none of those notifies on its own.
    pub place: Option<Place>,
    pub thread_notify: bool,
    pub sender_notify: bool,
}

/// What a notification says, and where a click on it goes. On macOS the three lines are the
/// system's title, subtitle and body; elsewhere the desktop shows the app's name on its own and the
/// subtitle stands in for the title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    pub title: String,
    pub subtitle: String,
    pub body: String,
    /// None for the sample from the settings screen, which has nothing to open.
    pub target: Option<NotifyTarget>,
}

/// When this process started looking. Anything dated before it was already in the mailbox when the
/// app came up, and a device that was off for a week has a week of mail to bring in and nothing to
/// say about any of it.
fn activation() -> i64 {
    static AT: OnceLock<i64> = OnceLock::new();
    *AT.get_or_init(write::now_ms)
}

// ---------------------------------------------------------------------------------------------
// The engine's half
// ---------------------------------------------------------------------------------------------

struct Candidate {
    id: String,
    date_ms: i64,
    from_name: Option<String>,
    from_address: String,
    subject: String,
    snippet: String,
    seen: bool,
    draft: bool,
    sent: bool,
    labels: String,
}

/// What landed since the last pass and is worth mentioning, oldest first, with the mark advanced
/// over everything looked at so a restart does not say it again.
///
/// The mark starts at the moment it is first read rather than at zero, so a fresh install announces
/// nothing about the backlog it has just crawled, and it is never allowed to fall behind the moment
/// this process started, so a laptop that was shut for a week is quiet about the week. Candidates
/// are read by date rather than by the ids the change log named, because a message whose metadata
/// failed to arrive in the pass that listed it is hydrated by a later one and would otherwise never
/// be mentioned, and because a message that arrived during a recovery listing lands here on the
/// next pass rather than nowhere.
pub fn arrivals(conn: &Connection, account_id: &str, now: i64) -> Result<Vec<Arrival>, String> {
    let held = write::meta_i64(conn, MARK_KEY)?;
    let floor = match held {
        Some(mark) => mark.max(activation()),
        None => now,
    };

    let mut stmt = conn
        .prepare(
            "SELECT id, date_ms, from_name, from_address, subject, snippet, seen, draft, sent, labels
               FROM messages
              WHERE hydrated = 1 AND transient = 0 AND date_ms > ?1
              ORDER BY date_ms ASC, id ASC",
        )
        .map_err(|e| e.to_string())?;
    let candidates: Vec<Candidate> = stmt
        .query_map([floor], |row| {
            Ok(Candidate {
                id: row.get(0)?,
                date_ms: row.get(1)?,
                from_name: row.get(2)?,
                from_address: row.get(3)?,
                subject: row.get(4)?,
                snippet: row.get(5)?,
                seen: row.get::<_, i64>(6)? != 0,
                draft: row.get::<_, i64>(7)? != 0,
                sent: row.get::<_, i64>(8)? != 0,
                labels: row.get(9)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mark = candidates
        .iter()
        .map(|candidate| candidate.date_ms)
        .max()
        .unwrap_or(floor)
        .max(floor);
    if held != Some(mark) {
        write::meta_set(conn, MARK_KEY, &mark.to_string())?;
    }

    let own = write::meta_get(conn, write::OWN_ADDRESS_KEY)?.unwrap_or_default();
    let holds_replies = routing::holds_replies(conn);

    let mut out = Vec::new();
    for candidate in candidates {
        if candidate.seen || candidate.draft || candidate.sent {
            continue;
        }
        if !own.is_empty() && candidate.from_address.eq_ignore_ascii_case(&own) {
            continue;
        }
        let labels: Vec<String> = serde_json::from_str(&candidate.labels).unwrap_or_default();
        if labels
            .iter()
            .any(|label| label == write::LABEL_TRASH || label == write::LABEL_SPAM)
        {
            continue;
        }
        let Some(facts) = thread_facts(conn, &candidate.id)? else {
            continue;
        };
        if facts.trashed || facts.spam {
            continue;
        }
        // A thread merged into another shows under that other one's key, and that is the key the
        // person set the switches on.
        let key = state::read::effective_key(conn, &facts.thread_key)?;
        let flags = state::read::flags_of(conn, &key)?;
        if flags.ignored {
            continue;
        }
        let sender_notify = state::read::contact(conn, &candidate.from_address)?
            .map(|contact| contact.notify)
            .unwrap_or(false);

        out.push(Arrival {
            account_id: account_id.to_string(),
            message_id: candidate.id,
            thread_key: key,
            sender: candidate
                .from_name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| candidate.from_address.clone()),
            sender_address: candidate.from_address,
            subject: candidate.subject,
            snippet: candidate.snippet,
            place: facts.place(holds_replies),
            thread_notify: flags.notify,
            sender_notify,
        });
    }
    Ok(out)
}

/// The row a place is decided from, read the way `mirror::read` reads it.
struct ThreadFacts {
    thread_key: String,
    in_inbox: bool,
    trashed: bool,
    spam: bool,
    piled: bool,
    snoozed: bool,
    returned: bool,
    to_inbox: bool,
    to_feed: bool,
    to_paper_trail: bool,
    has_rule: bool,
    in_thread: bool,
}

impl ThreadFacts {
    /// `mirror::read::place_clause` for the three routed places, in the order it tries them. The
    /// routing predicates are the same strings the list is built from; what is repeated here is
    /// only the shape around them, because the list's clause is private to the list and a
    /// notification about a thread the Inbox would not show is the bug this is avoiding.
    fn place(&self, holds_replies: bool) -> Option<Place> {
        if self.trashed || self.spam {
            return None;
        }
        // A piled thread is in its pile and a snoozed thread is away, unless it has come back.
        let loose = !self.piled && (!self.snoozed || self.returned);
        if !loose {
            return None;
        }
        let routed_to_inbox =
            self.to_inbox || (!holds_replies && !self.has_rule && self.in_thread);
        if self.in_inbox && routed_to_inbox {
            Some(Place::Inbox)
        } else if self.to_feed {
            Some(Place::Feed)
        } else if self.to_paper_trail {
            Some(Place::PaperTrail)
        } else {
            None
        }
    }
}

/// The routing predicates run over the message itself rather than the thread's latest row, which
/// is the same address on a thread that just grew by this message and the right one when two
/// people wrote in the same pass.
fn thread_facts(conn: &Connection, message_id: &str) -> Result<Option<ThreadFacts>, String> {
    let sql = format!(
        "SELECT th.thread_key, th.in_inbox, th.trashed, th.spam,
                pl.thread_key IS NOT NULL, sn.thread_key IS NOT NULL, rt.thread_key IS NOT NULL,
                {to_inbox}, {to_feed}, {to_paper_trail}, {has_rule}, {in_thread}
           FROM messages t
           JOIN threads th ON th.provider_thread_id = t.provider_thread_id
           LEFT JOIN state.piles pl ON pl.thread_key = th.thread_key
           LEFT JOIN state.snoozes sn ON sn.thread_key = th.thread_key
           LEFT JOIN state.returned rt ON rt.thread_key = th.thread_key
          WHERE t.id = ?1",
        to_inbox = routing::rule_is(Destination::Inbox),
        to_feed = routing::rule_is(Destination::Feed),
        to_paper_trail = routing::rule_is(Destination::PaperTrail),
        has_rule = routing::HAS_RULE,
        in_thread = routing::IN_THREAD,
    );
    conn.query_row(&sql, [message_id], |row| {
        Ok(ThreadFacts {
            thread_key: row.get(0)?,
            in_inbox: row.get::<_, i64>(1)? != 0,
            trashed: row.get::<_, i64>(2)? != 0,
            spam: row.get::<_, i64>(3)? != 0,
            piled: row.get::<_, i64>(4)? != 0,
            snoozed: row.get::<_, i64>(5)? != 0,
            returned: row.get::<_, i64>(6)? != 0,
            to_inbox: row.get::<_, i64>(7)? != 0,
            to_feed: row.get::<_, i64>(8)? != 0,
            to_paper_trail: row.get::<_, i64>(9)? != 0,
            has_rule: row.get::<_, i64>(10)? != 0,
            in_thread: row.get::<_, i64>(11)? != 0,
        })
    })
    .optional()
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------------
// The preference, and the words
// ---------------------------------------------------------------------------------------------

/// The name a place goes by in `settings.notify_places`, which is its name in the contract.
fn place_name(place: Place) -> &'static str {
    match place {
        Place::Inbox => "inbox",
        Place::Feed => "feed",
        Place::PaperTrail => "paper-trail",
        _ => "",
    }
}

/// The arrivals a device with these places opted in would be told about: the thread's own switch,
/// the sender's, or the place. Pure, so the whole rule is one function and one test.
pub fn wanted(arrivals: &[Arrival], places: &[String]) -> Vec<Arrival> {
    arrivals
        .iter()
        .filter(|arrival| {
            arrival.thread_notify
                || arrival.sender_notify
                || arrival
                    .place
                    .map(|place| places.iter().any(|wanted| wanted == place_name(place)))
                    .unwrap_or(false)
        })
        .cloned()
        .collect()
}

/// One notification for a pass: the sender and the subject when one message came, a count and the
/// senders when several did. Nothing for none. A click on the first opens the thread in the list
/// it shows in; a click on the second opens the account's Inbox, since a group is no one thread.
pub fn text(arrivals: &[Arrival]) -> Option<Text> {
    match arrivals {
        [] => None,
        [one] => Some(Text {
            title: APP_NAME.to_string(),
            subtitle: one.sender.clone(),
            body: body_of(one),
            target: Some(NotifyTarget {
                account_id: one.account_id.clone(),
                // A thread announced by its own switch from a pile or a snooze shows in none of the
                // three lists, and Everything is the one list that has it.
                place: one.place.unwrap_or(Place::Everything),
                thread_key: Some(one.thread_key.clone()),
            }),
        }),
        many => {
            let mut senders: Vec<&str> = Vec::new();
            for arrival in many {
                if !senders.contains(&arrival.sender.as_str()) {
                    senders.push(&arrival.sender);
                }
            }
            Some(Text {
                title: APP_NAME.to_string(),
                subtitle: format!("{} new messages", many.len()),
                body: named(&senders),
                target: Some(NotifyTarget {
                    account_id: many[0].account_id.clone(),
                    place: Place::Inbox,
                    thread_key: None,
                }),
            })
        }
    }
}

/// The subject alone: a banner is a glance, and the thread is one click away. The snippet only
/// stands in for a subject the message has none of.
fn body_of(arrival: &Arrival) -> String {
    let subject = arrival.subject.trim();
    if !subject.is_empty() {
        return subject.to_string();
    }
    let snippet = arrival.snippet.trim();
    if snippet.is_empty() {
        "(no subject)".to_string()
    } else {
        clip(snippet, SNIPPET_CHARS)
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// "Ana", "Ana and Bo", "Ana, Bo and Cy", "Ana, Bo, Cy and 2 others".
fn named(senders: &[&str]) -> String {
    let shown = senders.len().min(NAMED_SENDERS);
    let rest = senders.len() - shown;
    let mut names: Vec<String> = senders[..shown].iter().map(|s| s.to_string()).collect();
    if rest == 1 {
        names.push("1 other".to_string());
    } else if rest > 1 {
        names.push(format!("{rest} others"));
    }
    match names.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

// ---------------------------------------------------------------------------------------------
// The app's half
// ---------------------------------------------------------------------------------------------

/// What `AppSink` does with a pass's arrivals: reads the device settings, keeps what they ask for
/// and posts one notification. Off its caller's thread, because on macOS the first post is what
/// raises the system's permission dialog and that blocks until it is answered, which is no reason
/// to hold up the sync pass that produced the arrivals. A failure to post is written to the log and
/// nowhere else: the pass is over and the mail is on the screen.
pub fn announce(app: &tauri::AppHandle, account_id: &str, arrivals: &[Arrival]) {
    let Ok(settings) = crate::settings::load(app) else {
        return;
    };
    if !settings.notifications {
        return;
    }
    // The account's own address and aliases live in the registry and settings, which the engine
    // never sees. Anything the engine let through from one of them is dropped here.
    let mut own: Vec<String> = settings
        .accounts
        .iter()
        .filter(|row| row.account_id == account_id)
        .flat_map(|row| row.aliases.iter().cloned())
        .collect();
    if let Ok(Some(entry)) = crate::accounts::find(app, account_id) {
        own.push(entry.email);
    }
    let mine: Vec<Arrival> = wanted(arrivals, &settings.notify_places)
        .into_iter()
        .filter(|arrival| {
            !own.iter()
                .any(|address| address.eq_ignore_ascii_case(&arrival.sender_address))
        })
        .collect();
    if let Some(text) = text(&mine) {
        let app = app.clone();
        std::thread::spawn(move || {
            let _ = post(&app, &text);
        });
    }
}

/// Posts one notification, and writes down why when it could not.
fn post(app: &tauri::AppHandle, text: &Text) -> Result<(), String> {
    let result = platform::post(app, text);
    if let Err(e) = &result {
        crate::log::note("notify", &format!("could not post \"{}\": {e}", text.body));
    }
    result
}

#[cfg(target_os = "macos")]
mod platform {
    use super::Text;
    use crate::dto::NotifyPermission;

    pub fn post(_app: &tauri::AppHandle, text: &Text) -> Result<(), String> {
        super::macos::post(text)
    }

    pub fn permission() -> Result<NotifyPermission, String> {
        super::macos::permission()
    }

    pub fn request() -> Result<NotifyPermission, String> {
        super::macos::request()
    }

    pub fn open_settings() -> Result<(), String> {
        super::macos::open_settings()
    }
}

/// Everywhere else the plugin is the right tool: D-Bus on Linux needs no permission and has no
/// settings pane to send anybody to.
#[cfg(not(target_os = "macos"))]
mod platform {
    use tauri_plugin_notification::NotificationExt;

    use super::Text;
    use crate::dto::NotifyPermission;

    /// The freedesktop sound name for new mail. Every notification carries it: whether it is heard
    /// is the system's setting, in the system's pane, the same as on macOS.
    const SOUND: &str = "message-new-email";

    /// The plugin has a title and a body, and the desktop draws the app's name over both on its
    /// own, so the subtitle takes the title's place: the sender in bold, the subject under it.
    /// The plugin has no click to answer, so nothing opens from one here.
    pub fn post(app: &tauri::AppHandle, text: &Text) -> Result<(), String> {
        let title = if text.subtitle.is_empty() { &text.title } else { &text.subtitle };
        app.notification()
            .builder()
            .title(title.clone())
            .body(text.body.clone())
            .sound(SOUND)
            .show()
            .map_err(|e| e.to_string())
    }

    pub fn permission() -> Result<NotifyPermission, String> {
        Ok(NotifyPermission::Granted)
    }

    pub fn request() -> Result<NotifyPermission, String> {
        Ok(NotifyPermission::Granted)
    }

    pub fn open_settings() -> Result<(), String> {
        Ok(())
    }
}

/// Whether the system will show this app's notifications.
#[tauri::command(async)]
pub fn notify_permission() -> Result<NotifyPermission, String> {
    platform::permission()
}

/// Asks the system, when it has not been asked, and says what the answer is. On macOS the question
/// is a dialog and this returns when it is dismissed.
#[tauri::command(async)]
pub fn notify_request() -> Result<NotifyPermission, String> {
    let answer = platform::request();
    if let Err(e) = &answer {
        crate::log::note("notify", &format!("could not ask for permission: {e}"));
    }
    answer
}

/// Opens the system's own notification settings for this app, where a refusal is undone.
#[tauri::command(async)]
pub fn notify_open_settings() -> Result<(), String> {
    platform::open_settings()
}

/// One sample notification, for the settings screen's button and the moment somebody opts in
/// during the first run.
#[tauri::command(async)]
pub fn notify_test(app: tauri::AppHandle) -> Result<(), String> {
    post(
        &app,
        &Text {
            title: APP_NAME.to_string(),
            subtitle: String::new(),
            body: "This is what new mail will look like.".to_string(),
            target: None,
        },
    )
}

// ---------------------------------------------------------------------------------------------
// A click
// ---------------------------------------------------------------------------------------------

/// Where the last click pointed, until the front end takes it. The click that launches the app
/// lands before the webview is listening, so the delegate leaves the target here and says so; the
/// front end asks on its way up and again on every event, and whichever read comes first empties
/// the slot.
static OPENED: Mutex<Option<NotifyTarget>> = Mutex::new(None);

/// What the platform's delegate calls on a click. Raising the window is the platform's part; this
/// is the part that says which thread. Nothing for the sample, which has nowhere to go.
pub fn opened(app: &tauri::AppHandle, target: Option<NotifyTarget>) {
    let Some(target) = target else { return };
    if let Ok(mut slot) = OPENED.lock() {
        *slot = Some(target);
    }
    let _ = app.emit(OPENED_EVENT, ());
}

/// The target of the last click, once: None after the first read, and always for the sample.
#[tauri::command]
pub fn notify_take() -> Option<NotifyTarget> {
    OPENED.lock().ok().and_then(|mut slot| slot.take())
}

