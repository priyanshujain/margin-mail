// The `Provider` implementation for an IMAP mailbox.
//
// OWNED BY THE IMAP PROVIDER PACKAGE. The trait itself is frozen: nothing here may change
// `provider/mod.rs`, because the Gmail implementation and the whole engine test suite are written
// against it. Where IMAP has no equivalent of something Gmail has, the answer is the emptiest
// honest one rather than a new trait method.
//
// The three mappings that are not obvious, and are decided here rather than left to the reader:
//
//   - There is no thread id. `MessageRef::thread_id` is left empty and the mirror derives the
//     thread from References, which is what it already does for every provider.
//   - There are no labels. `labels()` reports the mailbox list and `set_labels` is a MOVE, so one
//     message is in one folder and the app's own places do the rest from the state database.
//   - There is no history log. `cursor_now` and `changes_since` carry a JSON object of per folder
//     state: UIDVALIDITY, UIDNEXT and HIGHESTMODSEQ, which is the CONDSTORE cursor when the server
//     has one and the UID high water mark when it does not.
//
// One correction to the first of those, and it is load bearing. `MessageRef::thread_id` really is
// left empty, because at listing time all this has is a UID and there is nothing to derive a
// thread from. `RawHeaders::thread_id` is not: the mirror groups its `threads` rows by
// `provider_thread_id`, so an empty one there would collapse the entire mailbox into a single
// conversation. It carries the same key the mirror would derive, computed from the same rule, so
// the provider's id and the portable key agree by construction.
//
// A UIDVALIDITY change is `NeedsFullSync` and nothing cleverer. Mailspring remaps ids by hashing
// message content when a mailbox is renumbered; that is a lot of machinery to avoid one listing
// pass on an event that happens roughly never, and `changes::reconcile` is already written and
// already tested. Premature, so rejected.

use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};

use async_imap::imap_proto::{
    AttributeValue, MailboxDatum, MessageSection, Response, ResponseCode, SectionPath,
};

use crate::dto::{FlagPatch, MailConfig, Person};
use crate::provider::{
    Change, Changes, ListPage, MessageRef, Provider, ProviderError, ProviderLabel, ProviderProfile,
    ProviderSettings, RawHeaders, SentIds,
};

use super::folders::{self, Role, Roles};
use super::session::{self, Session};
use super::tls::Refused;
use super::{smtp, IMAP_KEY, SMTP_KEY};

/// How long a pooled connection is believed without a NOOP. A mail server drops an idle
/// connection without saying so, and the failure that follows is a parse error on the next
/// command rather than anything that names itself.
const IDLE_TRUST_MS: i64 = 30_000;

/// How long the mailbox list is believed before it is read again. Folders are made and renamed by
/// hand, so minutes is the right order of magnitude.
const FOLDERS_TRUST_MS: i64 = 5 * 60_000;

/// How often the flags of the recent range are read again on a server with neither CONDSTORE nor
/// QRESYNC. Every poll would be a fetch of hundreds of flag lines every twelve seconds.
const FLAG_SCAN_EVERY_MS: i64 = 5 * 60_000;

/// How far back that scan reaches, in UIDs per folder. Flags on old mail change rarely, and the
/// storage window means most of it is not on this device anyway.
const FLAG_SCAN_UIDS: u32 = 500;

/// How long a cursor may go without a listing pass on a server that cannot report deletions. The
/// engine's answer to `NeedsFullSync` is `changes::reconcile`, which is exactly the pass that
/// finds them.
const RECONCILE_EVERY_MS: i64 = 30 * 60_000;

/// UIDs asked about in one FETCH. Keeps the command line short enough for every server's parser
/// and the answer small enough to hold.
const FETCH_CHUNK: usize = 200;

/// A first sync lists a window, and `SINCE` is a date rather than a moment, so the window is
/// widened by a day rather than losing whatever sat on the boundary.
const SINCE_SLACK_MS: i64 = 86_400_000;

/// How many times to look for our own sent copy before appending one, and how long to wait
/// between looks. Several servers file the copy a moment after the SMTP transaction returns.
const SENT_LOOKS: u32 = 4;
const SENT_REST_MS: u64 = 700;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------
// Message ids
// ---------------------------------------------------------------------------------------------

/// The shape marker on an id, so a stored id from a later scheme is recognised rather than misread.
const ID_PREFIX: &str = "i1";

/// Where a message is: the mailbox, the numbering that mailbox was on, and the UID within it.
///
/// UIDVALIDITY is in the id rather than beside it because that is what makes a stale id
/// detectable. A mailbox that has been renumbered hands out the same UIDs for different messages,
/// and an id that cannot be told apart from a live one is an id that fetches somebody else's mail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub folder: String,
    pub uidvalidity: u32,
    pub uid: u32,
}

impl Located {
    pub fn id(&self) -> String {
        let folder = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.folder.as_bytes());
        format!("{ID_PREFIX}:{}:{}:{folder}", self.uidvalidity, self.uid)
    }

    /// Whether this id still names what it named when it was written.
    pub fn stale(&self, uidvalidity: u32) -> bool {
        self.uidvalidity != uidvalidity
    }
}

/// The id read back. `None` for anything this scheme did not write, which is a stored id from
/// another provider or a corrupted row, and either way not something to guess about.
///
/// The folder is base64 rather than written out because a mailbox name may contain any byte the
/// server likes, colons included, and a separator that can appear inside a field is not a
/// separator.
pub fn locate(id: &str) -> Option<Located> {
    let mut parts = id.splitn(4, ':');
    if parts.next()? != ID_PREFIX {
        return None;
    }
    let uidvalidity = parts.next()?.parse().ok()?;
    let uid = parts.next()?.parse().ok()?;
    let folder = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts.next()?)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())?;
    Some(Located {
        folder,
        uidvalidity,
        uid,
    })
}

// ---------------------------------------------------------------------------------------------
// The cursor
// ---------------------------------------------------------------------------------------------

/// The shape of the cursor object. A cursor written by an older build is not read: it is treated
/// as absent, which sends the account down the full sync path, which is correct and cheap.
const CURSOR_VERSION: u32 = 1;

/// What one mailbox looked like at the moment the cursor was taken.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderState {
    pub uidvalidity: u32,
    pub uidnext: u32,
    /// The CONDSTORE cursor, absent on a server that has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modseq: Option<u64>,
}

/// Everything `changes_since` needs to know about where it left off.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub v: u32,
    /// When the recent range's flags were last read in full. Only used on a server without
    /// CONDSTORE, where there is no other way to notice a message was read elsewhere.
    #[serde(default)]
    pub scanned_ms: i64,
    /// When a listing pass last happened. Only used on a server without QRESYNC, where there is
    /// no other way to notice a message was deleted elsewhere.
    #[serde(default)]
    pub listed_ms: i64,
    /// Keyed by mailbox path, and ordered, so the same state always serialises to the same string
    /// and a cursor can be compared against another one.
    #[serde(default)]
    pub folders: BTreeMap<String, FolderState>,
}

impl Cursor {
    pub fn read(text: &str) -> Option<Cursor> {
        serde_json::from_str::<Cursor>(text)
            .ok()
            .filter(|cursor| cursor.v == CURSOR_VERSION)
    }

    pub fn write(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Whether a mailbox has been renumbered under us, which invalidates every id in it.
    pub fn renumbered(&self, folder: &str, uidvalidity: u32) -> bool {
        self.folders
            .get(folder)
            .map(|held| held.uidvalidity != uidvalidity)
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------------------------
// The connection
// ---------------------------------------------------------------------------------------------

pub struct Imap {
    pub app: tauri::AppHandle,
    pub account_id: String,
    pub config: MailConfig,
    /// One connection, taken for one operation at a time.
    ///
    /// Not one connection for the life of the process: a mail server closes an idle one without
    /// warning, and the confusing failure that follows is the reason this is checked with a NOOP
    /// rather than assumed. Not one connection per call either, because a login is several round
    /// trips and the poll loop runs every twelve seconds.
    held: tokio::sync::Mutex<Option<Held>>,
}

impl Imap {
    pub fn new(app: tauri::AppHandle, account_id: &str, config: MailConfig) -> Imap {
        Imap {
            app,
            account_id: account_id.to_string(),
            config,
            held: tokio::sync::Mutex::new(None),
        }
    }

    /// The address this account is. `imap_connect` makes the lowercased address the account id,
    /// so it is the address; the username is the fallback for a registry written another way.
    fn address(&self) -> String {
        if self.account_id.contains('@') {
            return self.account_id.clone();
        }
        self.config.imap.username.clone()
    }

    fn password(&self, leg: &str) -> Result<String, ProviderError> {
        super::load_password(&self.account_id, leg)
            .map_err(ProviderError::Other)?
            .ok_or_else(|| {
                ProviderError::Auth("this account's password is not on this device".to_string())
            })
    }

    /// Takes the pooled connection for one operation, opening or replacing it when it has gone.
    ///
    /// The guard is held for the whole operation, so two calls never interleave on one socket.
    /// IMAP is one command at a time anyway, and a second connection to serialise against would
    /// be a second thing to keep alive for no gain.
    async fn ready(&self) -> Result<Connected<'_>, ProviderError> {
        let mut guard = self.held.lock().await;
        let now = now_ms();

        if let Some(live) = guard.as_mut() {
            if now - live.checked_ms > IDLE_TRUST_MS {
                match live.session.run("NOOP").await {
                    Ok(()) => live.checked_ms = now,
                    Err(_) => *guard = None,
                }
            }
        }

        if guard.is_none() {
            let password = self.password(IMAP_KEY)?;
            let session = session::open(&self.config.imap, &password)
                .await
                .map_err(refused)?;
            *guard = Some(Held::new(session, now).await);
        }

        Ok(Connected { guard })
    }
}

/// The pooled connection, taken.
struct Connected<'a> {
    guard: tokio::sync::MutexGuard<'a, Option<Held>>,
}

impl Connected<'_> {
    fn live(&mut self) -> &mut Held {
        self.guard.as_mut().expect("the connection was just opened")
    }

    /// Records how the operation went.
    ///
    /// A failure takes the connection with it unless it is one of the two that say nothing about
    /// the socket. Anything else may have left the stream halfway through a response, and reusing
    /// a stream in that state turns one failure into every failure after it.
    fn done<T>(mut self, outcome: Result<T, ProviderError>) -> Result<T, ProviderError> {
        if let Err(error) = &outcome {
            if !matches!(error, ProviderError::NeedsFullSync | ProviderError::NotFound) {
                *self.guard = None;
            }
        }
        outcome
    }
}

/// One live connection and what has been learned over it.
struct Held {
    session: Session,
    /// When the connection was last known to be answering.
    checked_ms: i64,
    /// The mailbox list and the roles read off it, with the moment it was read.
    folders: Option<(Mailboxes, i64)>,
    /// Whether an Archive has already been asked for over this connection, so a server that will
    /// not make one is asked once rather than every five minutes.
    archive_tried: bool,
    condstore: bool,
    qresync: bool,
}

/// A mailbox as SELECT described it.
#[derive(Debug, Clone, Default)]
struct Selected {
    uidvalidity: u32,
    uidnext: u32,
    modseq: Option<u64>,
    exists: u32,
}

/// The mailbox list with its roles worked out.
#[derive(Debug, Clone, Default)]
struct Mailboxes {
    listed: Vec<folders::Listed>,
    roles: Roles,
}

impl Mailboxes {
    /// Every mailbox a sync visits: everything selectable except the one carrying `\All`.
    ///
    /// A message in All Mail is by definition also somewhere else, so syncing it means holding
    /// every message twice and then deciding which copy is real. Proton Bridge duplicates every
    /// message into its own All Mail, which is what made Mailspring thrash messages between
    /// folders and burn CPU until they special cased Bridge by name. Skipping the role rather
    /// than the server generalises their fix to every mailbox that publishes an everything view.
    fn synced(&self) -> Vec<String> {
        self.listed
            .iter()
            .filter(|entry| entry.selectable())
            .map(|entry| entry.path.clone())
            .filter(|path| self.roles.all.as_deref() != Some(path.as_str()))
            .collect()
    }
}

/// One FETCH line, reduced to what this file reads out of it.
#[derive(Debug, Clone, Default)]
struct Fetched {
    uid: u32,
    flags: Vec<String>,
    internal_date_ms: i64,
    size: u32,
    /// The bytes of whichever `BODY[...]` section was asked for, and the MIME headers of one when
    /// both were asked for at once.
    section: Option<Vec<u8>>,
    mime: Option<Vec<u8>>,
}

impl Held {
    async fn new(session: Session, now: i64) -> Held {
        let mut held = Held {
            session,
            checked_ms: now,
            folders: None,
            archive_tried: false,
            condstore: false,
            qresync: false,
        };
        // QRESYNC implies CONDSTORE, and enabling it is what makes the server send VANISHED at
        // all. Both are asked for once per connection because ENABLE is per connection.
        if held.session.has("QRESYNC") && held.session.run("ENABLE QRESYNC").await.is_ok() {
            held.qresync = true;
            held.condstore = true;
        } else if held.session.has("CONDSTORE") && held.session.run("ENABLE CONDSTORE").await.is_ok()
        {
            held.condstore = true;
        }
        held
    }

    fn can(&self, capability: &str) -> bool {
        self.session.has(capability)
    }

    /// The mailbox list, read again once it is stale enough to be worth checking.
    async fn mailboxes(&mut self) -> Result<Mailboxes, ProviderError> {
        let now = now_ms();
        if let Some((held, read_ms)) = &self.folders {
            if now - read_ms < FOLDERS_TRUST_MS {
                return Ok(held.clone());
            }
        }

        let mut listed = folders::list(&mut self.session).await.map_err(refused)?;
        let mut roles = folders::assign(&listed);
        if roles.archive.is_none() && !self.archive_tried {
            self.archive_tried = true;
            // A server that will not make one leaves the role empty and the caller degrades. It is
            // not a reason to fail a sync.
            let _ = folders::ensure_archive(&mut self.session, &mut roles).await;
            // Read the list again when one was made, so the new mailbox is synced from this pass
            // rather than from whenever the list next goes stale. Anything filed into a mailbox
            // nothing is syncing looks to somebody like the message vanished.
            if roles.archive.is_some() {
                listed = folders::list(&mut self.session).await.map_err(refused)?;
                roles = folders::assign(&listed);
            }
        }

        let mailboxes = Mailboxes { listed, roles };
        self.folders = Some((mailboxes.clone(), now));
        Ok(mailboxes)
    }

    /// SELECT, every time rather than when the mailbox changes.
    ///
    /// A cached selection would have to be invalidated by every move, append and expunge, and one
    /// missed invalidation is a UIDNEXT that is quietly one behind, which is a message that never
    /// arrives. One extra round trip per operation is the cheaper mistake.
    async fn select(&mut self, path: &str) -> Result<Selected, ProviderError> {
        let command = if self.condstore {
            format!("SELECT {} (CONDSTORE)", session::quoted(path))
        } else {
            format!("SELECT {}", session::quoted(path))
        };
        let mut found = Selected::default();
        self.session
            .command(&command, |response| match response {
                Response::Data {
                    code: Some(ResponseCode::UidValidity(value)),
                    ..
                } => found.uidvalidity = *value,
                Response::Data {
                    code: Some(ResponseCode::UidNext(value)),
                    ..
                } => found.uidnext = *value,
                Response::Data {
                    code: Some(ResponseCode::HighestModSeq(value)),
                    ..
                } => found.modseq = Some(*value),
                Response::MailboxData(MailboxDatum::Exists(count)) => found.exists = *count,
                _ => {}
            })
            .await
            .map_err(|e| oops(&e))?;

        // A mailbox with no UIDVALIDITY has no stable ids at all, which is a server this app
        // cannot mirror without inventing something. Better to say so than to make ids up.
        if found.uidvalidity == 0 {
            return Err(ProviderError::Other(format!(
                "{path} did not report a UIDVALIDITY"
            )));
        }
        Ok(found)
    }

    async fn uid_search(&mut self, query: &str) -> Result<Vec<u32>, ProviderError> {
        let mut found: Vec<u32> = Vec::new();
        self.session
            .command(&format!("UID SEARCH {query}"), |response| {
                if let Response::MailboxData(MailboxDatum::Search(uids)) = response {
                    found.extend(uids.iter().copied());
                }
            })
            .await
            .map_err(|e| oops(&e))?;
        found.sort_unstable();
        found.dedup();
        Ok(found)
    }

    /// A UID FETCH, with any VANISHED lines it produced alongside the answers.
    async fn uid_fetch(
        &mut self,
        set: &str,
        items: &str,
    ) -> Result<(Vec<Fetched>, Vec<u32>), ProviderError> {
        let mut fetched: Vec<Fetched> = Vec::new();
        let mut vanished: Vec<u32> = Vec::new();
        self.session
            .command(&format!("UID FETCH {set} {items}"), |response| {
                match response {
                    Response::Fetch(_, attributes) => {
                        let line = fetched_from(attributes);
                        // A FETCH without a UID is a line about a message we cannot name, which
                        // happens when a server volunteers a flag change mid command.
                        if line.uid != 0 {
                            fetched.push(line);
                        }
                    }
                    Response::Vanished { uids, .. } => {
                        for range in uids {
                            vanished.extend(range.clone());
                        }
                    }
                    _ => {}
                }
            })
            .await
            .map_err(|e| oops(&e))?;
        Ok((fetched, vanished))
    }

    async fn uid_store(&mut self, set: &str, change: &str) -> Result<(), ProviderError> {
        // `.SILENT` so the server does not narrate every message back at us.
        self.session
            .run(&format!("UID STORE {set} {change}"))
            .await
            .map_err(|e| oops(&e))
    }

    /// A move, by whichever of the two ways the server has. `UID MOVE` is one command and one
    /// atomic result; without it the three step version is what MOVE was defined to replace, and
    /// it needs UIDPLUS so the expunge takes only the messages that were copied.
    async fn move_uids(&mut self, set: &str, to: &str) -> Result<(), ProviderError> {
        if self.can("MOVE") {
            return self
                .session
                .run(&format!("UID MOVE {set} {}", session::quoted(to)))
                .await
                .map_err(|e| oops(&e));
        }

        self.session
            .run(&format!("UID COPY {set} {}", session::quoted(to)))
            .await
            .map_err(|e| oops(&e))?;
        self.uid_store(set, "+FLAGS.SILENT (\\Deleted)").await?;
        if self.can("UIDPLUS") {
            self.session
                .run(&format!("UID EXPUNGE {set}"))
                .await
                .map_err(|e| oops(&e))?;
        }
        // Without UIDPLUS there is no way to expunge only these, and a bare EXPUNGE would take
        // every message anybody had ever marked deleted in this mailbox. The copy has been made
        // and the original is flagged; the server's own housekeeping finishes the job.
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Reading a FETCH
// ---------------------------------------------------------------------------------------------

fn fetched_from(attributes: &[AttributeValue<'_>]) -> Fetched {
    let mut line = Fetched::default();
    for attribute in attributes {
        match attribute {
            AttributeValue::Uid(uid) => line.uid = *uid,
            AttributeValue::Rfc822Size(size) => line.size = *size,
            AttributeValue::Flags(flags) => {
                line.flags = flags.iter().map(|flag| flag.to_string()).collect()
            }
            AttributeValue::InternalDate(stamp) => {
                line.internal_date_ms = internal_date_ms(stamp).unwrap_or(0)
            }
            AttributeValue::BodySection { section, data, .. } => {
                let Some(data) = data else { continue };
                match section {
                    Some(SectionPath::Part(_, Some(MessageSection::Mime))) => {
                        line.mime = Some(data.to_vec())
                    }
                    _ => line.section = Some(data.to_vec()),
                }
            }
            AttributeValue::Rfc822Header(Some(data)) => line.section = Some(data.to_vec()),
            _ => {}
        }
    }
    line
}

/// `INTERNALDATE` as the RFC writes it. The zone is part of the value and is honoured rather than
/// assumed, because a message filed at midnight in Auckland is a different day everywhere else.
fn internal_date_ms(stamp: &str) -> Option<i64> {
    chrono::DateTime::parse_from_str(stamp.trim(), "%d-%b-%Y %H:%M:%S %z")
        .ok()
        .map(|when| when.timestamp_millis())
}

fn has_flag(flags: &[String], wanted: &str) -> bool {
    flags.iter().any(|held| held.eq_ignore_ascii_case(wanted))
}

// ---------------------------------------------------------------------------------------------
// Folders as labels
// ---------------------------------------------------------------------------------------------

/// The prefix on a mailbox used as a label id.
///
/// Without it a folder somebody called "SPAM" would be indistinguishable from the mirror's own
/// system name for spam, and the mirror reads those five names to decide where a message lives.
const LABEL_PREFIX: &str = "folder:";

fn label_id(path: &str) -> String {
    format!("{LABEL_PREFIX}{path}")
}

fn label_path(id: &str) -> Option<&str> {
    id.strip_prefix(LABEL_PREFIX)
}

/// The label set for one message: where it sits, and the two flags the mirror reads a state out of.
///
/// The mirror knows five system names and nothing else, so a mailbox becomes one of them or none
/// of them, and none of them is what this app calls filed. The mailbox's own path rides along
/// under its prefix so `labels()` and the Label place are talking about the same thing.
fn labels_for(path: &str, role: Option<Role>, flags: &[String]) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    match role {
        Some(Role::Inbox) => labels.push("INBOX".to_string()),
        Some(Role::Sent) => labels.push("SENT".to_string()),
        Some(Role::Drafts) => labels.push("DRAFT".to_string()),
        Some(Role::Trash) => labels.push("TRASH".to_string()),
        Some(Role::Spam) => labels.push("SPAM".to_string()),
        _ => {}
    }
    if !has_flag(flags, "\\Seen") {
        labels.push("UNREAD".to_string());
    }
    if has_flag(flags, "\\Flagged") {
        labels.push("STARRED".to_string());
    }
    // A message flagged as a draft outside the Drafts mailbox is still a draft, and the composer
    // reads that flag to decide whether a row can be opened for editing.
    if has_flag(flags, "\\Draft") && !labels.iter().any(|held| held == "DRAFT") {
        labels.push("DRAFT".to_string());
    }
    labels.push(label_id(path));
    labels
}

/// The `\Seen` and `\Flagged` sides of a flag patch, as the two STORE arguments.
fn flag_changes(patch: &FlagPatch) -> (Vec<String>, Vec<String>) {
    let mut add = Vec::new();
    let mut remove = Vec::new();
    for (value, flag) in [(patch.seen, "\\Seen"), (patch.starred, "\\Flagged")] {
        match value {
            Some(true) => add.push(flag.to_string()),
            Some(false) => remove.push(flag.to_string()),
            None => {}
        }
    }
    (add, remove)
}

/// Where a flag patch says a message should end up, if anywhere.
///
/// Trash beats spam beats archive because all three are ways of saying "not in the inbox" and the
/// narrowest one is the one somebody meant. Clearing any of them means the inbox, which is where
/// Gmail's own label arithmetic puts a message that has stopped being archived.
fn destination(patch: &FlagPatch) -> Option<Role> {
    if patch.trashed == Some(true) {
        return Some(Role::Trash);
    }
    if patch.spam == Some(true) {
        return Some(Role::Spam);
    }
    if patch.archived == Some(true) {
        return Some(Role::Archive);
    }
    if patch.trashed == Some(false) || patch.spam == Some(false) || patch.archived == Some(false) {
        return Some(Role::Inbox);
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Headers
// ---------------------------------------------------------------------------------------------

/// A header block as the pairs the trait carries, with folding undone.
///
/// The pairs are kept in order and repeats are kept, because a message may carry `Received` a
/// dozen times and the caller reads the first of a name. Values stay exactly as the sender wrote
/// them: decoding encoded words is the MIME parser's job further down.
///
/// It stops at the blank line, so handing it a whole message reads the headers rather than every
/// line of the body that happens to contain a colon.
fn header_pairs(block: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(block);
    let mut pairs: Vec<(String, String)> = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            break;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some((_, value)) = pairs.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        pairs.push((name.trim().to_string(), value.trim().to_string()));
    }
    pairs
}

fn header_of<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(held, _)| held.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn bare_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('<').trim_end_matches('>').trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// The same rule `mirror::write::thread_key` applies, applied here so the provider's thread id and
/// the mirror's portable key are the same string rather than two guesses that usually agree.
///
/// This is what stands in for a thread id. The mirror groups its `threads` rows by the provider's
/// id, so leaving it empty would file the whole mailbox as one conversation.
fn thread_of(pairs: &[(String, String)], fallback: &str) -> String {
    let first = |name: &str| {
        header_of(pairs, name).and_then(|raw| raw.split_whitespace().find_map(bare_id))
    };
    first("references")
        .or_else(|| first("in-reply-to"))
        .or_else(|| header_of(pairs, "message-id").and_then(bare_id))
        .unwrap_or_else(|| fallback.to_string())
}

fn raw_headers(located: &Located, role: Option<Role>, line: &Fetched) -> RawHeaders {
    let id = located.id();
    let headers = header_pairs(line.section.as_deref().unwrap_or_default());
    RawHeaders {
        thread_id: thread_of(&headers, &id),
        id,
        label_ids: labels_for(&located.folder, role, &line.flags),
        internal_date_ms: line.internal_date_ms,
        size: line.size,
        // IMAP has no snippet and there is no honest way to make one out of a header block. The
        // list shows the subject until a body is fetched, which is what opening a thread does.
        snippet: String::new(),
        headers,
    }
}

// ---------------------------------------------------------------------------------------------
// Command arguments
// ---------------------------------------------------------------------------------------------

fn uid_set(uids: &[u32]) -> String {
    uids.iter()
        .map(|uid| uid.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// The `SINCE` argument, which is a date and not a moment, so the window is opened a day wider
/// rather than losing whatever sat on the boundary.
fn since(after_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(after_ms - SINCE_SLACK_MS)
        .unwrap_or_else(chrono::Utc::now)
        .format("%d-%b-%Y")
        .to_string()
}

/// A search term as a quoted string, with everything that could end the line taken out first.
/// A query is somebody's typing and it arrives here without having been anywhere near a parser.
fn searchable(query: &str) -> String {
    let cleaned: String = query
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    session::quoted(cleaned.trim())
}

// ---------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------

fn refused(refusal: Refused) -> ProviderError {
    match refusal {
        Refused::Auth(said) => ProviderError::Auth(said),
        Refused::Unreachable(said) => ProviderError::Network(said),
        other => ProviderError::Other(other.to_string()),
    }
}

/// A command that came back NO or BAD is the server declining, not the network failing, and the
/// engine's circuit breaker treats the two differently.
fn oops(error: &async_imap::error::Error) -> ProviderError {
    use async_imap::error::Error;
    match error {
        Error::Io(_) | Error::ConnectionLost => ProviderError::Network(error.to_string()),
        Error::No(said) | Error::Bad(said) => ProviderError::Other(said.clone()),
        other => ProviderError::Other(other.to_string()),
    }
}

// ---------------------------------------------------------------------------------------------
// The work
// ---------------------------------------------------------------------------------------------

/// One page of ids, which for this provider is one mailbox.
///
/// The page token is the index of the next mailbox to visit. Ids come back newest first within a
/// mailbox, which is the order that matters: `note_listed` writes them in the order they arrive
/// and hydration follows that order, so the newest mail is the first thing a first sync fills in.
/// Across mailboxes there is no order to give, because there is no cheap way to interleave them.
async fn list_page(
    live: &mut Held,
    after_ms: Option<i64>,
    page: Option<&str>,
) -> Result<ListPage, ProviderError> {
    let synced = live.mailboxes().await?.synced();
    let start: usize = page.and_then(|token| token.parse().ok()).unwrap_or(0);
    let Some(path) = synced.get(start).cloned() else {
        return Ok(ListPage::default());
    };

    let next_page = (start + 1 < synced.len()).then(|| (start + 1).to_string());

    // A mailbox that has been renamed or deleted since the list was read is an empty page and the
    // next one, not a failed crawl.
    let selected = match live.select(&path).await {
        Ok(selected) => selected,
        Err(ProviderError::Network(said)) => return Err(ProviderError::Network(said)),
        Err(_) => {
            return Ok(ListPage {
                next_page,
                ..ListPage::default()
            })
        }
    };
    // `UNDELETED` rather than `ALL`. A message somebody deleted in another client and has not
    // expunged yet is one every other client has already stopped showing, and it is also how the
    // copy-and-flag version of a move stops leaving the original behind on a server with no MOVE.
    let query = match after_ms {
        Some(ms) => format!("UNDELETED SINCE {}", since(ms)),
        None => "UNDELETED".to_string(),
    };
    let mut uids = live.uid_search(&query).await?;
    uids.reverse();

    Ok(ListPage {
        messages: uids
            .into_iter()
            .map(|uid| MessageRef {
                id: Located {
                    folder: path.clone(),
                    uidvalidity: selected.uidvalidity,
                    uid,
                }
                .id(),
                // Nothing to derive one from until the headers arrive. The mirror holds a
                // placeholder until then and `fetch_headers` replaces it with the real key.
                thread_id: String::new(),
            })
            .collect(),
        next_page,
        estimate: None,
    })
}

/// The cursor that means "everything from now on": every synced mailbox as it stands.
async fn cursor_of(live: &mut Held) -> Result<Cursor, ProviderError> {
    let now = now_ms();
    let mut cursor = Cursor {
        v: CURSOR_VERSION,
        scanned_ms: now,
        listed_ms: now,
        folders: BTreeMap::new(),
    };
    for path in live.mailboxes().await?.synced() {
        // A mailbox that is not there any more leaves no entry, so if it comes back it reads as
        // one the cursor has never seen and goes down the listing path.
        let selected = match live.select(&path).await {
            Ok(selected) => selected,
            Err(ProviderError::Network(said)) => return Err(ProviderError::Network(said)),
            Err(_) => continue,
        };
        cursor.folders.insert(
            path,
            FolderState {
                uidvalidity: selected.uidvalidity,
                uidnext: selected.uidnext,
                modseq: selected.modseq,
            },
        );
    }
    Ok(cursor)
}

/// One incremental pass over every synced mailbox.
///
/// Two strategies, and which one applies is the server's to decide. With QRESYNC enabled, one
/// `CHANGEDSINCE ... VANISHED` fetch per mailbox says exactly what changed and exactly what was
/// removed, and there is nothing to guess. Without it there are no VANISHED lines at all, so
/// deletions are invisible and pretending otherwise would leave rows in the mirror for mail that
/// is gone; the answer is a periodic `NeedsFullSync`, which is the listing pass `changes::reconcile`
/// already knows how to do.
async fn changes_of(live: &mut Held, cursor: &Cursor) -> Result<Changes, ProviderError> {
    let mailboxes = live.mailboxes().await?;
    let now = now_ms();

    // A cursor that has gone long enough without a listing is stale about deletions whatever else
    // it knows, and the honest thing is to say so before doing any work. The stamp is only moved
    // on by a pass in which every mailbox could be read precisely, so a server that answers
    // VANISHED for all of them never comes down this path and one that cannot comes down it every
    // half hour.
    if now - cursor.listed_ms > RECONCILE_EVERY_MS {
        return Err(ProviderError::NeedsFullSync);
    }
    let scan_flags = now - cursor.scanned_ms > FLAG_SCAN_EVERY_MS;

    let mut next = Cursor {
        v: CURSOR_VERSION,
        scanned_ms: if scan_flags { now } else { cursor.scanned_ms },
        listed_ms: cursor.listed_ms,
        folders: BTreeMap::new(),
    };
    let mut changes: Vec<Change> = Vec::new();
    // Set by any mailbox this pass could not read precisely, which is what holds the listing stamp
    // still. Decided per mailbox rather than per connection: a server can advertise CONDSTORE and
    // still answer NOMODSEQ for one mailbox, and that mailbox needs the fallback even though its
    // neighbours do not.
    let mut inexact = false;

    for path in mailboxes.synced() {
        // The mailbox list is up to a few minutes old, so one of them may have been renamed or
        // deleted since it was read. That is not a failed pass: it is a mailbox the listing pass
        // will account for.
        let selected = match live.select(&path).await {
            Ok(selected) => selected,
            Err(ProviderError::Network(said)) => return Err(ProviderError::Network(said)),
            Err(_) => {
                inexact = true;
                continue;
            }
        };
        let role = mailboxes.roles.role_of(&path);

        // The mailbox was renumbered, so every id this app holds for it names a different message
        // now. There is no remapping to attempt: the listing pass is the answer.
        if cursor.renumbered(&path, selected.uidvalidity) {
            return Err(ProviderError::NeedsFullSync);
        }

        // A server that will not say what the next UID is leaves nothing to compare against, and
        // treating a missing UIDNEXT as zero would report every message in the mailbox as new on
        // every poll. The listing pass is the fallback, which is what holding the stamp arranges.
        if selected.uidnext == 0 {
            inexact = true;
            continue;
        }

        let state = FolderState {
            uidvalidity: selected.uidvalidity,
            uidnext: selected.uidnext,
            modseq: selected.modseq,
        };
        next.folders.insert(path.clone(), state);

        let located = |uid: u32| Located {
            folder: path.clone(),
            uidvalidity: selected.uidvalidity,
            uid,
        };

        let Some(before) = cursor.folders.get(&path).copied() else {
            // A mailbox the cursor has never seen, because somebody made it or moved mail into it
            // since the last pass. Enumerating it here would ignore the storage window and could
            // pull in years of archived mail on one poll, so the listing pass does it instead:
            // that one knows the window and is the one the engine already has a recovery path for.
            return Err(ProviderError::NeedsFullSync);
        };
        if before.uidnext == 0 {
            inexact = true;
            continue;
        }

        match (live.qresync, before.modseq, selected.modseq) {
            (true, Some(was), Some(_)) => {
                let (lines, vanished) = live
                    .uid_fetch("1:*", &format!("(UID FLAGS) (CHANGEDSINCE {was} VANISHED)"))
                    .await?;
                for line in lines {
                    let id = located(line.uid).id();
                    if line.uid >= before.uidnext {
                        changes.push(Change::Added(MessageRef {
                            id,
                            thread_id: String::new(),
                        }));
                    } else {
                        changes.push(Change::LabelsChanged {
                            id,
                            labels: labels_for(&path, role, &line.flags),
                        });
                    }
                }
                for uid in vanished {
                    changes.push(Change::Deleted(located(uid).id()));
                }
            }
            _ => {
                // No VANISHED to be had here, so this mailbox is not fully accounted for and the
                // listing pass is what will notice anything that left it.
                inexact = true;

                // New mail is whatever sits at or above the UIDNEXT the cursor recorded. A range
                // ending in `*` always yields the highest UID even when it is below the low end,
                // which is the one bit of RFC 3501 range arithmetic that catches everybody, so the
                // answers are filtered rather than trusted.
                for uid in live
                    .uid_search(&format!("UID {}:* UNDELETED", before.uidnext.max(1)))
                    .await?
                    .into_iter()
                    .filter(|uid| *uid >= before.uidnext)
                {
                    changes.push(Change::Added(MessageRef {
                        id: located(uid).id(),
                        thread_id: String::new(),
                    }));
                }

                if scan_flags && before.uidnext > 1 {
                    let low = before.uidnext.saturating_sub(FLAG_SCAN_UIDS).max(1);
                    let high = before.uidnext.saturating_sub(1);
                    let (lines, _) = live.uid_fetch(&format!("{low}:{high}"), "(UID FLAGS)").await?;
                    for line in lines {
                        changes.push(Change::LabelsChanged {
                            id: located(line.uid).id(),
                            labels: labels_for(&path, role, &line.flags),
                        });
                    }
                }
            }
        }
    }

    if !inexact {
        next.listed_ms = now;
    }

    Ok(Changes {
        changes,
        cursor: next.write(),
        // Every mailbox is walked in one call. A page token would only mean holding the same
        // cursor still while the folder list changed underneath it.
        next_page: None,
    })
}

/// Metadata for a set of ids, one mailbox at a time.
///
/// Ids whose UIDVALIDITY no longer matches are left out of the answer rather than reported. That
/// is exactly how the caller learns they are gone: `hydrate::headers` deletes the rows for ids the
/// provider did not hand back.
async fn hydrate(live: &mut Held, ids: &[String]) -> Result<Vec<RawHeaders>, ProviderError> {
    let mailboxes = live.mailboxes().await?;
    let mut by_folder: BTreeMap<String, Vec<Located>> = BTreeMap::new();
    for id in ids {
        let Some(located) = locate(id) else { continue };
        by_folder
            .entry(located.folder.clone())
            .or_default()
            .push(located);
    }

    let mut out = Vec::with_capacity(ids.len());
    for (path, wanted) in by_folder {
        // A mailbox that has been renamed or deleted since the ids were written takes its ids with
        // it, which is a deletion rather than a failure. A socket that has stopped answering is
        // not, and swallowing that one would turn a network blip into a mirror full of holes.
        let selected = match live.select(&path).await {
            Ok(selected) => selected,
            Err(ProviderError::Network(said)) => return Err(ProviderError::Network(said)),
            Err(_) => continue,
        };
        let role = mailboxes.roles.role_of(&path);
        let uids: Vec<u32> = wanted
            .iter()
            .filter(|located| !located.stale(selected.uidvalidity))
            .map(|located| located.uid)
            .collect();

        for chunk in uids.chunks(FETCH_CHUNK) {
            let (lines, _) = live
                .uid_fetch(
                    &uid_set(chunk),
                    "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER])",
                )
                .await?;
            for line in &lines {
                let located = Located {
                    folder: path.clone(),
                    uidvalidity: selected.uidvalidity,
                    uid: line.uid,
                };
                out.push(raw_headers(&located, role, line));
            }
        }
    }
    Ok(out)
}

/// The bytes of one message, or `NotFound` when the id no longer names anything.
async fn body_of(live: &mut Held, id: &str) -> Result<Vec<u8>, ProviderError> {
    let located = locate(id).ok_or(ProviderError::NotFound)?;
    let selected = live.select(&located.folder).await?;
    if located.stale(selected.uidvalidity) {
        return Err(ProviderError::NotFound);
    }
    let (lines, _) = live
        .uid_fetch(&located.uid.to_string(), "(BODY.PEEK[])")
        .await?;
    lines
        .into_iter()
        .find_map(|line| line.section)
        .ok_or(ProviderError::NotFound)
}

/// One MIME part, decoded.
///
/// The part is fetched with its own MIME headers and the two are stapled back together into a
/// message of one part, which the ordinary parser then decodes. `BODY[1.2]` comes back in
/// whatever transfer encoding the sender used, and the headers that say which one are in
/// `BODY[1.2.MIME]`, so asking for both and letting `mail-parser` do the work is the only version
/// of this that handles base64, quoted-printable and 8bit without a second decoder in this file.
async fn part_of(
    live: &mut Held,
    message_id: &str,
    part_id: &str,
) -> Result<Vec<u8>, ProviderError> {
    let located = locate(message_id).ok_or(ProviderError::NotFound)?;
    // A part id is a dotted path of numbers and nothing else. Anything else would be somebody
    // else's idea of a part id going straight into a command line.
    if part_id.is_empty()
        || !part_id
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.')
    {
        return Err(ProviderError::NotFound);
    }
    let selected = live.select(&located.folder).await?;
    if located.stale(selected.uidvalidity) {
        return Err(ProviderError::NotFound);
    }

    let (lines, _) = live
        .uid_fetch(
            &located.uid.to_string(),
            &format!("(BODY.PEEK[{part_id}.MIME] BODY.PEEK[{part_id}])"),
        )
        .await?;
    let line = lines.into_iter().next().ok_or(ProviderError::NotFound)?;
    let body = line.section.ok_or(ProviderError::NotFound)?;

    let mut rebuilt = line.mime.unwrap_or_default();
    if !rebuilt.is_empty() && !rebuilt.ends_with(b"\r\n") {
        rebuilt.extend_from_slice(b"\r\n");
    }
    rebuilt.extend_from_slice(b"\r\n");
    rebuilt.extend_from_slice(&body);

    let parsed = mail_parser::MessageParser::default()
        .parse(&rebuilt)
        .ok_or(ProviderError::NotFound)?;
    Ok(match parsed.parts.first().map(|part| &part.body) {
        Some(mail_parser::PartType::Binary(bytes))
        | Some(mail_parser::PartType::InlineBinary(bytes)) => bytes.to_vec(),
        Some(mail_parser::PartType::Text(text)) | Some(mail_parser::PartType::Html(text)) => {
            text.as_bytes().to_vec()
        }
        // Nothing the parser recognised, so the bytes as they came is the best answer available.
        _ => body,
    })
}

/// A flag change and, if the patch asked for one, a move.
///
/// Flags go first because a move gives the message a new UID that this call has no way to learn:
/// only UIDPLUS reports it, and only for a copy. The id in the mirror is stale from the moment the
/// move lands, which the next pass corrects by seeing the message leave one mailbox and arrive in
/// another. The conversation survives it because the thread key comes from References and not from
/// where the message happens to be.
async fn apply_flags(
    live: &mut Held,
    ids: &[String],
    patch: &FlagPatch,
) -> Result<(), ProviderError> {
    let mailboxes = live.mailboxes().await?;
    let (add, remove) = flag_changes(patch);
    let target = match destination(patch) {
        None => None,
        Some(role) => {
            let held = match role {
                Role::Trash => mailboxes.roles.trash.clone(),
                Role::Spam => mailboxes.roles.spam.clone(),
                Role::Archive => mailboxes.roles.archive.clone(),
                _ => mailboxes.roles.inbox.clone(),
            };
            // Nowhere to put it is a failure and not a silent success. Returning `Ok` would drop
            // the outbox row, leave the mirror saying the message was filed and let the next pass
            // quietly put it back, which looks to somebody like the app undoing their keystroke.
            Some(held.ok_or_else(|| {
                ProviderError::Other(format!(
                    "this account has no {} mailbox to file into",
                    match role {
                        Role::Trash => "Trash",
                        Role::Spam => "Spam",
                        Role::Archive => "Archive",
                        _ => "Inbox",
                    }
                ))
            })?)
        }
    };

    let mut by_folder: BTreeMap<String, Vec<Located>> = BTreeMap::new();
    for id in ids {
        let Some(located) = locate(id) else { continue };
        by_folder
            .entry(located.folder.clone())
            .or_default()
            .push(located);
    }

    for (path, wanted) in by_folder {
        let selected = live.select(&path).await?;
        let uids: Vec<u32> = wanted
            .iter()
            .filter(|located| !located.stale(selected.uidvalidity))
            .map(|located| located.uid)
            .collect();
        if uids.is_empty() {
            continue;
        }
        let set = uid_set(&uids);

        if !add.is_empty() {
            live.uid_store(&set, &format!("+FLAGS.SILENT ({})", add.join(" ")))
                .await?;
        }
        if !remove.is_empty() {
            live.uid_store(&set, &format!("-FLAGS.SILENT ({})", remove.join(" ")))
                .await?;
        }
        if let Some(target) = &target {
            if target != &path {
                live.move_uids(&set, target).await?;
            }
        }
    }
    Ok(())
}

/// A label change, which on IMAP is a move. Adding a mailbox means going there; removing the only
/// one somebody had means leaving, and leaving means the Archive.
async fn apply_labels(
    live: &mut Held,
    ids: &[String],
    add: &[String],
    remove: &[String],
) -> Result<(), ProviderError> {
    let mailboxes = live.mailboxes().await?;
    let target = add
        .iter()
        .find_map(|id| label_path(id))
        .map(str::to_string)
        .or_else(|| {
            remove
                .iter()
                .any(|id| label_path(id).is_some())
                .then(|| mailboxes.roles.archive.clone())
                .flatten()
        });
    let Some(target) = target else {
        return Ok(());
    };

    let mut by_folder: BTreeMap<String, Vec<Located>> = BTreeMap::new();
    for id in ids {
        let Some(located) = locate(id) else { continue };
        by_folder
            .entry(located.folder.clone())
            .or_default()
            .push(located);
    }

    for (path, wanted) in by_folder {
        if path == target {
            continue;
        }
        let selected = live.select(&path).await?;
        let uids: Vec<u32> = wanted
            .iter()
            .filter(|located| !located.stale(selected.uidvalidity))
            .map(|located| located.uid)
            .collect();
        if uids.is_empty() {
            continue;
        }
        live.move_uids(&uid_set(&uids), &target).await?;
    }
    Ok(())
}

/// The sent copy, filed the way every provider needs it filed.
///
/// Some servers file it themselves the moment the SMTP transaction completes, some never do, and
/// there is no capability that says which. So the Sent mailbox is searched for the `Message-ID`
/// this message went out with, a few times over a couple of seconds because the filing is not
/// always finished when the send returns. Exactly one hit means the server did it and that UID is
/// the answer. None means it did not, and one is appended. More than one means something already
/// went wrong and appending a third copy would not improve it.
///
/// The caller swallows whatever comes back: the mail has already left, so none of this may turn
/// into a send to retry. The `Result` is here so a connection that broke on the way is noticed and
/// replaced rather than reused.
async fn file_sent(
    live: &mut Held,
    message_id: Option<&str>,
    raw: &[u8],
) -> Result<Option<Located>, ProviderError> {
    let Some(sent) = live.mailboxes().await?.roles.sent else {
        return Ok(None);
    };

    if let Some(message_id) = message_id {
        let query = format!("HEADER Message-ID {}", session::quoted(message_id));
        for attempt in 0..SENT_LOOKS {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(SENT_REST_MS)).await;
            }
            let selected = live.select(&sent).await?;
            let hits = live.uid_search(&query).await?;
            if hits.len() == 1 {
                return Ok(Some(Located {
                    folder: sent,
                    uidvalidity: selected.uidvalidity,
                    uid: hits[0],
                }));
            }
            // More than one already means something went wrong upstream, and a third copy would
            // not improve it.
            if !hits.is_empty() {
                return Ok(None);
            }
        }
    }

    let before = live.select(&sent).await?.uidnext;
    live.session
        .append(&sent, "\\Seen", raw)
        .await
        .map_err(|e| oops(&e))?;

    // The appended UID is only reported under UIDPLUS, and it rides in a response code that
    // `command` does not keep, so it is looked up instead: it is at or above the UIDNEXT that was
    // there a moment ago, and it is the newest thing in the mailbox.
    let selected = live.select(&sent).await?;
    Ok(live
        .uid_search(&format!("UID {}:*", before.max(1)))
        .await?
        .into_iter()
        .filter(|uid| *uid >= before)
        .max()
        .map(|uid| Located {
            folder: sent,
            uidvalidity: selected.uidvalidity,
            uid,
        }))
}

/// The envelope, read back out of the built message.
///
/// `smtp::send` takes its recipients rather than parsing them, because Bcc must reach the server
/// and must not reach the message; the trait hands this provider only the bytes, so this is the
/// one place that reads them back. Bcc is included, and the builder is the thing that decides
/// whether the header survives into what is sent.
fn envelope(raw: &[u8]) -> (String, Vec<String>) {
    // Headers only. A message with a twenty megabyte attachment on it is not worth parsing twice
    // to find out who it is addressed to.
    let Some(message) = mail_parser::MessageParser::default().parse_headers(raw) else {
        return (String::new(), Vec::new());
    };
    let from = addresses(message.from()).into_iter().next().unwrap_or_default();
    let mut to = Vec::new();
    for list in [message.to(), message.cc(), message.bcc()] {
        for address in addresses(list) {
            if !to.contains(&address) {
                to.push(address);
            }
        }
    }
    (from, to)
}

fn addresses(address: Option<&mail_parser::Address<'_>>) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |value: Option<&str>| {
        if let Some(value) = value {
            let value = value.trim().to_lowercase();
            if !value.is_empty() {
                out.push(value);
            }
        }
    };
    match address {
        Some(mail_parser::Address::List(list)) => {
            for entry in list {
                push(entry.address.as_deref());
            }
        }
        Some(mail_parser::Address::Group(groups)) => {
            for group in groups {
                for entry in &group.addresses {
                    push(entry.address.as_deref());
                }
            }
        }
        None => {}
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The trait
// ---------------------------------------------------------------------------------------------

impl Provider for Imap {
    fn list(
        &self,
        after_ms: Option<i64>,
        page: Option<&str>,
    ) -> impl Future<Output = Result<ListPage, ProviderError>> + Send {
        let page = page.map(|token| token.to_string());
        async move {
            let mut open = self.ready().await?;
            let outcome = list_page(open.live(), after_ms, page.as_deref()).await;
            open.done(outcome)
        }
    }

    fn changes_since(
        &self,
        cursor: &str,
        _page: Option<&str>,
    ) -> impl Future<Output = Result<Changes, ProviderError>> + Send {
        let cursor = cursor.to_string();
        async move {
            // A cursor this build cannot read is one from another provider or another scheme, and
            // guessing at one would quietly sync the wrong thing.
            let Some(cursor) = Cursor::read(&cursor) else {
                return Err(ProviderError::NeedsFullSync);
            };
            let mut open = self.ready().await?;
            let outcome = changes_of(open.live(), &cursor).await;
            open.done(outcome)
        }
    }

    fn cursor_now(&self) -> impl Future<Output = Result<String, ProviderError>> + Send {
        async move {
            let mut open = self.ready().await?;
            let outcome = cursor_of(open.live()).await.map(|cursor| cursor.write());
            open.done(outcome)
        }
    }

    fn fetch_headers(
        &self,
        ids: &[String],
    ) -> impl Future<Output = Result<Vec<RawHeaders>, ProviderError>> + Send {
        let ids = ids.to_vec();
        async move {
            let mut open = self.ready().await?;
            let outcome = hydrate(open.live(), &ids).await;
            open.done(outcome)
        }
    }

    fn fetch_body(&self, id: &str) -> impl Future<Output = Result<Vec<u8>, ProviderError>> + Send {
        let id = id.to_string();
        async move {
            let mut open = self.ready().await?;
            let outcome = body_of(open.live(), &id).await;
            open.done(outcome)
        }
    }

    fn fetch_attachment(
        &self,
        message_id: &str,
        attachment_id: &str,
    ) -> impl Future<Output = Result<Vec<u8>, ProviderError>> + Send {
        let (message_id, attachment_id) = (message_id.to_string(), attachment_id.to_string());
        async move {
            let mut open = self.ready().await?;
            let outcome = part_of(open.live(), &message_id, &attachment_id).await;
            open.done(outcome)
        }
    }

    fn set_flags(
        &self,
        ids: &[String],
        patch: &FlagPatch,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send {
        let (ids, patch) = (ids.to_vec(), patch.clone());
        async move {
            let mut open = self.ready().await?;
            let outcome = apply_flags(open.live(), &ids, &patch).await;
            open.done(outcome)
        }
    }

    /// The mailbox list, which is what stands in for labels. A mailbox with a role is a system
    /// one: the app draws its own name for it, and somebody renaming it would be renaming a place
    /// rather than a folder.
    fn labels(&self) -> impl Future<Output = Result<Vec<ProviderLabel>, ProviderError>> + Send {
        async move {
            let mut open = self.ready().await?;
            let outcome = open.live().mailboxes().await.map(|mailboxes| {
                mailboxes
                    .listed
                    .iter()
                    .filter(|entry| entry.selectable())
                    .map(|entry| ProviderLabel {
                        id: label_id(&entry.path),
                        name: entry.path.clone(),
                        kind: match mailboxes.roles.role_of(&entry.path) {
                            Some(_) => "system".to_string(),
                            None => "user".to_string(),
                        },
                    })
                    .collect()
            });
            open.done(outcome)
        }
    }

    fn set_labels(
        &self,
        ids: &[String],
        add: &[String],
        remove: &[String],
    ) -> impl Future<Output = Result<(), ProviderError>> + Send {
        let (ids, add, remove) = (ids.to_vec(), add.to_vec(), remove.to_vec());
        async move {
            let mut open = self.ready().await?;
            let outcome = apply_labels(open.live(), &ids, &add, &remove).await;
            open.done(outcome)
        }
    }

    /// Nothing is built here: the bytes arrive built, with the `Message-ID` already stamped on
    /// them by `send::queue`, which is what makes a retry after a crash detectable.
    fn send(
        &self,
        raw: &[u8],
        _thread_hint: Option<&str>,
    ) -> impl Future<Output = Result<SentIds, ProviderError>> + Send {
        let raw = raw.to_vec();
        async move {
            let password = self
                .password(SMTP_KEY)
                .or_else(|_| self.password(IMAP_KEY))?;
            let (from, to) = envelope(&raw);
            let from = if from.is_empty() { self.address() } else { from };
            if to.is_empty() {
                return Err(ProviderError::Other(
                    "that message names nobody to send it to".to_string(),
                ));
            }
            smtp::send(&self.config.smtp, &password, &from, &to, &raw)
                .await
                .map_err(refused)?;

            let headers = header_pairs(&raw);
            let message_id = header_of(&headers, "message-id").and_then(bare_id);
            let thread_id = thread_of(&headers, "");

            // The mail has gone, so a copy that could not be filed is a copy that is missing
            // rather than a send to retry. Retrying would send the message twice.
            let filed = match self.ready().await {
                Ok(mut open) => {
                    let outcome = file_sent(open.live(), message_id.as_deref(), &raw).await;
                    open.done(outcome).unwrap_or(None)
                }
                Err(_) => None,
            };

            Ok(SentIds {
                id: filed.map(|located| located.id()).unwrap_or_default(),
                thread_id,
            })
        }
    }

    /// A draft is an APPEND into the Drafts mailbox, and an update is a new one plus the removal
    /// of the old, because IMAP has no way to edit a message in place.
    fn draft_put(
        &self,
        provider_draft_id: Option<&str>,
        raw: &[u8],
        _thread_hint: Option<&str>,
    ) -> impl Future<Output = Result<String, ProviderError>> + Send {
        let existing = provider_draft_id.map(|id| id.to_string());
        let raw = raw.to_vec();
        async move {
            let mut open = self.ready().await?;
            let outcome = put_draft(open.live(), existing.as_deref(), &raw).await;
            open.done(outcome)
        }
    }

    fn draft_delete(
        &self,
        provider_draft_id: &str,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send {
        let id = provider_draft_id.to_string();
        async move {
            let mut open = self.ready().await?;
            let outcome = remove_draft(open.live(), &id).await;
            open.done(outcome)
        }
    }

    /// `UID SEARCH TEXT` across the synced mailboxes, one per page.
    ///
    /// TEXT is the whole message, headers and body, and it is the only search term every server
    /// implements the same way. Nothing here parses the query: the local index already did that,
    /// and this is the pass that reaches past what the device holds.
    fn search(
        &self,
        query: &str,
        page: Option<&str>,
    ) -> impl Future<Output = Result<ListPage, ProviderError>> + Send {
        let term = searchable(query);
        let page = page.map(|token| token.to_string());
        async move {
            // Two characters inside the quotes is not a search, it is a scan of the whole mailbox.
            if term.len() <= 4 {
                return Ok(ListPage::default());
            }
            let mut open = self.ready().await?;
            let outcome = search_page(open.live(), &term, page.as_deref()).await;
            open.done(outcome)
        }
    }

    /// An IMAP account has one address and no server-side signature. Nothing is asked of the
    /// server here, because there is nothing on it to ask.
    fn settings(&self) -> impl Future<Output = Result<ProviderSettings, ProviderError>> + Send {
        async move {
            Ok(ProviderSettings {
                aliases: vec![self.address()],
                signature: String::new(),
            })
        }
    }

    /// The address, the name the account was added under, and what the mailboxes add up to.
    fn profile(&self) -> impl Future<Output = Result<ProviderProfile, ProviderError>> + Send {
        async move {
            let name = crate::accounts::find(&self.app, &self.account_id)
                .ok()
                .flatten()
                .map(|entry| entry.name)
                .unwrap_or_default();

            let mut open = self.ready().await?;
            let outcome = count_messages(open.live()).await;
            let messages_total = open.done(outcome)?;

            Ok(ProviderProfile {
                email: self.address(),
                name,
                messages_total,
            })
        }
    }

    /// An IMAP server has no address book. The mirror's own correspondents are the whole of
    /// autocomplete on these accounts, which is what the empty list means here.
    fn contacts(&self) -> impl Future<Output = Result<Vec<Person>, ProviderError>> + Send {
        async { Ok(Vec::new()) }
    }
}

/// One page of a provider search, which for this provider is one mailbox.
async fn search_page(
    live: &mut Held,
    term: &str,
    page: Option<&str>,
) -> Result<ListPage, ProviderError> {
    let synced = live.mailboxes().await?.synced();
    let start: usize = page.and_then(|token| token.parse().ok()).unwrap_or(0);
    let Some(path) = synced.get(start).cloned() else {
        return Ok(ListPage::default());
    };
    let selected = live.select(&path).await?;
    let mut uids = live.uid_search(&format!("UNDELETED TEXT {term}")).await?;
    uids.reverse();
    Ok(ListPage {
        messages: uids
            .into_iter()
            .map(|uid| MessageRef {
                id: Located {
                    folder: path.clone(),
                    uidvalidity: selected.uidvalidity,
                    uid,
                }
                .id(),
                thread_id: String::new(),
            })
            .collect(),
        next_page: (start + 1 < synced.len()).then(|| (start + 1).to_string()),
        estimate: None,
    })
}

/// What the synced mailboxes add up to, which is the closest thing to Gmail's own total.
async fn count_messages(live: &mut Held) -> Result<u32, ProviderError> {
    let mut total = 0u32;
    for path in live.mailboxes().await?.synced() {
        if let Ok(selected) = live.select(&path).await {
            total = total.saturating_add(selected.exists);
        }
    }
    Ok(total)
}

/// Writes a draft, and only then takes the old copy away. The other order loses the draft when
/// the append fails.
async fn put_draft(
    live: &mut Held,
    existing: Option<&str>,
    raw: &[u8],
) -> Result<String, ProviderError> {
    let drafts = live
        .mailboxes()
        .await?
        .roles
        .drafts
        .ok_or_else(|| ProviderError::Other("this account has no Drafts mailbox".to_string()))?;

    let before = live.select(&drafts).await?.uidnext;
    live.session
        .append(&drafts, "\\Draft \\Seen", raw)
        .await
        .map_err(|e| oops(&e))?;

    // The appended UID is only reported under UIDPLUS, and it rides in a response code that
    // `command` does not keep, so it is looked up: it is at or above the UIDNEXT that was there a
    // moment ago, and it is the newest thing in the mailbox.
    let selected = live.select(&drafts).await?;
    let uid = live
        .uid_search(&format!("UID {}:*", before.max(1)))
        .await?
        .into_iter()
        .filter(|uid| *uid >= before)
        .max()
        .ok_or_else(|| ProviderError::Other("the draft was written but not found".to_string()))?;

    if let Some(existing) = existing {
        let _ = remove_draft(live, existing).await;
    }

    Ok(Located {
        folder: drafts,
        uidvalidity: selected.uidvalidity,
        uid,
    }
    .id())
}

/// Marks a draft deleted and, where the server can do it precisely, expunges it.
///
/// A bare EXPUNGE would take every message anybody had ever flagged deleted in that mailbox, on
/// any device, so without UIDPLUS the flag is set and the server's own housekeeping is left to
/// finish. A draft that lingers with `\Deleted` on it is invisible in every client that respects
/// the flag, which is the outcome that was wanted.
async fn remove_draft(live: &mut Held, id: &str) -> Result<(), ProviderError> {
    let Some(located) = locate(id) else {
        return Ok(());
    };
    let Ok(selected) = live.select(&located.folder).await else {
        return Ok(());
    };
    if located.stale(selected.uidvalidity) {
        return Ok(());
    }
    let set = located.uid.to_string();
    live.uid_store(&set, "+FLAGS.SILENT (\\Deleted)").await?;
    if live.can("UIDPLUS") {
        let _ = live.session.run(&format!("UID EXPUNGE {set}")).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(listed: &[&str]) -> Vec<String> {
        listed.iter().map(|flag| flag.to_string()).collect()
    }

    #[test]
    fn an_id_carries_the_mailbox_the_numbering_and_the_uid() {
        let located = Located {
            folder: "INBOX".to_string(),
            uidvalidity: 1_408_806_928,
            uid: 4827,
        };
        let id = located.id();
        assert_eq!(locate(&id), Some(located));
    }

    /// The whole reason UIDVALIDITY is inside the id: a mailbox that has been renumbered hands out
    /// the same UIDs for different messages, and an id that cannot be told apart from a live one
    /// fetches somebody else's mail.
    #[test]
    fn a_renumbered_mailbox_makes_an_old_id_detectably_stale() {
        let located = locate(
            &Located {
                folder: "INBOX".to_string(),
                uidvalidity: 100,
                uid: 7,
            }
            .id(),
        )
        .expect("an id this scheme wrote");
        assert!(located.stale(101));
        assert!(!located.stale(100));
    }

    #[test]
    fn a_mailbox_named_with_anything_at_all_survives_the_round_trip() {
        for path in [
            "INBOX",
            "[Gmail]/Sent Mail",
            "Work:Invoices",
            "INBOX.Ünread",
            "a/b/c:1:2:3",
            "",
        ] {
            let located = Located {
                folder: path.to_string(),
                uidvalidity: 9,
                uid: 1,
            };
            assert_eq!(locate(&located.id()).map(|back| back.folder), Some(path.to_string()));
        }
    }

    #[test]
    fn an_id_from_somewhere_else_is_not_guessed_at() {
        assert_eq!(locate("18f9a2b3c4d50000"), None);
        assert_eq!(locate("i2:1:2:SU5CT1g"), None);
        assert_eq!(locate("i1:notanumber:2:SU5CT1g"), None);
        assert_eq!(locate("i1:1:2"), None);
        assert_eq!(locate("i1:1:2:!!!not base64!!!"), None);
    }

    #[test]
    fn a_cursor_survives_the_round_trip_through_the_meta_table() {
        let mut cursor = Cursor {
            v: CURSOR_VERSION,
            scanned_ms: 1_785_808_800_000,
            listed_ms: 1_785_808_700_000,
            folders: BTreeMap::new(),
        };
        cursor.folders.insert(
            "INBOX".to_string(),
            FolderState {
                uidvalidity: 1_408_806_928,
                uidnext: 4_828,
                modseq: Some(9_912_351),
            },
        );
        cursor.folders.insert(
            "Archive".to_string(),
            FolderState {
                uidvalidity: 3,
                uidnext: 1,
                modseq: None,
            },
        );
        let written = cursor.write();
        assert_eq!(Cursor::read(&written), Some(cursor));
        // Ordered, so the same state is the same string and two cursors can be compared.
        assert_eq!(written, Cursor::read(&written).expect("a cursor").write());
    }

    #[test]
    fn a_cursor_this_build_did_not_write_is_not_read() {
        assert_eq!(Cursor::read("9912351"), None);
        assert_eq!(Cursor::read(""), None);
        assert_eq!(Cursor::read(r#"{"v":2,"folders":{}}"#), None);
    }

    #[test]
    fn a_renumbered_mailbox_is_recognised_against_the_cursor() {
        let cursor = Cursor::read(
            r#"{"v":1,"folders":{"INBOX":{"uidvalidity":100,"uidnext":50}}}"#,
        )
        .expect("a cursor");
        assert!(cursor.renumbered("INBOX", 101));
        assert!(!cursor.renumbered("INBOX", 100));
        // A mailbox the cursor has never seen has not been renumbered, it is new.
        assert!(!cursor.renumbered("Archive", 7));
    }

    #[test]
    fn a_mailbox_becomes_the_system_name_the_mirror_reads() {
        assert_eq!(
            labels_for("INBOX", Some(Role::Inbox), &flags(&["\\Seen"])),
            ["INBOX", "folder:INBOX"]
        );
        assert_eq!(
            labels_for("Trash", Some(Role::Trash), &[]),
            ["TRASH", "UNREAD", "folder:Trash"]
        );
        assert_eq!(
            labels_for("Work", None, &flags(&["\\Seen", "\\Flagged"])),
            ["STARRED", "folder:Work"]
        );
    }

    /// A folder somebody called "SPAM" must not read as the mirror's own spam label, which is what
    /// the prefix on a folder label is for.
    #[test]
    fn a_folder_named_after_a_system_label_is_still_only_a_folder() {
        let labels = labels_for("SPAM", None, &flags(&["\\Seen"]));
        assert_eq!(labels, ["folder:SPAM"]);
        assert!(!labels.iter().any(|held| held == "SPAM"));
    }

    #[test]
    fn a_flag_patch_is_two_stores_and_at_most_one_move() {
        let (add, remove) = flag_changes(&FlagPatch {
            seen: Some(true),
            starred: Some(false),
            ..FlagPatch::default()
        });
        assert_eq!(add, ["\\Seen"]);
        assert_eq!(remove, ["\\Flagged"]);
        assert_eq!(destination(&FlagPatch::default()), None);
        assert_eq!(
            destination(&FlagPatch {
                archived: Some(true),
                ..FlagPatch::default()
            }),
            Some(Role::Archive)
        );
        // The narrowest of the three wins, because all three mean "not in the inbox".
        assert_eq!(
            destination(&FlagPatch {
                archived: Some(true),
                spam: Some(true),
                trashed: Some(true),
                ..FlagPatch::default()
            }),
            Some(Role::Trash)
        );
        // Clearing any of them means it comes back.
        assert_eq!(
            destination(&FlagPatch {
                trashed: Some(false),
                ..FlagPatch::default()
            }),
            Some(Role::Inbox)
        );
    }

    #[test]
    fn a_header_block_becomes_pairs_with_the_folding_undone() {
        let block = b"From: Ana Ruiz <ana@example.com>\r\n\
                      Subject: The lease\r\n\
                      References: <a@example.com>\r\n <b@example.com>\r\n\
                      Received: from one\r\n\
                      Received: from two\r\n\r\n";
        let pairs = header_pairs(block);
        assert_eq!(header_of(&pairs, "from"), Some("Ana Ruiz <ana@example.com>"));
        assert_eq!(
            header_of(&pairs, "REFERENCES"),
            Some("<a@example.com> <b@example.com>")
        );
        // First match wins, and both are kept.
        assert_eq!(header_of(&pairs, "received"), Some("from one"));
        assert_eq!(
            pairs.iter().filter(|(name, _)| name == "Received").count(),
            2
        );
    }

    /// A whole message goes in when a send reads its own `Message-ID` back, and a body line with a
    /// colon in it is not a header.
    #[test]
    fn the_headers_end_at_the_blank_line() {
        let raw = b"Subject: The lease\r\n\r\nNote to self: this is not a header.\r\n";
        let pairs = header_pairs(raw);
        assert_eq!(pairs.len(), 1);
        assert_eq!(header_of(&pairs, "note to self"), None);
    }

    /// The key has to be the one `mirror::write::thread_key` would derive from the same headers,
    /// or the provider's own thread id and the portable key disagree and a conversation splits.
    #[test]
    fn the_thread_key_is_the_mirrors_rule() {
        let with_references = header_pairs(
            b"Message-ID: <c@example.com>\r\nIn-Reply-To: <b@example.com>\r\n\
              References: <a@example.com> <b@example.com>\r\n",
        );
        assert_eq!(thread_of(&with_references, "fallback"), "a@example.com");

        let reply_only = header_pairs(b"Message-ID: <c@example.com>\r\nIn-Reply-To: <b@example.com>\r\n");
        assert_eq!(thread_of(&reply_only, "fallback"), "b@example.com");

        let root = header_pairs(b"Message-ID: <c@example.com>\r\n");
        assert_eq!(thread_of(&root, "fallback"), "c@example.com");

        // A message with no identity at all keeps its own id, so it is a conversation of one
        // rather than joining an empty group with every other such message.
        assert_eq!(thread_of(&header_pairs(b"Subject: none\r\n"), "i1:1:2:x"), "i1:1:2:x");
    }

    #[test]
    fn an_internal_date_is_read_in_the_zone_the_server_wrote_it_in() {
        assert_eq!(
            internal_date_ms("17-Jul-1996 02:44:25 -0700"),
            Some(837_596_665_000)
        );
        assert_eq!(internal_date_ms("not a date"), None);
    }

    #[test]
    fn a_search_term_cannot_carry_a_line_ending_into_a_command() {
        assert_eq!(searchable("lease"), "\"lease\"");
        assert_eq!(searchable("a\r\nUID SEARCH ALL"), "\"aUID SEARCH ALL\"");
        assert_eq!(searchable("say \"hello\""), "\"say \\\"hello\\\"\"");
        assert_eq!(searchable("back\\slash"), "\"back\\\\slash\"");
    }

    #[test]
    fn a_uid_set_is_the_comma_list_a_server_expects() {
        assert_eq!(uid_set(&[1, 2, 40]), "1,2,40");
        assert_eq!(uid_set(&[]), "");
    }

    #[test]
    fn everything_selectable_is_synced_except_the_mailbox_that_holds_everything() {
        let listed = vec![
            folders::Listed {
                path: "INBOX".to_string(),
                delimiter: "/".to_string(),
                flags: Vec::new(),
            },
            folders::Listed {
                path: "[Gmail]".to_string(),
                delimiter: "/".to_string(),
                flags: vec!["\\Noselect".to_string()],
            },
            folders::Listed {
                path: "[Gmail]/All Mail".to_string(),
                delimiter: "/".to_string(),
                flags: vec!["\\All".to_string()],
            },
            folders::Listed {
                path: "[Gmail]/Sent Mail".to_string(),
                delimiter: "/".to_string(),
                flags: vec!["\\Sent".to_string()],
            },
        ];
        let roles = folders::assign(&listed);
        let mailboxes = Mailboxes { listed, roles };
        assert_eq!(mailboxes.synced(), ["INBOX", "[Gmail]/Sent Mail"]);
    }

    #[test]
    fn a_label_id_is_a_mailbox_path_that_cannot_be_mistaken_for_anything_else() {
        assert_eq!(label_id("Work/Invoices"), "folder:Work/Invoices");
        assert_eq!(label_path("folder:Work/Invoices"), Some("Work/Invoices"));
        assert_eq!(label_path("INBOX"), None);
        assert_eq!(label_path("STARRED"), None);
    }

    #[test]
    fn the_envelope_is_every_recipient_including_the_blind_ones() {
        let raw = b"From: You <you@example.com>\r\n\
                    To: Ana <ana@example.com>, bo@example.com\r\n\
                    Cc: Cy <cy@example.com>\r\n\
                    Bcc: quiet@example.com\r\n\
                    Subject: The lease\r\n\r\nHello.\r\n";
        let (from, to) = envelope(raw);
        assert_eq!(from, "you@example.com");
        assert_eq!(
            to,
            [
                "ana@example.com",
                "bo@example.com",
                "cy@example.com",
                "quiet@example.com"
            ]
        );
    }

    #[test]
    fn a_since_argument_is_a_day_wider_than_the_window_it_was_asked_for() {
        // 2026-09-04T00:00:00Z, so a date granular SINCE that lost the boundary would say the 4th.
        assert_eq!(since(1_788_480_000_000), "03-Sep-2026");
    }

    /// A login, the mailbox list and the roles read off a real account. Off by default because it
    /// needs one, and run with the account's own details in the environment:
    ///
    ///     IMAP_HOST=imap.example.com IMAP_PORT=993 IMAP_USER=me@example.com IMAP_PASS=secret \
    ///     cargo test --lib imap::provider::tests::a_real_server -- --ignored --nocapture
    #[test]
    #[ignore]
    fn a_real_server() {
        let host = std::env::var("IMAP_HOST").expect("IMAP_HOST");
        let port: u16 = std::env::var("IMAP_PORT")
            .expect("IMAP_PORT")
            .parse()
            .expect("a port");
        let username = std::env::var("IMAP_USER").expect("IMAP_USER");
        let password = std::env::var("IMAP_PASS").expect("IMAP_PASS");

        let server = crate::dto::ServerConfig {
            host,
            port,
            security: if port == 993 {
                crate::dto::Security::Tls
            } else {
                crate::dto::Security::StartTls
            },
            auth: crate::dto::AuthKind::Password,
            username,
        };

        tauri::async_runtime::block_on(async {
            let mut session = session::open(&server, &password)
                .await
                .expect("a logged in session");
            println!("capabilities: {:?}", session.capabilities);
            let listed = folders::list(&mut session).await.expect("a mailbox list");
            for entry in &listed {
                println!("{} {:?}", entry.path, entry.flags);
            }
            println!("roles: {:?}", folders::assign(&listed));
            session.close().await;
        });
    }
}
