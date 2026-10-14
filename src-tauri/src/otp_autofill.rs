use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailCode {
    id: String,
    code: String,
    domain: String,
    label: String,
    expires_at_ms: i64,
}

#[derive(Serialize)]
pub struct AutoFillStatus {
    available: bool,
    enabled: bool,
}

#[tauri::command(async)]
pub fn otp_autofill_status() -> AutoFillStatus {
    AutoFillStatus {
        available: native::available(),
        enabled: native::enabled().unwrap_or(false),
    }
}

#[tauri::command]
pub async fn otp_autofill_enable(app: tauri::AppHandle) -> Result<bool, String> {
    if !native::available() {
        return Err("Email OTP AutoFill requires macOS 15 or later.".into());
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || native::enable(sender))
        .map_err(|error| error.to_string())?;
    receiver.await.map_err(|error| error.to_string())?
}

pub fn validate_enabled() -> Result<(), String> {
    if native::enabled()? {
        Ok(())
    } else {
        Err("Enable Margin Mail in macOS AutoFill & Passwords first.".into())
    }
}

pub fn clear() -> Result<(), String> {
    native::publish(&[], None)
}

pub fn code_in(subject: &str, body: &str) -> Option<String> {
    use std::sync::LazyLock;
    static CUE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
        r"(?i)\b(verification|authentication|log[ -]?in|sign[ -]?in|one[ -]?time|otp|passcode|code|pin)\b"
    ).unwrap()
    });
    static TOKEN: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\b(?:\d{3,4}[- ]\d{3,4}|[A-Z0-9]{4,8})\b").unwrap());
    static DURATION: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)^\s*(seconds?|minutes?|hours?|days?)\b").unwrap());
    static NOT_A_CODE_BEFORE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?i)(ending\s+(with|in)|card|account|a/c|x{2,}|\*{2,}|inr|rs\.?|usd|eur|amount|total|[$€£₹#]|order|invoice|ref(erence)?|ticket|tracking|number\s+is|call|phone|tel|(promo|coupon|discount|referral|gift)\s+code\s*(is|:)?)\s*$").unwrap()
    });
    static NOT_A_CODE_AFTER: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^[.,:]\d").unwrap());
    let (body, _) = crate::sanitize::quoted::split_text(body);
    let text = format!("{subject}\n{body}");
    static NOT_A_CUE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?i)(promo|coupon|discount|referral|gift|zip|postal|area|country|source|qr|bar)\s*$").unwrap()
    });
    let cues: Vec<(usize, usize)> = CUE
        .find_iter(&text)
        .filter(|cue| !NOT_A_CUE.is_match(&text[..cue.start()]))
        .map(|cue| (cue.start(), cue.end()))
        .collect();
    let mut best: Option<(usize, String)> = None;
    let mut tied = false;
    for token in TOKEN.find_iter(&text) {
        let value = token.as_str().replace(['-', ' '], "");
        let value = value.as_str();
        if !value.bytes().any(|byte| byte.is_ascii_digit())
            || (!value.bytes().all(|byte| byte.is_ascii_digit()) && value.len() < 6)
        {
            continue;
        }
        let before = &text[text.floor_char_boundary(token.start().saturating_sub(24))..token.start()];
        let after = &text[token.end()..];
        if DURATION.is_match(after) || NOT_A_CODE_BEFORE.is_match(before) || NOT_A_CODE_AFTER.is_match(after) {
            continue;
        }
        let Some(distance) = cues
            .iter()
            .map(|&(start, end)| {
                if end <= token.start() {
                    token.start() - end
                } else {
                    start.saturating_sub(token.end())
                }
            })
            .filter(|&distance| distance <= 90)
            .min()
        else {
            continue;
        };
        match &best {
            Some((closest, code)) if distance == *closest && code != value => tied = true,
            Some((closest, _)) if distance >= *closest => {}
            _ => {
                best = Some((distance, value.to_string()));
                tied = false;
            }
        }
    }
    if tied {
        None
    } else {
        best.map(|(_, code)| code)
    }
}

pub async fn refresh(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    if !native::available() {
        return Ok(());
    }
    if !crate::settings::load(app)?.otp_autofill_enabled || crate::app_lock::is_locked(app) {
        return Ok(());
    }
    let db = app.state::<crate::db::Db>();
    let now = crate::mirror::write::now_ms();
    let mut codes = Vec::new();
    for account_id in crate::sync::attached() {
        let rows = db.with(&account_id, |conn| {
            let mut query = conn
                .prepare(
                    "SELECT m.id, m.subject, m.from_address, m.from_name, m.date_ms, b.text
                 FROM messages m LEFT JOIN bodies b ON b.message_id = m.id
                 JOIN threads t ON t.provider_thread_id = m.provider_thread_id
                 WHERE m.date_ms > ?1 AND m.date_ms <= ?2 AND m.sent = 0
                 AND m.draft = 0 AND m.hydrated = 1 AND t.spam = 0 AND t.trashed = 0
                 ORDER BY m.date_ms DESC LIMIT 20",
                )
                .map_err(|error| error.to_string())?;
            let rows = query
                .query_map(rusqlite::params![now - 180_000, now], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                })
                .map_err(|error| error.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())
        })?;
        for (id, subject, sender, name, date, stored_body) in rows {
            let body = if let Some(body) = stored_body {
                body
            } else {
                let Some(remote) = crate::sync::remote_for(&account_id) else {
                    continue;
                };
                let raw = match remote.fetch_body(&id).await {
                    Ok(raw) => raw,
                    Err(_) => continue,
                };
                db.with(&account_id, |conn| {
                    let options = crate::sync::hydrate::render_options(conn)?;
                    crate::mirror::write::store_body(conn, &id, &raw, &options)?;
                    conn.query_row(
                        "SELECT text FROM bodies WHERE message_id = ?1",
                        [&id],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(|error| error.to_string())
                })?
            };
            let Some(code) = code_in(&subject, &body) else {
                continue;
            };
            let Some((_, domain)) = sender.rsplit_once('@') else {
                continue;
            };
            let domain = domain.to_lowercase();
            if !domain.contains('.')
                || domain.chars().any(|character| {
                    !(character.is_ascii_alphanumeric() || character == '.' || character == '-')
                })
            {
                continue;
            }
            codes.push(EmailCode {
                id: format!("{account_id}:{id}"),
                code,
                domain,
                label: name
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or(sender),
                expires_at_ms: date + 180_000,
            });
        }
    }
    if !crate::settings::load(app)?.otp_autofill_enabled || crate::app_lock::is_locked(app) {
        return clear();
    }
    codes.retain(|code| code.expires_at_ms > crate::mirror::write::now_ms());
    native::publish(&codes, Some(app))
}

#[cfg(target_os = "macos")]
mod native {
    use super::EmailCode;
    use block2::RcBlock;
    use objc2::{
        msg_send,
        rc::{Allocated, Retained},
        runtime::{AnyClass, Bool},
    };
    use objc2_foundation::{NSArray, NSError, NSFileManager, NSObject, NSString};
    use std::sync::{mpsc, Mutex};

    #[link(name = "AuthenticationServices", kind = "framework")]
    unsafe extern "C" {}
    static PUBLISH: Mutex<()> = Mutex::new(());

    pub fn available() -> bool {
        let bundled = std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.parent()?.parent().map(|path| {
                    path.join("PlugIns/MarginMailAutoFill.appex/Contents/embedded.provisionprofile")
                        .is_file()
                })
            })
            .unwrap_or(false);
        bundled
            && AnyClass::get(c"ASOneTimeCodeCredentialIdentity").is_some()
            && NSFileManager::defaultManager()
                .containerURLForSecurityApplicationGroupIdentifier(&NSString::from_str(
                    "TQV87WLXK3.studio.margin.mail",
                ))
                .is_some()
    }

    fn store() -> Result<Retained<NSObject>, String> {
        let class =
            AnyClass::get(c"ASCredentialIdentityStore").ok_or("macOS AutoFill is unavailable")?;
        Ok(unsafe { msg_send![class, sharedStore] })
    }

    pub fn enabled() -> Result<bool, String> {
        if !available() {
            return Ok(false);
        }
        let (sender, receiver) = mpsc::channel();
        let reply = RcBlock::new(move |state: *mut NSObject| {
            let enabled: Bool = unsafe { msg_send![state, isEnabled] };
            let _ = sender.send(enabled.as_bool());
        });
        unsafe {
            let _: () =
                msg_send![&*store()?, getCredentialIdentityStoreStateWithCompletion: &*reply];
        }
        receiver.recv().map_err(|error| error.to_string())
    }

    pub fn enable(sender: tokio::sync::oneshot::Sender<Result<bool, String>>) {
        let Some(class) = AnyClass::get(c"ASSettingsHelper") else {
            let _ = sender.send(Err("macOS AutoFill is unavailable".into()));
            return;
        };
        let sender = std::sync::Mutex::new(Some(sender));
        let reply = RcBlock::new(move |enabled: Bool| {
            if let Some(sender) = sender.lock().ok().and_then(|mut sender| sender.take()) {
                let _ = sender.send(Ok(enabled.as_bool()));
            }
        });
        unsafe {
            let _: () = msg_send![class, requestToTurnOnCredentialProviderExtensionWithCompletionHandler: &*reply];
        }
    }

    pub fn publish(codes: &[EmailCode], app: Option<&tauri::AppHandle>) -> Result<(), String> {
        if !available() {
            return Ok(());
        }
        let _guard = PUBLISH.lock().map_err(|error| error.to_string())?;
        let codes = if let Some(app) = app {
            if !crate::settings::load(app)?.otp_autofill_enabled || crate::app_lock::is_locked(app)
            {
                &[]
            } else {
                codes
            }
        } else {
            codes
        };
        let directory = NSFileManager::defaultManager()
            .containerURLForSecurityApplicationGroupIdentifier(&NSString::from_str(
                "TQV87WLXK3.studio.margin.mail",
            ))
            .and_then(|url| url.path())
            .ok_or("Margin Mail AutoFill signing is not configured")?;
        let path = std::path::PathBuf::from(directory.to_string());
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        let temporary = path.join("email-otp.pending");
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(&serde_json::to_vec(codes).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        std::fs::rename(&temporary, path.join("email-otp.json"))
            .map_err(|error| error.to_string())?;
        if !enabled()? {
            return Ok(());
        }
        let service_class = AnyClass::get(c"ASCredentialServiceIdentifier")
            .ok_or("macOS AutoFill is unavailable")?;
        let identity_class = AnyClass::get(c"ASOneTimeCodeCredentialIdentity")
            .ok_or("Email OTP AutoFill requires macOS 15")?;
        let mut identities: Vec<Retained<NSObject>> = Vec::new();
        for code in codes {
            let allocated: Allocated<NSObject> = unsafe { msg_send![service_class, alloc] };
            let service: Retained<NSObject> = unsafe {
                msg_send![allocated, initWithIdentifier: &*NSString::from_str(&code.domain), type: 0isize]
            };
            let allocated: Allocated<NSObject> = unsafe { msg_send![identity_class, alloc] };
            let identity = unsafe {
                msg_send![allocated, initWithServiceIdentifier: &*service, label: &*NSString::from_str(&code.label), recordIdentifier: &*NSString::from_str(&code.id)]
            };
            identities.push(identity);
        }
        let entries = NSArray::from_retained_slice(&identities);
        let (sender, receiver) = mpsc::channel();
        let reply = RcBlock::new(move |success: Bool, error: *mut NSError| {
            let result = if success.as_bool() {
                Ok(())
            } else if error.is_null() {
                Err("macOS could not update OTP suggestions".into())
            } else {
                Err(unsafe { &*error }.localizedDescription().to_string())
            };
            let _ = sender.send(result);
        });
        unsafe {
            let _: () = msg_send![&*store()?, replaceCredentialIdentityEntries: &*entries, completion: &*reply];
        }
        receiver.recv().map_err(|error| error.to_string())?
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use super::EmailCode;
    pub fn available() -> bool {
        false
    }
    pub fn enabled() -> Result<bool, String> {
        Ok(false)
    }
    pub fn publish(_: &[EmailCode], _: Option<&tauri::AppHandle>) -> Result<(), String> {
        Ok(())
    }
    pub fn enable(sender: tokio::sync::oneshot::Sender<Result<bool, String>>) {
        let _ = sender.send(Err("Email OTP AutoFill requires macOS".into()));
    }
}
