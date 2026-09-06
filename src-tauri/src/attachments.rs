// Bytes fetched because somebody asked for them: the pictures in a message, and the files hanging
// off it.
//
// Nothing here runs during a sync. A message arrives as headers, its body arrives when the thread
// is opened, and everything in this file happens later still, when a reader presses something. The
// two halves sit together because they are the same job under two names: fetch on demand, cap what
// is fetched, and never let the webview make a request of its own.
//
// Remote images are the reason that last clause is written down. The sanitiser has no network by
// design, so the only way to show a picture is for Rust to fetch it, without cookies and without a
// referrer, and hand the bytes back through `RenderOptions::remote_images`. The sender learns that
// somebody asked, once, from an address, and nothing else: no repeat opens, no forwarded reads,
// and no correlation with anything else the app does.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use base64::Engine;
use futures::stream::{self, StreamExt};
use mail_parser::{MessageParser, MimeHeaders, PartType};
use rusqlite::Connection;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

use crate::db::Db;
use crate::dto::MessageView;
use crate::mime::{self, RenderOptions, Rendered};
use crate::mirror::read::{self, AttachmentRow};
use crate::mirror::write;
use crate::sync::{self, Remote};

/// How many remote images one message may have before the answer is a sentence rather than a
/// download. A newsletter has a dozen; a body with hundreds is either broken or a probe, and
/// either way nobody wants to wait for it.
const MAX_IMAGES: usize = 60;
/// Per image, and in total. A picture in a message is a picture, not a payload.
const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_IMAGES_TOTAL_BYTES: usize = 16 * 1024 * 1024;
/// How many of one message's pictures are in flight together. A newsletter spreads them over two
/// or three hosts, and one host that will not answer must not hold the others behind it, which is
/// what fetching them one after another did: a dead CDN was a full timeout per picture, in a row,
/// while the reader watched a button that had not changed.
const IMAGES_AT_ONCE: usize = 8;

/// The ceiling on a `data:` URI. Base64 costs a third on top, the whole string crosses the IPC
/// boundary as JSON and is then held in the webview, so a preview of something enormous is a hang
/// with a progress bar in front of it. Above this the answer is Save.
const MAX_DATA_URL_BYTES: u64 = 8 * 1024 * 1024;

/// What the cache is allowed to hold when the settings file cannot be read. It matches
/// `settings::defaults`, so an unreadable file behaves like a fresh install rather than like no
/// cache at all.
const DEFAULT_CACHE_MB: u64 = 512;

const CACHE_DIR: &str = "attachments";

// ---------------------------------------------------------------------------------------------
// Remote images
// ---------------------------------------------------------------------------------------------

/// No cookie store is compiled into this build at all, so there is nothing to send even by
/// accident, and `referer(false)` keeps a redirect from naming the URL it came from. The timeouts
/// are short: this runs while somebody is looking at the message, and a picture that has not
/// arrived in ten seconds is a picture that is not coming.
fn image_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .referer(false)
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .expect("could not build the image client")
    })
}

/// The remote images this body would like, or a refusal. Rendering with images off is what names
/// them, so the list is the sanitiser's own and not a second scan of the markup.
fn wanted(raw: &[u8], options: &RenderOptions) -> Result<Vec<String>, String> {
    let mut urls: Vec<String> = Vec::new();
    for url in mime::render(raw, options)?.blocked_urls {
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    if urls.len() > MAX_IMAGES {
        return Err(format!(
            "This message asks for {} remote images, which is more than Margin Mail will fetch at \
             once ({MAX_IMAGES}). None were fetched.",
            urls.len()
        ));
    }
    Ok(urls)
}

/// The same body again with the fetched bytes in hand. The trackers stay removed: that rule lives
/// in the sanitiser and this passes through it rather than around it, because "show me the
/// pictures" is not "file an open report".
fn shown(
    raw: &[u8],
    options: &RenderOptions,
    fetched: HashMap<String, Vec<u8>>,
) -> Result<Rendered, String> {
    let options = RenderOptions {
        allow_remote_images: true,
        remote_images: fetched,
        ..options.clone()
    };
    mime::render(raw, &options)
}

/// One image, capped. Anything that fails, redirects too far or runs over the cap is left blocked
/// rather than reported: a picture that would not come is a picture that is not there.
async fn fetch_image(url: &str) -> Option<Vec<u8>> {
    let mut response = image_client().get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut bytes: Vec<u8> = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if bytes.len() + chunk.len() > MAX_IMAGE_BYTES {
                    return None;
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(_) => return None,
        }
    }
    (!bytes.is_empty()).then_some(bytes)
}

/// All of them, `IMAGES_AT_ONCE` at a time, in whatever order they arrive. Dropping the stream
/// once the total cap is reached is what cancels the fetches still in flight.
async fn fetch_images(urls: &[String]) -> HashMap<String, Vec<u8>> {
    let mut fetched = HashMap::new();
    let mut total = 0usize;
    let mut results = stream::iter(urls.iter().cloned())
        .map(|url| async move {
            let bytes = fetch_image(&url).await;
            (url, bytes)
        })
        .buffer_unordered(IMAGES_AT_ONCE);
    while let Some((url, bytes)) = results.next().await {
        if total >= MAX_IMAGES_TOTAL_BYTES {
            break;
        }
        let Some(bytes) = bytes else {
            continue;
        };
        total += bytes.len();
        fetched.insert(url, bytes);
    }
    fetched
}

// ---------------------------------------------------------------------------------------------
// The attachment cache
// ---------------------------------------------------------------------------------------------

fn cache_dir(db: &Db, account_id: &str) -> PathBuf {
    db.account_dir(account_id).join(CACHE_DIR)
}

/// A file name derived from the attachment id, which is already unique within the account. The
/// extension is kept so the OS opens the file with the right thing.
fn cache_name(row: &AttachmentRow) -> String {
    let stem: String = row
        .id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    match row.filename.rsplit_once('.') {
        Some((_, extension)) if !extension.is_empty() && extension.len() <= 8 => {
            let extension: String = extension
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect();
            if extension.is_empty() {
                stem
            } else {
                format!("{stem}.{}", extension.to_lowercase())
            }
        }
        _ => stem,
    }
}

/// The bytes of one part of a message that is already on the device.
///
/// `mime::render` only carries the bytes of a part small enough to keep beside the row, and the
/// mirror keys an attachment by its MIME part path, so this matches on what the row does hold: the
/// decoded length and the file name. Body parts are skipped the same way the renderer skips them,
/// so a text body the same length as a file cannot be mistaken for it.
fn part_bytes(raw: &[u8], row: &AttachmentRow) -> Option<Vec<u8>> {
    let message = MessageParser::default().parse(raw)?;
    let bodies: HashSet<usize> = message
        .html_body
        .iter()
        .chain(message.text_body.iter())
        .map(|index| *index as usize)
        .collect();

    let mut fallback = None;
    for (index, part) in message.parts.iter().enumerate() {
        if bodies.contains(&index) {
            continue;
        }
        let bytes = match &part.body {
            PartType::Binary(bytes) | PartType::InlineBinary(bytes) => bytes.to_vec(),
            PartType::Text(text) | PartType::Html(text) => text.as_bytes().to_vec(),
            PartType::Multipart(_) | PartType::Message(_) => continue,
        };
        if bytes.len() as u64 != row.size {
            continue;
        }
        if part.attachment_name() == Some(row.filename.as_str()) {
            return Some(bytes);
        }
        fallback.get_or_insert(bytes);
    }
    fallback
}

/// The bytes of one attachment: from the cache, else out of the message already on the device,
/// else from the provider.
///
/// The provider is last because it is rarely needed and, for Gmail, rarely possible: the mirror
/// keys an attachment by its MIME part path and `attachments.get` wants Gmail's own attachment id,
/// which nothing stores. A message with an attachment row has a body, and a body is the whole
/// message, so the bytes are almost always here already.
async fn bytes_of(
    db: &Db,
    account_id: &str,
    remote: Option<&dyn Remote>,
    row: &AttachmentRow,
    cap_bytes: u64,
) -> Result<Vec<u8>, String> {
    let dir = cache_dir(db, account_id);
    if let Some(path) = &row.cached_path {
        if let Ok(bytes) = std::fs::read(path) {
            return Ok(bytes);
        }
        db.with(account_id, |conn| write::attachment_uncached(conn, &row.id))?;
    }

    let raw = db.with(account_id, |conn| read::raw_body(conn, &row.message_id))?;
    let bytes = match raw.as_deref().and_then(|raw| part_bytes(raw, row)) {
        Some(bytes) => bytes,
        None => {
            let remote = remote.ok_or("that file is not on this device")?;
            remote
                .fetch_attachment(&row.message_id, &row.part_id)
                .await
                .map_err(|e| e.to_string())?
        }
    };

    put(db, account_id, &dir, row, &bytes, cap_bytes)?;
    Ok(bytes)
}

/// Writes the bytes into the cache and keeps the cache inside its cap.
fn put(
    db: &Db,
    account_id: &str,
    dir: &Path,
    row: &AttachmentRow,
    bytes: &[u8],
    cap_bytes: u64,
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(cache_name(row));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    db.with(account_id, |conn| {
        write::attachment_cached(conn, &row.id, &path.to_string_lossy(), write::now_ms())?;
        trim(conn, dir, cap_bytes)
    })
}

/// Keeps the cache honest in both directions.
///
/// `mirror::evict` deletes an attachment row without touching the file it points at, because
/// eviction runs inside one SQL transaction and has no business on the filesystem. A file with no
/// row is therefore the expected shape rather than a bug, and this is where it goes. After that,
/// least recently fetched first, until what is left fits the cap.
///
/// The newest is never given up, whatever the cap says. It is the file somebody is waiting on, and
/// a cache too small for one file should still hand that file over.
fn trim(conn: &Connection, dir: &Path, cap_bytes: u64) -> Result<(), String> {
    let held = read::cached_attachments(conn)?;
    let known: HashSet<PathBuf> = held.iter().map(|(_, path, _)| PathBuf::from(path)).collect();

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && !known.contains(&path) {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    let mut total: u64 = held.iter().map(|(_, _, size)| *size).sum();
    let mut index = 0;
    while total > cap_bytes && index + 1 < held.len() {
        let (id, path, size) = &held[index];
        let _ = std::fs::remove_file(path);
        write::attachment_uncached(conn, id)?;
        total = total.saturating_sub(*size);
        index += 1;
    }
    Ok(())
}

/// The cap the settings screen set, in bytes. A settings file that will not load is not a reason
/// to refuse somebody their attachment, so it falls back rather than failing.
fn cap_bytes(app: &tauri::AppHandle) -> u64 {
    crate::settings::load(app)
        .map(|settings| settings.attachment_cache_mb as u64)
        .ok()
        .filter(|megabytes| *megabytes > 0)
        .unwrap_or(DEFAULT_CACHE_MB)
        * 1024
        * 1024
}

// ---------------------------------------------------------------------------------------------
// Finding things
// ---------------------------------------------------------------------------------------------

fn db_of(app: &tauri::AppHandle) -> Result<tauri::State<'_, Db>, String> {
    app.try_state::<Db>()
        .ok_or_else(|| "the mirror is not open yet".to_string())
}

fn account_holding(db: &Db, sql: &str, id: &str) -> Result<String, String> {
    for (account_id, _) in sync::accounts(db, None) {
        let held = db.with(&account_id, |conn| {
            conn.query_row(sql, [id], |row| row.get::<_, i64>(0))
                .map_err(|e| e.to_string())
        })?;
        if held > 0 {
            return Ok(account_id);
        }
    }
    Err("that is not on this device".to_string())
}

fn attachment_of(db: &Db, id: &str) -> Result<(String, AttachmentRow), String> {
    let account_id = account_holding(db, "SELECT COUNT(*) FROM attachments WHERE id = ?1", id)?;
    let row = db
        .with(&account_id, |conn| read::attachment_row(conn, id))?
        .ok_or("that file is not on this device")?;
    Ok((account_id, row))
}

/// Somewhere in the downloads directory that is not already taken.
fn free_path(dir: &Path, filename: &str) -> PathBuf {
    let safe: String = filename
        .chars()
        .map(|c| if std::path::is_separator(c) { '-' } else { c })
        .collect();
    let safe = safe.trim().trim_matches('.').to_string();
    let safe = if safe.is_empty() {
        "attachment".to_string()
    } else {
        safe
    };
    let path = dir.join(&safe);
    if !path.exists() {
        return path;
    }
    let (stem, extension) = match safe.rsplit_once('.') {
        Some((stem, extension)) => (stem.to_string(), format!(".{extension}")),
        None => (safe.clone(), String::new()),
    };
    for nth in 2..1000 {
        let candidate = dir.join(format!("{stem} ({nth}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem} ({}){extension}", write::now_ms()))
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

/// The reader has asked to see the pictures.
///
/// The answer is a view and not a stored row on purpose: the `bodies` table has no column saying
/// whether images were loaded, so this is per view rather than remembered, and closing the thread
/// puts the block back. Inventing a column for it would also be inventing a policy, and the policy
/// that roams with a person is the per sender allowance on the contact card, not a flag on a body.
#[tauri::command]
pub async fn message_show_images(
    app: tauri::AppHandle,
    message_id: String,
) -> Result<MessageView, String> {
    let db = db_of(&app)?;
    let account_id = account_holding(
        db.inner(),
        "SELECT COUNT(*) FROM messages WHERE id = ?1",
        &message_id,
    )?;

    let (raw, options) = db.with(&account_id, |conn| {
        let raw = read::raw_body(conn, &message_id)?
            .ok_or("that message has not been fetched yet")?;
        Ok((raw, sync::hydrate::render_options(conn)?))
    })?;

    let urls = wanted(&raw, &options)?;
    let rendered = shown(&raw, &options, fetch_images(&urls).await)?;

    let mut view = db.with(&account_id, |conn| {
        read::message_view(conn, &account_id, &message_id, &options.own_addresses)
    })?;
    view.html = rendered.html;
    view.quoted_html = rendered.quoted_html;
    view.trackers = rendered.trackers;
    view.blocked_images = rendered.blocked_images;
    view.images_loaded = true;
    Ok(view)
}

/// The inline preview. Refuses anything too big to be a data URI before it fetches a byte, so the
/// answer to a hundred megabyte video is a sentence rather than a spinner.
#[tauri::command]
pub async fn attachment_data_url(
    app: tauri::AppHandle,
    attachment_id: String,
) -> Result<String, String> {
    let db = db_of(&app)?;
    let (account_id, row) = attachment_of(db.inner(), &attachment_id)?;
    if let Some(refusal) = too_big_to_preview(&row) {
        return Err(refusal);
    }
    let remote = sync::remote_for(&account_id);
    let bytes = bytes_of(
        db.inner(),
        &account_id,
        remote.as_deref(),
        &row,
        cap_bytes(&app),
    )
    .await?;
    Ok(format!(
        "data:{};base64,{}",
        row.mime_type,
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

/// Writes it to the downloads directory and hands back the path, which is what the toast names.
#[tauri::command]
pub async fn attachment_save(
    app: tauri::AppHandle,
    attachment_id: String,
) -> Result<String, String> {
    let db = db_of(&app)?;
    let (account_id, row) = attachment_of(db.inner(), &attachment_id)?;
    let remote = sync::remote_for(&account_id);
    let bytes = bytes_of(
        db.inner(),
        &account_id,
        remote.as_deref(),
        &row,
        cap_bytes(&app),
    )
    .await?;

    let downloads = app.path().download_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&downloads).map_err(|e| e.to_string())?;
    let path = free_path(&downloads, &row.filename);
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

/// Hands the cached file to the OS. The cache is where it opens from, so opening the same file
/// twice costs nothing the second time.
#[tauri::command]
pub async fn attachment_open(app: tauri::AppHandle, attachment_id: String) -> Result<(), String> {
    let db = db_of(&app)?;
    let (account_id, row) = attachment_of(db.inner(), &attachment_id)?;
    let remote = sync::remote_for(&account_id);
    bytes_of(
        db.inner(),
        &account_id,
        remote.as_deref(),
        &row,
        cap_bytes(&app),
    )
    .await?;

    let path = db
        .with(&account_id, |conn| read::attachment_row(conn, &row.id))?
        .and_then(|row| row.cached_path)
        .ok_or("that file could not be put on the disk")?;
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| e.to_string())
}

/// Decided from the row rather than from the bytes, so an enormous file is refused before anything
/// is fetched and the answer arrives at once.
fn too_big_to_preview(row: &AttachmentRow) -> Option<String> {
    (row.size > MAX_DATA_URL_BYTES).then(|| {
        format!(
            "{} is {}, which is too big to preview here. Save it instead.",
            row.filename,
            megabytes(row.size)
        )
    })
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use rusqlite::Connection;
    use tauri::async_runtime::block_on;

    use crate::db;
    use crate::provider::fake::FakeProvider;
    use crate::sync::{hydrate, Store};

    use super::*;

    struct Memory(Mutex<Connection>);

    impl Store for Memory {
        fn with<T, F: FnOnce(&Connection) -> Result<T, String>>(&self, f: F) -> Result<T, String> {
            let conn = self.0.lock().map_err(|e| e.to_string())?;
            f(&conn)
        }
    }

    impl Memory {
        fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T, String>) -> T {
            self.with(f).expect("the mirror")
        }
    }

    fn store() -> Memory {
        Memory(Mutex::new(db::memory().expect("a pair of in-memory databases")))
    }

    fn eml(id: &str, headers: &str, body: &str) -> Vec<u8> {
        format!(
            "Message-ID: <{id}@example.test>\r\nFrom: Ana <ana@example.test>\r\n\
             To: You <you@example.test>\r\nSubject: {id}\r\n\
             Date: Wed, 2 Sep 2026 10:00:00 +0000\r\n{headers}\r\n{body}"
        )
        .into_bytes()
    }

    /// A body with one ordinary remote image and one tracking pixel from a named vendor.
    fn with_images() -> Vec<u8> {
        eml(
            "images",
            "Content-Type: text/html; charset=utf-8\r\n",
            "<html><body><p>Hello</p>\
             <img src=\"https://shop.example/photo.jpg\" width=\"640\" height=\"420\">\
             <img src=\"https://track.hubspot.com/__ptq.gif?a=1\" width=\"1\" height=\"1\">\
             </body></html>\r\n",
        )
    }

    fn options() -> RenderOptions {
        RenderOptions {
            allow_remote_images: false,
            link_cleaning: true,
            remote_images: HashMap::new(),
            own_addresses: vec!["you@example.test".to_string()],
        }
    }

    /// Eight bytes of PNG magic, which is all the sanitiser sniffs before it will inline anything.
    fn png() -> Vec<u8> {
        b"\x89PNG\r\n\x1a\nrest".to_vec()
    }

    #[test]
    fn showing_images_inlines_what_was_fetched_and_leaves_the_trackers_removed() {
        let raw = with_images();
        let urls = wanted(&raw, &options()).expect("the blocked images");
        assert_eq!(urls, vec!["https://shop.example/photo.jpg".to_string()]);

        let blocked = mime::render(&raw, &options()).expect("a render");
        assert_eq!(blocked.blocked_images, 1);
        assert_eq!(blocked.trackers.len(), 1);

        let mut fetched = HashMap::new();
        fetched.insert(urls[0].clone(), png());
        let rendered = shown(&raw, &options(), fetched).expect("a second render");

        assert!(
            rendered.html.contains("data:image/png;base64,"),
            "the fetched bytes are inlined: {}",
            rendered.html
        );
        assert_eq!(rendered.blocked_images, 0);
        assert_eq!(
            rendered.trackers.len(),
            1,
            "the pixel is still named and still gone"
        );
        assert_eq!(rendered.trackers[0].vendor, "HubSpot");
        assert!(
            !rendered.html.contains("track.hubspot.com"),
            "showing the pictures is not filing an open report: {}",
            rendered.html
        );
    }

    #[test]
    fn a_body_with_more_images_than_the_cap_is_refused_before_anything_is_fetched() {
        let mut body = String::from("<html><body>");
        for index in 0..(MAX_IMAGES + 1) {
            body.push_str(&format!(
                "<img src=\"https://shop.example/{index}.jpg\" width=\"400\" height=\"300\">"
            ));
        }
        body.push_str("</body></html>\r\n");
        let raw = eml("many", "Content-Type: text/html; charset=utf-8\r\n", &body);

        let refused = wanted(&raw, &options()).expect_err("a refusal");
        assert!(refused.contains("more than Margin Mail will fetch"), "{refused}");
        assert!(refused.contains("None were fetched."), "{refused}");
    }

    #[test]
    fn showing_images_changes_nothing_that_is_stored() {
        let store = store();
        let fake = FakeProvider::new();
        let raw = with_images();
        fake.add_eml("m1", "t1", &["INBOX"], &raw);
        block_on(hydrate::headers(&store, &fake, &["m1".to_string()], false)).expect("headers");
        block_on(hydrate::body(&store, &fake, "m1")).expect("a body");

        let before = store.read(|conn| {
            conn.query_row(
                "SELECT html, blocked_images FROM bodies WHERE message_id = 'm1'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(|e| e.to_string())
        });
        assert_eq!(before.1, 1, "the stored render has the image blocked");

        let held = store.read(|conn| read::raw_body(conn, "m1")).expect("the bytes");
        let urls = wanted(&held, &options()).expect("the blocked images");
        let mut fetched = HashMap::new();
        fetched.insert(urls[0].clone(), png());
        let rendered = shown(&held, &options(), fetched).expect("a render with images");
        assert!(rendered.html.contains("data:image/png;base64,"));

        let after = store.read(|conn| {
            conn.query_row(
                "SELECT html, blocked_images FROM bodies WHERE message_id = 'm1'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(|e| e.to_string())
        });
        assert_eq!(after, before, "the row is untouched, so the next open blocks again");
    }

    // -- the attachment cache -------------------------------------------------------------------

    /// A message with one attached file, built as multipart so the parts are real parts.
    fn with_attachment(id: &str, filename: &str, bytes: &[u8]) -> Vec<u8> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let mut body = String::from("--sep\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n");
        body.push_str("Here it is.\r\n");
        body.push_str(&format!(
            "--sep\r\nContent-Type: application/octet-stream\r\n\
             Content-Disposition: attachment; filename=\"{filename}\"\r\n\
             Content-Transfer-Encoding: base64\r\n\r\n"
        ));
        for chunk in encoded.as_bytes().chunks(76) {
            body.push_str(&String::from_utf8_lossy(chunk));
            body.push_str("\r\n");
        }
        body.push_str("--sep--\r\n");
        eml(
            id,
            "Content-Type: multipart/mixed; boundary=\"sep\"\r\n",
            &body,
        )
    }

    fn only_attachment(store: &Memory, message_id: &str) -> AttachmentRow {
        let rows = store.read(|conn| read::attachments(conn, message_id));
        assert_eq!(rows.len(), 1, "{rows:?}");
        store
            .read(|conn| read::attachment_row(conn, &rows[0].id))
            .expect("the row")
    }

    /// The cache without the Tauri handle in front of it: the same order of preference, against a
    /// directory a test owns.
    fn fetch(
        store: &Memory,
        dir: &Path,
        remote: Option<&dyn Remote>,
        row: &AttachmentRow,
        cap: u64,
    ) -> Result<Vec<u8>, String> {
        let row = store
            .read(|conn| read::attachment_row(conn, &row.id))
            .expect("the row");
        if let Some(path) = &row.cached_path {
            if let Ok(bytes) = std::fs::read(path) {
                return Ok(bytes);
            }
            store.read(|conn| write::attachment_uncached(conn, &row.id));
        }
        let raw = store.read(|conn| read::raw_body(conn, &row.message_id));
        let bytes = match raw.as_deref().and_then(|raw| part_bytes(raw, &row)) {
            Some(bytes) => bytes,
            None => {
                let remote = remote.ok_or("that file is not on this device")?;
                block_on(remote.fetch_attachment(&row.message_id, &row.part_id))
                    .map_err(|e| e.to_string())?
            }
        };
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join(cache_name(&row));
        std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
        store.read(|conn| {
            write::attachment_cached(conn, &row.id, &path.to_string_lossy(), write::now_ms())?;
            trim(conn, dir, cap)
        });
        Ok(bytes)
    }

    fn a_message_with_a_file(
        store: &Memory,
        fake: &FakeProvider,
        id: &str,
        filename: &str,
        bytes: &[u8],
    ) -> AttachmentRow {
        fake.add_eml(id, id, &["INBOX"], &with_attachment(id, filename, bytes));
        block_on(hydrate::headers(store, fake, &[id.to_string()], false)).expect("headers");
        block_on(hydrate::body(store, fake, id)).expect("a body");
        only_attachment(store, id)
    }

    #[test]
    fn a_file_already_on_the_device_is_served_without_asking_the_provider() {
        let store = store();
        let fake = FakeProvider::new();
        let dir = tempfile::tempdir().expect("a cache directory");
        let row = a_message_with_a_file(&store, &fake, "m1", "notes.pdf", b"the file itself");

        let bytes = fetch(&store, dir.path(), Some(&fake), &row, 1 << 30).expect("the bytes");
        assert_eq!(bytes, b"the file itself");
        assert!(
            !fake.calls().iter().any(|call| call.starts_with("fetch_attachment")),
            "the whole message was already here: {:?}",
            fake.calls()
        );
    }

    #[test]
    fn an_attachment_is_fetched_once_and_served_from_the_cache_the_second_time() {
        let store = store();
        let fake = FakeProvider::new();
        let dir = tempfile::tempdir().expect("a cache directory");
        let row = a_message_with_a_file(&store, &fake, "m1", "notes.pdf", b"the file itself");
        fake.set_attachment(&row.part_id, b"the file itself".to_vec());

        // The message is gone but its rows are not, which is the one shape that has to reach the
        // provider.
        store.read(|conn| {
            conn.execute("UPDATE bodies SET raw = NULL WHERE message_id = 'm1'", [])
                .map(|_| ())
                .map_err(|e| e.to_string())
        });

        let first = fetch(&store, dir.path(), Some(&fake), &row, 1 << 30).expect("the bytes");
        let second = fetch(&store, dir.path(), Some(&fake), &row, 1 << 30).expect("the bytes again");
        assert_eq!(first, b"the file itself");
        assert_eq!(second, first);

        let fetches = fake
            .calls()
            .into_iter()
            .filter(|call| call.starts_with("fetch_attachment"))
            .count();
        assert_eq!(fetches, 1, "the second time came off the disk");
        assert!(
            store
                .read(|conn| read::attachment_row(conn, &row.id))
                .expect("the row")
                .cached_path
                .is_some(),
            "and the row says where"
        );
    }

    #[test]
    fn a_file_too_big_for_a_data_uri_is_refused_with_a_reason() {
        let store = store();
        let fake = FakeProvider::new();
        let row = a_message_with_a_file(&store, &fake, "m1", "film.mov", b"small in the test");
        assert_eq!(too_big_to_preview(&row), None, "an ordinary file previews");

        store.read(|conn| {
            conn.execute(
                "UPDATE attachments SET size = ?1 WHERE id = ?2",
                rusqlite::params![(MAX_DATA_URL_BYTES + 1) as i64, row.id],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        });
        let row = store
            .read(|conn| read::attachment_row(conn, &row.id))
            .expect("the row");

        assert_eq!(
            too_big_to_preview(&row).as_deref(),
            Some("film.mov is 8.0 MB, which is too big to preview here. Save it instead."),
            "and it says which file and how big before it fetches a byte"
        );
    }

    #[test]
    fn the_cache_gives_up_the_oldest_file_to_stay_inside_its_cap() {
        let store = store();
        let fake = FakeProvider::new();
        let dir = tempfile::tempdir().expect("a cache directory");
        let first = a_message_with_a_file(&store, &fake, "m1", "one.bin", &vec![b'a'; 4_000]);
        let second = a_message_with_a_file(&store, &fake, "m2", "two.bin", &vec![b'b'; 4_000]);

        // Room for one of them, and the older one is the one that goes.
        fetch(&store, dir.path(), Some(&fake), &first, 5_000).expect("the first");
        fetch(&store, dir.path(), Some(&fake), &second, 5_000).expect("the second");

        let held: Vec<String> = store
            .read(read::cached_attachments)
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        assert_eq!(held, vec![second.id.clone()]);

        let files: Vec<String> = std::fs::read_dir(dir.path())
            .expect("the cache directory")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(files.len(), 1, "{files:?}");
        assert!(
            std::fs::read(dir.path().join(&files[0])).expect("the file") == vec![b'b'; 4_000],
            "the one that is left is the one that was asked for last"
        );
        assert!(
            store
                .read(|conn| read::attachment_row(conn, &first.id))
                .expect("the row")
                .cached_path
                .is_none(),
            "and the row it dropped says so, so storage_used stays honest"
        );
    }

    #[test]
    fn a_file_whose_row_was_evicted_is_swept_off_the_disk() {
        let store = store();
        let fake = FakeProvider::new();
        let dir = tempfile::tempdir().expect("a cache directory");
        let row = a_message_with_a_file(&store, &fake, "m1", "one.bin", b"contents");
        fetch(&store, dir.path(), Some(&fake), &row, 1 << 30).expect("the bytes");

        let orphan = dir.path().join("left-behind.bin");
        std::fs::write(&orphan, b"whatever").expect("an orphan");

        store.read(|conn| trim(conn, dir.path(), 1 << 30));
        assert!(!orphan.exists(), "eviction cannot reach the disk, so this does");
        assert!(dir.path().join(cache_name(&row)).exists());
    }
}
