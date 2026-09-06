// The OAuth desktop flow, ported from Margin Calendar's `google/auth.rs`, which took it from
// margin's `gdrive.rs`. What is different here is scopes: the calendar asks for one and asks once,
// and this app asks for six, is told which of them it actually got, and has to be able to ask
// again for a seventh.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::LazyLock;
use std::time::Instant;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

use crate::accounts;
use crate::dto::{AccountKind, AuthEvent};
use crate::google::secrets;

/// What every connect asks for, in the order the consent screen lists them. The mirror of `SCOPES`
/// in `src/ipc.ts`, and the two files have to agree: the frontend renders the consent copy from
/// its copy of the list.
pub const BASE_SCOPES: [&str; 6] = [
    "openid",
    "email",
    "https://www.googleapis.com/auth/gmail.modify",
    "https://www.googleapis.com/auth/gmail.settings.basic",
    "https://www.googleapis.com/auth/contacts.readonly",
    "https://www.googleapis.com/auth/contacts.other.readonly",
];

/// Without this there is no app. Reading mail, marking it read and moving it are the whole of what
/// Margin Mail does, so an account that withheld it is refused rather than added in a state where
/// every screen is an error.
pub const REQUIRED_SCOPE: &str = "https://www.googleapis.com/auth/gmail.modify";

/// How long the listener waits for Google to come back. Two minutes is generous on a desktop, where
/// the browser is a window away and a keyboard is a keyboard.
#[cfg(desktop)]
pub const AUTH_TIMEOUT_SECS: u64 = 120;

/// The same wait on a phone, which has to cover an email and a password typed on glass, a password
/// manager round trip, and very often a 2FA prompt in a third app.
#[cfg(mobile)]
pub const AUTH_TIMEOUT_SECS: u64 = 900;

/// The same again for the deep link flow, where the wait is even less this app's to control: the
/// browser is a separate app there, so this process may be backgrounded for all of it while it does
/// nothing but hold a verifier.
#[cfg(mobile)]
pub const PENDING_TIMEOUT_SECS: u64 = 900;

/// Refresh this many seconds before Google would expire the token, so a request in flight when the
/// clock rolls over does not come back 401.
const EXPIRY_SKEW_SECS: u64 = 60;

const CREDENTIALS_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/google-credentials.json"));

pub static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        // The same keep-alive as the Gmail client, for the same reason: the refresh happens once
        // an hour, which is exactly the request that would otherwise land on a connection nobody
        // has checked since the last one.
        .http2_keep_alive_interval(Duration::from_secs(20))
        .http2_keep_alive_timeout(Duration::from_secs(10))
        .http2_keep_alive_while_idle(true)
        .tcp_keepalive(Duration::from_secs(30))
        .build()
        .expect("could not build the HTTP client")
});

/// Up to three OAuth clients, of which a build uses exactly one.
///
/// `installed` is a Desktop client: confidential, so it has a secret, and allowed to redirect to
/// loopback on any port without registering it first. `android` and `ios` are public clients with
/// no secret at all, and they redirect to a custom URI scheme instead.
///
/// A phone uses `installed` unless its own block is present, which is a deliberate choice and the
/// reason mobile sign-in needs no console work. `connect` says what the two flows cost.
#[derive(Deserialize)]
struct CredentialsFile {
    installed: Credentials,
    // One file on every platform and one block of it per build, so on the other four the field is
    // parsed and never read, which is the shape and not an oversight.
    #[serde(default)]
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    android: Option<Credentials>,
    #[serde(default)]
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    ios: Option<Credentials>,
}

#[derive(Deserialize)]
pub struct Credentials {
    pub client_id: String,
    /// Absent for Android and iOS clients. A public client has nothing to keep secret, so PKCE is
    /// the only thing standing between an intercepted code and a token, which is why the verifier
    /// is not optional anywhere in this file.
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default = "default_auth_uri")]
    pub auth_uri: String,
    #[serde(default = "default_token_uri")]
    pub token_uri: String,
    /// Mobile only, and only when Google's console shows something other than the default below.
    #[serde(default)]
    pub redirect_uri: Option<String>,
    /// Set at load time rather than read from the file: true when this client came out of the
    /// `android` or `ios` block. It is the only thing that decides which of the two mobile flows
    /// runs, so it travels with the client that forces the choice rather than being worked out
    /// again wherever the answer is needed.
    #[serde(skip)]
    pub platform_client: bool,
}

fn default_auth_uri() -> String {
    "https://accounts.google.com/o/oauth2/auth".to_string()
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".to_string()
}

/// The custom URI scheme an Android build redirects to. It is the package name, which is Google's
/// documented form for an Android client and, unlike the reversed client id, is known at build
/// time, so it can sit in AndroidManifest.xml rather than being pasted in per install.
#[cfg(target_os = "android")]
pub const ANDROID_REDIRECT: &str = "studio.margin.mail:/oauth2redirect";

/// iOS gets no such choice: Google requires the reversed client id, so the scheme is only knowable
/// once the client id is.
#[cfg(target_os = "ios")]
fn reversed_client_id(client_id: &str) -> String {
    let base = client_id
        .strip_suffix(".apps.googleusercontent.com")
        .unwrap_or(client_id);
    format!("com.googleusercontent.apps.{base}:/oauth2redirect")
}

const NOT_SET_UP: &str =
    "Margin Mail is not set up yet. Add a real OAuth client to google-credentials.json and rebuild.";

/// The one client this build signs in with. A missing platform block is not an error: it means the
/// desktop client and the loopback flow, which is what a phone gets until somebody decides
/// otherwise. Every caller has to agree on the answer, because a refresh token belongs to the
/// client that obtained it.
pub fn load_credentials() -> Result<Credentials, String> {
    let parsed: CredentialsFile = serde_json::from_str(CREDENTIALS_JSON)
        .map_err(|e| format!("invalid google-credentials.json: {e}"))?;

    #[cfg(target_os = "android")]
    let (mut creds, platform_client) = match parsed.android {
        Some(creds) => (creds, true),
        None => (parsed.installed, false),
    };
    #[cfg(target_os = "ios")]
    let (mut creds, platform_client) = match parsed.ios {
        Some(creds) => (creds, true),
        None => (parsed.installed, false),
    };
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let (mut creds, platform_client) = (parsed.installed, false);
    creds.platform_client = platform_client;

    if creds.client_id.starts_with("YOUR_CLIENT_ID")
        || creds
            .client_secret
            .as_deref()
            .is_some_and(|secret| secret.starts_with("YOUR_CLIENT_SECRET"))
    {
        return Err(NOT_SET_UP.to_string());
    }
    Ok(creds)
}

/// Where Google sends the browser back to on the custom scheme flow. The loopback flow binds a port
/// per attempt and works its own redirect out in `connect_by_loopback`, so this is only for the
/// platforms that have been given their own client.
#[cfg(mobile)]
pub fn redirect_uri(creds: &Credentials) -> String {
    if let Some(explicit) = &creds.redirect_uri {
        return explicit.clone();
    }
    #[cfg(target_os = "android")]
    {
        ANDROID_REDIRECT.to_string()
    }
    #[cfg(not(target_os = "android"))]
    {
        reversed_client_id(&creds.client_id)
    }
}

/// Reads the body to a String first so Google's error payload survives into the message.
pub async fn read_json<T: DeserializeOwned>(
    resp: reqwest::Response,
    context: &str,
) -> Result<T, String> {
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("{context} failed ({status}): {text}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("{context}: could not parse response: {e}"))
}

#[derive(Default)]
pub struct Session {
    pub access_token: Option<String>,
    pub access_expiry: u64,
    pub email: Option<String>,
}

/// A consent round trip that has left for the browser and not come back.
///
/// Desktop does not need this: the loopback listener holds the verifier on its own stack for the
/// two minutes it is alive. Mobile has no listener. The browser is a separate app, this process
/// may be backgrounded while the user consents, and the answer arrives later as a deep link with
/// nothing but a code and a state parameter, so the verifier has to be waiting for it here.
#[cfg(mobile)]
pub struct Pending {
    pub state: String,
    pub verifier: String,
    pub redirect: String,
    pub expires: u64,
}

/// One entry per connected account. The outer Mutex is tokio's, not std's, because
/// `valid_access_token` holds it across the refresh await to keep refresh single-flight.
#[derive(Default)]
pub struct AuthState {
    pub sessions: Mutex<HashMap<String, Session>>,
    #[cfg(mobile)]
    pub pending: Mutex<Option<Pending>>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn random_b64(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

fn pkce_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// The base list plus whatever this request needs on top, space separated for Google.
///
/// There is no such thing here as adding a scope to an existing grant. Installed apps get no
/// incremental authorization, so the extras are always asked for alongside everything else and the
/// token that comes back replaces the stored one. Sending only the extra would work exactly once,
/// and the new token would be missing every scope the app already had.
fn scope_string(extra: &[String]) -> String {
    let mut scopes: Vec<&str> = BASE_SCOPES.to_vec();
    for scope in extra {
        let scope = scope.trim();
        if !scope.is_empty() && !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    scopes.join(" ")
}

/// What the token response says was granted, which is not what was asked for. Granular consent
/// puts a tick box next to each scope and the user is entitled to clear any of them.
///
/// Google normalises as it goes: ask for `email` and the response says
/// `https://www.googleapis.com/auth/userinfo.email`. Stored as it arrives rather than translated
/// back, because this is a record of what Google said, and everything that has to check a scope
/// checks for one of the full URLs anyway.
fn granted_scopes(scope: Option<&str>) -> Vec<String> {
    scope
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

fn missing_required(granted: &[String]) -> Vec<String> {
    if granted.iter().any(|scope| scope == REQUIRED_SCOPE) {
        return Vec::new();
    }
    vec![REQUIRED_SCOPE.to_string()]
}

fn write_http_message(stream: &mut TcpStream, message: &str) {
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Margin Mail</title></head>\
         <body style=\"font-family:system-ui,sans-serif;text-align:center;padding-top:80px;color:#222\">\
         <h2>{message}</h2></body></html>"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

#[derive(Debug, PartialEq, Eq)]
enum Redirect {
    Code(String),
    Denied(String),
    Mismatch,
    /// Anything else the browser asked for on the way, including the favicon.
    Waiting,
}

fn request_path(request: &str) -> &str {
    request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("")
}

fn parse_redirect(path: &str, expected_state: &str) -> Redirect {
    if path == "/favicon.ico" {
        return Redirect::Waiting;
    }
    let parsed = match url::Url::parse(&format!("http://127.0.0.1{path}")) {
        Ok(parsed) => parsed,
        Err(_) => return Redirect::Waiting,
    };
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            _ => {}
        }
    }
    if let Some(error) = error {
        return Redirect::Denied(error);
    }
    match (code, state) {
        (Some(code), Some(state)) if state == expected_state => Redirect::Code(code),
        (Some(_), _) => Redirect::Mismatch,
        _ => Redirect::Waiting,
    }
}

/// What the frontend is told when the user backed out rather than finishing: closing the consent
/// browser on a phone, or pressing Cancel on Google's own screen anywhere.
///
/// `emit_auth` compares against this exact string to set `AuthEvent.cancelled`, which is what stops
/// the frontend reporting a change of mind as a failure. So it is a constant on every platform, and
/// every path that means "the user chose not to" must return this rather than wording its own.
const CANCELLED: &str = "Sign-in was cancelled.";

fn await_code(
    listener: TcpListener,
    expected_state: &str,
    deadline: Instant,
) -> Result<String, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    loop {
        if Instant::now() > deadline {
            return Err("Timed out waiting for Google authorization.".to_string());
        }
        // Closing the consent page is the mobile equivalent of closing the browser tab, and the
        // one abandonment the OS actually tells us about. Polled here rather than interrupting the
        // loop, so this stays the only place that decides an attempt is over.
        #[cfg(mobile)]
        if crate::google::browser::cancelled() {
            return Err(CANCELLED.to_string());
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_nonblocking(false).ok();
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                match parse_redirect(request_path(&request), expected_state) {
                    Redirect::Code(code) => {
                        write_http_message(
                            &mut stream,
                            "Connected to Margin Mail. You can close this tab.",
                        );
                        return Ok(code);
                    }
                    Redirect::Denied(error) => {
                        write_http_message(
                            &mut stream,
                            "Authorization was cancelled. You can close this tab.",
                        );
                        // `access_denied` is Google's word for the user pressing Cancel on the
                        // consent screen, which is the same decision as closing the browser and
                        // deserves the same quiet handling. Anything else really did go wrong.
                        if error == "access_denied" {
                            return Err(CANCELLED.to_string());
                        }
                        return Err(format!("Google authorization failed: {error}"));
                    }
                    Redirect::Mismatch => {
                        write_http_message(
                            &mut stream,
                            "Could not verify the request. You can close this tab.",
                        );
                        return Err("State mismatch during Google authorization.".to_string());
                    }
                    Redirect::Waiting => write_http_message(&mut stream, "Waiting for Google…"),
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(150));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    id_token: Option<String>,
    /// Space separated, and absent from a refresh response, which is why it is an Option and why
    /// the stored list is only ever written by `finish`.
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Deserialize)]
struct UserInfo {
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct IdClaims {
    /// Google's stable user id. It survives an email change, which the email does not.
    #[serde(default)]
    sub: Option<String>,
    #[serde(default)]
    email: Option<String>,
    /// Only ever present if somebody adds `profile` to the scope list, which nothing here does.
    #[serde(default)]
    name: Option<String>,
}

/// The id_token arrived over TLS straight from Google's token endpoint, so verifying its signature
/// would only re-prove what the transport already proved. Only the payload is read.
fn id_token_claims(id_token: &str) -> Option<IdClaims> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The name an account starts life with. `profile` is not in the scope list, so Google tells us
/// nothing but the address and the local part is the best guess available. It is a starting value,
/// not a fact: `account_set_name` exists because this will sometimes be wrong.
fn display_name(claims: &IdClaims, email: &str) -> String {
    match claims.name.as_deref() {
        Some(name) if !name.trim().is_empty() => name.trim().to_string(),
        _ => email.split('@').next().unwrap_or(email).to_string(),
    }
}

/// Google rejects an empty `client_secret` rather than ignoring it, so a public mobile client has
/// to omit the field entirely rather than send a blank one.
fn with_secret<'a>(
    creds: &'a Credentials,
    mut form: Vec<(&'a str, &'a str)>,
) -> Vec<(&'a str, &'a str)> {
    if let Some(secret) = creds.client_secret.as_deref() {
        form.push(("client_secret", secret));
    }
    form
}

/// The calendar reached for reqwest's `.form()`. reqwest 0.13 puts that behind a feature this
/// build does not turn on, and `url` is already here and encodes by the same rules, so the three
/// token requests below build their own body rather than the manifest growing a feature for it.
fn form_body(pairs: &[(&str, &str)]) -> String {
    let mut form = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        form.append_pair(key, value);
    }
    form.finish()
}

const FORM_TYPE: (&str, &str) = ("content-type", "application/x-www-form-urlencoded");

async fn exchange_code(
    creds: &Credentials,
    code: &str,
    redirect: &str,
    verifier: &str,
) -> Result<TokenResponse, String> {
    let form = with_secret(
        creds,
        vec![
            ("client_id", creds.client_id.as_str()),
            ("code", code),
            ("code_verifier", verifier),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect),
        ],
    );
    let resp = HTTP
        .post(&creds.token_uri)
        .header(FORM_TYPE.0, FORM_TYPE.1)
        .body(form_body(&form))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    read_json(resp, "Google token exchange").await
}

async fn refresh_access_token(
    creds: &Credentials,
    refresh_token: &str,
) -> Result<TokenResponse, String> {
    let form = with_secret(
        creds,
        vec![
            ("client_id", creds.client_id.as_str()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ],
    );
    let resp = HTTP
        .post(&creds.token_uri)
        .header(FORM_TYPE.0, FORM_TYPE.1)
        .body(form_body(&form))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    read_json(resp, "Google token refresh").await
}

async fn fetch_email(access_token: &str) -> Result<String, String> {
    let resp = HTTP
        .get("https://www.googleapis.com/oauth2/v2/userinfo")
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let info: UserInfo = read_json(resp, "Google account lookup").await?;
    Ok(info.email.unwrap_or_default())
}

/// Hands the grant back to Google, before the token is forgotten, so what is left on Google's
/// permissions page reflects what this machine actually holds.
///
/// This is the whole grant, for every Margin app, because they share one OAuth client and the
/// endpoint acts on the authorization behind a token rather than on the string it is handed;
/// nothing here can revoke less than that. Only `remove` calls it, and only after the user has
/// been told what it takes with it.
///
/// The answer is kept. A revoke that never reached Google leaves the grant standing, and a screen
/// that says it was revoked on the strength of a request that was merely sent is lying.
async fn revoke(token: &str) -> Result<(), String> {
    let resp = HTTP
        .post("https://oauth2.googleapis.com/revoke")
        .header(FORM_TYPE.0, FORM_TYPE.1)
        .body(form_body(&[("token", token)]))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    let text = resp.text().await.unwrap_or_default();
    Err(format!("Google answered {status}: {text}"))
}

/// The consent URL. Identical on every platform apart from the redirect it asks Google to come
/// back to, which is the whole of the difference between the desktop and mobile flows.
///
/// A hint is an address already known: an account that is connected and asking for another scope,
/// or the address somebody just typed on the connect screen. Either way the chooser is skipped and
/// Google goes straight to consent for that address. Without one the chooser is the point: Google
/// would otherwise silently reuse whoever is signed in.
fn auth_url(
    creds: &Credentials,
    redirect: &str,
    challenge: &str,
    csrf: &str,
    scopes: &str,
    hint: Option<&str>,
) -> String {
    let prompt = match hint {
        Some(_) => "consent",
        None => "select_account consent",
    };
    let mut url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}&code_challenge_method=S256&state={}&access_type=offline&prompt={}",
        creds.auth_uri,
        urlencode(&creds.client_id),
        urlencode(redirect),
        urlencode(scopes),
        challenge,
        urlencode(csrf),
        urlencode(prompt),
    );
    if let Some(hint) = hint {
        url.push_str(&format!("&login_hint={}", urlencode(hint)));
    }
    url
}

/// Hands the URL to whichever browser the OS considers the user's, in its own process. Never an
/// in-app webview: Google blocks the embedded-webview flow outright, and it deserves to be blocked,
/// because a webview the app controls can read what the user types into it. `browser.rs` covers the
/// mobile surfaces, which are the system's browser too and are only in front of the app rather than
/// inside it.
pub(super) fn open_in_browser(app: &tauri::AppHandle, url: &str) {
    use tauri_plugin_opener::OpenerExt;
    let _ = app.opener().open_url(url.to_string(), None::<&str>);
}

/// One finished consent. `missing_required` non-empty means nothing was stored: the token was
/// revoked again and there is no account, because the app cannot work without that scope.
struct Granted {
    account_id: String,
    email: String,
    scopes: Vec<String>,
    missing_required: Vec<String>,
}

fn emit_auth(app: &tauri::AppHandle, outcome: Result<Granted, String>) {
    let event = match outcome {
        Ok(granted) if granted.missing_required.is_empty() => {
            // The account is written and nothing more. The engine is handed it by `account_start`,
            // once the window it is to hold has been chosen on the panel that waits for it; the
            // registry alone is what a launch attaches, with the default window, if that never
            // happens. It used to be attached nowhere at all after a connect from Settings, and
            // sat on "Nothing here" until the next launch.
            crate::emit_store_changed(app, "accounts");
            AuthEvent {
                ok: true,
                error: None,
                account_id: Some(granted.account_id),
                email: Some(granted.email),
                cancelled: false,
                granted_scopes: granted.scopes,
                missing_required: Vec::new(),
            }
        }
        // No account id, because there is no account: the frontend has enough here to name the
        // address, name the permission and offer to try again.
        Ok(refused) => AuthEvent {
            ok: false,
            error: Some(format!(
                "{} was not added: Margin Mail cannot do anything without permission to read and change your mail.",
                refused.email
            )),
            account_id: None,
            email: Some(refused.email),
            cancelled: false,
            granted_scopes: refused.scopes,
            missing_required: refused.missing_required,
        },
        // Compared against the constant the cancel paths raise, rather than matched on its text,
        // so rewording it cannot quietly turn a cancel back into an error on screen.
        Err(error) => AuthEvent {
            ok: false,
            cancelled: error == CANCELLED,
            error: Some(error),
            account_id: None,
            email: None,
            granted_scopes: Vec::new(),
            missing_required: Vec::new(),
        },
    };
    let _ = app.emit("auth", event);
}

/// A new account. Returns the consent URL; the answer arrives later as the `auth` event.
///
/// The hint is the address typed on the connect screen, so the consent page opens on that account
/// rather than on the chooser. Anything that is not an address is treated as no hint at all.
pub async fn connect(
    app: tauri::AppHandle,
    extra_scopes: Vec<String>,
    hint: Option<String>,
) -> Result<String, String> {
    let hint = hint
        .map(|typed| typed.trim().to_string())
        .filter(|typed| typed.contains('@'));
    start(app, extra_scopes, hint).await
}

/// The same consent again for an account that is already connected, to pick up a scope that was
/// withheld or never asked for.
///
/// Before making this cleverer: there is no way to add `calendar.events` to a live token. Google
/// does not offer incremental authorization to installed apps, so the only thing that can be done
/// is the whole flow again with the base list plus the extras, replacing the stored token. That is
/// why answering one invite costs a trip through the consent screen, and why the button that leads
/// here says so first.
pub async fn grant(
    app: tauri::AppHandle,
    account_id: String,
    extra_scopes: Vec<String>,
) -> Result<String, String> {
    let entry = accounts::find(&app, &account_id)?
        .ok_or_else(|| format!("Account {account_id} is not connected."))?;
    start(app, extra_scopes, Some(entry.email)).await
}

/// One consent request, on all five platforms.
///
/// The loopback flow is the default everywhere, phones included. Google lets a Desktop client
/// redirect to loopback on any port with nothing registered in advance, and the token endpoint
/// checks the client id, the secret and the redirect and has no idea which OS asked. What used to
/// make this impossible on a phone was not the protocol but the browser: sending the user out to
/// Safari suspends this process, and a suspended process is not accepting on its socket. An in-app
/// browser does not leave, which is why `browser.rs` exists and why this branch is now the common
/// one.
///
/// The custom scheme flow runs instead when the credentials file has a client for this platform.
/// That is Google's stated guidance, and the thing to reach for if they ever start enforcing it,
/// but on iOS it is worth having for its own sake: it is the only way to reach
/// `ASWebAuthenticationSession`, which is the only iOS browser that shares Safari's cookies.
async fn start(
    app: tauri::AppHandle,
    extra_scopes: Vec<String>,
    hint: Option<String>,
) -> Result<String, String> {
    let creds = load_credentials()?;
    let scopes = scope_string(&extra_scopes);
    #[cfg(mobile)]
    if creds.platform_client {
        return connect_by_deep_link(app, creds, scopes, hint).await;
    }
    connect_by_loopback(app, creds, scopes, hint).await
}

async fn connect_by_loopback(
    app: tauri::AppHandle,
    creds: Credentials,
    scopes: String,
    hint: Option<String>,
) -> Result<String, String> {
    let verifier = random_b64(64);
    let challenge = pkce_challenge(&verifier);
    let csrf = random_b64(24);

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}");
    let url = auth_url(&creds, &redirect, &challenge, &csrf, &scopes, hint.as_deref());

    #[cfg(desktop)]
    open_in_browser(&app, &url);
    #[cfg(mobile)]
    crate::google::browser::open(&app, &url);

    let app_bg = app.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = complete_auth(&app_bg, listener, csrf, verifier, redirect, creds).await;
        emit_auth(&app_bg, outcome);
    });

    Ok(url)
}

/// The custom scheme flow. What is stashed here is the PKCE verifier and the CSRF state, which is
/// the only thing tying the code that comes back to the request that went out, since there is no
/// listener holding either on its own stack.
///
/// Where the answer comes back from differs by platform, and so does what it is worth.
///
/// iOS hands the URL to `ASWebAuthenticationSession`, which reports the callback straight to a
/// completion handler. It is the only iOS surface that shares Safari's cookies, so an account
/// already signed in on the phone is offered by name rather than asking for a password again. That
/// is the reason this flow is worth the console visit on iOS, and `browser.rs` says the rest.
///
/// Android opens the external browser and waits for the deep link, which is a genuinely separate
/// app: this process may be backgrounded, or killed outright, for the whole of it. Android needs
/// none of this, because a Custom Tab already shares Chrome's cookies and the loopback flow above
/// already works, so this is only ever reached when somebody has gone and made an Android client.
#[cfg(mobile)]
async fn connect_by_deep_link(
    app: tauri::AppHandle,
    creds: Credentials,
    scopes: String,
    hint: Option<String>,
) -> Result<String, String> {
    let verifier = random_b64(64);
    let challenge = pkce_challenge(&verifier);
    let csrf = random_b64(24);
    let redirect = redirect_uri(&creds);
    let url = auth_url(&creds, &redirect, &challenge, &csrf, &scopes, hint.as_deref());

    {
        let state = app.state::<AuthState>();
        let mut pending = state.pending.lock().await;
        *pending = Some(Pending {
            state: csrf,
            verifier,
            redirect: redirect.clone(),
            expires: now() + PENDING_TIMEOUT_SECS,
        });
    }

    #[cfg(target_os = "ios")]
    crate::google::browser::authenticate(&app, &url, callback_scheme(&redirect));
    #[cfg(not(target_os = "ios"))]
    open_in_browser(&app, &url);
    Ok(url)
}

/// `ASWebAuthenticationSession` matches on the scheme alone and wants it bare, so the path and the
/// colon that `redirect_uri` builds have to come back off.
#[cfg(target_os = "ios")]
fn callback_scheme(redirect: &str) -> &str {
    redirect.split(':').next().unwrap_or(redirect)
}

/// The consent surface ended with no callback URL at all: the user closed it, or it could not be
/// shown. Either way the pending verifier is spent, and the panel is waiting on an answer that is
/// never coming, so it is told. A cancel says so plainly rather than reporting a failure.
#[cfg(mobile)]
pub async fn abandon_pending(app: tauri::AppHandle, reason: Option<String>) {
    let auth = app.state::<AuthState>();
    let pending = {
        let mut slot = auth.pending.lock().await;
        slot.take()
    };
    if pending.is_some() {
        emit_auth(&app, Err(reason.unwrap_or_else(|| CANCELLED.to_string())));
    }
}

/// Every URL the OS hands the app on a registered scheme, including ones that have nothing to do
/// with consent. A link that carries no `state` we are waiting for is ignored in silence rather
/// than reported, because another feature may want that scheme later and a stray link is not an
/// authorization failure worth putting on screen.
#[cfg(mobile)]
pub async fn handle_redirect(app: tauri::AppHandle, incoming: &url::Url) {
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (key, value) in incoming.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            _ => {}
        }
    }
    let state = match state {
        Some(state) => state,
        None => return,
    };

    // Taken, not read: a code is good once, so leaving the verifier in place would let a replayed
    // link start a second exchange.
    let auth = app.state::<AuthState>();
    let pending = {
        let mut slot = auth.pending.lock().await;
        match slot.as_ref() {
            Some(p) if p.state == state => slot.take(),
            // A link whose state matches nothing is either stale or forged. Either way it is not
            // this app's pending request, so it is not this app's business.
            _ => return,
        }
    };
    let pending = match pending {
        Some(pending) => pending,
        None => return,
    };

    if let Some(error) = error {
        emit_auth(&app, Err(format!("Google authorization failed: {error}")));
        return;
    }
    if now() > pending.expires {
        emit_auth(
            &app,
            Err("The sign-in took too long. Try connecting again.".to_string()),
        );
        return;
    }
    let code = match code {
        Some(code) => code,
        None => {
            emit_auth(&app, Err("Google returned no authorization code.".to_string()));
            return;
        }
    };

    let outcome = match load_credentials() {
        Ok(creds) => finish(&app, &creds, &code, &pending.redirect, &pending.verifier).await,
        Err(e) => Err(e),
    };
    emit_auth(&app, outcome);
}

async fn complete_auth(
    app: &tauri::AppHandle,
    listener: TcpListener,
    csrf: String,
    verifier: String,
    redirect: String,
    creds: Credentials,
) -> Result<Granted, String> {
    let code = tauri::async_runtime::spawn_blocking(move || {
        let deadline = Instant::now() + Duration::from_secs(AUTH_TIMEOUT_SECS);
        await_code(listener, &csrf, deadline)
    })
    .await
    .map_err(|e| e.to_string())?;

    // Nothing takes the consent page down on its own once the listener has its answer, and what it
    // is showing by then is the listener's own "you can close this" page. Taken away here rather
    // than after the exchange, and whatever the outcome was, so the app comes back the moment the
    // browser has nothing left to do. Desktop has no such surface and compiles this out.
    #[cfg(mobile)]
    crate::google::browser::close(app);

    finish(app, &creds, &code?, &redirect, &verifier).await
}

/// Code to stored account. Everything past the point where the two flows stop differing.
async fn finish(
    app: &tauri::AppHandle,
    creds: &Credentials,
    code: &str,
    redirect: &str,
    verifier: &str,
) -> Result<Granted, String> {
    let tokens = exchange_code(creds, code, redirect, verifier).await?;
    let refresh = tokens
        .refresh_token
        .clone()
        .ok_or("Google did not return a refresh token. Remove Margin from third party access on your Google account, which signs out every Margin app, and connect again.")?;

    let claims = tokens
        .id_token
        .as_deref()
        .and_then(id_token_claims)
        .unwrap_or_default();
    let email = match claims.email.clone() {
        Some(email) if !email.is_empty() => email,
        _ => fetch_email(&tokens.access_token).await?,
    };
    // The Google user id keys everything. It falls back to the email only when the id_token is
    // missing, which should not happen while `openid` is in the scope set.
    let account_id = claims.sub.clone().unwrap_or_else(|| email.clone());
    if account_id.is_empty() {
        return Err("Google returned neither a user id nor an email address.".to_string());
    }

    let scopes = granted_scopes(tokens.scope.as_deref());
    let missing = missing_required(&scopes);
    if !missing.is_empty() {
        // The token is dropped without being stored, and deliberately without being revoked. One
        // OAuth client covers every Margin app and revocation acts on the authorization rather
        // than on the one string, so tidying up here could take the sibling apps down with it, or
        // this account's own working token when the refusal came from `grant` rather than
        // `connect`. A refresh token nobody kept is the cheaper thing to leave behind.
        return Ok(Granted {
            account_id,
            email,
            scopes,
            missing_required: missing,
        });
    }

    secrets::store(&account_id, &refresh)?;
    accounts::upsert(
        app,
        &account_id,
        &email,
        &display_name(&claims, &email),
        scopes.clone(),
    )?;

    let state = app.state::<AuthState>();
    let mut sessions = state.sessions.lock().await;
    sessions.insert(
        account_id.clone(),
        Session {
            access_token: Some(tokens.access_token),
            access_expiry: now() + tokens.expires_in.saturating_sub(EXPIRY_SKEW_SECS),
            email: Some(email.clone()),
        },
    );

    Ok(Granted {
        account_id,
        email,
        scopes,
        missing_required: Vec::new(),
    })
}

/// Revoke, forget, then let go of the databases: the one way an account leaves this device.
///
/// The grant goes back to Google first, while the token is still here to name it. It is the whole
/// grant, for every Margin app on every machine, because the suite shares one OAuth client and the
/// endpoint acts on the authorization behind a token rather than on the string it is handed; the
/// settings screen says so before it offers the button. An IMAP account has nothing at Google to
/// hand back, and its passwords are simply forgotten.
///
/// The order after that is the point. The registry entry goes before anything else touches disk,
/// so a crash halfway through leaves a database with no token, which `Db::on_disk` can find and
/// clear up, rather than a live token for an account the user has told us to forget.
///
/// The account goes from this device whether or not Google answered: the person asked for it to
/// go, and keeping a token here because Google was unreachable would be the opposite of what they
/// pressed. What is reported is whether Google heard about it, which is the half the button
/// promised and the half they cannot see for themselves.
///
/// `keep_data` leaves the mirror and the state database on disk, set aside where nothing lists
/// them, so the decisions come back if the account is ever added again.
pub async fn remove(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, AuthState>,
    account_id: &str,
    keep_data: bool,
) -> Result<(), String> {
    let kind = accounts::find(app, account_id)?.map(|entry| entry.kind);
    let revoked = match kind {
        Some(AccountKind::Imap) => Ok(()),
        _ => match secrets::load(account_id) {
            Ok(Some(refresh)) => revoke(&refresh).await,
            Ok(None) => Ok(()),
            Err(e) => Err(e),
        },
    };

    {
        let mut sessions = state.sessions.lock().await;
        sessions.remove(account_id);
    }
    crate::sync::detach(account_id);
    accounts::remove(app, account_id)?;
    // Both kinds of secret whatever kind the account was: a registry entry that had already gone
    // says nothing about which it held, and deleting a key that is not there is nothing.
    let forgotten =
        secrets::delete(account_id).and_then(|_| crate::imap::delete_passwords(account_id));

    // The mirror is derived and the state database is not, but both are this account's and the
    // person asked for the account to go. `Db` is absent in a test and in any build that has not
    // opened one yet, which is the only case where there is nothing on disk to remove.
    if let Some(db) = app.try_state::<crate::db::Db>() {
        db.close(account_id)?;
        if keep_data {
            db.set_aside(account_id)?;
        } else {
            let dir = db.account_dir(account_id);
            if dir.exists() {
                std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
            }
        }
    }
    crate::emit_store_changed(app, "accounts");
    forgotten?;
    revoked.map_err(|e| {
        format!(
            "The account was removed from this device, but Google could not be reached to revoke Margin's access, so the grant is still standing: {e}"
        )
    })
}

/// Single-flight: the map lock is held across the refresh await, so concurrent callers queue on
/// one token request rather than each firing their own. That serializes refreshes across accounts
/// as well, which is the right trade for something that happens once an hour per account.
pub async fn valid_access_token(
    _app: &tauri::AppHandle,
    state: &AuthState,
    account_id: &str,
) -> Result<String, String> {
    let mut sessions = state.sessions.lock().await;
    if let Some(session) = sessions.get(account_id) {
        if let Some(token) = &session.access_token {
            if now() < session.access_expiry {
                return Ok(token.clone());
            }
        }
    }

    let refresh = secrets::load(account_id)?
        .ok_or_else(|| format!("Account {account_id} is not connected to Google."))?;
    let creds = load_credentials()?;
    let tokens = refresh_access_token(&creds, &refresh).await?;
    if let Some(rotated) = &tokens.refresh_token {
        if rotated != &refresh {
            secrets::store(account_id, rotated)?;
        }
    }

    let session = sessions.entry(account_id.to_string()).or_default();
    session.access_token = Some(tokens.access_token.clone());
    session.access_expiry = now() + tokens.expires_in.saturating_sub(EXPIRY_SKEW_SECS);
    Ok(tokens.access_token)
}

/// The token store needs a directory and the sessions map wants the stored emails, both of which
/// are only knowable once the app is up.
pub fn init_sessions(app: &tauri::AppHandle) {
    if let Ok(dir) = crate::library::app_data_dir(app) {
        secrets::init(dir);
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let accounts = match accounts::list(&app) {
            Ok(accounts) => accounts,
            Err(_) => return,
        };
        let state = app.state::<AuthState>();
        let mut sessions = state.sessions.lock().await;
        for account in accounts {
            sessions.entry(account.id).or_default().email = Some(account.email);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credentials() -> Credentials {
        Credentials {
            client_id: "123.apps.googleusercontent.com".to_string(),
            client_secret: Some("secret".to_string()),
            auth_uri: default_auth_uri(),
            token_uri: default_token_uri(),
            redirect_uri: None,
            platform_client: false,
        }
    }

    fn query(url: &str, key: &str) -> Option<String> {
        url::Url::parse(url)
            .ok()?
            .query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
    }

    #[test]
    fn pkce_challenge_matches_the_rfc7636_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn a_verifier_is_url_safe_and_unpadded() {
        let verifier = random_b64(64);
        assert!(verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    /// A challenge is a hash, so it is safe in a query string as it stands, which is why
    /// `auth_url` is the one parameter it does not encode.
    #[test]
    fn a_challenge_is_url_safe_and_derived_from_the_verifier() {
        let verifier = random_b64(64);
        let challenge = pkce_challenge(&verifier);
        assert!(challenge
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_eq!(challenge, pkce_challenge(&verifier));
        assert_ne!(challenge, pkce_challenge(&random_b64(64)));
    }

    #[test]
    fn request_path_reads_the_target_of_the_request_line() {
        let request = "GET /?code=abc HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(request_path(request), "/?code=abc");
        assert_eq!(request_path(""), "");
    }

    #[test]
    fn a_matching_state_yields_the_code() {
        assert_eq!(
            parse_redirect("/?code=4%2F0Ab&state=xyz&scope=openid", "xyz"),
            Redirect::Code("4/0Ab".to_string())
        );
    }

    #[test]
    fn a_foreign_state_is_rejected_rather_than_exchanged() {
        assert_eq!(
            parse_redirect("/?code=4/0Ab&state=other", "xyz"),
            Redirect::Mismatch
        );
        assert_eq!(parse_redirect("/?code=4/0Ab", "xyz"), Redirect::Mismatch);
    }

    #[test]
    fn a_denial_carries_googles_reason() {
        assert_eq!(
            parse_redirect("/?error=access_denied&state=xyz", "xyz"),
            Redirect::Denied("access_denied".to_string())
        );
    }

    #[test]
    fn incidental_requests_keep_the_listener_waiting() {
        assert_eq!(parse_redirect("/favicon.ico", "xyz"), Redirect::Waiting);
        assert_eq!(parse_redirect("/", "xyz"), Redirect::Waiting);
        assert_eq!(parse_redirect("", "xyz"), Redirect::Waiting);
    }

    /// The state that goes out in the URL is the state that has to come back, encoded on the way
    /// out and decoded on the way in, or every sign-in is a mismatch.
    #[test]
    fn the_state_parameter_survives_the_round_trip() {
        let csrf = random_b64(24);
        let url = auth_url(
            &credentials(),
            "http://127.0.0.1:41234",
            &pkce_challenge(&random_b64(64)),
            &csrf,
            &scope_string(&[]),
            None,
        );
        let returned = query(&url, "state").expect("a state parameter");
        assert_eq!(returned, csrf);
        assert_eq!(
            parse_redirect(&format!("/?code=4/0Ab&state={returned}"), &csrf),
            Redirect::Code("4/0Ab".to_string())
        );
    }

    #[test]
    fn the_consent_url_carries_the_scopes_and_the_redirect() {
        let url = auth_url(
            &credentials(),
            "http://127.0.0.1:41234",
            "challenge",
            "csrf",
            &scope_string(&["https://www.googleapis.com/auth/drive.file".to_string()]),
            None,
        );
        assert_eq!(
            query(&url, "redirect_uri").as_deref(),
            Some("http://127.0.0.1:41234")
        );
        assert_eq!(query(&url, "code_challenge_method").as_deref(), Some("S256"));
        assert_eq!(query(&url, "access_type").as_deref(), Some("offline"));
        assert_eq!(query(&url, "prompt").as_deref(), Some("select_account consent"));
        assert!(query(&url, "login_hint").is_none());
        let scopes = query(&url, "scope").expect("a scope parameter");
        assert!(scopes.starts_with("openid email "));
        assert!(scopes.ends_with(" https://www.googleapis.com/auth/drive.file"));
    }

    /// Re-consent for an account that is already here goes straight to that account rather than
    /// the chooser, or granting one scope is an invitation to connect somebody else by accident.
    #[test]
    fn a_grant_names_the_account_it_is_for() {
        let url = auth_url(
            &credentials(),
            "http://127.0.0.1:41234",
            "challenge",
            "csrf",
            &scope_string(&[]),
            Some("someone@example.test"),
        );
        assert_eq!(
            query(&url, "login_hint").as_deref(),
            Some("someone@example.test")
        );
        assert_eq!(query(&url, "prompt").as_deref(), Some("consent"));
    }

    #[test]
    fn extra_scopes_are_asked_for_alongside_the_base_list_and_never_alone() {
        let scopes = scope_string(&["https://www.googleapis.com/auth/calendar.events".to_string()]);
        let asked: Vec<&str> = scopes.split(' ').collect();
        assert_eq!(&asked[..BASE_SCOPES.len()], &BASE_SCOPES[..]);
        assert_eq!(
            asked.last(),
            Some(&"https://www.googleapis.com/auth/calendar.events")
        );
    }

    #[test]
    fn an_extra_that_is_already_asked_for_is_not_asked_for_twice() {
        let scopes = scope_string(&[REQUIRED_SCOPE.to_string(), "  ".to_string()]);
        assert_eq!(scopes, BASE_SCOPES.join(" "));
    }

    #[test]
    fn the_granted_list_comes_from_the_token_response() {
        let response: TokenResponse = serde_json::from_str(
            r#"{"access_token":"at","expires_in":3599,"scope":"openid https://www.googleapis.com/auth/gmail.modify","token_type":"Bearer"}"#,
        )
        .expect("parse");
        assert_eq!(
            granted_scopes(response.scope.as_deref()),
            vec![
                "openid".to_string(),
                "https://www.googleapis.com/auth/gmail.modify".to_string()
            ]
        );
    }

    /// A refresh response carries no `scope` at all, and an empty list is not the same claim as
    /// "nothing was granted": only `finish` ever writes the stored list, from the exchange.
    #[test]
    fn a_token_response_without_a_scope_field_parses_to_nothing_granted() {
        let response: TokenResponse =
            serde_json::from_str(r#"{"access_token":"at","expires_in":3599}"#).expect("parse");
        assert!(response.scope.is_none());
        assert!(granted_scopes(response.scope.as_deref()).is_empty());
        assert!(granted_scopes(Some("   ")).is_empty());
    }

    #[test]
    fn an_account_without_the_gmail_scope_is_refused_and_says_which_one() {
        let granted = granted_scopes(Some(
            "openid https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/contacts.readonly",
        ));
        assert_eq!(missing_required(&granted), vec![REQUIRED_SCOPE.to_string()]);
        assert!(missing_required(&granted_scopes(None)).len() == 1);
    }

    #[test]
    fn the_gmail_scope_is_the_only_one_an_account_is_refused_for() {
        let granted = granted_scopes(Some(REQUIRED_SCOPE));
        assert!(missing_required(&granted).is_empty());
    }

    #[test]
    fn id_token_claims_reads_the_subject_and_email() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"sub":"11829","email":"a@b.test","aud":"x"}"#);
        let claims = id_token_claims(&format!("header.{payload}.signature")).expect("claims");
        assert_eq!(claims.sub.as_deref(), Some("11829"));
        assert_eq!(claims.email.as_deref(), Some("a@b.test"));
    }

    #[test]
    fn a_malformed_id_token_is_none_rather_than_a_panic() {
        assert!(id_token_claims("not-a-jwt").is_none());
        assert!(id_token_claims("header..signature").is_none());
    }

    #[test]
    fn a_name_falls_back_to_the_address_because_profile_is_not_asked_for() {
        let claims = IdClaims::default();
        assert_eq!(display_name(&claims, "sam.reed@example.test"), "sam.reed");
        let named = IdClaims {
            name: Some("Sam Reed".to_string()),
            ..IdClaims::default()
        };
        assert_eq!(display_name(&named, "sam.reed@example.test"), "Sam Reed");
    }
}
