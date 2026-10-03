use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::Manager;

pub struct AppLock {
    enabled: AtomicBool,
    unlocked: AtomicBool,
}

impl AppLock {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled: AtomicBool::new(enabled),
            unlocked: AtomicBool::new(!enabled),
        }
    }

    pub fn configure(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
    }

    pub fn is_locked(&self) -> bool {
        self.enabled.load(Ordering::Acquire) && !self.unlocked.load(Ordering::Acquire)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockStatus {
    enabled: bool,
    locked: bool,
    available: bool,
    unavailable_reason: Option<String>,
}

pub fn permits<R: tauri::Runtime>(invoke: &tauri::ipc::Invoke<R>) -> bool {
    matches!(invoke.message.command(), "app_lock_status" | "app_unlock")
        || !invoke.message.state_ref().get::<AppLock>().is_locked()
}

pub fn is_locked(app: &tauri::AppHandle) -> bool {
    app.try_state::<AppLock>()
        .map(|lock| lock.is_locked())
        .unwrap_or(true)
}

pub fn validate_setting_change(before: bool, after: bool) -> Result<(), String> {
    if before != after {
        authenticate(if after {
            "Authenticate to require unlocking your mailbox when opening the app."
        } else {
            "Authenticate to turn off the mailbox opening lock."
        })?;
    }
    Ok(())
}

#[tauri::command(async)]
pub fn app_lock_status(app: tauri::AppHandle) -> LockStatus {
    let lock = app.state::<AppLock>();
    let unavailable_reason = availability().err();
    LockStatus {
        enabled: lock.enabled.load(Ordering::Acquire),
        locked: lock.is_locked(),
        available: unavailable_reason.is_none(),
        unavailable_reason,
    }
}

#[tauri::command(async)]
pub fn app_unlock(app: tauri::AppHandle) -> Result<(), String> {
    let lock = app.state::<AppLock>();
    if lock.is_locked() {
        authenticate("Unlock your mailbox to read and write email.")?;
        lock.unlocked.store(true, Ordering::Release);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn availability() -> Result<(), String> {
    use objc2_local_authentication::{LAContext, LAPolicy};

    let context = unsafe { LAContext::new() };
    unsafe { context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthentication) }
        .map_err(|error| error.localizedDescription().to_string())
}

#[cfg(target_os = "macos")]
fn authenticate(reason: &str) -> Result<(), String> {
    use std::sync::mpsc;

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAPolicy};

    let context = unsafe { LAContext::new() };
    unsafe { context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthentication) }
        .map_err(|error| error.localizedDescription().to_string())?;
    let (sender, receiver) = mpsc::channel();
    let reply = RcBlock::new(move |success: Bool, error: *mut NSError| {
        let result = if success.as_bool() {
            Ok(())
        } else {
            let message = if error.is_null() {
                "Authentication was not completed.".to_string()
            } else {
                unsafe { &*error }.localizedDescription().to_string()
            };
            Err(message)
        };
        let _ = sender.send(result);
    });
    unsafe {
        context.evaluatePolicy_localizedReason_reply(
            LAPolicy::DeviceOwnerAuthentication,
            &NSString::from_str(reason),
            &reply,
        );
    }
    receiver
        .recv()
        .map_err(|_| "macOS did not complete authentication.".to_string())?
}

#[cfg(not(target_os = "macos"))]
fn availability() -> Result<(), String> {
    Err("The app opening lock is currently available on macOS.".to_string())
}

#[cfg(not(target_os = "macos"))]
fn authenticate(_reason: &str) -> Result<(), String> {
    availability()
}
