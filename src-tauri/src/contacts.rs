// The person behind an address: where their mail delivers, whether it notifies, the private note,
// and the threads and the files they have sent.
//
// Every field of the card is a journalled event through `state::write`, so a decision made here
// roams with the state database and survives the mirror being thrown away. The destination is the
// one that looks different and is not: it is a sender rule, so it goes through the same
// `screener::decide` the Screener's own keys use, and it therefore applies to the sender's existing
// threads at once. A move that only affected future mail would read as a move that did not work.
//
// The card is also the only way to reverse a screening decision, which is why screening somebody
// out is one of the four destinations rather than a verb of its own.

use rusqlite::{Connection, OptionalExtension};
use tauri::Manager;

use crate::db::Db;
use crate::dto::{
    Attachment, ContactCard, ContactPatch, Destination, Person, Place, SenderRule, ThreadQuery,
    ThreadSummary, Unsubscribe,
};
use crate::mirror;
use crate::routing;
use crate::screener;
use crate::state;
use crate::sync;

/// As many as the popover has room for without becoming a list of its own.
const RECENT_THREADS: u32 = 5;
const RECENT_FILES: usize = 6;
const SUGGESTIONS: usize = 8;

// ---------------------------------------------------------------------------------------------
// Reading one person
// ---------------------------------------------------------------------------------------------

/// The name to print. The state database holds decisions about an address and never a name, because
/// a name is something the mail said rather than something the person chose, so this is the
/// mirror's answer: the address book's spelling first, then the latest message's.
fn name_of(conn: &Connection, address: &str) -> Result<Option<String>, String> {
    let found: Option<String> = conn
        .query_row(
            "SELECT name FROM correspondents WHERE lower(address) = ?1 AND name IS NOT NULL",
            [address],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(name) = found.filter(|name: &String| !name.trim().is_empty()) {
        return Ok(Some(name));
    }
    let latest: Option<String> = conn
        .query_row(
            "SELECT from_name FROM messages
              WHERE lower(from_address) = ?1 AND from_name IS NOT NULL AND from_name <> ''
              ORDER BY date_ms DESC LIMIT 1",
            [address],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(latest.filter(|name| !name.trim().is_empty()))
}

/// The sender's recent threads, as a list page rather than as a second query.
///
/// `from:` is the search index's own operator, so this is the query the search bar would run and
/// the row that comes back is the row a list draws. The exact address is checked again afterwards
/// because the index is tokenised: a phrase match over `ana example org` would also accept
/// `ana@example.org.uk`, and a card showing somebody else's mail is worse than a card showing less.
fn recent(
    conn: &Connection,
    account_id: &str,
    account_color: &str,
    address: &str,
    now: i64,
) -> Result<Vec<ThreadSummary>, String> {
    let query = ThreadQuery {
        account_id: Some(account_id.to_string()),
        place: Place::Search,
        label_id: None,
        query: Some(format!("from:{address}")),
        // Room to drop the near misses the index let through and still fill the card.
        limit: RECENT_THREADS * 4,
        cursor: None,
    };
    let page = mirror::read::threads_list(conn, account_id, account_color, &query, now)?;
    let wanted = address.trim().to_lowercase();
    Ok(page
        .threads
        .into_iter()
        .filter(|thread| {
            thread.from.address.to_lowercase() == wanted
                || thread
                    .participants
                    .iter()
                    .any(|person| person.address.to_lowercase() == wanted)
        })
        .map(|mut thread| {
            // A group head is a list's answer to "where am I", and a card is not a place: these
            // threads sit under the card's own Recent threads heading, newest first, with nothing
            // between them. The empty group is what a view with no grouping already means, and it
            // is what a list renderer already draws no head for.
            thread.group = String::new();
            thread
        })
        .take(RECENT_THREADS as usize)
        .collect())
}

/// What this sender has attached, newest first. Inline parts are the body's own images rather than
/// files somebody sent, so they are not files.
fn files(conn: &Connection, address: &str, limit: usize) -> Result<Vec<Attachment>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.message_id, a.filename, a.mime_type, a.size, a.inline, a.content_id,
                    a.cached_path IS NOT NULL
               FROM attachments a JOIN messages m ON m.id = a.message_id
              WHERE lower(m.from_address) = ?1 AND a.inline = 0 AND a.filename <> ''
              ORDER BY m.date_ms DESC, a.filename ASC
              LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![address, limit as i64], |row| {
            Ok(Attachment {
                id: row.get(0)?,
                message_id: row.get(1)?,
                filename: row.get(2)?,
                mime_type: row.get(3)?,
                size: row.get::<_, i64>(4)? as u64,
                inline: row.get::<_, i64>(5)? != 0,
                content_id: row.get(6)?,
                cached: row.get::<_, i64>(7)? != 0,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// `List-Unsubscribe` in the three forms it arrives in, from the latest message that carried one.
/// RFC 8058's one-click is the header pair, and without the `POST` half the URL is a page to visit
/// rather than a button to press.
fn unsubscribe(conn: &Connection, address: &str) -> Result<Option<Unsubscribe>, String> {
    let held: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT list_unsubscribe, list_unsub_post FROM messages
              WHERE lower(from_address) = ?1 AND list_unsubscribe IS NOT NULL
                AND list_unsubscribe <> ''
              ORDER BY date_ms DESC LIMIT 1",
            [address],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((header, post)) = held else {
        return Ok(None);
    };
    let mut mailto = None;
    let mut url = None;
    for part in header.split(',') {
        let value = part
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .trim();
        if value.starts_with("mailto:") {
            mailto = Some(value.to_string());
        } else if value.starts_with("http") {
            url = Some(value.to_string());
        }
    }
    if mailto.is_none() && url.is_none() {
        return Ok(None);
    }
    Ok(Some(Unsubscribe {
        one_click: url.is_some()
            && post
                .map(|value| value.to_ascii_lowercase().contains("one-click"))
                .unwrap_or(false),
        mailto,
        url,
    }))
}

/// Where this sender's mail goes, and when that was decided.
///
/// Nobody has to have been decided about: a card can be opened on somebody still waiting in the
/// Screener, and it says what would happen to them rather than nothing at all. That is the
/// suggestion function's answer, which is the same sentence the Screener card would print.
fn routed(conn: &Connection, address: &str) -> Result<(Destination, bool, Option<i64>), String> {
    if let Some(rule) = state::read::rule_for(conn, "", address)? {
        return Ok((rule.destination, rule.is_domain, Some(rule.decided_at_ms)));
    }
    let facts = routing::facts_for_sender(conn, address)?.unwrap_or_default();
    Ok((routing::suggest(&facts).destination, false, None))
}

/// The whole card for one person, on one account.
pub fn card(
    conn: &Connection,
    account_id: &str,
    account_color: &str,
    address: &str,
    now: i64,
) -> Result<ContactCard, String> {
    let address = address.trim().to_lowercase();
    if address.is_empty() {
        return Err("a contact card needs an address".into());
    }
    let held = state::read::contact(conn, &address)?
        .unwrap_or_else(|| state::read::Contact::unknown(&address));
    let (destination, domain_rule, screened_at_ms) = routed(conn, &address)?;
    Ok(ContactCard {
        person: Person {
            name: name_of(conn, &address)?,
            address: address.clone(),
        },
        account_id: account_id.to_string(),
        destination,
        domain_rule,
        domain_rule_allowed: domain_rule_allowed(&address),
        notify: held.notify,
        screened_at_ms,
        note: held.note,
        allow_remote_images: held.allow_remote_images,
        auto_trash_days: held.auto_trash_days,
        bundle: held.bundle,
        recent_threads: recent(conn, account_id, account_color, &address, now)?,
        files: files(conn, &address, RECENT_FILES)?,
        unsubscribe: unsubscribe(conn, &address)?,
    })
}

/// Whether "everyone at this domain" is a group of any kind. Consumer domains are not, which is why
/// the toggle is not offered rather than offered and then refused.
fn domain_rule_allowed(address: &str) -> bool {
    match state::write::domain_of(address) {
        Some(domain) => !state::write::is_consumer_domain(&domain),
        None => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------------------------

/// Where a sender's mail delivers, by address or by their whole domain.
///
/// Turning the domain toggle on has to take the address rule away first, because an address rule
/// beats a domain rule and the toggle would otherwise read as off the moment the card was asked
/// again. Turning it off does the opposite and leaves the domain rule standing: it still decides
/// everyone else at that domain, and quietly returning all of them to the Screener is a much larger
/// change than the one this card offered.
fn set_destination(
    conn: &Connection,
    address: &str,
    destination: Destination,
    whole_domain: bool,
) -> Result<(), String> {
    if whole_domain {
        let domain = state::write::domain_of(address)
            .ok_or_else(|| format!("{address} has no domain to set a rule for"))?;
        // `set_rule` owns the refusal, so it is asked before anything is undone: a change that is
        // going to be refused must not leave the sender with no rule at all on the way there.
        if state::write::is_consumer_domain(&domain) {
            return state::write::set_rule(conn, &domain, true, destination, None);
        }
        state::write::clear_rule(conn, address)?;
    }
    screener::decide(conn, address, destination, whole_domain)
}

/// True when the patch touches the contact record rather than the sender rule. Written out rather
/// than inferred, because `set_contact` appends the whole record and a patch that only moved
/// somebody would otherwise write a second event saying nothing.
fn touches_contact(patch: &ContactPatch) -> bool {
    patch.notify.is_some()
        || patch.note.is_some()
        || patch.allow_remote_images.is_some()
        || patch.auto_trash_days.is_some()
        || patch.bundle.is_some()
}

/// Applies the card's patch. Absent is unchanged, in both halves.
pub fn update(conn: &Connection, address: &str, patch: &ContactPatch) -> Result<(), String> {
    let address = address.trim().to_lowercase();
    if address.is_empty() {
        return Err("a contact needs an address".into());
    }
    if patch.destination.is_some() || patch.domain_rule.is_some() {
        let (held, held_domain, _) = routed(conn, &address)?;
        set_destination(
            conn,
            &address,
            patch.destination.unwrap_or(held),
            patch.domain_rule.unwrap_or(held_domain),
        )?;
    }
    if touches_contact(patch) {
        state::write::set_contact(conn, &address, patch)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The Contacts place
// ---------------------------------------------------------------------------------------------

/// Everyone with a rule, searchable.
///
/// A domain rule decides senders rather than being one, so it contributes the addresses at that
/// domain the device has actually heard from rather than a row spelling the domain out: the card is
/// about a person and `contact_card` is keyed on an address.
///
/// Three statements for the whole page rather than a card's worth per row: the rules, the contact
/// records and the names, joined in Rust. `ContactCard` is the frozen return type and it carries a
/// person's threads, files and unsubscribe header, none of which a row draws, so they are left
/// empty here and filled when a card is opened. Answering them per row would be three more queries
/// per sender for something nothing reads.
pub fn list(conn: &Connection, account_id: &str, query: &str) -> Result<Vec<ContactCard>, String> {
    let rules = state::read::rules(conn, account_id)?;
    let mut addresses: Vec<String> = Vec::new();
    let mut domains: Vec<String> = Vec::new();
    for rule in &rules {
        if rule.is_domain {
            domains.push(rule.subject.clone());
        } else {
            addresses.push(rule.subject.clone());
        }
    }
    for domain in &domains {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT lower(address) FROM correspondents
                  WHERE instr(lower(address), '@') > 1
                    AND substr(lower(address), instr(address, '@') + 1) = ?1",
            )
            .map_err(|e| e.to_string())?;
        let found = stmt
            .query_map([domain], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        addresses.extend(found);
    }
    addresses.sort();
    addresses.dedup();

    let records: std::collections::HashMap<String, state::read::Contact> =
        state::read::contacts(conn)?
            .into_iter()
            .map(|record| (record.address.clone(), record))
            .collect();
    let names = names(conn)?;
    let needle = query.trim().to_lowercase();

    let mut out = Vec::new();
    for address in addresses {
        let name = names.get(&address).cloned();
        if !needle.is_empty() {
            let matched = address.contains(&needle)
                || name
                    .as_deref()
                    .map(|name| name.to_lowercase().contains(&needle))
                    .unwrap_or(false);
            if !matched {
                continue;
            }
        }
        let held = records
            .get(&address)
            .cloned()
            .unwrap_or_else(|| state::read::Contact::unknown(&address));
        // Every address here came from a rule, so one decides it. The one that does is found in the
        // set already in hand rather than asked for again, because a query per row is what a list
        // must not do.
        let Some(rule) = deciding(&rules, &address) else {
            continue;
        };
        out.push(ContactCard {
            person: Person {
                name,
                address: address.clone(),
            },
            account_id: account_id.to_string(),
            destination: rule.destination,
            domain_rule: rule.is_domain,
            domain_rule_allowed: domain_rule_allowed(&address),
            notify: held.notify,
            screened_at_ms: Some(rule.decided_at_ms),
            note: held.note,
            allow_remote_images: held.allow_remote_images,
            auto_trash_days: held.auto_trash_days,
            bundle: held.bundle,
            recent_threads: Vec::new(),
            files: Vec::new(),
            unsubscribe: None,
        });
    }
    Ok(out)
}

/// The rule deciding an address, out of the set already in hand. An address rule beats a domain
/// rule, which is the order `state::read::rule_for` puts them in and the order this repeats.
fn deciding<'a>(rules: &'a [SenderRule], address: &str) -> Option<&'a SenderRule> {
    if let Some(rule) = rules
        .iter()
        .find(|rule| !rule.is_domain && rule.subject == address)
    {
        return Some(rule);
    }
    let domain = state::write::domain_of(address)?;
    rules
        .iter()
        .find(|rule| rule.is_domain && rule.subject == domain)
}

/// Every name the mirror knows, in one statement, so the list above is not a lookup per row.
fn names(conn: &Connection) -> Result<std::collections::HashMap<String, String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT lower(address), name FROM correspondents
              WHERE name IS NOT NULL AND name <> ''",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut out = std::collections::HashMap::new();
    for row in rows {
        let (address, name) = row.map_err(|e| e.to_string())?;
        out.insert(address, name);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Autocomplete
// ---------------------------------------------------------------------------------------------

/// A `LIKE` pattern's own wildcards, so somebody typing a percent sign is typing a percent sign.
fn like_escaped(prefix: &str) -> String {
    prefix
        .trim()
        .to_lowercase()
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Autocomplete, in one local query.
///
/// Both passes are the same table. `correspondents` is everyone this account has written to or
/// heard from, and the sync engine tops it up from the provider's address book with `source` saying
/// which is which, so the mirror's own people rank above the provider's and nobody's address is
/// ever sent anywhere to be looked up. That last part is what the schema promises and it is the
/// reason this is not a network call.
///
/// Frequency before recency, with a message you sent counting double: writing to somebody is a
/// stronger statement that you know them than receiving from them, which is why a mailing list you
/// have never answered does not outrank a colleague. Recency breaks the ties.
pub fn suggest(conn: &Connection, prefix: &str, limit: usize) -> Result<Vec<Person>, String> {
    let needle = like_escaped(prefix);
    if needle.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(
            "SELECT address, name FROM correspondents
              WHERE address <> ''
                AND (lower(address) LIKE ?1 || '%' ESCAPE '\\'
                  OR lower(name) LIKE ?1 || '%' ESCAPE '\\'
                  OR lower(name) LIKE '% ' || ?1 || '%' ESCAPE '\\')
              ORDER BY CASE WHEN source = 'mirror' THEN 0 ELSE 1 END ASC,
                       seen_count + 2 * sent_count DESC,
                       last_ms DESC,
                       address ASC
              LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![needle, limit as i64], |row| {
            Ok(Person {
                name: row
                    .get::<_, Option<String>>(1)?
                    .filter(|name| !name.trim().is_empty()),
                address: row.get::<_, String>(0)?.to_lowercase(),
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

fn db_of(app: &tauri::AppHandle) -> Result<tauri::State<'_, Db>, String> {
    app.try_state::<Db>()
        .ok_or_else(|| "the mirror is not open yet".to_string())
}

#[tauri::command(async)]
pub fn contact_card(
    app: tauri::AppHandle,
    account_id: String,
    address: String,
) -> Result<ContactCard, String> {
    let db = db_of(&app)?;
    let color = sync::accounts(db.inner(), Some(&account_id))
        .into_iter()
        .next()
        .map(|(_, color)| color)
        .unwrap_or_else(|| "hue-1".to_string());
    let now = mirror::write::now_ms();
    db.with(&account_id, |conn| {
        card(conn, &account_id, &color, &address, now)
    })
}

#[tauri::command(async)]
pub fn contact_update(
    app: tauri::AppHandle,
    account_id: String,
    address: String,
    patch: ContactPatch,
) -> Result<(), String> {
    let db = db_of(&app)?;
    let moved = patch.destination.is_some() || patch.domain_rule.is_some();
    db.with(&account_id, |conn| update(conn, &address, &patch))?;
    // A destination is a sender rule, and every routed place is a query over those rules, so the
    // lists somebody is looking at are already wrong by the time this returns.
    crate::emit_store_changed(&app, if moved { "threads state" } else { "state" });
    Ok(())
}

#[tauri::command(async)]
pub fn contacts_list(
    app: tauri::AppHandle,
    account_id: Option<String>,
    query: String,
) -> Result<Vec<ContactCard>, String> {
    let db = db_of(&app)?;
    let mut all = Vec::new();
    for (id, _) in sync::accounts(db.inner(), account_id.as_deref()) {
        all.extend(db.with(&id, |conn| list(conn, &id, &query))?);
    }
    all.sort_by(|a, b| a.person.address.cmp(&b.person.address));
    Ok(all)
}

#[tauri::command(async)]
pub fn contacts_suggest(
    app: tauri::AppHandle,
    account_id: String,
    prefix: String,
) -> Result<Vec<Person>, String> {
    let db = db_of(&app)?;
    db.with(&account_id, |conn| suggest(conn, &prefix, SUGGESTIONS))
}

#[cfg(test)]
mod tests;
