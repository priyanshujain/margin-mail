// The two commands behind `z` and behind the Undo on every toast, and the stack the modules that
// hand out tokens share.
//
// They are here rather than in `mirror` because undo is not a mirror concept: reversing a send
// means taking a message out of the outbox before its hold expires, and reversing a pile or a
// snooze means an event in the state database. A token says who owns it, and this is the one place
// that knows the whole list, so a new kind of reversal is a prefix here rather than a second undo
// command the frontend has to learn.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::clips;
use crate::decisions;
use crate::mirror;
use crate::piles;
use crate::screener;
use crate::send;
use crate::snooze;
use crate::unsubscribe;

/// Deep enough that nobody reaches the bottom of it in a session, shallow enough that it is not a
/// second copy of what it reverses.
const DEPTH: usize = 50;

fn next_token() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The order things happened in, across every stack.
///
/// Each module holds what its own reversals are made of, and none of them can see the others, so
/// `z` needs somewhere that knows an archive happened after a pile. This is a ledger of tokens and
/// their labels and nothing else: the entry itself stays with the module that knows how to apply
/// it. Every push notes itself here and every take forgets itself, so undoing from a toast cannot
/// leave `z` pointing at something that has already been taken back.
///
/// A type rather than a pair of free functions over a static, so that a test can hold one of its
/// own. There is exactly one in a running app, and it is `ledger()`.
#[derive(Default)]
pub struct Ledger {
    held: Mutex<VecDeque<(String, String)>>,
}

impl Ledger {
    pub fn note(&self, token: &str, label: &str) {
        if let Ok(mut held) = self.held.lock() {
            held.push_front((token.to_string(), label.to_string()));
            while held.len() > DEPTH {
                held.pop_back();
            }
        }
    }

    pub fn forget(&self, token: &str) {
        if let Ok(mut held) = self.held.lock() {
            held.retain(|(held_token, _)| held_token != token);
        }
    }

    /// The newest reversal still standing, which is what `z` reaches for.
    pub fn latest(&self) -> Option<(String, String)> {
        self.held.lock().ok()?.front().cloned()
    }

    /// Where a token sits, newest first. For tests; nothing in the app asks.
    #[cfg(test)]
    fn position(&self, token: &str) -> Option<usize> {
        self.held
            .lock()
            .ok()?
            .iter()
            .position(|(held, _)| held == token)
    }
}

fn ledger() -> &'static Ledger {
    static LEDGER: OnceLock<Ledger> = OnceLock::new();
    LEDGER.get_or_init(Ledger::default)
}

pub fn note(token: &str, label: &str) {
    ledger().note(token, label);
}

pub fn forget(token: &str) {
    ledger().forget(token);
}

/// One module's reversals, held in Rust rather than handed to the frontend, because a bulk action
/// over mixed prior state cannot be reversed from what the frontend knew.
///
/// Generic over what a reversal is, and one static per module rather than one shared stack, because
/// what a pile reverses and what a label change reverses have nothing in common but the word undo.
/// The prefix is what `owns` reads, so a token never reaches the wrong module.
pub struct Stack<T> {
    prefix: &'static str,
    held: Mutex<VecDeque<(String, T)>>,
}

impl<T> Stack<T> {
    pub const fn new(prefix: &'static str) -> Stack<T> {
        Stack {
            prefix,
            held: Mutex::new(VecDeque::new()),
        }
    }

    /// The label is what the toast says and what `z` reports having undone, so it is taken here
    /// rather than kept only on the `Undo` the command returns: a reversal that cannot say what it
    /// was is a reversal that cannot be offered.
    pub fn push(&self, entry: T, label: &str) -> String {
        let token = format!("{}-{}", self.prefix, next_token());
        if let Ok(mut held) = self.held.lock() {
            held.push_front((token.clone(), entry));
            while held.len() > DEPTH {
                held.pop_back();
            }
        }
        note(&token, label);
        token
    }

    /// Reads an entry without taking it, for the one case that needs to look before it acts: a
    /// toast holds an undo token and Send now needs the outbox row behind it.
    pub fn peek(&self, token: &str) -> Option<T>
    where
        T: Clone,
    {
        let held = self.held.lock().ok()?;
        held.iter()
            .find(|(held_token, _)| held_token == token)
            .map(|(_, entry)| entry.clone())
    }

    pub fn take(&self, token: &str) -> Option<T> {
        forget(token);
        let mut held = self.held.lock().ok()?;
        let at = held.iter().position(|(held_token, _)| held_token == token)?;
        held.remove(at).map(|(_, entry)| entry)
    }

    /// The separator is checked as well as the prefix, so a module called `pile` never claims a
    /// token belonging to one called `piles`.
    pub fn owns(&self, token: &str) -> bool {
        token
            .strip_prefix(self.prefix)
            .map(|rest| rest.starts_with('-'))
            .unwrap_or(false)
    }
}

/// `z`. Reverses the most recent reversible action and returns what it was called, for the toast.
/// Nothing to take back is not an error: the key does nothing and says nothing, which is the rule
/// docs/keyboard.md sets for a key that would act on nothing.
#[tauri::command(async)]
pub fn undo_last(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let Some((token, label)) = ledger().latest() else {
        return Ok(None);
    };
    apply(&app, &token)?;
    Ok(Some(label))
}

/// The Undo on a particular toast, which names the action it belongs to rather than the latest one.
/// A toast that has scrolled past two more actions must still undo its own.
#[tauri::command(async)]
pub fn undo_token(app: tauri::AppHandle, token: String) -> Result<(), String> {
    apply(&app, &token)
}

/// Hands a token to whoever owns it.
fn apply(app: &tauri::AppHandle, token: &str) -> Result<(), String> {
    forget(token);
    // A token says who owns it. A screening decision reverses a row in the state database and a
    // triage action reverses a set of provider labels, and the two have nothing in common but the
    // word undo, so they keep their own stacks and this is the one place that knows them all.
    if screener::owns(token) {
        return screener::undo_apply(app, token);
    }
    if piles::owns(token) {
        return piles::undo_apply(app, token);
    }
    if snooze::owns(token) {
        return snooze::undo_apply(app, token);
    }
    if decisions::owns(token) {
        return decisions::undo_apply(app, token);
    }
    if clips::owns(token) {
        return clips::undo_apply(app, token);
    }
    if unsubscribe::owns(token) {
        return unsubscribe::undo_apply(app, token);
    }
    if send::owns(token) {
        return send::undo_apply(app, token);
    }
    mirror::undo_apply(app, token)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `z` used to reach only the mirror's own stack, so a pile or a rename could be undone from
    /// its toast and not from the keyboard. What makes it work is that every stack shares one token
    /// counter, so the ledger's order is the order things actually happened in.
    ///
    /// A ledger of its own rather than the app's: that one is process wide and bounded, and a suite
    /// that runs in parallel would evict these two entries out from under the assertion.
    #[test]
    fn the_ledger_orders_reversals_and_forgets_what_is_taken_back() {
        let ledger = Ledger::default();
        ledger.note("pile-1", "Replied later to Ana");
        ledger.note("flags-2", "Archived 3 threads");

        assert_eq!(
            ledger.latest(),
            Some(("flags-2".to_string(), "Archived 3 threads".to_string())),
            "z reaches the newest, whichever stack it came from"
        );
        assert!(ledger.position("pile-1") > ledger.position("flags-2"));

        // Undoing from a toast rather than from `z` must not leave the ledger pointing at
        // something that has already been taken back.
        ledger.forget("flags-2");
        assert_eq!(
            ledger.latest(),
            Some(("pile-1".to_string(), "Replied later to Ana".to_string()))
        );
        ledger.forget("pile-1");
        assert_eq!(ledger.latest(), None);
    }

    #[test]
    fn a_stack_notes_itself_in_the_ledger_and_forgets_itself_when_taken() {
        static HERE: Stack<u8> = Stack::new("ledger-test");
        let token = HERE.push(9, "Set aside");
        assert!(ledger().position(&token).is_some());
        assert_eq!(HERE.take(&token), Some(9));
        assert_eq!(ledger().position(&token), None);
    }

    #[test]
    fn a_token_never_reaches_the_wrong_stack() {
        static PILE: Stack<u8> = Stack::new("pile");
        static PILES: Stack<u8> = Stack::new("piles");
        let token = PILES.push(7, "Set aside");
        assert!(PILES.owns(&token));
        assert!(!PILE.owns(&token), "a prefix is not a name");
        PILES.take(&token);
    }
}
