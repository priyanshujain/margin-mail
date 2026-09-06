// Google Drive, for the encrypted state journal. The HTTP parts are ported from margin's
// `gdrive.rs`; the encryption, the journal and the merge belong to the backup package, not here.
//
// The thing to be accurate about, because the architecture document had it wrong until this week:
// margin does not use Drive's hidden app data space. It holds the `drive.file` scope and writes
// into an ordinary visible folder called `margin` at the root of the user's Drive, which is
// deliberate. A person should be able to open their Drive, see their own backup, and delete it
// without asking anyone. Margin Mail keeps the same scope, adds no new one, and writes under
// `margin/mail/<account-hash>/<device-id>/` inside that same folder, so the whole suite lives in
// one place a person can see rather than three hidden ones they cannot.
//
// What `drive.file` means in practice: this app can only see files it created itself. Listing the
// folder therefore returns Margin's own files and nothing else, which is why a `q` of "everything
// in this folder" is safe here and would be a privacy problem under a wider scope. It also means
// that if the user deletes the folder, the app cannot find its own history and starts again, which
// is the correct behaviour for a backup somebody threw away.

use serde::{Deserialize, Serialize};

use crate::google::api::{self, ApiError};

const FILES: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";

pub const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";

/// The suite's one folder, at the root of the user's Drive and visible in it.
pub const FOLDER_NAME: &str = "margin";

/// This app's corner of it. The sibling apps use their own.
pub const MAIL_FOLDER: &str = "mail";

const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

/// Said in the user's words rather than the API's, because "insufficient authentication scopes" is
/// not a sentence anybody can act on. The wording matches the tick box on Google's consent screen.
pub const REAUTH_MESSAGE: &str = "Margin Mail needs the \"See, edit, create and delete only the specific Google Drive files you use with this app\" permission to back up. Connect again and tick that box.";

/// Granular consent means the tick box is the user's to clear, so the granted scope list is checked
/// before the first call rather than a 403 being explained after it.
pub fn has_scope(granted: &[String]) -> bool {
    granted.iter().any(|scope| scope == SCOPE)
}

/// Drive's query language is SQL-shaped and takes single-quoted literals, so a folder name or a
/// device id carrying a quote would otherwise end the string early.
pub fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Bytes, as a string, because JSON has no int64.
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub modified_time: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileList {
    #[serde(default)]
    files: Vec<File>,
    #[serde(default)]
    next_page_token: Option<String>,
}

fn upload_boundary() -> String {
    use rand::Rng;
    let suffix: u128 = rand::thread_rng().gen();
    format!("margin-mail-upload-{suffix:032x}")
}

async fn find_one(access_token: &str, query: &str) -> Result<Option<File>, ApiError> {
    let resp = api::HTTP
        .get(api::url_with(
            FILES,
            &[
                ("q", query),
                ("fields", "files(id,name,size,modifiedTime)"),
                ("spaces", "drive"),
                ("pageSize", "1"),
            ],
        ))
        .bearer_auth(access_token)
        .send()
        .await?;
    let list: FileList = api::read_json(resp, "Drive lookup", SCOPE).await?;
    Ok(list.files.into_iter().next())
}

/// Idempotent: the folder is looked up first and only created when it is not there. A `parent` of
/// None means the root of the user's Drive, which is where `margin` lives.
pub async fn ensure_folder(
    access_token: &str,
    name: &str,
    parent: Option<&str>,
) -> Result<String, ApiError> {
    let mut query = format!(
        "name = '{}' and mimeType = '{FOLDER_MIME}' and trashed = false",
        escape(name)
    );
    if let Some(parent) = parent {
        query.push_str(&format!(" and '{}' in parents", escape(parent)));
    }
    if let Some(folder) = find_one(access_token, &query).await? {
        return Ok(folder.id);
    }

    let mut body = serde_json::json!({ "name": name, "mimeType": FOLDER_MIME });
    if let Some(parent) = parent {
        body["parents"] = serde_json::json!([parent]);
    }
    let resp = api::HTTP
        .post(api::url_with(FILES, &[("fields", "id")]))
        .bearer_auth(access_token)
        .json(&body)
        .send()
        .await?;
    let created: File = api::read_json(resp, "Drive folder creation", SCOPE).await?;
    Ok(created.id)
}

/// Walks a path of folder names, creating what is missing, and returns the id of the last one.
/// `["margin", "mail", account_hash, device_id]` is the whole of this app's use of it.
pub async fn ensure_path(access_token: &str, segments: &[&str]) -> Result<String, ApiError> {
    let mut parent: Option<String> = None;
    for segment in segments {
        let id = ensure_folder(access_token, segment, parent.as_deref()).await?;
        parent = Some(id);
    }
    parent.ok_or_else(|| ApiError::Other("an empty Drive path is not a folder".to_string()))
}

/// The folder this device's journal segments go in.
pub async fn ensure_mail_folder(
    access_token: &str,
    account_hash: &str,
    device_id: &str,
) -> Result<String, ApiError> {
    ensure_path(
        access_token,
        &[FOLDER_NAME, MAIL_FOLDER, account_hash, device_id],
    )
    .await
}

pub async fn find_file(
    access_token: &str,
    folder_id: &str,
    name: &str,
) -> Result<Option<File>, ApiError> {
    let query = format!(
        "'{}' in parents and name = '{}' and trashed = false",
        escape(folder_id),
        escape(name)
    );
    find_one(access_token, &query).await
}

pub async fn list_folder(access_token: &str, folder_id: &str) -> Result<Vec<File>, ApiError> {
    let query = format!("'{}' in parents and trashed = false", escape(folder_id));
    let mut files = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut params = vec![
            ("q", query.as_str()),
            ("fields", "nextPageToken,files(id,name,size,modifiedTime)"),
            ("spaces", "drive"),
            ("pageSize", "1000"),
        ];
        if let Some(token) = &page_token {
            params.push(("pageToken", token.as_str()));
        }
        let resp = api::HTTP
            .get(api::url_with(FILES, &params))
            .bearer_auth(access_token)
            .send()
            .await?;
        let page: FileList = api::read_json(resp, "Drive file list", SCOPE).await?;
        files.extend(page.files);
        match page.next_page_token {
            Some(token) if !token.is_empty() => page_token = Some(token),
            _ => return Ok(files),
        }
    }
}

/// The metadata and the bytes in one round trip, which is what `uploadType=multipart` is for.
/// Passing `existing_id` overwrites in place rather than leaving two files with the same name,
/// which Drive allows and which would make the journal ambiguous.
pub async fn upload(
    access_token: &str,
    folder_id: &str,
    name: &str,
    bytes: &[u8],
    existing_id: Option<&str>,
) -> Result<File, ApiError> {
    let boundary = upload_boundary();
    let metadata = match existing_id {
        Some(_) => serde_json::json!({ "name": name }),
        None => serde_json::json!({ "name": name, "parents": [folder_id] }),
    };

    let mut body: Vec<u8> = Vec::with_capacity(bytes.len() + 512);
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(serde_json::to_string(&metadata).unwrap_or_default().as_bytes());
    body.extend_from_slice(
        format!("\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let params = [
        ("uploadType", "multipart"),
        ("fields", "id,name,size,modifiedTime"),
    ];
    let request = match existing_id {
        Some(id) => api::HTTP.patch(api::url_with(
            &format!("{UPLOAD}/{}", api::path_segment(id)),
            &params,
        )),
        None => api::HTTP.post(api::url_with(UPLOAD, &params)),
    };
    let resp = request
        .bearer_auth(access_token)
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/related; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await?;
    api::read_json(resp, &format!("Drive upload of {name}"), SCOPE).await
}

/// `alt=media` is the difference between the file and the file's metadata.
pub async fn download(access_token: &str, file_id: &str) -> Result<Vec<u8>, ApiError> {
    let resp = api::HTTP
        .get(api::url_with(
            &format!("{FILES}/{}", api::path_segment(file_id)),
            &[("alt", "media")],
        ))
        .bearer_auth(access_token)
        .send()
        .await?;
    api::read_bytes(resp, "Drive download", SCOPE).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_tick_box_is_caught_before_the_call_rather_than_explained_after_it() {
        let granted = vec![
            "openid".to_string(),
            "https://www.googleapis.com/auth/gmail.modify".to_string(),
        ];
        assert!(!has_scope(&granted));

        let mut granted = granted;
        granted.push(SCOPE.to_string());
        assert!(has_scope(&granted));
    }

    #[test]
    fn the_reauth_message_names_the_tick_box_rather_than_the_scope() {
        assert!(!REAUTH_MESSAGE.contains("drive.file"));
        assert!(REAUTH_MESSAGE.contains("specific Google Drive files"));
    }

    #[test]
    fn a_quote_in_a_name_cannot_end_a_drive_query_early() {
        assert_eq!(escape("margin"), "margin");
        assert_eq!(escape("ana's device"), "ana\\'s device");
        assert_eq!(escape("back\\slash"), "back\\\\slash");
    }

    #[test]
    fn the_suite_shares_one_visible_folder_and_mail_lives_under_it() {
        assert_eq!(FOLDER_NAME, "margin");
        assert_eq!(MAIL_FOLDER, "mail");
        assert_ne!(FOLDER_NAME, "appDataFolder");
        assert_eq!(SCOPE, "https://www.googleapis.com/auth/drive.file");
    }

    #[test]
    fn every_upload_boundary_is_its_own() {
        assert_ne!(upload_boundary(), upload_boundary());
    }
}
