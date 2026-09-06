// Unsubscribe, and the two follow ups it offers.
//
// Three paths in order of preference, from `docs/features.md` section 12. RFC 8058's one-click is
// the only one that is a confirmation: the sender published a header saying a POST to this URL is
// enough, and it is enough. A `mailto:` is a message that has to be sent. Anything else is a page,
// and opening a page is not a confirmation, so the app says so rather than claiming to have done
// something.
//
// An unsubscribe endpoint is a URL a stranger chose, so the POST is made the way the image fetch in
// `attachments` is made: no cookie store is compiled into this build at all, `referer(false)` stops
// a redirect naming where it came from, and a redirect to a different host is not followed. A list
// that answers by sending you somewhere else has not confirmed anything either.

use std::sync::OnceLock;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension};
use tauri_plugin_opener::OpenerExt;

use crate::decisions::db_of;
use crate::dto::{Destination, Draft, FlagPatch, Person, SenderRule, Undo};
use crate::mirror::{self, write::OutboxRow};
use crate::state;
use crate::sync::outbox;
use crate::undo::Stack;

static UNDO: Stack<UndoUnsubscribe> = Stack::new("unsub");

/// RFC 8058's body, exactly. The header promises the sender will honour this and nothing else.
const ONE_CLICK_BODY: &str = "List-Unsubscribe=One-Click";

// ---------------------------------------------------------------------------------------------
// Which of the three
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Path {
    /// A POST the app makes itself, which is a confirmation.
    OneClick { url: String },
    /// A message the app sends, which is also a confirmation once it goes.
    Mailto { mailto: String },
    /// A page for the browser, which is not.
    Link { url: String },
}

/// What this sender's mail offers, read from the newest message of theirs that carries the header.
/// The newest, because a list that moved to one-click last month should not be unsubscribed from
/// through the address it published a year ago.
pub fn path_for(conn: &Connection, address: &str) -> Result<Path, String> {
    let address = address.trim().to_lowercase();
    let held: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT list_unsubscribe, list_unsub_post FROM messages
              WHERE lower(from_address) = ?1
                AND list_unsubscribe IS NOT NULL AND list_unsubscribe <> ''
              ORDER BY date_ms DESC, id DESC LIMIT 1",
            [&address],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;

    let missing = || format!("{address} does not offer a way to unsubscribe");
    let (header, post) = held.ok_or_else(missing)?;
    let found = mirror::read::unsubscribe(header.as_deref(), post.as_deref()).ok_or_else(missing)?;

    // One click, then the message, then the page. The order is the order of how much of a
    // confirmation each one is.
    match (found.one_click, found.url, found.mailto) {
        (true, Some(url), _) if url.starts_with("https://") => Ok(Path::OneClick { url }),
        (_, _, Some(mailto)) => Ok(Path::Mailto { mailto }),
        (_, Some(url), _) => Ok(Path::Link { url }),
        _ => Err(missing()),
    }
}

// ---------------------------------------------------------------------------------------------
// The POST
// ---------------------------------------------------------------------------------------------

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .referer(false)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let started_at = attempt.previous().first().and_then(|url| url.host_str());
                match (started_at, attempt.url().host_str()) {
                    (Some(first), Some(now)) if first == now && attempt.previous().len() <= 3 => {
                        attempt.follow()
                    }
                    _ => attempt.stop(),
                }
            }))
            .build()
            .expect("could not build the unsubscribe client")
    })
}

async fn post(url: &str) -> Result<(), String> {
    let response = client()
        .post(url)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(ONE_CLICK_BODY)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    if status.is_redirection() {
        let elsewhere = response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| url::Url::parse(value).ok())
            .and_then(|url| url.host_str().map(str::to_string))
            .unwrap_or_else(|| "somewhere else".to_string());
        return Err(format!("the list answered by sending us to {elsewhere}"));
    }
    if !status.is_success() {
        return Err(format!("the list answered {}", status.as_u16()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The mailto
// ---------------------------------------------------------------------------------------------

/// The message a `mailto:` unsubscribe asks for. Queued rather than sent: the send pipeline is a
/// later milestone, and an outbox row is what it drains.
pub fn mailto_draft(account_id: &str, mailto: &str) -> Result<Draft, String> {
    let parsed = url::Url::parse(mailto).map_err(|e| e.to_string())?;
    let address = parsed.path().trim().to_string();
    if address.is_empty() {
        return Err("that unsubscribe address is empty".into());
    }
    let mut subject = "unsubscribe".to_string();
    let mut body = String::new();
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "subject" => subject = value.into_owned(),
            "body" => body = value.into_owned(),
            _ => {}
        }
    }
    Ok(Draft {
        id: None,
        account_id: account_id.to_string(),
        thread_key: None,
        in_reply_to: None,
        from_alias: None,
        to: vec![Person {
            name: None,
            address,
        }],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject,
        body_html: body,
        attachments: Vec::new(),
        remind_at_ms: None,
    })
}

fn queue_mailto(conn: &Connection, account_id: &str, mailto: &str) -> Result<(), String> {
    let draft = mailto_draft(account_id, mailto)?;
    mirror::write::enqueue(
        conn,
        &OutboxRow {
            op: mirror::write::OP_SEND.to_string(),
            payload: serde_json::to_string(&draft).map_err(|e| e.to_string())?,
            created_at: mirror::write::now_ms(),
            ..OutboxRow::default()
        },
    )
    .map(|_| ())
}

// ---------------------------------------------------------------------------------------------
// The two follow ups
// ---------------------------------------------------------------------------------------------

fn threads_from(conn: &Connection, address: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT thread_key FROM threads
              WHERE lower(from_address) = ?1 AND trashed = 0",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([address], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Trashing everything from a sender is an ordinary flag change: it lands locally and is pushed
/// behind, exactly as pressing `#` on each of them would be.
fn trash_everything(conn: &Connection, address: &str) -> Result<Vec<String>, String> {
    let keys = threads_from(conn, address)?;
    if keys.is_empty() {
        return Ok(keys);
    }
    let ids = mirror::write::message_ids_for_keys(conn, &keys)?;
    let patch = FlagPatch {
        trashed: Some(true),
        ..FlagPatch::default()
    };
    mirror::write::apply_flags(conn, &ids, &patch)?;
    outbox::queue_flags(conn, &ids, &patch, keys.first().map(String::as_str))?;
    Ok(keys)
}

fn untrash(conn: &Connection, keys: &[String]) -> Result<(), String> {
    let ids = mirror::write::message_ids_for_keys(conn, keys)?;
    if ids.is_empty() {
        return Ok(());
    }
    let patch = FlagPatch {
        trashed: Some(false),
        ..FlagPatch::default()
    };
    mirror::write::apply_flags(conn, &ids, &patch)?;
    outbox::queue_flags(conn, &ids, &patch, keys.first().map(String::as_str))
}

// ---------------------------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------------------------

/// The request itself cannot be taken back, which is the point of it. What can is everything the
/// two tick boxes did on this machine: the rule that screened them out, and the threads that went
/// to Trash.
pub struct UndoUnsubscribe {
    pub account_id: String,
    pub address: String,
    /// `None` when the rule was not touched, `Some(None)` when there was no rule before.
    pub rule_before: Option<Option<SenderRule>>,
    pub untrash: Vec<String>,
}

pub fn owns(token: &str) -> bool {
    UNDO.owns(token)
}

pub fn undo_apply(app: &tauri::AppHandle, token: &str) -> Result<(), String> {
    let entry = UNDO
        .take(token)
        .ok_or("that change can no longer be taken back")?;
    let db = db_of(app)?;
    db.with(&entry.account_id, |conn| {
        if let Some(rule) = &entry.rule_before {
            match rule {
                Some(rule) => state::write::set_rule(
                    conn,
                    &rule.subject,
                    rule.is_domain,
                    rule.destination,
                    rule.reason.as_deref(),
                )?,
                None => state::write::clear_rule(conn, &entry.address)?,
            }
        }
        untrash(conn, &entry.untrash)
    })?;
    crate::emit_store_changed(app, "threads state");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The command
// ---------------------------------------------------------------------------------------------

#[tauri::command]
pub async fn unsubscribe(
    app: tauri::AppHandle,
    account_id: String,
    address: String,
    also_trash: bool,
    also_screen_out: bool,
) -> Result<Undo, String> {
    let db = db_of(&app)?;
    let address = address.trim().to_lowercase();
    let path = db.with(&account_id, |conn| path_for(conn, &address))?;

    let said = match &path {
        Path::OneClick { url } => match post(url).await {
            Ok(()) => format!("Unsubscribed from {address}"),
            // A failed POST is not a failed command: the link is still there to be opened, and
            // saying which happened is the whole difference between the two paths.
            Err(why) => {
                let _ = app.opener().open_url(url.clone(), None::<&str>);
                format!("Could not confirm ({why}), so the page is open")
            }
        },
        Path::Mailto { mailto } => {
            db.with(&account_id, |conn| queue_mailto(conn, &account_id, mailto))?;
            format!("Unsubscribe message queued for {address}")
        }
        Path::Link { url } => {
            app.opener()
                .open_url(url.clone(), None::<&str>)
                .map_err(|e| e.to_string())?;
            format!("Opened the unsubscribe page for {address}")
        }
    };

    let (rule_before, trashed) = db.with(&account_id, |conn| {
        let rule_before = if also_screen_out {
            let before = state::read::rule_for(conn, &account_id, &address)?;
            state::write::set_rule(
                conn,
                &address,
                false,
                Destination::ScreenedOut,
                Some("Unsubscribed"),
            )?;
            Some(before)
        } else {
            None
        };
        let trashed = if also_trash {
            trash_everything(conn, &address)?
        } else {
            Vec::new()
        };
        Ok((rule_before, trashed))
    })?;

    let mut label = said;
    if !trashed.is_empty() {
        label.push_str(&format!(", and trashed {}", threads(trashed.len())));
    }
    if rule_before.is_some() {
        label.push_str(", and screened them out");
    }

    let token = UNDO.push(
        UndoUnsubscribe {
            account_id,
            address,
            rule_before,
            untrash: trashed,
        },
        &label,
    );
    crate::emit_store_changed(&app, "threads state");
    Ok(Undo {
        token,
        label,
        undo_ms: 0,
    })
}

fn threads(count: usize) -> String {
    if count == 1 {
        "1 thread".to_string()
    } else {
        format!("{count} threads")
    }
}

#[cfg(test)]
mod tests;
