// Move to Inbox, Feed or Paper Trail.
//
// A place is a view, so moving is a decision rather than a provider call, the same as a Screener
// card. One thread is moved by a row of its own in `state.thread_places`, which beats its sender's
// rule and goes with every message that lands in the thread later. Everything from a sender is the
// sender's rule, and setting it takes back the moves made by hand on that sender's threads, because
// "everything" means those too.
//
// A moved thread comes off a pile, because a piled thread shows only in its pile and a move that
// left it there would not move it anywhere you could see.

use rusqlite::{Connection, OptionalExtension};

use crate::decisions::{db_of, holders};
use crate::dto::{Destination, Pile, SenderRule, Undo};
use crate::state;
use crate::undo::Stack;

static UNDO: Stack<UndoMove> = Stack::new("move");

/// What one account looked like before, which is what a move is reversed from.
#[derive(Default)]
pub struct Before {
    pub places: Vec<(String, Option<Destination>)>,
    pub piles: Vec<(String, Pile, i64)>,
    pub rules: Vec<(String, Option<SenderRule>)>,
}

pub struct UndoMove {
    pub before: Vec<(String, Before)>,
}

fn sender_of(conn: &Connection, thread_key: &str) -> Result<Option<String>, String> {
    let address: Option<String> = conn
        .query_row(
            "SELECT lower(from_address) FROM threads WHERE thread_key = ?1",
            [thread_key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(address.filter(|a| !a.is_empty()))
}

fn pile_row(conn: &Connection, thread_key: &str) -> Result<Option<(Pile, i64)>, String> {
    let held: Option<(String, i64)> = conn
        .query_row(
            "SELECT pile, position FROM state.piles WHERE thread_key = ?1",
            [thread_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(held.and_then(|(name, position)| {
        state::read::pile_named(&name).map(|pile| (pile, position))
    }))
}

fn take_off_pile(conn: &Connection, key: &str, before: &mut Before) -> Result<(), String> {
    if let Some((pile, position)) = pile_row(conn, key)? {
        before.piles.push((key.to_string(), pile, position));
        state::write::clear_pile(conn, key)?;
    }
    Ok(())
}

fn hand_back(conn: &Connection, key: &str, before: &mut Before) -> Result<(), String> {
    if let Some(place) = state::read::thread_place(conn, key)? {
        before.places.push((key.to_string(), Some(place)));
        state::write::set_thread_place(conn, key, None)?;
    }
    Ok(())
}

/// Moves each thread on its own.
pub fn move_threads(
    conn: &Connection,
    keys: &[String],
    destination: Destination,
) -> Result<Before, String> {
    let mut before = Before::default();
    for key in keys {
        before
            .places
            .push((key.clone(), state::read::thread_place(conn, key)?));
        state::write::set_thread_place(conn, key, Some(destination))?;
        take_off_pile(conn, key, &mut before)?;
    }
    Ok(before)
}

/// Moves everything from the senders of these threads, and returns who they were.
pub fn move_senders(
    conn: &Connection,
    account_id: &str,
    keys: &[String],
    destination: Destination,
) -> Result<(Before, Vec<String>), String> {
    let mut before = Before::default();
    let mut senders: Vec<String> = Vec::new();
    for key in keys {
        if let Some(address) = sender_of(conn, key)? {
            if !senders.contains(&address) {
                senders.push(address);
            }
        }
        take_off_pile(conn, key, &mut before)?;
    }
    for address in &senders {
        let rule = state::read::rule_for(conn, account_id, address)?;
        before.rules.push((address.clone(), rule));
        state::write::set_rule(conn, address, false, destination, Some("Moved by hand"))?;

        let mut stmt = conn
            .prepare(
                "SELECT tp.thread_key FROM state.thread_places tp
                   JOIN threads t ON t.thread_key = tp.thread_key
                  WHERE lower(t.from_address) = ?1",
            )
            .map_err(|e| e.to_string())?;
        let placed = stmt
            .query_map([address], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for key in placed {
            hand_back(conn, &key, &mut before)?;
        }
    }
    Ok((before, senders))
}

/// Puts back what a move changed, in the reverse of the order it changed it.
pub fn restore(conn: &Connection, before: &Before) -> Result<(), String> {
    for (address, rule) in before.rules.iter().rev() {
        match rule {
            Some(rule) if !rule.is_domain => state::write::set_rule(
                conn,
                &rule.subject,
                false,
                rule.destination,
                rule.reason.as_deref(),
            )?,
            // A sender routed by their domain goes back to it, and a sender with no rule at all goes
            // back to waiting in the Screener.
            _ => state::write::clear_address_rule(conn, address)?,
        }
    }
    for (key, place) in before.places.iter().rev() {
        state::write::set_thread_place(conn, key, *place)?;
    }
    for (key, pile, position) in &before.piles {
        state::write::set_pile_at(conn, key, *pile, *position)?;
    }
    Ok(())
}

fn place_name(destination: Destination) -> &'static str {
    match destination {
        Destination::Inbox => "Inbox",
        Destination::Feed => "Feed",
        Destination::PaperTrail => "Paper Trail",
        Destination::ScreenedOut => "Screened out",
    }
}

pub fn owns(token: &str) -> bool {
    UNDO.owns(token)
}

pub fn undo_apply(app: &tauri::AppHandle, token: &str) -> Result<(), String> {
    let entry = UNDO
        .take(token)
        .ok_or("that move can no longer be taken back")?;
    let db = db_of(app)?;
    for (account_id, before) in &entry.before {
        db.with(account_id, |conn| restore(conn, before))?;
    }
    crate::emit_store_changed(app, "threads screener state");
    Ok(())
}

#[tauri::command(async)]
pub fn thread_move(
    app: tauri::AppHandle,
    keys: Vec<String>,
    destination: Destination,
    everything_from_sender: bool,
) -> Result<Undo, String> {
    if destination == Destination::ScreenedOut {
        return Err("screening out is the Screener's, not a move".into());
    }
    let db = db_of(&app)?;
    let mut before = Vec::new();
    let mut senders: Vec<String> = Vec::new();
    for (account_id, held) in holders(db.inner(), &keys)? {
        let changed = db.with(&account_id, |conn| {
            if everything_from_sender {
                let (changed, moved) = move_senders(conn, &account_id, &held, destination)?;
                senders.extend(moved);
                Ok(changed)
            } else {
                move_threads(conn, &held, destination)
            }
        })?;
        before.push((account_id, changed));
    }

    let name = place_name(destination);
    let label = if everything_from_sender {
        match senders.as_slice() {
            [one] => format!("Everything from {one} goes to {name}"),
            many => format!("Everything from {} senders goes to {name}", many.len()),
        }
    } else {
        match keys.len() {
            1 => format!("Moved to {name}"),
            many => format!("Moved {many} threads to {name}"),
        }
    };
    let token = UNDO.push(UndoMove { before }, &label);
    crate::emit_store_changed(&app, "threads screener state");
    Ok(Undo {
        token,
        label,
        undo_ms: 0,
    })
}

#[cfg(test)]
mod tests;
