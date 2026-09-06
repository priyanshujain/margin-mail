// `impl BackupStore for Drive`: three operations onto the folder half of `google::drive`.
//
// The folder is `margin/mail/<account-hash>/<device-id>/`, inside the same visible `margin` folder
// the other two apps in the suite write to. That is the shape the architecture settled on and the
// reason it matters is worth repeating here: margin holds `drive.file` and writes into an ordinary
// folder rather than the hidden app data space, so a person can open their Drive, see their backup
// and delete it without asking anyone. Mail keeps the scope, adds none, and puts encrypted segments
// under it.
//
// `drive.file` also means this app can only see files it created, so listing a folder returns
// Margin's own files and nothing else. That is what makes a bare "everything in this folder" query
// safe here when it would be a privacy problem under a wider scope.
//
// Drive has no paths, only folders with parents, so a name is walked segment by segment. The walk
// creates what is missing, on a read as well as a write, because the first thing a fresh device
// does is list a folder that does not exist yet and the answer to that is an empty folder rather
// than an error.

use std::future::Future;

use tauri::Manager;

use crate::accounts;
use crate::google::auth::{self, AuthState};
use crate::google::drive as gdrive;

use super::store::BackupStore;

pub struct Drive {
    app: tauri::AppHandle,
    account_id: String,
}

impl Drive {
    pub fn new(app: tauri::AppHandle, account_id: impl Into<String>) -> Drive {
        Drive {
            app,
            account_id: account_id.into(),
        }
    }

    /// The scope is checked before the token is asked for, so a cleared tick box on the consent
    /// page reads as the sentence naming that tick box rather than as a 403 from Drive.
    async fn token(&self) -> Result<String, String> {
        let granted = accounts::granted_scopes(&self.app, &self.account_id)?;
        if !gdrive::has_scope(&granted) {
            return Err(gdrive::REAUTH_MESSAGE.to_string());
        }
        let state = self
            .app
            .try_state::<AuthState>()
            .ok_or_else(|| "this app has no Google session store".to_string())?;
        auth::valid_access_token(&self.app, &state, &self.account_id).await
    }
}

/// A name is `<account-hash>/<device-id>/<file>`, which is the folders it lives under and the file
/// itself. Anything else is a caller bug rather than a user's problem, so it says so plainly.
fn split(name: &str) -> Result<(&str, &str, &str), String> {
    let mut parts = name.split('/');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(account), Some(device), Some(file), None)
            if !account.is_empty() && !device.is_empty() && !file.is_empty() =>
        {
            Ok((account, device, file))
        }
        _ => Err(format!("{name} is not a backup segment name")),
    }
}

impl BackupStore for Drive {
    fn put(&self, name: &str, bytes: &[u8]) -> impl Future<Output = Result<(), String>> + Send {
        let name = name.to_string();
        let bytes = bytes.to_vec();
        async move {
            let token = self.token().await?;
            let (account, device, file) = split(&name)?;
            let folder = gdrive::ensure_path(
                &token,
                &[gdrive::FOLDER_NAME, gdrive::MAIL_FOLDER, account, device],
            )
            .await
            .map_err(|e| e.to_string())?;
            // Overwriting in place rather than adding a second file with the same name, which
            // Drive would allow and which would make the segment ambiguous. A segment is written
            // once in the ordinary course of things; this is the re-run after an interruption.
            let existing = gdrive::find_file(&token, &folder, file)
                .await
                .map_err(|e| e.to_string())?;
            gdrive::upload(
                &token,
                &folder,
                file,
                &bytes,
                existing.as_ref().map(|held| held.id.as_str()),
            )
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
        }
    }

    fn get(&self, name: &str) -> impl Future<Output = Result<Vec<u8>, String>> + Send {
        let name = name.to_string();
        async move {
            let token = self.token().await?;
            let (account, device, file) = split(&name)?;
            let folder = gdrive::ensure_path(
                &token,
                &[gdrive::FOLDER_NAME, gdrive::MAIL_FOLDER, account, device],
            )
            .await
            .map_err(|e| e.to_string())?;
            let held = gdrive::find_file(&token, &folder, file)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("{name} is not in the backup folder any more"))?;
            gdrive::download(&token, &held.id)
                .await
                .map_err(|e| e.to_string())
        }
    }

    /// A prefix is either one account's folder or one device's folder inside it. There is no third
    /// depth, so the walk is two loops rather than a recursion, and a listing that came back with
    /// something at another depth would be somebody else's file rather than a segment.
    fn list(&self, prefix: &str) -> impl Future<Output = Result<Vec<String>, String>> + Send {
        let prefix = prefix.to_string();
        async move {
            let token = self.token().await?;
            let parts: Vec<&str> = prefix.split('/').filter(|part| !part.is_empty()).collect();
            let mut names = Vec::new();
            match parts.as_slice() {
                [account] => {
                    let folder = gdrive::ensure_path(
                        &token,
                        &[gdrive::FOLDER_NAME, gdrive::MAIL_FOLDER, account],
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                    for device in gdrive::list_folder(&token, &folder)
                        .await
                        .map_err(|e| e.to_string())?
                    {
                        for file in gdrive::list_folder(&token, &device.id)
                            .await
                            .map_err(|e| e.to_string())?
                        {
                            names.push(format!("{account}/{}/{}", device.name, file.name));
                        }
                    }
                }
                [account, device] => {
                    let folder = gdrive::ensure_path(
                        &token,
                        &[gdrive::FOLDER_NAME, gdrive::MAIL_FOLDER, account, device],
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                    for file in gdrive::list_folder(&token, &folder)
                        .await
                        .map_err(|e| e.to_string())?
                    {
                        names.push(format!("{account}/{device}/{}", file.name));
                    }
                }
                _ => return Err(format!("{prefix} is not a backup folder")),
            }
            Ok(names)
        }
    }
}
