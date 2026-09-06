// auth.rs     OAuth, token refresh. A loopback listener everywhere, a deep link when a phone has
//             been given its own OAuth client.
// browser.rs  the consent page in front of the app on a phone, and taking it away again
// secrets.rs  refresh tokens, encrypted on disk, same on every platform
//
// api.rs      typed Gmail REST wrapper, the quota accountant and the backoff schedule
// batch.rs    Gmail's multipart batch endpoint, by hand
// gmail.rs    impl Provider for Gmail
// people.rs   contacts, for the screening seed and autocomplete
// calendar.rs enough of the Calendar API to answer an invite
// drive.rs    the backup store's HTTP half
//
// The account registry auth.rs writes to is `crate::accounts`, which is a file rather than a
// table and knows nothing about Google. It owns `accounts_list`, `account_set_color` and
// `account_set_name`; the three commands here are the ones that talk to Google.

pub mod api;
pub mod auth;
pub mod batch;
#[cfg(mobile)]
pub mod browser;
pub mod calendar;
pub mod drive;
pub mod gmail;
pub mod people;
pub mod secrets;

pub use auth::AuthState;

/// Returns the consent URL. Completion arrives on the frontend as the `auth` event.
///
/// `login_hint` is the address typed on the connect screen, when there was one. Google then opens
/// on that account rather than on a chooser, which is what somebody who has just typed their
/// address expects to happen next.
#[tauri::command]
pub async fn account_connect(
    app: tauri::AppHandle,
    extra_scopes: Vec<String>,
    login_hint: Option<String>,
) -> Result<String, String> {
    auth::connect(app, extra_scopes, login_hint).await
}

/// The same, for an account that is already connected and needs a scope it was not granted. It is
/// the whole consent again, because Google offers installed apps nothing narrower.
#[tauri::command]
pub async fn account_grant(
    app: tauri::AppHandle,
    account_id: String,
    extra_scopes: Vec<String>,
) -> Result<String, String> {
    auth::grant(app, account_id, extra_scopes).await
}

/// Takes Margin's access away at Google and forgets the account here, in that order. One OAuth
/// client covers the whole suite, so the revoke signs the person out of every Margin app; the
/// settings screen says so before it offers the button. `keep_data` leaves the account's mail and
/// decisions on disk, set aside for the day it is added again.
#[tauri::command]
pub async fn account_remove(
    app: tauri::AppHandle,
    state: tauri::State<'_, AuthState>,
    account_id: String,
    keep_data: bool,
) -> Result<(), String> {
    auth::remove(&app, &state, &account_id, keep_data).await
}
