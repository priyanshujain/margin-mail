// Reply later and Set aside.
//
// A pile is a stack of cards at the foot of a list column and a place of its own, and the key that
// puts a thread on one takes it off again. Coming off needs no record of where the thread came
// from, because a place is a view: a thread with no pile row is already in whichever place its
// sender's rule puts it, and nothing had to remember that.
//
// Nothing here touches the provider. A piled thread keeps every Gmail label it had; it simply does
// not render in the list it came from.

use rusqlite::{Connection, OptionalExtension};

use crate::decisions::{db_of, holders};
use crate::dto::{Pile, Undo};
use crate::state;
use crate::undo::Stack;

static UNDO: Stack<UndoPiles> = Stack::new("pile");

/// The pile a thread is on and where in the stack it sits. `state::read` answers the first half and
/// not the second, because nothing but a reversal has ever needed to put a card back exactly where
/// it was rather than on top.
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

pub struct Toggled {
    pub before: Vec<(String, Option<(Pile, i64)>)>,
    pub added: usize,
    pub removed: usize,
}

/// Puts each thread on the pile, or takes it off when that is where it already is. A thread on the
/// other pile moves across, which is one row either way: a thread is on at most one pile.
pub fn toggle(conn: &Connection, keys: &[String], pile: Pile) -> Result<Toggled, String> {
    let mut toggled = Toggled {
        before: Vec::new(),
        added: 0,
        removed: 0,
    };
    for key in keys {
        let before = pile_row(conn, key)?;
        toggled.before.push((key.clone(), before));
        match before {
            Some((held, _)) if held == pile => {
                state::write::clear_pile(conn, key)?;
                toggled.removed += 1;
            }
            _ => {
                state::write::set_pile(conn, key, pile)?;
                toggled.added += 1;
            }
        }
    }
    Ok(toggled)
}

pub fn pile_title(pile: Pile) -> &'static str {
    match pile {
        Pile::ReplyLater => "Reply later",
        Pile::SetAside => "Set aside",
    }
}

fn label(pile: Pile, added: usize, removed: usize) -> String {
    let name = pile_title(pile);
    match (added, removed) {
        (0, 0) => format!("Nothing to move to {name}"),
        (1, 0) => format!("Moved to {name}"),
        (many, 0) => format!("Moved {many} threads to {name}"),
        (0, 1) => format!("Taken out of {name}"),
        (0, many) => format!("Took {many} threads out of {name}"),
        (added, removed) => format!("Moved {added} in and {removed} out of {name}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------------------------

/// The prior row per key, per account, which is the only thing a toggle over a mixed selection can
/// be reversed from: half of it went on and half of it came off.
pub struct UndoPiles {
    pub before: Vec<(String, Vec<(String, Option<(Pile, i64)>)>)>,
}

pub fn owns(token: &str) -> bool {
    UNDO.owns(token)
}

pub fn undo_apply(app: &tauri::AppHandle, token: &str) -> Result<(), String> {
    let entry = UNDO
        .take(token)
        .ok_or("that change can no longer be taken back")?;
    let db = db_of(app)?;
    for (account_id, before) in &entry.before {
        db.with(account_id, |conn| {
            for (key, held) in before {
                match held {
                    Some((pile, position)) => {
                        state::write::set_pile_at(conn, key, *pile, *position)?
                    }
                    None => state::write::clear_pile(conn, key)?,
                }
            }
            Ok(())
        })?;
    }
    crate::emit_store_changed(app, "threads state");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

#[tauri::command(async)]
pub fn pile_toggle(app: tauri::AppHandle, keys: Vec<String>, pile: Pile) -> Result<Undo, String> {
    let db = db_of(&app)?;
    let mut before = Vec::new();
    let (mut added, mut removed) = (0usize, 0usize);
    for (account_id, held) in holders(db.inner(), &keys)? {
        let toggled = db.with(&account_id, |conn| toggle(conn, &held, pile))?;
        added += toggled.added;
        removed += toggled.removed;
        before.push((account_id, toggled.before));
    }

    let label = label(pile, added, removed);
    let token = UNDO.push(UndoPiles { before }, &label);
    crate::emit_store_changed(&app, "threads state");
    Ok(Undo {
        token,
        label,
        undo_ms: 0,
    })
}

#[cfg(test)]
mod tests;
