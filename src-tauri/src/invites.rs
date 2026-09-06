// Answering an invitation.
//
// This is the one feature in the app that needs a permission the account was not asked for when it
// was added, and Google gives installed apps no way to add one to a live grant: the answer is to run
// the whole consent again with the full list, which `google::account_grant` does. So the scope is
// checked here, before anything is fetched and before a token is even asked for, and the refusal
// names the scope so the frontend can put a Grant button under one sentence. Discovering it as a
// 403 halfway through would mean the card had already claimed to be answering.
//
// Everything after the check is `google::calendar`: find the event by the `UID` out of the message,
// import it when Gmail did not add it, patch this account's attendee row.

use rusqlite::{Connection, OptionalExtension};
use tauri::Manager;

use crate::db::Db;
use crate::decisions::db_of;
use crate::dto::{Invite, InviteResponse};
use crate::google::auth::{self, AuthState};
use crate::google::calendar;
use crate::mime;
use crate::mirror::read;
use crate::sync;

/// What answering needs before it can start: the invitation itself and the address answering it.
#[derive(Debug)]
pub struct Plan {
    pub invite: Invite,
    pub self_email: String,
}

/// The refusal, naming the scope. One sentence and the reason, because the frontend turns it into
/// one line and a button rather than a stack trace.
pub fn check_scope(granted: &[String]) -> Result<(), String> {
    if granted.iter().any(|scope| scope == calendar::SCOPE) {
        return Ok(());
    }
    Err(format!(
        "This account has not granted {}, which is what answering an invitation needs. Google \
         cannot add one permission to an app like this, so granting it runs the whole consent \
         again.",
        calendar::SCOPE
    ))
}

/// The invitation on a message that is already on the device. Parsed from the raw bytes rather than
/// from anything stored, because the card is a view of the `text/calendar` part and nothing else.
pub fn invite_of(conn: &Connection, message_id: &str) -> Result<Invite, String> {
    let raw = read::raw_body(conn, message_id)?
        .ok_or("that message has not been fetched yet, so its invitation cannot be read")?;
    let options = sync::hydrate::render_options(conn)?;
    mime::render(&raw, &options)?
        .invite
        .ok_or_else(|| "that message carries no invitation".to_string())
}

/// Gathers what the answer needs, scope first. An account without the permission makes no request
/// at all, which is the whole reason this is a step of its own.
pub fn plan(conn: &Connection, message_id: &str, granted: &[String]) -> Result<Plan, String> {
    check_scope(granted)?;
    let invite = invite_of(conn, message_id)?;
    let self_email = crate::send::own_address(conn)?
        .ok_or("this account has no address of its own yet, so it cannot answer as anybody")?;
    Ok(Plan { invite, self_email })
}

fn account_holding(db: &Db, message_id: &str) -> Result<String, String> {
    for (account_id, _) in sync::accounts(db, None) {
        let held: Option<String> = db.with(&account_id, |conn| {
            conn.query_row("SELECT id FROM messages WHERE id = ?1", [message_id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())
        })?;
        if held.is_some() {
            return Ok(account_id);
        }
    }
    Err("that message is not on this device".to_string())
}

#[tauri::command]
pub async fn invite_respond(
    app: tauri::AppHandle,
    message_id: String,
    response: InviteResponse,
) -> Result<(), String> {
    let db = db_of(&app)?;
    let account_id = account_holding(db.inner(), &message_id)?;
    let granted = crate::accounts::granted_scopes(&app, &account_id)?;
    check_scope(&granted)?;

    // Writes the account's own address into the mirror on the way past, which is where `plan` reads
    // it from and where a send with no app handle reads it from too.
    let _ = crate::send::own_person(&app, db.inner(), &account_id);
    let plan = db.with(&account_id, |conn| plan(conn, &message_id, &granted))?;

    let state = app
        .try_state::<AuthState>()
        .ok_or("this app has no Google session store")?;
    let token = auth::valid_access_token(&app, state.inner(), &account_id).await?;
    calendar::respond(&token, &plan.invite, &plan.self_email, response)
        .await
        .map_err(String::from)?;
    Ok(())
}

#[cfg(test)]
mod tests;
