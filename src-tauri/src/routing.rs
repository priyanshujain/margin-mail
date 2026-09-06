// Which of the three boxes a sender belongs in, and why.
//
// Two things live here and they are deliberately separate. The **suggestion** is a pure function of
// one message's headers: it is what the Screener card prints as its reason, and it is what a first
// run uses to route everyone the account already knows. The **destination** is what the person
// decided, which is a row in the state database and beats the suggestion always.
//
// The suggestion is a table rather than a chain of ifs, because `docs/features.md` promises that
// "the rules are a table in the source and the reason strings are the table's rows, so a wrong
// suggestion is a one-line fix". A person who asks why their newsletter went to the Feed should be
// able to be shown the row that decided it.

use rusqlite::{Connection, OptionalExtension};

use crate::dto::Destination;
use crate::state;

/// The headers a suggestion is allowed to look at.
///
/// Every one of these is a column on `messages`, which is why they are columns: routing runs over
/// every message of a first sync, and a JSON extract per row is the difference between a second and
/// a minute. The body is not here and must not be: a classifier that reads the prose is a
/// classifier that cannot explain itself in a sentence.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub from: String,
    pub subject: String,
    pub list_id: Option<String>,
    pub list_unsubscribe: Option<String>,
    pub precedence: Option<String>,
    pub auto_submitted: Option<String>,
    /// The provider's labels, read only for its own categories, which are a hint and never a rule.
    pub labels: Vec<String>,
}

impl Facts {
    fn local_part(&self) -> String {
        self.from
            .split('@')
            .next()
            .unwrap_or_default()
            .to_lowercase()
    }

    fn has_label(&self, label: &str) -> bool {
        self.labels.iter().any(|held| held == label)
    }

    /// A local part that announces nobody is reading replies.
    ///
    /// Matched on whole words, split on the separators an address actually uses, so `no-reply`,
    /// `noreply`, `mail.notifications` and `bounces-1234` are all caught while `norepeat@` and
    /// `alertisimo@` are people. A prefix match would catch those two as well, which is why this
    /// does not do one: a person wrongly screened into the Paper Trail is mail they never see.
    fn is_machine_address(&self) -> bool {
        let local = self.local_part();
        let words: Vec<&str> = local
            .split(|c: char| c == '.' || c == '-' || c == '_' || c == '+')
            .collect();
        MACHINE_WORDS
            .iter()
            .any(|needle| local == *needle || words.iter().any(|word| word == needle))
    }

    fn is_bulk(&self) -> bool {
        let precedence = self.precedence.as_deref().unwrap_or("").to_lowercase();
        precedence == "bulk" || precedence == "list" || precedence == "junk"
    }

    fn is_automated(&self) -> bool {
        self.auto_submitted
            .as_deref()
            .map(|value| !value.eq_ignore_ascii_case("no"))
            .unwrap_or(false)
    }

    fn is_list(&self) -> bool {
        self.list_id.is_some() || self.list_unsubscribe.is_some()
    }

    fn subject_is_transactional(&self) -> bool {
        let subject = self.subject.to_lowercase();
        TRANSACTIONAL_WORDS
            .iter()
            .any(|word| subject.split_whitespace().any(|found| found.trim_matches(|c: char| !c.is_alphanumeric()) == *word))
    }
}

/// The local parts that mean a machine sent it. Deliberately a list rather than a regular
/// expression: somebody will need to add to it, and adding a word to a list is a smaller act than
/// editing a pattern.
const MACHINE_WORDS: [&str; 16] = [
    "noreply",
    "no-reply",
    "donotreply",
    "do-not-reply",
    "notifications",
    "notification",
    "notify",
    "mailer",
    "mailer-daemon",
    "bounce",
    "bounces",
    "postmaster",
    "automated",
    "auto",
    "alerts",
    "alert",
];

/// The words that make a subject a receipt. From `docs/features.md` section 2, rule 3.
const TRANSACTIONAL_WORDS: [&str; 14] = [
    "receipt",
    "order",
    "confirmation",
    "confirmed",
    "invoice",
    "shipped",
    "payment",
    "verify",
    "code",
    "ticket",
    "itinerary",
    "reservation",
    "statement",
    "delivery",
];

/// One row of the table: the question, the answer, and the sentence the card prints.
pub struct Rule {
    pub id: &'static str,
    pub reason: &'static str,
    pub destination: Destination,
    pub matches: fn(&Facts) -> bool,
}

/// The table, in order. The first row that matches decides.
///
/// The order is the whole of the logic and it is the order `docs/features.md` sets out, with one
/// thing worth saying out loud: transactional comes before bulk, because a receipt with an
/// unsubscribe footer is still a receipt, and a person who filed it under newsletters would never
/// find it again.
pub const RULES: &[Rule] = &[
    Rule {
        id: "transactional",
        reason: "Sent by a service on someone's behalf, so Paper Trail",
        destination: Destination::PaperTrail,
        matches: |facts| {
            facts.is_machine_address()
                && (facts.is_list() || facts.is_automated() || facts.subject_is_transactional())
        },
    },
    Rule {
        id: "receipt-subject",
        reason: "Reads like a receipt, so Paper Trail",
        destination: Destination::PaperTrail,
        matches: |facts| facts.subject_is_transactional() && (facts.is_automated() || facts.is_list()),
    },
    Rule {
        id: "category-updates",
        reason: "Gmail files this sender under Updates, so Paper Trail",
        destination: Destination::PaperTrail,
        matches: |facts| facts.has_label("CATEGORY_UPDATES"),
    },
    Rule {
        id: "list",
        reason: "Carries an unsubscribe header, so Feed",
        destination: Destination::Feed,
        matches: |facts| facts.is_list(),
    },
    Rule {
        id: "bulk",
        reason: "Sent to a list rather than to you, so Feed",
        destination: Destination::Feed,
        matches: |facts| facts.is_bulk(),
    },
    Rule {
        id: "category-promotions",
        reason: "Gmail files this sender under Promotions, so Feed",
        destination: Destination::Feed,
        matches: |facts| facts.has_label("CATEGORY_PROMOTIONS"),
    },
    Rule {
        id: "machine",
        reason: "Nobody reads replies to this address, so Paper Trail",
        destination: Destination::PaperTrail,
        matches: |facts| facts.is_machine_address() || facts.is_automated(),
    },
    // The last row matches everything, which is what makes this a table rather than a table and a
    // fallback: written by a person until something says otherwise.
    Rule {
        id: "person",
        reason: "Written by a person, so Inbox",
        destination: Destination::Inbox,
        matches: |_| true,
    },
];

/// The suggestion for one message, and the row that decided it.
pub fn suggest(facts: &Facts) -> &'static Rule {
    RULES
        .iter()
        .find(|rule| (rule.matches)(facts))
        .unwrap_or(&RULES[RULES.len() - 1])
}

// ---------------------------------------------------------------------------------------------
// Reading the facts back out of the mirror
// ---------------------------------------------------------------------------------------------

/// The facts for the earliest message from a sender, which is the message a Screener card shows and
/// the one a suggestion is about. First contact is what a sender rule is about, so a later message
/// from the same person must not change the suggestion under them.
pub fn facts_for_sender(conn: &Connection, address: &str) -> Result<Option<Facts>, String> {
    conn.query_row(
        "SELECT from_address, subject, list_id, list_unsubscribe, precedence, auto_submitted, labels
           FROM messages
          WHERE lower(from_address) = lower(?1)
          ORDER BY date_ms ASC
          LIMIT 1",
        [address],
        row_to_facts,
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn facts_for_message(conn: &Connection, message_id: &str) -> Result<Option<Facts>, String> {
    conn.query_row(
        "SELECT from_address, subject, list_id, list_unsubscribe, precedence, auto_submitted, labels
           FROM messages WHERE id = ?1",
        [message_id],
        row_to_facts,
    )
    .optional()
    .map_err(|e| e.to_string())
}

fn row_to_facts(row: &rusqlite::Row<'_>) -> rusqlite::Result<Facts> {
    let labels: String = row.get(6)?;
    Ok(Facts {
        from: row.get(0)?,
        subject: row.get(1)?,
        list_id: row.get(2)?,
        list_unsubscribe: row.get(3)?,
        precedence: row.get(4)?,
        auto_submitted: row.get(5)?,
        labels: serde_json::from_str(&labels).unwrap_or_default(),
    })
}

// ---------------------------------------------------------------------------------------------
// The overrides
// ---------------------------------------------------------------------------------------------

/// Whether a reply to a thread the account is already in waits in the Screener anyway.
///
/// The setting lives in `settings.json`, which this side of the app cannot read without an app
/// handle, and a list page is one SQL statement. So `settings::save` copies it into the mirror's own
/// `meta`, and this reads it from there. Off out of the box: a sender rule is about first contact,
/// not about conversations.
pub fn holds_replies(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'hold-replies'",
        [],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .map(|value| value == "1")
    .unwrap_or(false)
}

/// True when this account has written in the thread, which is what "a thread you are already in"
/// means in practice and is one indexed column rather than a walk up the `References` chain.
pub const IN_THREAD: &str = "EXISTS (SELECT 1 FROM messages m \
     WHERE m.provider_thread_id = t.provider_thread_id AND m.sent = 1)";

/// True when the sender has a rule, of either kind. The domain is taken from the address in SQL so
/// that the whole question stays inside the statement.
pub const HAS_RULE: &str = "EXISTS (SELECT 1 FROM state.sender_rules r \
     WHERE (r.is_domain = 0 AND r.subject = lower(t.from_address)) \
        OR (r.is_domain = 1 AND r.subject = substr(lower(t.from_address), \
             instr(t.from_address, '@') + 1)))";

/// The predicate for a sender whose destination is a given one.
pub fn rule_is(destination: Destination) -> String {
    format!(
        "EXISTS (SELECT 1 FROM state.sender_rules r \
           WHERE ((r.is_domain = 0 AND r.subject = lower(t.from_address)) \
              OR (r.is_domain = 1 AND r.subject = substr(lower(t.from_address), \
                   instr(t.from_address, '@') + 1))) \
             AND r.destination = '{}' \
             AND NOT EXISTS (SELECT 1 FROM state.sender_rules a \
                   WHERE a.is_domain = 0 AND a.subject = lower(t.from_address) \
                     AND a.destination <> '{}'))",
        state::write::destination_name(destination),
        state::write::destination_name(destination),
    )
}

// ---------------------------------------------------------------------------------------------
// The first run seed
// ---------------------------------------------------------------------------------------------

/// Screens in everyone the account already knows, routing each by the suggestion function.
///
/// Three sources at once, because a month of mail is not by itself a good answer to who a person
/// knows: every sender and every recipient inside the storage window, the provider's own address
/// book, and the Sent mail inside the window. All three are already on the device by the time this
/// runs, in `correspondents`, which the sync engine fills as it hydrates.
///
/// Safe to run twice, which matters because it runs on connect and a connect can be retried. A rule
/// that already exists is left exactly as it is, so a decision somebody made by hand is never
/// overwritten by a later seed, and neither is one they made in the Screener.
/// The flag that makes the seed a once per account event.
const SEEDED: &str = "screener-seeded";
const SEEDED_COUNT: &str = "screener-seeded-count";

/// Runs the seed the first time and never again.
///
/// The guard is the whole point. Screening in everyone the account already knows is right once, at
/// setup; doing it on every sync would screen in every new sender the moment they wrote, which is
/// the Screener not existing. After this has run, a sender nobody has decided about waits.
///
/// The count is remembered as well as returned, because the panel that says "163 senders were
/// screened in" is shown after the sync that did it and may be opened again.
pub fn seed_once(conn: &Connection) -> Result<u32, String> {
    if let Some(screened) = remembered(conn)? {
        return Ok(screened);
    }
    let screened = seed(conn)?;
    mark(conn, screened)?;
    Ok(screened)
}

/// What asking for the seed came back with.
#[derive(Debug, PartialEq, Eq)]
pub enum Seeded {
    /// The crawl has not finished, so there is nobody to look at yet. Nothing was marked.
    NotYet,
    /// Ran just now, and screened in this many.
    Ran(u32),
    /// Ran on an earlier pass, and screened in this many then.
    Already(u32),
}

/// The mark of the one repeat a seed that found nobody is allowed, so the repair below runs once.
const RESEEDED: &str = "screener-reseeded";

/// The seed, once the mirror is ready for it, and a refusal until then.
///
/// Ready means the first sync has listed the window and every row has its metadata. Before that
/// the correspondents table is a fraction of who the account knows, and a seed drawn from it would
/// mark itself done with a count of nobody. That happened: the front end asked for the seed the
/// moment consent came back, on a mirror with no rows in it, and every sender of the account then
/// waited in the Screener for ever while the Inbox said "Nothing here". The engine runs this on
/// every pass and the command behind the first-run panel runs it too, and neither can now seed a
/// half-known mailbox.
///
/// An account that was marked that way before this rule existed is repaired here rather than by
/// asking anybody to remove it and connect it again: a seed on record as having screened in nobody
/// is run once more when the mirror is ready, and once only, because a mailbox with mail in it
/// where the seed truly finds nobody is one where everyone already has a rule, and running it on
/// every pass after that would be work for the same answer.
pub fn seed_if_ready(conn: &Connection) -> Result<Seeded, String> {
    let remembered = remembered(conn)?;
    let reseeded = meta(conn, RESEEDED)?.is_some();
    if let Some(screened) = remembered {
        if screened > 0 || reseeded {
            return Ok(Seeded::Already(screened));
        }
    }
    let ready = crate::mirror::write::meta_get(conn, crate::mirror::write::FIRST_SYNC_KEY)?
        .is_some()
        && crate::mirror::write::count(conn, "SELECT COUNT(*) FROM messages WHERE hydrated = 0")?
            == 0;
    if !ready {
        return Ok(match remembered {
            Some(screened) => Seeded::Already(screened),
            None => Seeded::NotYet,
        });
    }
    let screened = seed(conn)?;
    mark(conn, screened)?;
    if remembered.is_some() {
        mark_meta(conn, RESEEDED, "1")?;
    }
    Ok(Seeded::Ran(screened))
}

fn meta(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| row.get(0))
        .optional()
        .map_err(|e| e.to_string())
}

fn mark_meta(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// The count the seed left behind, or nothing if it has never run.
fn remembered(conn: &Connection) -> Result<Option<u32>, String> {
    if meta(conn, SEEDED)?.is_none() {
        return Ok(None);
    }
    Ok(Some(
        meta(conn, SEEDED_COUNT)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
    ))
}

fn mark(conn: &Connection, screened: u32) -> Result<(), String> {
    mark_meta(conn, SEEDED, "1")?;
    mark_meta(conn, SEEDED_COUNT, &screened.to_string())
}

pub fn seed(conn: &Connection) -> Result<u32, String> {
    let known = known_addresses(conn)?;
    let mut screened = 0u32;

    for address in known {
        if state::read::destination_for(conn, &address)?.is_some() {
            continue;
        }
        let Some(facts) = facts_for_sender(conn, &address)? else {
            // Somebody the provider's address book knows and this device has never heard from. They
            // are screened in as a person, because that is what a contact is.
            state::write::set_rule(
                conn,
                &address,
                false,
                Destination::Inbox,
                Some("In your contacts, so Inbox"),
            )?;
            screened += 1;
            continue;
        };
        let rule = suggest(&facts);
        state::write::set_rule(conn, &address, false, rule.destination, Some(rule.reason))?;
        screened += 1;
    }

    Ok(screened)
}

/// Everyone the account already knows, from the three sources at once, lowercased and deduplicated.
fn known_addresses(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT lower(address) FROM correspondents
              WHERE address <> '' AND instr(address, '@') > 1
              ORDER BY last_ms DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(from: &str, subject: &str) -> Facts {
        Facts {
            from: from.to_string(),
            subject: subject.to_string(),
            ..Facts::default()
        }
    }

    #[test]
    fn a_person_writing_for_the_first_time_goes_to_the_inbox() {
        let rule = suggest(&facts("maya@example.org", "Dinner on Thursday?"));
        assert_eq!(rule.id, "person");
        assert_eq!(rule.destination, Destination::Inbox);
    }

    #[test]
    fn a_newsletter_goes_to_the_feed() {
        let mut f = facts("hello@thelongread.example", "The week in review");
        f.list_id = Some("<thelongread.example>".into());
        f.list_unsubscribe = Some("<https://thelongread.example/u/1>".into());
        let rule = suggest(&f);
        assert_eq!(rule.id, "list");
        assert_eq!(rule.destination, Destination::Feed);
    }

    #[test]
    fn a_receipt_with_an_unsubscribe_footer_is_still_a_receipt() {
        let mut f = facts("no-reply@spotify.example", "Your receipt from Spotify");
        f.list_unsubscribe = Some("<mailto:u@spotify.example>".into());
        let rule = suggest(&f);
        assert_eq!(rule.destination, Destination::PaperTrail);
        assert_eq!(rule.id, "transactional");
    }

    #[test]
    fn a_no_reply_address_goes_to_the_paper_trail_even_with_a_plain_subject() {
        let rule = suggest(&facts("noreply@bank.example", "Hello"));
        assert_eq!(rule.destination, Destination::PaperTrail);
    }

    #[test]
    fn precedence_bulk_on_its_own_is_the_feed() {
        let mut f = facts("news@brand.example", "Spring picks");
        f.precedence = Some("bulk".into());
        let rule = suggest(&f);
        assert_eq!(rule.id, "bulk");
    }

    #[test]
    fn an_automated_message_from_a_named_address_is_the_paper_trail() {
        let mut f = facts("builds@ci.example", "Build 4417 passed");
        f.auto_submitted = Some("auto-generated".into());
        assert_eq!(suggest(&f).destination, Destination::PaperTrail);
    }

    #[test]
    fn auto_submitted_no_is_a_person() {
        let mut f = facts("russell@northgate.example", "The lease");
        f.auto_submitted = Some("no".into());
        assert_eq!(suggest(&f).id, "person");
    }

    #[test]
    fn gmails_own_categories_are_a_hint_and_only_a_hint() {
        let mut promo = facts("hello@brand.example", "Twenty percent off");
        promo.labels = vec!["CATEGORY_PROMOTIONS".into()];
        assert_eq!(suggest(&promo).destination, Destination::Feed);

        let mut updates = facts("team@service.example", "Your weekly summary");
        updates.labels = vec!["CATEGORY_UPDATES".into()];
        assert_eq!(suggest(&updates).destination, Destination::PaperTrail);
    }

    #[test]
    fn a_machine_word_inside_a_name_is_not_a_machine() {
        // `norepeat` starts with neither `noreply` nor `no-reply`, and `alerting` is a word a
        // person can be called. A list of words matched on word boundaries has to say so.
        assert_eq!(suggest(&facts("norepeat@example.org", "Hello")).id, "person");
        assert_eq!(suggest(&facts("alertisimo@example.org", "Hello")).id, "person");
    }

    #[test]
    fn every_rule_has_a_reason_that_names_its_destination() {
        for rule in RULES {
            assert!(!rule.reason.is_empty(), "{} has no reason", rule.id);
            let named = match rule.destination {
                Destination::Inbox => "Inbox",
                Destination::Feed => "Feed",
                Destination::PaperTrail => "Paper Trail",
                Destination::ScreenedOut => "Screened out",
            };
            assert!(
                rule.reason.contains(named),
                "{} says {:?} but its reason does not name it: {}",
                rule.id,
                rule.destination,
                rule.reason
            );
        }
    }

    #[test]
    fn the_last_rule_matches_everything_so_there_is_always_a_suggestion() {
        let last = &RULES[RULES.len() - 1];
        assert!((last.matches)(&Facts::default()));
    }
}
