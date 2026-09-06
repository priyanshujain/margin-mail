// `impl Provider for Gmail`: the REST layer below, the sync engine above, and the mapping between
// the two in one file.
//
// `Gmail` holds an app handle and an account id and nothing else. The token is fetched per request
// through `auth::valid_access_token`, which is single-flight and refreshes when it needs to, in the
// same shape as the calendar's `Transport for Google`. Holding one here would mean holding a stale
// one, and the sync engine builds and drops these freely.
//
// The four mappings worth reading before anything else:
//
//   `changes_since` is `history.list`, and its 404 is `NeedsFullSync` rather than a failure. A user
//   who has not opened the app for a fortnight will hit it; Google says a historyId is valid for
//   about a week and sometimes only hours.
//
//   `cursor_now` is the `historyId` from `users.getProfile`, recorded before a first sync lists
//   anything, so the first partial sync covers everything that changed during the crawl.
//
//   `list` with a window is a `q` of `after:<unix seconds>`. There is no other way to fetch a date
//   range cheaply: history has no query and `messages.list` has no date parameter.
//
//   `set_flags` is label arithmetic. Seen removes `UNREAD` and archived removes `INBOX`, both
//   inversions; starred, trashed and spam add theirs. `provider/fake.rs` holds the same mapping in
//   one function and this one matches it, because a fake that disagrees with the real thing about
//   what "archived" means is worse than no fake at all.

use std::future::Future;
use std::time::Duration;

use tauri::Manager;

use crate::dto::{FlagPatch, Person};
use crate::google::api::{self, ApiError, Call};
use crate::google::auth::{self, AuthState};
use crate::google::{batch, people};
use crate::provider::{
    Change, Changes, ListPage, MessageRef, Provider, ProviderError, ProviderLabel, ProviderProfile,
    ProviderSettings, RawHeaders, SentIds,
};

pub struct Gmail {
    app: tauri::AppHandle,
    account_id: String,
}

impl Gmail {
    pub fn new(app: tauri::AppHandle, account_id: impl Into<String>) -> Gmail {
        Gmail {
            app,
            account_id: account_id.into(),
        }
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    async fn token(&self) -> Result<String, ProviderError> {
        let state = self.app.try_state::<AuthState>().ok_or_else(|| {
            ProviderError::Auth("this app has no Google session store".to_string())
        })?;
        auth::valid_access_token(&self.app, state.inner(), &self.account_id)
            .await
            .map_err(session_error)
    }

    /// Hydration. One batch of 50 is 1,000 units, so the quota accountant is told the whole cost
    /// before the request leaves rather than 20 units at a time after the fact.
    ///
    /// Per-part failures are the point of doing this by hand. A 404 is a message deleted between
    /// the list and the fetch, which is routine and is dropped; a 429 is only that part's problem
    /// and those ids go round again on the backoff schedule. Anything else fails the call, because
    /// a hydration pass that quietly returns 30 of 50 messages is a mirror with holes in it that
    /// nothing downstream can detect.
    async fn hydrate(&self, ids: Vec<String>) -> Result<Vec<RawHeaders>, ProviderError> {
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(api::BATCH_SIZE) {
            let mut pending: Vec<String> = chunk.to_vec();
            let mut attempt = 0;
            while !pending.is_empty() {
                let token = self.token().await?;
                let units = pending.len() as u32 * Call::MessagesGet.units();
                api::spend(&self.account_id, units).await;

                let parts: Vec<batch::Part> = pending
                    .iter()
                    .map(|id| batch::Part::get(id.clone(), api::metadata_path(id)))
                    .collect();
                let answers = batch::run(&token, &parts).await?;

                let mut again = Vec::new();
                let mut delay = 0u64;
                for answer in answers {
                    if answer.is_success() {
                        let message: api::Message =
                            answer.json(Call::MessagesGet.name(), Call::MessagesGet.scope())?;
                        out.push(headers_from(&message));
                        continue;
                    }
                    match answer.error(Call::MessagesGet.name(), Call::MessagesGet.scope()) {
                        ApiError::NotFound(_) => {}
                        // A 400 inside a batch is about that one id, since everything about the
                        // account arrives as 401, 403, 429 or a 5xx and fails the call below.
                        // Like the 404 it will not get better, and failing the call would put the
                        // same message at the head of every pass for ever.
                        ApiError::Other(said) if answer.status == 400 => {
                            crate::log::note(
                                &self.account_id,
                                &format!("skipped message {} in hydration: {said}", answer.id),
                            );
                        }
                        ApiError::RateLimited { retry_after_ms } => {
                            delay = delay.max(retry_after_ms);
                            again.push(answer.id);
                        }
                        other => return Err(other.into()),
                    }
                }

                if again.is_empty() {
                    break;
                }
                attempt += 1;
                let wait = delay.max(api::backoff(attempt));
                if attempt >= api::MAX_ATTEMPTS {
                    return Err(ProviderError::RateLimited {
                        retry_after_ms: wait,
                    });
                }
                tokio::time::sleep(Duration::from_millis(wait)).await;
                pending = again;
            }
        }
        Ok(out)
    }

    /// One id goes through `messages.modify` at 5 units rather than `batchModify` at a flat 50,
    /// which is what a single triage keystroke costs. Ten or more, and the flat rate wins.
    async fn modify(
        &self,
        ids: Vec<String>,
        add: Vec<String>,
        remove: Vec<String>,
    ) -> Result<(), ProviderError> {
        if ids.is_empty() || (add.is_empty() && remove.is_empty()) {
            return Ok(());
        }
        let token = self.token().await?;
        for chunk in ids.chunks(api::MAX_MODIFY_IDS) {
            if let [only] = chunk {
                self.modify_one(&token, only, &add, &remove).await?;
                continue;
            }
            api::spend(&self.account_id, Call::BatchModify.units()).await;
            match api::with_retry(|| api::messages_batch_modify(&token, chunk, &add, &remove))
                .await
            {
                Ok(()) => {}
                // `batchModify` has no per-id report: one id that has gone since it was listed
                // takes the whole call down with a 400 or a 404, and nothing says which. Asking
                // for each on its own lets the ones still there go and the one that is not be
                // forgotten, at 5 units apiece against the 50 already spent.
                Err(ApiError::NotFound(_)) | Err(ApiError::Other(_)) => {
                    for id in chunk {
                        self.modify_one(&token, id, &add, &remove).await?;
                    }
                }
                Err(other) => return Err(other.into()),
            }
        }
        Ok(())
    }

    /// One message. A 404 is a message that has gone since it was listed, and a label change on a
    /// message that has gone has already happened as far as anyone can tell, so it is done.
    async fn modify_one(
        &self,
        token: &str,
        id: &str,
        add: &[String],
        remove: &[String],
    ) -> Result<(), ProviderError> {
        api::spend(&self.account_id, Call::MessagesModify.units()).await;
        match api::with_retry(|| api::messages_modify(token, id, add, remove)).await {
            Ok(_) | Err(ApiError::NotFound(_)) => Ok(()),
            Err(other) => Err(other.into()),
        }
    }
}

/// What the session layer's one `String` means to the engine.
///
/// `auth::valid_access_token` answers with a string for everything from a revoked refresh token to
/// a train tunnel, and all of it used to become `Auth`. So a token refresh that could not be sent
/// put "Signed out" in the account chip with the token endpoint's URL under it, on an account that
/// was signed in perfectly well. Only Google refusing the token is a sign-out. Not reaching Google
/// is the network, and Google's own 5xx is Google's own bad day.
pub fn session_error(said: String) -> ProviderError {
    // reqwest's transport failures, as `auth` stringifies them.
    if said.starts_with("error sending request")
        || said.starts_with("request or response body error")
        || said.starts_with("error decoding response body")
    {
        return ProviderError::Network(api::strip_urls(&said));
    }
    // `auth::read_json` writes "<what> failed (<status line>): <body>".
    match failed_status(&said) {
        Some(429) | Some(500..=599) => ProviderError::RateLimited {
            retry_after_ms: api::DEFAULT_RETRY_MS,
        },
        Some(_) => ProviderError::Auth(token_refusal(&said)),
        None => ProviderError::Auth(api::strip_urls(&said)),
    }
}

fn failed_status(said: &str) -> Option<u16> {
    let (_, rest) = said.split_once(" failed (")?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// The token endpoint's body is `{"error":"invalid_grant","error_description":"Token has been
/// expired or revoked."}`, and the description is the sentence worth keeping.
fn token_refusal(said: &str) -> String {
    let body = said.split_once("): ").map(|(_, body)| body).unwrap_or(said);
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        for key in ["error_description", "error"] {
            if let Some(text) = value.get(key).and_then(|v| v.as_str()) {
                return api::strip_urls(text);
            }
        }
    }
    api::strip_urls(said)
}

/// The label a flag maps onto, and whether setting the flag adds it. Kept in the same shape as
/// `provider::fake::label_for` so the two cannot drift.
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

/// A flag patch as the two label lists Gmail takes. An absent field is unchanged and appears in
/// neither list.
pub fn label_changes(patch: &FlagPatch) -> (Vec<String>, Vec<String>) {
    let mut add = Vec::new();
    let mut remove = Vec::new();
    for (flag, value) in [
        ("seen", patch.seen),
        ("starred", patch.starred),
        ("archived", patch.archived),
        ("trashed", patch.trashed),
        ("spam", patch.spam),
    ] {
        let Some(on) = value else { continue };
        let (label, adds_when_on) = label_for(flag);
        if label.is_empty() {
            continue;
        }
        let should_hold = if adds_when_on { on } else { !on };
        if should_hold {
            add.push(label.to_string());
        } else {
            remove.push(label.to_string());
        }
    }
    (add, remove)
}

/// The one place a 404 is not a failure. Gmail answers `history.list` with one once the cursor has
/// aged out of its log, and the answer is a full list rather than an error to report.
pub fn history_error(error: ApiError) -> ProviderError {
    match error {
        ApiError::NotFound(_) => ProviderError::NeedsFullSync,
        // The only things `history.list` is ever sent are the cursor and a page token from the
        // same log, so a 400 can only be about those, and a cursor Gmail will not read is cured
        // the same way as one it has forgotten. Reported as an error it would come back every
        // pass for ever.
        ApiError::Other(said) if said.contains("(400)") => ProviderError::NeedsFullSync,
        other => other.into(),
    }
}

pub fn headers_from(message: &api::Message) -> RawHeaders {
    RawHeaders {
        id: message.id.clone(),
        thread_id: message.thread_id.clone(),
        label_ids: message.label_ids.clone(),
        internal_date_ms: message.internal_date_ms(),
        size: message.size_estimate,
        // Gmail's snippet arrives HTML escaped, so an apostrophe reaches us as `&#39;` and a
        // quotation mark as `&quot;`. It is a preview line rendered as text and never as markup,
        // so the entities have to come out here: nothing downstream is going to do it, and the
        // list showing `I&#39;m` where the subject beside it shows `I'm` is the tell.
        snippet: crate::sanitize::decode_entities(&message.snippet),
        headers: message.header_pairs(),
    }
}

pub fn list_page(page: api::MessageIdsPage) -> ListPage {
    ListPage {
        messages: page
            .messages
            .into_iter()
            .map(|m| MessageRef {
                id: m.id,
                thread_id: m.thread_id,
            })
            .collect(),
        next_page: page.next_page_token.filter(|token| !token.is_empty()),
        estimate: page.result_size_estimate,
    }
}

/// The change log, flattened. `labelsAdded` and `labelsRemoved` both carry the message's whole
/// label set as it stands after the change rather than the delta, so one `LabelsChanged` per record
/// per message says everything; a record that both added and removed a label would otherwise emit
/// the same change twice.
///
/// `messagesDeleted` means permanently deleted. Trashing arrives as a `TRASH` label add, which is
/// why the engine can treat this list as ordinary state and not as a special case.
pub fn changes_from(page: &api::HistoryPage) -> Changes {
    let mut changes: Vec<Change> = Vec::new();
    for record in &page.history {
        for added in &record.messages_added {
            changes.push(Change::Added(MessageRef {
                id: added.message.id.clone(),
                thread_id: added.message.thread_id.clone(),
            }));
        }
        for deleted in &record.messages_deleted {
            changes.push(Change::Deleted(deleted.message.id.clone()));
        }
        for changed in record.labels_added.iter().chain(record.labels_removed.iter()) {
            let change = Change::LabelsChanged {
                id: changed.message.id.clone(),
                labels: changed.message.label_ids.clone(),
            };
            if changes.last() != Some(&change) {
                changes.push(change);
            }
        }
    }
    Changes {
        changes,
        cursor: page.history_id.clone(),
        next_page: page.next_page_token.clone().filter(|token| !token.is_empty()),
    }
}

/// The account's own address first, then the verified aliases. An alias still waiting on its
/// confirmation mail is left out: Gmail rewrites `From` to the primary when it is sent from, which
/// looks to the user like the app ignored what they picked.
pub fn settings_from(send_as: &[api::SendAs]) -> ProviderSettings {
    let primary = send_as.iter().find(|entry| entry.is_primary);
    let mut aliases: Vec<String> = primary
        .map(|entry| vec![entry.send_as_email.clone()])
        .unwrap_or_default();
    for entry in send_as {
        if entry.is_primary || entry.verification_status == "pending" {
            continue;
        }
        if !aliases.contains(&entry.send_as_email) {
            aliases.push(entry.send_as_email.clone());
        }
    }
    ProviderSettings {
        aliases,
        signature: primary.map(|e| e.signature.clone()).unwrap_or_default(),
    }
}

impl Provider for Gmail {
    fn list(
        &self,
        after_ms: Option<i64>,
        page: Option<&str>,
    ) -> impl Future<Output = Result<ListPage, ProviderError>> + Send {
        let query = after_ms.map(api::window_query);
        let page = page.map(|p| p.to_string());
        async move {
            let token = self.token().await?;
            api::spend(&self.account_id, Call::MessagesList.units()).await;
            let answer =
                api::with_retry(|| api::messages_list(&token, query.as_deref(), page.as_deref()))
                    .await?;
            Ok(list_page(answer))
        }
    }

    fn changes_since(
        &self,
        cursor: &str,
        page: Option<&str>,
    ) -> impl Future<Output = Result<Changes, ProviderError>> + Send {
        let cursor = cursor.to_string();
        let page = page.map(|p| p.to_string());
        async move {
            let token = self.token().await?;
            api::spend(&self.account_id, Call::HistoryList.units()).await;
            match api::with_retry(|| api::history_list(&token, &cursor, page.as_deref())).await {
                Ok(answer) => Ok(changes_from(&answer)),
                Err(error) => Err(history_error(error)),
            }
        }
    }

    fn cursor_now(&self) -> impl Future<Output = Result<String, ProviderError>> + Send {
        async move {
            let token = self.token().await?;
            api::spend(&self.account_id, Call::GetProfile.units()).await;
            let profile = api::with_retry(|| api::get_profile(&token)).await?;
            Ok(profile.history_id)
        }
    }

    fn fetch_headers(
        &self,
        ids: &[String],
    ) -> impl Future<Output = Result<Vec<RawHeaders>, ProviderError>> + Send {
        let ids = ids.to_vec();
        async move { self.hydrate(ids).await }
    }

    fn fetch_body(&self, id: &str) -> impl Future<Output = Result<Vec<u8>, ProviderError>> + Send {
        let id = id.to_string();
        async move {
            let token = self.token().await?;
            api::spend(&self.account_id, Call::MessagesGet.units()).await;
            Ok(api::with_retry(|| api::messages_get_raw(&token, &id)).await?)
        }
    }

    fn fetch_attachment(
        &self,
        message_id: &str,
        attachment_id: &str,
    ) -> impl Future<Output = Result<Vec<u8>, ProviderError>> + Send {
        let (message_id, attachment_id) = (message_id.to_string(), attachment_id.to_string());
        async move {
            let token = self.token().await?;
            api::spend(&self.account_id, Call::AttachmentsGet.units()).await;
            Ok(
                api::with_retry(|| api::attachment_get(&token, &message_id, &attachment_id))
                    .await?,
            )
        }
    }

    fn set_flags(
        &self,
        ids: &[String],
        patch: &FlagPatch,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send {
        let ids = ids.to_vec();
        let (add, remove) = label_changes(patch);
        async move { self.modify(ids, add, remove).await }
    }

    fn labels(&self) -> impl Future<Output = Result<Vec<ProviderLabel>, ProviderError>> + Send {
        async move {
            let token = self.token().await?;
            api::spend(&self.account_id, Call::LabelsList.units()).await;
            let labels = api::with_retry(|| api::labels_list(&token)).await?;
            Ok(labels
                .into_iter()
                .map(|label| ProviderLabel {
                    id: label.id,
                    name: label.name,
                    kind: label.kind.to_ascii_lowercase(),
                })
                .collect())
        }
    }

    fn set_labels(
        &self,
        ids: &[String],
        add: &[String],
        remove: &[String],
    ) -> impl Future<Output = Result<(), ProviderError>> + Send {
        let (ids, add, remove) = (ids.to_vec(), add.to_vec(), remove.to_vec());
        async move { self.modify(ids, add, remove).await }
    }

    fn send(
        &self,
        raw: &[u8],
        thread_hint: Option<&str>,
    ) -> impl Future<Output = Result<SentIds, ProviderError>> + Send {
        let raw = raw.to_vec();
        let thread = thread_hint.map(|t| t.to_string());
        async move {
            let token = self.token().await?;
            api::spend(&self.account_id, Call::MessagesSend.units()).await;
            let sent =
                api::with_retry_no_replay(|| api::messages_send(&token, &raw, thread.as_deref()))
                    .await?;
            Ok(SentIds {
                id: sent.id,
                thread_id: sent.thread_id,
            })
        }
    }

    /// The draft id is stable across updates and the message id inside it is not, so the id that
    /// comes back here is the draft's and the caller stores that one.
    fn draft_put(
        &self,
        provider_draft_id: Option<&str>,
        raw: &[u8],
        thread_hint: Option<&str>,
    ) -> impl Future<Output = Result<String, ProviderError>> + Send {
        let existing = provider_draft_id.map(|d| d.to_string());
        let raw = raw.to_vec();
        let thread = thread_hint.map(|t| t.to_string());
        async move {
            let token = self.token().await?;
            let draft = match &existing {
                Some(id) => {
                    api::spend(&self.account_id, Call::DraftsUpdate.units()).await;
                    api::with_retry(|| api::drafts_update(&token, id, &raw, thread.as_deref()))
                        .await?
                }
                None => {
                    api::spend(&self.account_id, Call::DraftsCreate.units()).await;
                    api::with_retry_no_replay(|| api::drafts_create(&token, &raw, thread.as_deref()))
                        .await?
                }
            };
            Ok(draft.id)
        }
    }

    fn draft_delete(
        &self,
        provider_draft_id: &str,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send {
        let id = provider_draft_id.to_string();
        async move {
            let token = self.token().await?;
            api::spend(&self.account_id, Call::DraftsDelete.units()).await;
            match api::with_retry(|| api::drafts_delete(&token, &id)).await {
                // A draft that is already gone is the outcome the caller wanted.
                Err(ApiError::NotFound(_)) | Ok(()) => Ok(()),
                Err(other) => Err(other.into()),
            }
        }
    }

    fn search(
        &self,
        query: &str,
        page: Option<&str>,
    ) -> impl Future<Output = Result<ListPage, ProviderError>> + Send {
        let query = api::search_query(query, None);
        let page = page.map(|p| p.to_string());
        async move {
            let token = self.token().await?;
            api::spend(&self.account_id, Call::MessagesList.units()).await;
            let answer =
                api::with_retry(|| api::messages_list(&token, Some(&query), page.as_deref()))
                    .await?;
            Ok(list_page(answer))
        }
    }

    fn settings(&self) -> impl Future<Output = Result<ProviderSettings, ProviderError>> + Send {
        async move {
            let token = self.token().await?;
            api::spend(&self.account_id, Call::SendAsList.units()).await;
            let send_as = api::with_retry(|| api::send_as_list(&token)).await?;
            Ok(settings_from(&send_as))
        }
    }

    /// Two calls for one answer: `users.getProfile` has no display name in it, and the only place
    /// Gmail keeps one is the primary send-as entry. Two units together, so it is not worth being
    /// clever about.
    fn profile(&self) -> impl Future<Output = Result<ProviderProfile, ProviderError>> + Send {
        async move {
            let token = self.token().await?;
            api::spend(&self.account_id, Call::GetProfile.units()).await;
            let profile = api::with_retry(|| api::get_profile(&token)).await?;

            api::spend(&self.account_id, Call::SendAsList.units()).await;
            let name = match api::with_retry(|| api::send_as_list(&token)).await {
                Ok(send_as) => send_as
                    .iter()
                    .find(|entry| entry.is_primary)
                    .map(|entry| entry.display_name.clone())
                    .unwrap_or_default(),
                // A cleared settings tick box costs a display name, not an account.
                Err(ApiError::InsufficientScope(_)) => String::new(),
                Err(other) => return Err(other.into()),
            };

            Ok(ProviderProfile {
                email: profile.email_address,
                name,
                messages_total: profile.messages_total,
            })
        }
    }

    fn contacts(&self) -> impl Future<Output = Result<Vec<Person>, ProviderError>> + Send {
        async move {
            let token = self.token().await?;
            Ok(people::all(&token).await?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three records out of one poll: a message arrived, it was read, and an old one was purged.
    const HISTORY: &str = r#"{
      "history": [
        {"id":"9912340",
         "messagesAdded":[{"message":{"id":"18f9a2b3c4d50000","threadId":"18f9a2b3c4d50000","labelIds":["UNREAD","INBOX","CATEGORY_PERSONAL"]}}]},
        {"id":"9912344",
         "labelsRemoved":[{"message":{"id":"18f9a2b3c4d50000","threadId":"18f9a2b3c4d50000","labelIds":["INBOX","CATEGORY_PERSONAL"]},"labelIds":["UNREAD"]}]},
        {"id":"9912348",
         "labelsAdded":[{"message":{"id":"18f9a2b3c4d50000","threadId":"18f9a2b3c4d50000","labelIds":["INBOX","CATEGORY_PERSONAL","STARRED"]},"labelIds":["STARRED"]}],
         "labelsRemoved":[{"message":{"id":"18f9a2b3c4d50000","threadId":"18f9a2b3c4d50000","labelIds":["INBOX","CATEGORY_PERSONAL","STARRED"]},"labelIds":["IMPORTANT"]}]},
        {"id":"9912350",
         "messagesDeleted":[{"message":{"id":"18f9a2b3c4d4ffff","threadId":"18f9a2b3c4d4ffff","labelIds":["TRASH"]}}]}
      ],
      "nextPageToken": "09876543210987654321",
      "historyId": "9912351"
    }"#;

    const SEND_AS: &str = r#"{"sendAs":[
      {"sendAsEmail":"you@example.com","displayName":"You","isPrimary":true,"isDefault":true,
       "signature":"<div>Sent from Margin Mail</div>","verificationStatus":""},
      {"sendAsEmail":"hello@yourdomain.example","displayName":"You","treatAsAlias":true,
       "signature":"","verificationStatus":"accepted"},
      {"sendAsEmail":"not-yet@yourdomain.example","displayName":"You","treatAsAlias":true,
       "signature":"","verificationStatus":"pending"}
    ]}"#;

    const METADATA: &str = r#"{
      "id":"18f9a2b3c4d50000","threadId":"18f9a2b3c4d4fff0",
      "labelIds":["UNREAD","INBOX","CATEGORY_PERSONAL"],
      "snippet":"I&#39;m attaching the lease, let me know &amp; I&#39;ll sign",
      "sizeEstimate":48213,"historyId":"9912344","internalDate":"1785808800000",
      "payload":{"headers":[
        {"name":"From","value":"Ana Ruiz <ana@example.com>"},
        {"name":"To","value":"you@example.com"},
        {"name":"Subject","value":"=?UTF-8?B?VGhlIGxlYXNl?="},
        {"name":"References","value":"<a@example.com> <b@example.com>"},
        {"name":"Content-Type","value":"multipart/mixed; boundary=\"x\""}
      ]}
    }"#;

    #[test]
    fn a_snippet_arrives_escaped_and_is_stored_as_the_text_it_reads_as() {
        let message: api::Message = serde_json::from_str(METADATA).expect("metadata");
        let headers = headers_from(&message);
        // Gmail escapes this field and nothing downstream unescapes it, so a row would otherwise
        // print `I&#39;m` beside a subject that prints `I'm`.
        assert_eq!(
            headers.snippet,
            "I'm attaching the lease, let me know & I'll sign"
        );
    }

    #[test]
    fn seen_and_archived_are_removals_and_the_rest_are_additions() {
        let (add, remove) = label_changes(&FlagPatch {
            seen: Some(true),
            archived: Some(true),
            starred: Some(true),
            trashed: None,
            spam: None,
        });
        assert_eq!(add, ["STARRED"]);
        assert_eq!(remove, ["UNREAD", "INBOX"]);
    }

    #[test]
    fn clearing_a_flag_is_the_other_direction() {
        let (add, remove) = label_changes(&FlagPatch {
            seen: Some(false),
            archived: Some(false),
            starred: Some(false),
            trashed: Some(false),
            spam: Some(false),
        });
        assert_eq!(add, ["UNREAD", "INBOX"]);
        assert_eq!(remove, ["STARRED", "TRASH", "SPAM"]);
    }

    #[test]
    fn an_absent_field_touches_no_label() {
        let (add, remove) = label_changes(&FlagPatch::default());
        assert!(add.is_empty() && remove.is_empty());
    }

    /// The mapping has to be the one `provider/fake.rs` applies, or every engine test is testing a
    /// different mailbox from the one the app talks to.
    #[test]
    fn the_flag_mapping_is_the_fakes_mapping() {
        for flag in ["seen", "starred", "archived", "trashed", "spam"] {
            let (label, adds_when_on) = label_for(flag);
            let expected = match flag {
                "seen" => ("UNREAD", false),
                "starred" => ("STARRED", true),
                "archived" => ("INBOX", false),
                "trashed" => ("TRASH", true),
                "spam" => ("SPAM", true),
                _ => unreachable!(),
            };
            assert_eq!((label, adds_when_on), expected, "{flag}");
        }
        assert_eq!(label_for("snoozed").0, "");
    }

    #[test]
    fn an_expired_change_log_is_a_full_sync_and_not_a_failure() {
        assert_eq!(
            history_error(ApiError::NotFound("Requested entity was not found.".into())),
            ProviderError::NeedsFullSync
        );
        assert_eq!(
            history_error(ApiError::Unauthorized("Invalid Credentials".into())),
            ProviderError::Auth("Invalid Credentials".into())
        );
    }

    #[test]
    fn a_cursor_gmail_will_not_read_is_a_full_sync_too() {
        assert_eq!(
            history_error(ApiError::Other(
                "Gmail history failed (400): Invalid startHistoryId".into()
            )),
            ProviderError::NeedsFullSync
        );
        assert!(matches!(
            history_error(ApiError::Other("Gmail history failed (403): Daily Limit Exceeded".into())),
            ProviderError::Other(_)
        ));
    }

    #[test]
    fn only_google_refusing_the_token_is_a_sign_out() {
        // The refresh could not be sent: the network, with the endpoint's URL gone.
        let error = session_error(
            "error sending request for url (https://oauth2.googleapis.com/token)".to_string(),
        );
        let ProviderError::Network(said) = error else {
            panic!("a transport failure is the network, got {error:?}");
        };
        assert!(!said.contains("http"), "{said}");

        // Google refused the refresh token: signed out, with Google's sentence and not its JSON.
        assert_eq!(
            session_error(
                r#"Google token refresh failed (400 Bad Request): {"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#
                    .to_string()
            ),
            ProviderError::Auth("Token has been expired or revoked.".to_string())
        );

        // The token service is down: nobody has been signed out.
        assert!(matches!(
            session_error("Google token refresh failed (503 Service Unavailable): <html>".to_string()),
            ProviderError::RateLimited { .. }
        ));

        // No token on this device at all is a sign-out.
        assert!(matches!(
            session_error("Account 1 is not connected to Google.".to_string()),
            ProviderError::Auth(_)
        ));
    }

    #[test]
    fn a_history_page_becomes_one_change_per_thing_that_happened() {
        let page: api::HistoryPage = serde_json::from_str(HISTORY).expect("the capture");
        let changes = changes_from(&page);
        assert_eq!(changes.cursor, "9912351");
        assert_eq!(changes.next_page.as_deref(), Some("09876543210987654321"));

        assert_eq!(
            changes.changes,
            vec![
                Change::Added(MessageRef {
                    id: "18f9a2b3c4d50000".into(),
                    thread_id: "18f9a2b3c4d50000".into(),
                }),
                Change::LabelsChanged {
                    id: "18f9a2b3c4d50000".into(),
                    labels: vec!["INBOX".into(), "CATEGORY_PERSONAL".into()],
                },
                // The record that both added and removed a label says the same thing twice, and is
                // only worth saying once.
                Change::LabelsChanged {
                    id: "18f9a2b3c4d50000".into(),
                    labels: vec![
                        "INBOX".into(),
                        "CATEGORY_PERSONAL".into(),
                        "STARRED".into()
                    ],
                },
                Change::Deleted("18f9a2b3c4d4ffff".into()),
            ]
        );
    }

    #[test]
    fn an_empty_history_page_still_carries_a_cursor() {
        let page: api::HistoryPage =
            serde_json::from_str(r#"{"historyId":"9912351"}"#).expect("an empty page");
        let changes = changes_from(&page);
        assert!(changes.changes.is_empty());
        assert_eq!(changes.cursor, "9912351");
        assert_eq!(changes.next_page, None);
    }

    #[test]
    fn metadata_becomes_headers_with_the_pairs_left_as_they_arrived() {
        let message: api::Message = serde_json::from_str(METADATA).expect("the capture");
        let headers = headers_from(&message);
        assert_eq!(headers.id, "18f9a2b3c4d50000");
        assert_eq!(headers.thread_id, "18f9a2b3c4d4fff0");
        assert_eq!(headers.internal_date_ms, 1_785_808_800_000);
        assert_eq!(headers.size, 48_213);
        assert_eq!(headers.label_ids.len(), 3);
        assert_eq!(headers.header("from"), Some("Ana Ruiz <ana@example.com>"));
        // Decoding encoded words is the MIME parser's job, not this layer's.
        assert_eq!(headers.header("Subject"), Some("=?UTF-8?B?VGhlIGxlYXNl?="));
        assert_eq!(headers.header("Bcc"), None);
    }

    #[test]
    fn the_accounts_own_address_comes_first_and_unverified_aliases_do_not_come_at_all() {
        let page: api::SendAsPage = serde_json::from_str(SEND_AS).expect("the capture");
        let settings = settings_from(&page.send_as);
        assert_eq!(
            settings.aliases,
            ["you@example.com", "hello@yourdomain.example"]
        );
        assert_eq!(settings.signature, "<div>Sent from Margin Mail</div>");
    }

    #[test]
    fn a_page_of_ids_keeps_googles_estimate_as_a_hint() {
        let page: api::MessageIdsPage = serde_json::from_str(
            r#"{"messages":[{"id":"a","threadId":"t1"},{"id":"b","threadId":"t1"}],
                "nextPageToken":"tok","resultSizeEstimate":20123}"#,
        )
        .expect("a list page");
        let listed = list_page(page);
        assert_eq!(listed.messages.len(), 2);
        assert_eq!(listed.messages[0].thread_id, "t1");
        assert_eq!(listed.next_page.as_deref(), Some("tok"));
        assert_eq!(listed.estimate, Some(20_123));
    }

    #[test]
    fn the_last_page_of_a_list_has_no_next_page() {
        let page: api::MessageIdsPage =
            serde_json::from_str(r#"{"messages":[{"id":"a","threadId":"t1"}],"nextPageToken":""}"#)
                .expect("a last page");
        assert_eq!(list_page(page).next_page, None);
    }
}
