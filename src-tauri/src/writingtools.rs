#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2::runtime::AnyClass;
    use objc2::{msg_send, sel, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSMenu};

    pub fn available() -> bool {
        let Some(class) = AnyClass::get(c"NSWritingToolsCoordinator") else { return false };
        unsafe {
            let responds: bool = msg_send![class, respondsToSelector: sel!(isWritingToolsAvailable)];
            responds && msg_send![class, isWritingToolsAvailable]
        }
    }

    fn writing_menu(mtm: MainThreadMarker) -> Option<Retained<NSMenu>> {
        let main = NSApplication::sharedApplication(mtm).mainMenu()?;
        for item in main.itemArray().iter() {
            let Some(edit) = item.submenu() else { continue };
            if edit.title().to_string() != "Edit" && item.title().to_string() != "Edit" { continue; }
            for child in edit.itemArray().iter() {
                if child.title().to_string() == "Writing Tools" { return child.submenu(); }
            }
        }
        None
    }

    pub fn perform(tool: &str) -> Result<(), String> {
        let mtm = MainThreadMarker::new().ok_or("Writing Tools must run on the main thread")?;
        if !available() {
            return Err("Apple Writing Tools are unavailable. Check Apple Intelligence in System Settings.".into());
        }
        let menu = writing_menu(mtm).ok_or("Apple Writing Tools are unavailable in the app’s Edit menu")?;
        menu.update();
        for (index, item) in menu.itemArray().iter().enumerate() {
            if item.title().to_string() != tool { continue; }
            if !item.isEnabled() {
                return Err("Select text in your message before using Apple Writing Tools.".into());
            }
            let action = item.action().ok_or("The Writing Tools action is unavailable")?;
            let application = NSApplication::sharedApplication(mtm);
            if unsafe { application.targetForAction_to_from(action, item.target().as_deref(), Some(&item)) }.is_none() {
                return Err("Apple Writing Tools cannot edit the selected text.".into());
            }
            menu.performActionForItemAtIndex(index as isize);
            return Ok(());
        }
        Err(format!("Apple {tool} is unavailable in the app’s Edit menu"))
    }
}

#[tauri::command]
pub fn writing_tools_available() -> bool {
    #[cfg(target_os = "macos")]
    { mac::available() }
    #[cfg(not(target_os = "macos"))]
    { false }
}

#[tauri::command]
pub async fn run_writing_tool(app: tauri::AppHandle, tool: String) -> Result<(), String> {
    if !matches!(tool.as_str(), "Proofread" | "Rewrite") {
        return Err("Unknown Writing Tools action".into());
    }
    if !crate::settings::load(&app)?.writing_tools_enabled {
        return Err("Enable Apple Writing Tools in Writing settings first".into());
    }
    #[cfg(target_os = "macos")]
    {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || { let _ = sender.send(mac::perform(&tool)); })
            .map_err(|error| error.to_string())?;
        receiver.await.map_err(|error| error.to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    { Err("Apple Writing Tools require macOS".into()) }
}
