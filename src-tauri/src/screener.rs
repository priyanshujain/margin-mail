// The gate.
//
// A message from a sender with no rule is held: it is in no box and the Inbox carries a pill saying
// how many senders are waiting. This module is what the four keys on a Screener card do.
//
// Nothing here sends anything and nothing here touches the provider. Somebody who screens out two
// hundred senders makes zero API calls, which is a claim `docs/architecture.md` makes and
// `no_provider_call_is_made_by_deciding` holds it to.

use rusqlite::Connection;
use tauri::Manager;

use crate::db::Db;
use crate::dto::{Destination, Person, ScreenerCard, Undo};
use crate::routing;
use crate::state;

/// One card per waiting sender, not per message: the card is about a person, and their second
/// message is a count rather than a second decision to make.
pub fn list(conn: &Connection, account_id: &str) -> Result<Vec<ScreenerCard>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT lower(t.from_address) AS address,
                    MAX(t.from_name) AS name,
                    COUNT(*) AS waiting,
                    MAX(t.latest_ms) AS latest_ms
               FROM threads t
              WHERE t.trashed = 0 AND t.spam = 0 AND t.from_address <> ''
                AND NOT {}
                AND NOT {}
              GROUP BY lower(t.from_address)
              ORDER BY latest_ms DESC",
            routing::HAS_RULE, routing::IN_THREAD
        ))
        .map_err(|e| e.to_string())?;

    let waiting = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, u32>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut cards = Vec::new();
    for (address, name, count) in waiting {
        // The card shows the first message, because a sender rule is about first contact and the
        // suggestion has to be about the message that is being judged.
        let first: Option<(String, String, String, i64)> = conn
            .query_row(
                "SELECT t.thread_key, t.subject, t.snippet, t.latest_ms
                   FROM threads t
                  WHERE lower(t.from_address) = ?1
                  ORDER BY t.latest_ms ASC
                  LIMIT 1",
                [&address],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|e| e.to_string())
            .ok();
        let Some((thread_key, subject, snippet, date_ms)) = first else {
            continue;
        };
        let facts = routing::facts_for_sender(conn, &address)?.unwrap_or_default();
        let rule = routing::suggest(&facts);
        cards.push(ScreenerCard {
            account_id: account_id.to_string(),
            sender: Person {
                name: name.filter(|n| !n.is_empty()),
                address,
            },
            thread_key,
            subject,
            snippet,
            date_ms,
            suggestion: rule.destination,
            reason: rule.reason.to_string(),
            waiting: count,
        });
    }
    Ok(cards)
}

/// Sets one sender's rule.
///
/// The decision applies to their existing threads at once rather than only to what arrives next,
/// which is why this returns after a `store-changed`: a decision that leaves the mail you were
/// looking at where it was reads as a decision that did not work.
pub fn decide(
    conn: &Connection,
    address: &str,
    destination: Destination,
    whole_domain: bool,
) -> Result<(), String> {
    let subject = if whole_domain {
        state::write::domain_of(address)
            .ok_or_else(|| format!("{address} has no domain to set a rule for"))?
    } else {
        address.trim().to_lowercase()
    };
    // `set_rule` refuses a domain rule on a consumer domain, and refusing there rather than here
    // is deliberate: the contact card and the picker both reach it, and one refusal is one rule.
    state::write::set_rule(conn, &subject, whole_domain, destination, None)
}

/// Screens out every sender currently waiting, as one reversal.
pub fn clear_all(conn: &Connection, account_id: &str) -> Result<Vec<String>, String> {
    let cards = list(conn, account_id)?;
    let mut cleared = Vec::new();
    for card in &cards {
        state::write::set_rule(
            conn,
            &card.sender.address,
            false,
            Destination::ScreenedOut,
            Some("Screened out with everyone else waiting"),
        )?;
        cleared.push(card.sender.address.clone());
    }
    Ok(cleared)
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

fn accounts(db: &Db, account_id: Option<String>) -> Vec<String> {
    match account_id {
        Some(id) => vec![id],
        None => db.on_disk(),
    }
}

#[tauri::command(async)]
pub fn screener_list(
    app: tauri::AppHandle,
    account_id: Option<String>,
) -> Result<Vec<ScreenerCard>, String> {
    let db = app.state::<Db>();
    let mut all = Vec::new();
    for id in accounts(&db, account_id) {
        all.extend(db.with(&id, |conn| list(conn, &id))?);
    }
    all.sort_by(|a, b| b.date_ms.cmp(&a.date_ms));
    Ok(all)
}

#[tauri::command(async)]
pub fn screener_decide(
    app: tauri::AppHandle,
    account_id: String,
    address: String,
    destination: Destination,
    whole_domain: bool,
) -> Result<Undo, String> {
    let db = app.state::<Db>();
    let before = db.with(&account_id, |conn| {
        state::read::rule_for(conn, &account_id, &address)
    })?;
    db.with(&account_id, |conn| {
        decide(conn, &address, destination, whole_domain)
    })?;

    let label = match destination {
        Destination::ScreenedOut => format!("Screened out {address}"),
        other => format!("{address} goes to {}", place_name(other)),
    };
    let token = undo::push(UndoRule {
        account_id: account_id.clone(),
        before: vec![(address, before)],
        label: label.clone(),
    });
    crate::emit_store_changed(&app, "threads screener state");
    Ok(Undo {
        token,
        label,
        undo_ms: 0,
    })
}

#[tauri::command(async)]
pub fn screener_clear_all(
    app: tauri::AppHandle,
    account_id: Option<String>,
) -> Result<Undo, String> {
    let db = app.state::<Db>();
    let mut before: Vec<(String, Option<crate::dto::SenderRule>)> = Vec::new();
    let mut cleared = 0usize;
    let ids = accounts(&db, account_id.clone());
    let first = ids.first().cloned().unwrap_or_default();

    for id in ids {
        let addresses = db.with(&id, |conn| clear_all(conn, &id))?;
        cleared += addresses.len();
        for address in addresses {
            before.push((address, None));
        }
    }

    let label = match cleared {
        0 => "Nobody was waiting".to_string(),
        1 => "Screened out 1 sender".to_string(),
        many => format!("Screened out {many} senders"),
    };
    let token = undo::push(UndoRule {
        account_id: first,
        before,
        label: label.clone(),
    });
    crate::emit_store_changed(&app, "threads screener state");
    Ok(Undo {
        token,
        label,
        undo_ms: 0,
    })
}

/// The first run pass. Returns how many senders were screened in, which is what the onboarding
/// panel says out loud, and nothing while the first sync is still bringing the mailbox in: the
/// seed reads the correspondents the crawl writes, and a seed taken early marks itself done over
/// an empty table. The front end asks again when the account's sync reports idle.
#[tauri::command(async)]
pub fn screener_seed(app: tauri::AppHandle, account_id: String) -> Result<Option<u32>, String> {
    let db = app.state::<Db>();
    match db.with(&account_id, routing::seed_if_ready)? {
        routing::Seeded::NotYet => Ok(None),
        routing::Seeded::Ran(screened) => {
            if screened > 0 {
                crate::emit_store_changed(&app, "threads screener state");
            }
            Ok(Some(screened))
        }
        routing::Seeded::Already(screened) => Ok(Some(screened)),
    }
}

fn place_name(destination: Destination) -> &'static str {
    match destination {
        Destination::Inbox => "the Inbox",
        Destination::Feed => "the Feed",
        Destination::PaperTrail => "the Paper Trail",
        Destination::ScreenedOut => "Screened out",
    }
}

// ---------------------------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------------------------

/// A screening decision reverses by putting back the rule that was there, which is usually no rule
/// at all. Held here rather than in `mirror`'s stack because what it restores is a state row rather
/// than a set of provider labels, and the two have nothing in common but the word undo.
pub struct UndoRule {
    pub account_id: String,
    pub before: Vec<(String, Option<crate::dto::SenderRule>)>,
    pub label: String,
}

pub mod undo {
    use std::collections::VecDeque;
    use std::sync::{Mutex, OnceLock};

    use super::UndoRule;

    const DEPTH: usize = 50;

    fn stack() -> &'static Mutex<VecDeque<(String, UndoRule)>> {
        static STACK: OnceLock<Mutex<VecDeque<(String, UndoRule)>>> = OnceLock::new();
        STACK.get_or_init(|| Mutex::new(VecDeque::new()))
    }

    pub fn push(entry: UndoRule) -> String {
        let token = format!("rule-{}", super::next_token());
        crate::undo::note(&token, &entry.label);
        if let Ok(mut held) = stack().lock() {
            held.push_front((token.clone(), entry));
            while held.len() > DEPTH {
                held.pop_back();
            }
        }
        token
    }

    pub fn take(token: &str) -> Option<UndoRule> {
        crate::undo::forget(token);
        let mut held = stack().lock().ok()?;
        let at = held.iter().position(|(held_token, _)| held_token == token)?;
        held.remove(at).map(|(_, entry)| entry)
    }
}

fn next_token() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Reverses a screening decision. Called by `undo` when the token is one of ours.
pub fn undo_apply(app: &tauri::AppHandle, token: &str) -> Result<(), String> {
    let entry = undo::take(token).ok_or("that decision can no longer be taken back")?;
    let db = app.state::<Db>();
    db.with(&entry.account_id, |conn| {
        for (address, rule) in &entry.before {
            match rule {
                Some(rule) => state::write::set_rule(
                    conn,
                    &rule.subject,
                    rule.is_domain,
                    rule.destination,
                    rule.reason.as_deref(),
                )?,
                // There was no rule, so the sender goes back to waiting. `state.sender_rules` has
                // no way to say "no rule", by design, so this is a delete rather than a value.
                None => state::write::clear_rule(conn, address)?,
            }
        }
        Ok(())
    })?;
    crate::emit_store_changed(app, "threads screener state");
    Ok(())
}

/// True when the token belongs to this module, so `undo` knows who to ask.
pub fn owns(token: &str) -> bool {
    token.starts_with("rule-")
}


#[cfg(test)]
mod tests;
