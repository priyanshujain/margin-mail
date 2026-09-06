// The macOS half of posting: UserNotifications, spoken to directly.
//
// `tauri-plugin-notification` goes through notify-rust and mac-notification-sys, which post with
// NSUserNotificationCenter, deprecated since 10.14. On macOS 26 that API still answers: the
// delegate is told the notification was delivered and `deliveredNotifications` counts it, and
// nothing appears, the app never shows up under System Settings > Notifications and the permission
// question is never asked. UNUserNotificationCenter is what the system listens to now, and it has
// two conditions of its own. The process has to be a bundled app with a real bundle signature: an
// ad-hoc one will do, the linker's own signature on the binary will not, which is what an unsigned
// `tauri build` leaves and why tauri.conf.json names a signing identity. And the app has to ask
// before it posts, which is a system dialog the first time and the remembered answer after that.
//
// A dev build is not a bundle, so here `request` answers with the system's own words: notifications
// are not allowed for this application. The log carries them; nothing else can.

use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread};
use objc2_foundation::{NSDictionary, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification,
    UNNotificationDefaultActionIdentifier, UNNotificationPresentationOptions,
    UNNotificationRequest, UNNotificationResponse, UNNotificationSetting, UNNotificationSettings,
    UNNotificationSound, UNShowPreviewsSetting, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};

use super::Text;
use crate::dto::{NotifyPermission, NotifyTarget};

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
static POSTED: AtomicU64 = AtomicU64::new(0);
static REPORTED: AtomicBool = AtomicBool::new(false);
/// The one key in a request's user info: the target, as JSON, so what a click reads back is the
/// same struct `post` was handed.
const TARGET_KEY: &str = "target";

define_class!(
    // SAFETY: NSObject has no subclassing requirements and `Delegate` has no `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "MarginMailNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        // Without this the system shows nothing while the app is frontmost, and the sample from
        // the settings screen is posted by a button in the app, which is frontmost by definition.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        // A click. The system activates the app on its own; this brings the window back, because
        // the red button hides it rather than closing it and AppKit will not show a hidden one.
        // Then the target the request carried goes to the front end, which opens the thread. Only
        // for the click itself: a notification dismissed from a category with a dismiss action
        // arrives here too, and dismissing is not asking to read.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &DynBlock<dyn Fn()>,
        ) {
            if let Some(app) = APP.get() {
                crate::show_main_window(app);
                // SAFETY: a constant the framework exports; reading it is the whole operation.
                let clicked = unsafe { UNNotificationDefaultActionIdentifier };
                if *response.actionIdentifier() == *clicked {
                    super::opened(app, target_of(response));
                }
            }
            completion.call(());
        }
    }
);

/// Installs the delegate. Once, from setup: the centre must know it before anything is posted,
/// and it is what a click on a notification comes back through.
pub fn install(app: tauri::AppHandle) {
    let _ = APP.set(app);
    let delegate: Retained<Delegate> = {
        let this = Delegate::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    };
    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // The centre holds its delegate weakly. This is the strong reference, for the life of the
    // process, which is exactly as long as the centre wants it.
    std::mem::forget(delegate);
}

/// What System Settings says, or Prompt when the app has never asked.
pub fn permission() -> Result<NotifyPermission, String> {
    status_of(&UNUserNotificationCenter::currentNotificationCenter())
}

/// What the system will do with a post: whether the app may post at all, whether a banner is on
/// for it, and whether the banner carries the text. The last two fail silently otherwise, as no
/// banner, or as one that reads "Notification" and nothing else.
struct Settings {
    status: UNAuthorizationStatus,
    alert: UNNotificationSetting,
    previews: UNShowPreviewsSetting,
}

fn settings_of(center: &UNUserNotificationCenter) -> Result<Settings, String> {
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        // SAFETY: the system hands the block a live settings object for the duration of the call.
        let settings = unsafe { settings.as_ref() };
        let _ = tx.send(Settings {
            status: settings.authorizationStatus(),
            alert: settings.alertSetting(),
            previews: settings.showPreviewsSetting(),
        });
    });
    center.getNotificationSettingsWithCompletionHandler(&block);
    rx.recv().map_err(|_| "macOS did not answer".to_string())
}

/// Once per process, on the first post: the system's own settings for the app, in the log,
/// because nothing on screen says why a banner did not come or came without its text.
fn report(settings: &Settings) {
    if REPORTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let alert = if settings.alert == UNNotificationSetting::Enabled {
        "on"
    } else if settings.alert == UNNotificationSetting::Disabled {
        "off"
    } else {
        "not supported"
    };
    let previews = if settings.previews == UNShowPreviewsSetting::Always {
        "always"
    } else if settings.previews == UNShowPreviewsSetting::WhenAuthenticated {
        "when unlocked"
    } else if settings.previews == UNShowPreviewsSetting::Never {
        "never"
    } else {
        "unknown"
    };
    crate::log::note(
        "notify",
        &format!("System Settings for this app: alerts {alert}, show previews {previews}"),
    );
}

fn status_of(center: &UNUserNotificationCenter) -> Result<NotifyPermission, String> {
    let status = settings_of(center)?.status;
    Ok(
        if status == UNAuthorizationStatus::Authorized || status == UNAuthorizationStatus::Provisional {
            NotifyPermission::Granted
        } else if status == UNAuthorizationStatus::Denied {
            NotifyPermission::Denied
        } else {
            NotifyPermission::Prompt
        },
    )
}

/// Asks, when the system has not been asked, and says what the answer is. The dialog is the
/// system's and this blocks until it is dismissed, so callers are off the main thread.
pub fn request() -> Result<NotifyPermission, String> {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let known = status_of(&center)?;
    if known != NotifyPermission::Prompt {
        return Ok(known);
    }
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |granted: Bool, error: *mut NSError| {
        let _ = tx.send((granted.as_bool(), describe(error)));
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &block,
    );
    let (granted, error) = rx.recv().map_err(|_| "macOS did not answer".to_string())?;
    if let Some(error) = error {
        return Err(error);
    }
    Ok(if granted {
        NotifyPermission::Granted
    } else {
        NotifyPermission::Denied
    })
}

/// One notification. Asks first when nobody has, so the first real mail is what raises the
/// question if the settings screen and the first run panel never did.
pub fn post(text: &Text) -> Result<(), String> {
    match request()? {
        NotifyPermission::Granted => {}
        NotifyPermission::Denied => {
            return Err("notifications are turned off for Margin Mail in System Settings".to_string())
        }
        NotifyPermission::Prompt => return Err("the system has not answered yet".to_string()),
    }
    report(&settings_of(&UNUserNotificationCenter::currentNotificationCenter())?);

    let content = UNMutableNotificationContent::new();

    content.setTitle(&NSString::from_str(&text.title));
    if !text.subtitle.is_empty() {
        content.setSubtitle(&NSString::from_str(&text.subtitle));
    }
    content.setBody(&NSString::from_str(&text.body));
    if let Some(json) = text.target.as_ref().and_then(|t| serde_json::to_string(t).ok()) {
        let value = NSString::from_str(&json);
        let info = NSDictionary::from_slices(&[&*NSString::from_str(TARGET_KEY)], &[&*value]);
        // SAFETY: a dictionary of strings is a dictionary; the system copies it and `target_of`
        // reads it back as the strings it was.
        let info: Retained<NSDictionary> = unsafe { Retained::cast_unchecked(info) };
        // SAFETY: the same: strings under string keys are what user info is for.
        unsafe { content.setUserInfo(&info) };
    }
    // The system's own sound, always: whether it is heard is the system's setting, in its pane.
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    let id = format!(
        "margin-mail-{}-{}",
        crate::mirror::write::now_ms(),
        POSTED.fetch_add(1, Ordering::Relaxed)
    );
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&id),
        &content,
        None,
    );

    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |error: *mut NSError| {
        let _ = tx.send(describe(error));
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&block));
    match rx.recv().map_err(|_| "macOS did not answer".to_string())? {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Opens System Settings on this app's row under Notifications, which is where a refusal lives.
pub fn open_settings() -> Result<(), String> {
    tauri_plugin_opener::open_url(
        "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id=studio.margin.mail",
        None::<&str>,
    )
    .map_err(|e| e.to_string())
}

/// The target `post` attached to the request, read back from a click on it. None for the sample.
fn target_of(response: &UNNotificationResponse) -> Option<NotifyTarget> {
    let info = response.notification().request().content().userInfo();
    let value = info.objectForKey(&*NSString::from_str(TARGET_KEY))?;
    let json = value.downcast::<NSString>().ok()?;
    serde_json::from_str(&json.to_string()).ok()
}

fn describe(error: *mut NSError) -> Option<String> {

    if error.is_null() {
        return None;
    }
    // SAFETY: a non-null error from the system is a live NSError for the duration of the callback.
    let error = unsafe { &*error };
    let text = error.localizedDescription().to_string();
    Some(if text.is_empty() || text == "(null)" {
        format!("UNErrorDomain code {}", error.code())
    } else {
        text
    })
}
