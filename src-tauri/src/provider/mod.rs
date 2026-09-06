// The seam between everything above and whoever is holding the mail.
//
// The sync engine never calls the Gmail client directly, so pagination, the change log's recovery
// path, the outbox drain and eviction can all be driven by a fake over an in-memory database in
// `cargo test`. That is the only way any of this is testable without credentials, and it is also
// what makes the second provider a day of work rather than a rewrite: Gmail over IMAP implements
// this trait and nothing above it changes.
//
// The trait is deliberately shaped by what every provider can do. Anything Gmail can do that IMAP
// cannot is either inside the Gmail implementation or in the state database, which is the whole
// reason the state database exists.
//
// Async in the calendar's shape: `impl Future + Send` on a `Sync` trait, so there is no boxing and
// no `async_trait` dependency.

pub mod fake;

use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::dto::{FlagPatch, Person};

/// What went wrong, at the granularity the engine actually branches on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// The token is gone or was revoked. The account needs consent again.
    Auth(String),
    /// The call needs a scope this account did not grant. Carries the scope, so the feature can
    /// offer a Grant button naming it.
    Scope(String),
    /// Back off and try again. Gmail's 429 and 403 rate limit answers land here.
    RateLimited { retry_after_ms: u64 },
    /// The change log is too old to be used. Gmail returns 404 from `history.list` once its
    /// history has expired, and the answer is a full list rather than a failure.
    NeedsFullSync,
    NotFound,
    Network(String),
    Other(String),
}

impl ProviderError {
    /// The variant as one word, for a log line. What a person is shown is decided by the engine
    /// from the variant itself; this is for reading the file afterwards.
    pub fn kind(&self) -> &'static str {
        match self {
            ProviderError::Auth(_) => "auth",
            ProviderError::Scope(_) => "scope",
            ProviderError::RateLimited { .. } => "rate-limited",
            ProviderError::NeedsFullSync => "needs-full-sync",
            ProviderError::NotFound => "not-found",
            ProviderError::Network(_) => "network",
            ProviderError::Other(_) => "other",
        }
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::Auth(m) => write!(f, "signed out: {m}"),
            // A scope is a URL, and a URL is not something to print in front of a person. Its last
            // segment (`gmail.modify`) says which permission without saying where it lives.
            ProviderError::Scope(s) => write!(
                f,
                "missing permission: {}",
                s.trim_end_matches('/').rsplit('/').next().unwrap_or(s)
            ),
            ProviderError::RateLimited { retry_after_ms } => {
                write!(f, "rate limited, retry in {retry_after_ms}ms")
            }
            ProviderError::NeedsFullSync => write!(f, "the change log has expired"),
            ProviderError::NotFound => write!(f, "not found"),
            ProviderError::Network(m) => write!(f, "network: {m}"),
            ProviderError::Other(m) => write!(f, "{m}"),
        }
    }
}

/// One message as the provider identifies it. Both ids are the provider's and neither ever reaches
/// the frontend or the state database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRef {
    pub id: String,
    pub thread_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListPage {
    pub messages: Vec<MessageRef>,
    pub next_page: Option<String>,
    /// The provider's guess at the total, for the progress bar. Gmail's is famously approximate,
    /// so it is a hint and never a count to divide by without checking.
    pub estimate: Option<u32>,
}

/// Metadata for one message: the headers the app parses plus the provider's own flags.
///
/// Headers arrive as a list of pairs rather than a map because a message may carry `Received` a
/// dozen times and `References` folded across lines, and a map would quietly lose the difference.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RawHeaders {
    pub id: String,
    pub thread_id: String,
    pub label_ids: Vec<String>,
    pub internal_date_ms: i64,
    pub size: u32,
    pub snippet: String,
    pub headers: Vec<(String, String)>,
}

impl RawHeaders {
    /// Case-insensitive, first match, which is what every header this app reads wants.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    Added(MessageRef),
    Deleted(String),
    LabelsChanged { id: String, labels: Vec<String> },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Changes {
    pub changes: Vec<Change>,
    /// The cursor to store and pass back next time.
    pub cursor: String,
    /// Set when the log had more than one page.
    pub next_page: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SentIds {
    pub id: String,
    pub thread_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderLabel {
    pub id: String,
    pub name: String,
    /// system or user
    pub kind: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderSettings {
    /// Verified send-as addresses, the account's own first.
    pub aliases: Vec<String>,
    pub signature: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub email: String,
    pub name: String,
    pub messages_total: u32,
}

pub trait Provider: Sync {
    /// Ids newest first, optionally limited to messages after a moment, which is how a storage
    /// window is fetched without asking for the whole mailbox.
    fn list(
        &self,
        after_ms: Option<i64>,
        page: Option<&str>,
    ) -> impl Future<Output = Result<ListPage, ProviderError>> + Send;

    /// The change log since a cursor, or `NeedsFullSync` when it has expired.
    fn changes_since(
        &self,
        cursor: &str,
        page: Option<&str>,
    ) -> impl Future<Output = Result<Changes, ProviderError>> + Send;

    /// The cursor that means "everything from now on", for the end of a first sync.
    fn cursor_now(&self) -> impl Future<Output = Result<String, ProviderError>> + Send;

    /// Batched. The implementation decides the batch size; the engine hands it whatever it has.
    fn fetch_headers(
        &self,
        ids: &[String],
    ) -> impl Future<Output = Result<Vec<RawHeaders>, ProviderError>> + Send;

    /// The raw RFC 2822 bytes. Bodies are always parsed from these, never from a provider's
    /// pre-parsed payload, because encoded words and legacy charsets arrive daily.
    fn fetch_body(&self, id: &str) -> impl Future<Output = Result<Vec<u8>, ProviderError>> + Send;

    fn fetch_attachment(
        &self,
        message_id: &str,
        attachment_id: &str,
    ) -> impl Future<Output = Result<Vec<u8>, ProviderError>> + Send;

    /// Seen, starred, archived, trashed and spam, coalesced by the caller into one call.
    fn set_flags(
        &self,
        ids: &[String],
        patch: &FlagPatch,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send;

    fn labels(&self) -> impl Future<Output = Result<Vec<ProviderLabel>, ProviderError>> + Send;

    fn set_labels(
        &self,
        ids: &[String],
        add: &[String],
        remove: &[String],
    ) -> impl Future<Output = Result<(), ProviderError>> + Send;

    fn send(
        &self,
        raw: &[u8],
        thread_hint: Option<&str>,
    ) -> impl Future<Output = Result<SentIds, ProviderError>> + Send;

    fn draft_put(
        &self,
        provider_draft_id: Option<&str>,
        raw: &[u8],
        thread_hint: Option<&str>,
    ) -> impl Future<Output = Result<String, ProviderError>> + Send;

    fn draft_delete(
        &self,
        provider_draft_id: &str,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send;

    /// Provider-side search, ids only. The hits are hydrated through the ordinary path.
    fn search(
        &self,
        query: &str,
        page: Option<&str>,
    ) -> impl Future<Output = Result<ListPage, ProviderError>> + Send;

    fn settings(&self) -> impl Future<Output = Result<ProviderSettings, ProviderError>> + Send;

    fn profile(&self) -> impl Future<Output = Result<ProviderProfile, ProviderError>> + Send;

    /// The provider's own address book, for the first run seed and for autocomplete's second pass.
    /// An implementation without one returns an empty list rather than an error.
    fn contacts(&self) -> impl Future<Output = Result<Vec<Person>, ProviderError>> + Send;
}
