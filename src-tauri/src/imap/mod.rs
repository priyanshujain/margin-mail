// The second provider: a mailbox reached over IMAP and SMTP with a password.
//
// Everything above `provider::Provider` is already written against the trait, so this module adds
// a mailbox rather than a mode. The parts that genuinely differ from Gmail are all here: there is
// no history log so the cursor is per folder, there are no labels so folders stand in for them,
// and there is no server-side notion of "archive" so one is found or made.
//
// The split between the files is the split between the jobs. `discover` turns an address into a
// pair of servers and never opens a mail connection. `tls` owns every socket and the single
// decision about a certificate. `session` is one authenticated IMAP connection. `folders` is the
// mapping from what a server calls its mailboxes onto the six roles the app needs. `provider` is
// the trait implementation, and `smtp` is the sending half.
//
// The passwords are sealed in the same blob as the Google refresh tokens, under a suffixed key.
// There is one sealed store in this app and adding a second would mean a second thing to get
// wrong when somebody restores a backup.

pub mod discover;
pub mod folders;
pub mod provider;
pub mod session;
pub mod smtp;
pub mod tls;

use std::path::PathBuf;
use std::sync::OnceLock;

use crate::dto::{ConnectReport, MailConfig, ServerConfig};
use crate::google::secrets;
use tls::Refused;

/// Which half of an account a password belongs to. SMTP is stored separately because a gateway
/// that wants different credentials from the mail store is common enough that Mailspring's manual
/// sheet does not even require an SMTP username.
pub const IMAP_KEY: &str = "imap";
pub const SMTP_KEY: &str = "smtp";

fn secret_key(account_id: &str, leg: &str) -> String {
    format!("{account_id}#{leg}")
}

pub fn store_password(account_id: &str, leg: &str, password: &str) -> Result<(), String> {
    secrets::store(&secret_key(account_id, leg), password)
}

pub fn load_password(account_id: &str, leg: &str) -> Result<Option<String>, String> {
    secrets::load(&secret_key(account_id, leg))
}

pub fn delete_passwords(account_id: &str) -> Result<(), String> {
    secrets::delete(&secret_key(account_id, IMAP_KEY))?;
    secrets::delete(&secret_key(account_id, SMTP_KEY))
}

// ---------------------------------------------------------------------------------------------
// The trust store on disk
// ---------------------------------------------------------------------------------------------

static TRUST_PATH: OnceLock<PathBuf> = OnceLock::new();

const TRUST_FILE: &str = "imap-trust.json";

/// Reads the accepted fingerprints back into `tls`. Called from `setup`, next to `secrets::init`,
/// because a decision somebody made last week has to survive a restart or they make it again every
/// week and stop reading it.
pub fn init(dir: PathBuf) {
    let path = dir.join(TRUST_FILE);
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, String>>(&text) {
            for (target, fingerprint) in map {
                if let Some((host, port)) = split_target(&target) {
                    tls::remember(&host, port, &fingerprint);
                }
            }
        }
    }
    let _ = TRUST_PATH.set(path);
}

fn split_target(target: &str) -> Option<(String, u16)> {
    let (host, port) = target.rsplit_once(':')?;
    Some((host.to_string(), port.parse().ok()?))
}

fn save_trust() -> Result<(), String> {
    let path = match TRUST_PATH.get() {
        Some(path) => path,
        None => return Ok(()),
    };
    let map: std::collections::HashMap<String, String> = tls::all_accepted().into_iter().collect();
    let text = serde_json::to_string_pretty(&map).map_err(|e| e.to_string())?;
    crate::library::atomic_write(path, text.as_bytes())
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

/// An address in, a pair of servers out, or nothing and the manual sheet is the next screen.
///
/// What comes back is what the provider published, `OAuth2` included. Gmail and Fastmail both say
/// OAuth2 first, and saying so is the truthful answer: it is how those accounts are meant to be
/// signed in to. The screen is what decides what to do about it, because the right answer differs.
/// A Google address belongs on the Google button one screen back. A Fastmail address belongs on an
/// app password, which the same document offers as its second choice.
#[tauri::command]
pub async fn imap_discover(email: String) -> Result<Option<MailConfig>, String> {
    Ok(discover::discover(&email).await)
}

/// A configuration with the password mechanism named, whatever it arrived saying.
///
/// There is no OAuth IMAP session in this app: OAuth over IMAP needs a client registered with that
/// provider, and the one Google client this suite shares is for the API rather than for `AUTH
/// XOAUTH2`. So a configuration reaching a connection has to name something that exists. A
/// provider offering only OAuth refuses the password and says so, which is a better failure than a
/// mechanism nothing implements silently doing nothing.
fn with_password_auth(mut config: MailConfig) -> MailConfig {
    config.imap.auth = crate::dto::AuthKind::Password;
    config.smtp.auth = crate::dto::AuthKind::Password;
    config
}

/// Opens both connections and logs in to each, without keeping either.
///
/// Not a `Result`: the interesting outcomes here are all things the screen draws. A wrong
/// password, a certificate to decide about and a host that does not answer are three different
/// panels, and collapsing them into one error string would mean parsing it back apart.
#[tauri::command]
pub async fn imap_test(
    config: MailConfig,
    imap_password: String,
    smtp_password: Option<String>,
) -> Result<ConnectReport, String> {
    let config = with_password_auth(config);
    if let Err(refused) = session::check(&config.imap, &imap_password).await {
        return Ok(report("imap", refused));
    }
    let smtp_password = smtp_password.unwrap_or_else(|| imap_password.clone());
    if let Err(refused) = smtp::check(&config.smtp, &smtp_password).await {
        return Ok(report("smtp", refused));
    }
    Ok(ConnectReport {
        ok: true,
        ..ConnectReport::default()
    })
}

fn report(leg: &str, refused: Refused) -> ConnectReport {
    let cert = match &refused {
        Refused::Certificate(question) => Some(question.clone()),
        _ => None,
    };
    ConnectReport {
        ok: false,
        kind: Some(kind_of(&refused).to_string()),
        failed: Some(leg.to_string()),
        message: Some(refused.to_string()),
        advice: advice_for(&refused),
        cert,
    }
}

/// The refusal, named. `Refused` already knows which of these it is, so nothing downstream should
/// have to work it out again from the sentence.
fn kind_of(refused: &Refused) -> &'static str {
    match refused {
        Refused::Unreachable(_) => "unreachable",
        Refused::Certificate(_) => "certificate",
        Refused::WrongHost { .. } => "wrong-host",
        Refused::Auth(_) => "auth",
        Refused::Other(_) => "other",
    }
}

/// A sentence that turns a refusal into something to go and do, when the server said enough to
/// know what that is. Several providers refuse an ordinary password with a URL in the message.
fn advice_for(refused: &Refused) -> Option<String> {
    let Refused::Auth(said) = refused else {
        return None;
    };
    let lower = said.to_lowercase();
    if lower.contains("application-specific")
        || lower.contains("app password")
        || lower.contains("app-specific")
    {
        return Some(
            "This account wants an app password rather than the one you sign in with.".to_string(),
        );
    }
    if lower.contains("bridge") {
        return Some(
            "Proton accounts connect through Bridge, using the password Bridge shows you."
                .to_string(),
        );
    }
    None
}

/// Accepts a certificate for one host and port, and remembers it across restarts.
#[tauri::command]
pub async fn imap_trust_cert(
    host: String,
    port: u16,
    fingerprint: String,
) -> Result<(), String> {
    tls::remember(&host, port, &fingerprint);
    save_trust()
}

#[tauri::command]
pub async fn imap_forget_cert(host: String, port: u16) -> Result<(), String> {
    tls::forget(&host, port);
    save_trust()
}

/// Adds the account. The configuration has already been tested by the screen that calls this, so
/// a failure here is a failure to write rather than a failure to connect.
#[tauri::command]
pub async fn imap_connect(
    app: tauri::AppHandle,
    email: String,
    name: String,
    config: MailConfig,
    imap_password: String,
    smtp_password: Option<String>,
) -> Result<crate::dto::Account, String> {
    // The address is the identity. Gmail hands back a stable numeric id and IMAP has nothing like
    // it, so the account id is the address itself, lowercased. Two accounts on the same address
    // are the same account.
    let id = email.trim().to_lowercase();
    if id.is_empty() {
        return Err("An address is needed.".to_string());
    }

    let config = with_password_auth(config);
    store_password(&id, IMAP_KEY, &imap_password)?;
    store_password(
        &id,
        SMTP_KEY,
        smtp_password.as_deref().unwrap_or(&imap_password),
    )?;
    crate::accounts::upsert_imap(&app, &id, &id, name.trim(), config)?;

    // Written and not yet syncing: `account_start` hands it to the engine once the window it is
    // to hold has been chosen, the same as a Google account.
    crate::accounts::list(&app)?
        .into_iter()
        .find(|account| account.id == id)
        .ok_or_else(|| "The account was written but could not be read back.".to_string())
}

/// The servers an account was set up with, for the Settings section that shows them.
#[tauri::command]
pub async fn imap_servers(
    app: tauri::AppHandle,
    account_id: String,
) -> Result<Option<MailConfig>, String> {
    Ok(crate::accounts::find(&app, &account_id)?.and_then(|entry| entry.servers))
}

/// The default port for a security, which is what the manual sheet fills in when somebody changes
/// the dropdown. Both halves agree on the shape and disagree on the numbers.
pub fn default_port(leg: &str, security: crate::dto::Security) -> u16 {
    use crate::dto::Security::*;
    match (leg, security) {
        (IMAP_KEY, Tls) => 993,
        (IMAP_KEY, _) => 143,
        (_, Tls) => 465,
        (_, StartTls) => 587,
        (_, Plain) => 25,
    }
}

/// A server as a config with nothing discovered, which is what the manual sheet starts from.
pub fn blank(leg: &str, host: &str, username: &str) -> ServerConfig {
    let security = crate::dto::Security::Tls;
    ServerConfig {
        host: host.to_string(),
        port: default_port(leg, security),
        security,
        auth: crate::dto::AuthKind::Password,
        username: username.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::Security;

    #[test]
    fn a_configuration_reaching_a_connection_always_names_a_mechanism_that_exists() {
        let oauth = ServerConfig {
            host: "imap.gmail.com".into(),
            port: 993,
            security: Security::Tls,
            auth: crate::dto::AuthKind::OAuth2,
            username: "someone@gmail.com".into(),
        };
        let config = with_password_auth(MailConfig {
            imap: oauth.clone(),
            smtp: oauth,
            source: "ispdb".into(),
            display_name: None,
        });
        assert_eq!(config.imap.auth, crate::dto::AuthKind::Password);
        assert_eq!(config.smtp.auth, crate::dto::AuthKind::Password);
        // Everything else the provider published survives, because only the mechanism was wrong.
        assert_eq!(config.imap.host, "imap.gmail.com");
        assert_eq!(config.source, "ispdb");
    }

    #[test]
    fn the_default_port_follows_the_security_for_each_half() {
        assert_eq!(default_port(IMAP_KEY, Security::Tls), 993);
        assert_eq!(default_port(IMAP_KEY, Security::StartTls), 143);
        assert_eq!(default_port(IMAP_KEY, Security::Plain), 143);
        assert_eq!(default_port(SMTP_KEY, Security::Tls), 465);
        assert_eq!(default_port(SMTP_KEY, Security::StartTls), 587);
        assert_eq!(default_port(SMTP_KEY, Security::Plain), 25);
    }

    #[test]
    fn a_target_splits_on_the_last_colon_so_an_address_with_colons_survives() {
        assert_eq!(
            split_target("mail.example.com:993"),
            Some(("mail.example.com".to_string(), 993))
        );
        assert_eq!(split_target("::1:1143"), Some(("::1".to_string(), 1143)));
        assert_eq!(split_target("mail.example.com"), None);
        assert_eq!(split_target("mail.example.com:imap"), None);
    }

    #[test]
    fn every_refusal_names_its_kind_so_the_screen_never_reads_it_out_of_the_sentence() {
        let cases: [(Refused, &str); 4] = [
            (Refused::Unreachable("no route".into()), "unreachable"),
            (Refused::Auth("bad password".into()), "auth"),
            (
                Refused::WrongHost {
                    expected: "a".into(),
                    found: "b".into(),
                },
                "wrong-host",
            ),
            (Refused::Other("something".into()), "other"),
        ];
        for (refused, expected) in cases {
            let out = report("imap", refused);
            assert_eq!(out.kind.as_deref(), Some(expected));
            assert!(!out.ok);
            assert_eq!(out.failed.as_deref(), Some("imap"));
        }
    }

    #[test]
    fn the_advice_only_speaks_when_the_server_said_something_actionable() {
        assert!(advice_for(&Refused::Auth("Invalid credentials".into())).is_none());
        assert!(advice_for(&Refused::Unreachable("no route".into())).is_none());
        assert!(advice_for(&Refused::Auth(
            "Application-specific password required".into()
        ))
        .is_some());
    }
}
