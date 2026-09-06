// An in-memory mailbox that implements `Provider`, for the engine's tests.
//
// It is a real little mailbox rather than a pile of canned responses: messages have ids, labels,
// dates and raw bytes, `list` pages through them newest first, flag changes mutate labels, and a
// history log accumulates so `changes_since` can answer from it. That matters because the bugs
// worth catching in a sync engine are the ones about ordering and recovery, and a stub that only
// replays a script cannot have them.
//
// Failures are scripted rather than simulated: `fail_next` makes the next call of a kind return an
// error, which is how the 429 backoff, the expired change log and the signed-out account get
// exercised. `withhold_body` is the other shape, a permanent refusal of one message rather than a
// one-off refusal of the next call, because a message deleted upstream between the listing and the
// fetch answers 404 every time it is asked for and that is what stalls a queue.

#![cfg(test)]

use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;

use crate::dto::{FlagPatch, Person};

use super::{
    Change, Changes, ListPage, MessageRef, Provider, ProviderError, ProviderLabel, ProviderProfile,
    ProviderSettings, RawHeaders, SentIds,
};

#[derive(Debug, Clone)]
pub struct FakeMessage {
    pub id: String,
    pub thread_id: String,
    pub labels: Vec<String>,
    pub date_ms: i64,
    pub snippet: String,
    pub raw: Vec<u8>,
}

impl FakeMessage {
    /// Builds one from raw RFC 2822 bytes, which is what the `.eml` corpus holds. The headers are
    /// read back out of the bytes rather than passed in separately, so a fixture cannot describe
    /// itself one way to the parser and another way to the engine.
    pub fn from_eml(id: &str, thread_id: &str, labels: &[&str], raw: &[u8]) -> Self {
        let text = String::from_utf8_lossy(raw);
        let head = text.split("\r\n\r\n").next().unwrap_or("");
        let head = if head.len() == text.len() {
            text.split("\n\n").next().unwrap_or("")
        } else {
            head
        };
        let date_ms = unfolded(head)
            .into_iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("date"))
            .and_then(|(_, v)| chrono::DateTime::parse_from_rfc2822(v.trim()).ok())
            .map(|d| d.timestamp_millis())
            .unwrap_or(0);
        let body = text
            .split_once("\r\n\r\n")
            .or_else(|| text.split_once("\n\n"))
            .map(|(_, b)| b)
            .unwrap_or("");
        FakeMessage {
            id: id.to_string(),
            thread_id: thread_id.to_string(),
            labels: labels.iter().map(|l| l.to_string()).collect(),
            date_ms,
            snippet: body.chars().filter(|c| !c.is_control()).take(120).collect(),
            raw: raw.to_vec(),
        }
    }

    fn headers(&self) -> Vec<(String, String)> {
        let text = String::from_utf8_lossy(&self.raw);
        let head = text
            .split_once("\r\n\r\n")
            .or_else(|| text.split_once("\n\n"))
            .map(|(h, _)| h)
            .unwrap_or(&text);
        unfolded(head)
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.trim().to_string()))
            .collect()
    }
}

/// Header folding is a continuation line starting with whitespace, and `References` is folded on
/// most real mail, so a naive line split loses half of it.
fn unfolded(head: &str) -> Vec<(&str, String)> {
    let mut out: Vec<(&str, String)> = Vec::new();
    for line in head.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = out.last_mut() {
                last.1.push(' ');
                last.1.push_str(line.trim());
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            out.push((name, value.trim().to_string()));
        }
    }
    out
}

/// Which call to fail, so a test can name one without a magic string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Call {
    List,
    Changes,
    Headers,
    Body,
    Attachment,
    Flags,
    Labels,
    Send,
    Draft,
    Search,
    Settings,
    Profile,
    Contacts,
}

#[derive(Default)]
struct Inner {
    messages: Vec<FakeMessage>,
    history: Vec<(u64, Change)>,
    next_history: u64,
    attachments: HashMap<String, Vec<u8>>,
    labels: Vec<ProviderLabel>,
    contacts: Vec<Person>,
    settings: ProviderSettings,
    profile: ProviderProfile,
    failures: HashMap<Call, Vec<ProviderError>>,
    /// Messages whose body is never handed over, however often it is asked for.
    withheld: Vec<String>,
    /// Messages `list` still names and `fetch_headers` never returns, which is what a message
    /// deleted between the listing and the fetch looks like: a 404 for one part of a batch.
    withheld_headers: Vec<String>,
    calls: Vec<String>,
    /// How many ids `list` and `search` return per page.
    page_size: usize,
    /// The oldest history the log still answers for. Below it, `changes_since` says full sync.
    history_floor: u64,
}

pub struct FakeProvider {
    inner: Mutex<Inner>,
}

impl Default for FakeProvider {
    fn default() -> Self {
        FakeProvider {
            inner: Mutex::new(Inner {
                page_size: 100,
                next_history: 1,
                labels: vec![
                    ProviderLabel {
                        id: "INBOX".into(),
                        name: "Inbox".into(),
                        kind: "system".into(),
                    },
                    ProviderLabel {
                        id: "UNREAD".into(),
                        name: "Unread".into(),
                        kind: "system".into(),
                    },
                    ProviderLabel {
                        id: "SENT".into(),
                        name: "Sent".into(),
                        kind: "system".into(),
                    },
                ],
                profile: ProviderProfile {
                    email: "you@example.com".into(),
                    name: "You".into(),
                    messages_total: 0,
                },
                settings: ProviderSettings {
                    aliases: vec!["you@example.com".into()],
                    signature: String::new(),
                },
                ..Inner::default()
            }),
        }
    }
}

impl FakeProvider {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("the fake provider's lock")
    }

    pub fn add(&self, message: FakeMessage) -> &Self {
        let mut inner = self.lock();
        let seq = inner.next_history;
        inner.next_history += 1;
        inner.history.push((
            seq,
            Change::Added(MessageRef {
                id: message.id.clone(),
                thread_id: message.thread_id.clone(),
            }),
        ));
        inner.messages.push(message);
        inner.messages.sort_by(|a, b| b.date_ms.cmp(&a.date_ms));
        inner.profile.messages_total = inner.messages.len() as u32;
        drop(inner);
        self
    }

    pub fn add_eml(&self, id: &str, thread_id: &str, labels: &[&str], raw: &[u8]) -> &Self {
        self.add(FakeMessage::from_eml(id, thread_id, labels, raw))
    }

    pub fn set_attachment(&self, attachment_id: &str, bytes: Vec<u8>) -> &Self {
        self.lock()
            .attachments
            .insert(attachment_id.to_string(), bytes);
        self
    }

    pub fn set_contacts(&self, contacts: Vec<Person>) -> &Self {
        self.lock().contacts = contacts;
        self
    }

    pub fn set_page_size(&self, size: usize) -> &Self {
        self.lock().page_size = size.max(1);
        self
    }

    /// Everything up to and including this history sequence stops being answerable, which is what
    /// Gmail does when its history expires and the only way to test the recovery path.
    pub fn expire_history_below(&self, seq: u64) -> &Self {
        self.lock().history_floor = seq;
        self
    }

    /// A message the mailbox keeps listing and will never hand the body of over, which is what
    /// Gmail does with one deleted between the listing and the fetch. Unlike `fail_next` it does
    /// not run out, because the point of it is a failure that repeats.
    pub fn withhold_body(&self, id: &str) -> &Self {
        self.lock().withheld.push(id.to_string());
        self
    }

    /// A message the listing keeps naming and the metadata fetch never answers for. Gmail does
    /// this for a message deleted after `messages.list` and before `messages.get`, as a 404 on
    /// one part of an otherwise happy batch.
    pub fn withhold_headers(&self, id: &str) -> &Self {
        self.lock().withheld_headers.push(id.to_string());
        self
    }

    pub fn fail_next(&self, call: Call, error: ProviderError) -> &Self {
        self.lock().failures.entry(call).or_default().push(error);
        self
    }

    pub fn calls(&self) -> Vec<String> {
        self.lock().calls.clone()
    }

    pub fn message_labels(&self, id: &str) -> Option<Vec<String>> {
        self.lock()
            .messages
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.labels.clone())
    }

    pub fn message_count(&self) -> usize {
        self.lock().messages.len()
    }

    fn record(&self, call: Call, note: String) -> Result<(), ProviderError> {
        let mut inner = self.lock();
        inner.calls.push(note);
        match inner.failures.get_mut(&call) {
            Some(queue) if !queue.is_empty() => Err(queue.remove(0)),
            _ => Ok(()),
        }
    }

    fn note_change(&self, inner: &mut Inner, change: Change) {
        let seq = inner.next_history;
        inner.next_history += 1;
        inner.history.push((seq, change));
    }
}

/// The label a flag maps onto, and whether setting the flag adds or removes it. `archived` and
/// `seen` are both inversions, which is exactly the sort of thing worth having in one place.
fn label_for(flag: &str) -> (&'static str, bool) {
    match flag {
        "seen" => ("UNREAD", false),
        "starred" => ("STARRED", true),
        "archived" => ("INBOX", false),
        "trashed" => ("TRASH", true),
        "spam" => ("SPAM", true),
        _ => ("", true),
    }
}

fn apply_flag(labels: &mut Vec<String>, flag: &str, on: bool) {
    let (label, adds_when_on) = label_for(flag);
    if label.is_empty() {
        return;
    }
    let should_hold = if adds_when_on { on } else { !on };
    let held = labels.iter().any(|l| l == label);
    if should_hold && !held {
        labels.push(label.to_string());
    } else if !should_hold && held {
        labels.retain(|l| l != label);
    }
}

impl Provider for FakeProvider {
    fn list(
        &self,
        after_ms: Option<i64>,
        page: Option<&str>,
    ) -> impl Future<Output = Result<ListPage, ProviderError>> + Send {
        let start: usize = page.and_then(|p| p.parse().ok()).unwrap_or(0);
        let note = format!("list after={after_ms:?} page={}", page.unwrap_or("-"));
        async move {
            self.record(Call::List, note)?;
            let inner = self.lock();
            let all: Vec<&FakeMessage> = inner
                .messages
                .iter()
                .filter(|m| after_ms.map(|after| m.date_ms >= after).unwrap_or(true))
                .collect();
            let end = (start + inner.page_size).min(all.len());
            let messages = all[start.min(all.len())..end]
                .iter()
                .map(|m| MessageRef {
                    id: m.id.clone(),
                    thread_id: m.thread_id.clone(),
                })
                .collect();
            Ok(ListPage {
                messages,
                next_page: (end < all.len()).then(|| end.to_string()),
                estimate: Some(all.len() as u32),
            })
        }
    }

    fn changes_since(
        &self,
        cursor: &str,
        _page: Option<&str>,
    ) -> impl Future<Output = Result<Changes, ProviderError>> + Send {
        let from: u64 = cursor.parse().unwrap_or(0);
        let note = format!("changes_since {cursor}");
        async move {
            self.record(Call::Changes, note)?;
            let inner = self.lock();
            if from < inner.history_floor {
                return Err(ProviderError::NeedsFullSync);
            }
            let changes = inner
                .history
                .iter()
                .filter(|(seq, _)| *seq > from)
                .map(|(_, change)| change.clone())
                .collect();
            Ok(Changes {
                changes,
                cursor: inner.next_history.saturating_sub(1).to_string(),
                next_page: None,
            })
        }
    }

    fn cursor_now(&self) -> impl Future<Output = Result<String, ProviderError>> + Send {
        async move {
            let inner = self.lock();
            Ok(inner.next_history.saturating_sub(1).to_string())
        }
    }

    fn fetch_headers(
        &self,
        ids: &[String],
    ) -> impl Future<Output = Result<Vec<RawHeaders>, ProviderError>> + Send {
        let ids = ids.to_vec();
        async move {
            self.record(Call::Headers, format!("fetch_headers {}", ids.len()))?;
            let inner = self.lock();
            Ok(inner
                .messages
                .iter()
                .filter(|m| ids.contains(&m.id) && !inner.withheld_headers.contains(&m.id))
                .map(|m| RawHeaders {
                    id: m.id.clone(),
                    thread_id: m.thread_id.clone(),
                    label_ids: m.labels.clone(),
                    internal_date_ms: m.date_ms,
                    size: m.raw.len() as u32,
                    snippet: m.snippet.clone(),
                    headers: m.headers(),
                })
                .collect())
        }
    }

    fn fetch_body(&self, id: &str) -> impl Future<Output = Result<Vec<u8>, ProviderError>> + Send {
        let id = id.to_string();
        async move {
            self.record(Call::Body, format!("fetch_body {id}"))?;
            let inner = self.lock();
            if inner.withheld.contains(&id) {
                return Err(ProviderError::NotFound);
            }
            inner
                .messages
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.raw.clone())
                .ok_or(ProviderError::NotFound)
        }
    }

    fn fetch_attachment(
        &self,
        message_id: &str,
        attachment_id: &str,
    ) -> impl Future<Output = Result<Vec<u8>, ProviderError>> + Send {
        let (message_id, attachment_id) = (message_id.to_string(), attachment_id.to_string());
        async move {
            self.record(
                Call::Attachment,
                format!("fetch_attachment {message_id} {attachment_id}"),
            )?;
            let inner = self.lock();
            inner
                .attachments
                .get(&attachment_id)
                .cloned()
                .ok_or(ProviderError::NotFound)
        }
    }

    fn set_flags(
        &self,
        ids: &[String],
        patch: &FlagPatch,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send {
        let ids = ids.to_vec();
        let patch = patch.clone();
        async move {
            self.record(Call::Flags, format!("set_flags {} {patch:?}", ids.len()))?;
            let mut inner = self.lock();
            let mut touched = Vec::new();
            for message in inner.messages.iter_mut().filter(|m| ids.contains(&m.id)) {
                for (name, value) in [
                    ("seen", patch.seen),
                    ("starred", patch.starred),
                    ("archived", patch.archived),
                    ("trashed", patch.trashed),
                    ("spam", patch.spam),
                ] {
                    if let Some(on) = value {
                        apply_flag(&mut message.labels, name, on);
                    }
                }
                touched.push((message.id.clone(), message.labels.clone()));
            }
            for (id, labels) in touched {
                self.note_change(&mut inner, Change::LabelsChanged { id, labels });
            }
            Ok(())
        }
    }

    fn labels(&self) -> impl Future<Output = Result<Vec<ProviderLabel>, ProviderError>> + Send {
        async move {
            self.record(Call::Labels, "labels".to_string())?;
            Ok(self.lock().labels.clone())
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
            self.record(
                Call::Labels,
                format!("set_labels {} +{} -{}", ids.len(), add.len(), remove.len()),
            )?;
            let mut inner = self.lock();
            let mut touched = Vec::new();
            for message in inner.messages.iter_mut().filter(|m| ids.contains(&m.id)) {
                for label in &add {
                    if !message.labels.contains(label) {
                        message.labels.push(label.clone());
                    }
                }
                message.labels.retain(|l| !remove.contains(l));
                touched.push((message.id.clone(), message.labels.clone()));
            }
            for (id, labels) in touched {
                self.note_change(&mut inner, Change::LabelsChanged { id, labels });
            }
            Ok(())
        }
    }

    fn send(
        &self,
        raw: &[u8],
        thread_hint: Option<&str>,
    ) -> impl Future<Output = Result<SentIds, ProviderError>> + Send {
        let raw = raw.to_vec();
        let thread_hint = thread_hint.map(|t| t.to_string());
        async move {
            self.record(Call::Send, format!("send {} bytes", raw.len()))?;
            let id = format!("sent-{}", self.lock().messages.len() + 1);
            let thread_id = thread_hint.clone().unwrap_or_else(|| id.clone());
            let message = FakeMessage::from_eml(&id, &thread_id, &["SENT"], &raw);
            self.add(message);
            Ok(SentIds { id, thread_id })
        }
    }

    fn draft_put(
        &self,
        provider_draft_id: Option<&str>,
        raw: &[u8],
        _thread_hint: Option<&str>,
    ) -> impl Future<Output = Result<String, ProviderError>> + Send {
        let existing = provider_draft_id.map(|d| d.to_string());
        let size = raw.len();
        async move {
            self.record(Call::Draft, format!("draft_put {existing:?} {size} bytes"))?;
            Ok(existing.unwrap_or_else(|| "draft-1".to_string()))
        }
    }

    fn draft_delete(
        &self,
        provider_draft_id: &str,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send {
        let id = provider_draft_id.to_string();
        async move {
            self.record(Call::Draft, format!("draft_delete {id}"))?;
            Ok(())
        }
    }

    fn search(
        &self,
        query: &str,
        page: Option<&str>,
    ) -> impl Future<Output = Result<ListPage, ProviderError>> + Send {
        let query = query.to_lowercase();
        let start: usize = page.and_then(|p| p.parse().ok()).unwrap_or(0);
        async move {
            self.record(Call::Search, format!("search {query}"))?;
            let inner = self.lock();
            let hits: Vec<&FakeMessage> = inner
                .messages
                .iter()
                .filter(|m| String::from_utf8_lossy(&m.raw).to_lowercase().contains(&query))
                .collect();
            let end = (start + inner.page_size).min(hits.len());
            Ok(ListPage {
                messages: hits[start.min(hits.len())..end]
                    .iter()
                    .map(|m| MessageRef {
                        id: m.id.clone(),
                        thread_id: m.thread_id.clone(),
                    })
                    .collect(),
                next_page: (end < hits.len()).then(|| end.to_string()),
                estimate: Some(hits.len() as u32),
            })
        }
    }

    fn settings(&self) -> impl Future<Output = Result<ProviderSettings, ProviderError>> + Send {
        async move {
            self.record(Call::Settings, "settings".to_string())?;
            Ok(self.lock().settings.clone())
        }
    }

    fn profile(&self) -> impl Future<Output = Result<ProviderProfile, ProviderError>> + Send {
        async move {
            self.record(Call::Profile, "profile".to_string())?;
            Ok(self.lock().profile.clone())
        }
    }

    fn contacts(&self) -> impl Future<Output = Result<Vec<Person>, ProviderError>> + Send {
        async move {
            self.record(Call::Contacts, "contacts".to_string())?;
            Ok(self.lock().contacts.clone())
        }
    }
}
