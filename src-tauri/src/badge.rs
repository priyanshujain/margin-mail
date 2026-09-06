// The number on the dock icon.
//
// One number, one icon, every account summed, and it is the count of unseen threads in the Inbox
// rather than the mailbox's unread count. Mailspring shows 999+ on a Gmail account because it
// counts every unread message carrying the `INBOX` label, and that number is exactly the noise
// this app was written to remove: what it is supposed to say is how many things are actually
// waiting for a decision.
//
// The count itself is `mirror::read::inbox_unseen`, which is the list's own definition of the
// group counted rather than selected. Nothing in this file knows what the Inbox is, on purpose.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::Manager;

use crate::db::Db;
use crate::mirror::read;

/// Long enough for a sync pass to finish landing what it landed, short enough that nobody sees the
/// lag on a number they read with their eyes.
const SETTLE_MS: u64 = 200;

/// True while a recount is already on its way, so a burst of changes costs one query rather than
/// one each.
static QUEUED: AtomicBool = AtomicBool::new(false);

/// macOS and Linux, and nowhere else.
///
/// `set_badge_count` compiles everywhere, so this is about honesty rather than about the build: on
/// Windows the call does nothing and the taskbar wants `set_overlay_icon` with an image drawn for
/// it, which is not written here, and Android has no badge at all. A platform with no badge to
/// paint has no reason to run the count either, so this gates the whole of it rather than the last
/// line of it.
const CARRIES_A_BADGE: bool = cfg!(any(target_os = "macos", target_os = "linux"));

/// Which `store-changed` scopes can move the number.
///
/// `emit_store_changed` fires for everything the app does and most of it cannot touch this count. A
/// body landing is scoped `thread`, a clip is `state`, a drained outbox is `outbox`, and none of
/// the three adds or removes a thread from New for you. Reading the scope here is what keeps the
/// badge a query per real change rather than a query per event.
///
/// `settings` is in the list because the toggle itself has to be obeyed at once, and `accounts`
/// because an account added or removed changes what there is to sum over.
pub fn moves_the_count(reason: &str) -> bool {
    reason
        .split_whitespace()
        .any(|scope| matches!(scope, "threads" | "accounts" | "settings"))
}

/// What the dock should show, which is nothing at all rather than a `0`.
///
/// Separate from the call that puts it there so that the rule can be tested without a running app,
/// and because `Some(0)` and `None` mean the same thing to the platform while only one of them is
/// what is meant here.
pub fn to_show(on: bool, count: i64) -> Option<i64> {
    (on && count > 0).then_some(count)
}

/// Every account's New for you, added up.
pub fn count(db: &Db) -> Result<i64, String> {
    let mut total = 0;
    for account_id in db.on_disk() {
        total += db.with(&account_id, read::inbox_unseen)?;
    }
    Ok(total)
}

/// The hook on the one notification path there is. `lib.rs` calls this from `emit_store_changed`,
/// so the badge follows the same signal the frontend does rather than needing its own.
pub fn on_store_changed(app: &tauri::AppHandle, reason: &str) {
    if moves_the_count(reason) {
        refresh(app);
    }
}

/// Recounts and repaints, once, shortly.
///
/// Deferred rather than done here because a first sync emits as it goes and thirty-nine counts on
/// the way to the fortieth are thirty-nine wasted, and because the emit sits on the return path of
/// whatever command made the change, which has no business waiting on a query nobody asked it for.
pub fn refresh(app: &tauri::AppHandle) {
    if !CARRIES_A_BADGE || QUEUED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
        QUEUED.store(false, Ordering::SeqCst);
        apply(&app);
    });
}

fn apply(app: &tauri::AppHandle) {
    let on = crate::settings::load(app)
        .map(|settings| settings.badge)
        .unwrap_or(false);

    // Turning it off is a clear, and it does not need the count to know that.
    if !on {
        show(app, None);
        return;
    }
    let Some(db) = app.try_state::<Db>() else {
        return;
    };
    // A badge cleared because a query failed reads as "nothing is waiting", which is the one thing
    // it must never say wrongly. Leave what is on the dock and try again at the next change.
    if let Ok(count) = count(db.inner()) {
        show(app, to_show(true, count));
    }
}

/// The window rather than the app, because that is where tauri hangs the call. On macOS it reaches
/// the dock tile, which is the app's and not the window's, so a window that has been closed to the
/// menu bar still carries the right number.
fn show(app: &tauri::AppHandle, count: Option<i64>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_badge_count(count);
    }
}
