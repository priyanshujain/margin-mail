# Gmail API capability map

What the Gmail REST API (v1) and its neighbours can and cannot do for a native desktop
client (Tauri 2, Rust backend, React frontend) that wants HEY and Superhuman style
workflows on top of a user's Gmail account. Everything below was checked against the
live Google docs on 2026-09-03 unless marked "unverified" or "third-party".

One headline before the detail: the Gmail API usage limits changed on 1 May 2026. The
per-user rate limit is now 6,000 quota units per minute (100 per second, not the 250
per second most older write-ups quote), `messages.get` costs 20 units (not 5) and
`threads.get` costs 40 (not 10). Every quota figure in this document uses the new
table. Plan sync around it.

## 1. Auth and distribution

### Scopes

| Scope | Unlocks | Class |
|---|---|---|
| `https://mail.google.com/` | Everything, including permanent delete (`messages.delete`, `batchDelete`, `threads.delete`) and IMAP/SMTP via XOAUTH2. The only scope IMAP accepts. | Restricted |
| `https://www.googleapis.com/auth/gmail.modify` | Read, list, modify labels, trash/untrash, send, drafts, insert/import, history, watch. Not permanent delete. | Restricted |
| `https://www.googleapis.com/auth/gmail.readonly` | Read and list only, including history and watch. | Restricted |
| `https://www.googleapis.com/auth/gmail.compose` | Drafts create/update/send and `messages.send`. No reading. | Restricted |
| `https://www.googleapis.com/auth/gmail.send` | `messages.send` only. | Sensitive |
| `https://www.googleapis.com/auth/gmail.insert` | `messages.insert` and `messages.import`. | Restricted |
| `https://www.googleapis.com/auth/gmail.labels` | `labels.*` only. | Non-sensitive |
| `https://www.googleapis.com/auth/gmail.metadata` | List and read headers and labels, never bodies. `format=full` and `format=raw` are refused, and `messages.list` refuses the `q` parameter under this scope. | Restricted |
| `https://www.googleapis.com/auth/gmail.settings.basic` | Filters, vacation responder, IMAP/POP/language settings, update of the primary send-as (signature, display name). | Restricted |
| `https://www.googleapis.com/auth/gmail.settings.sharing` | Send-as alias create/delete/verify, forwarding addresses, auto-forwarding, delegates. Every one of these methods is documented as "only available to service account clients that have been delegated domain-wide authority", so a consumer desktop client cannot use them at all. | Restricted |
| `https://www.googleapis.com/auth/contacts.other.readonly` | People API `otherContacts.list` and `otherContacts.search` (the auto-collected "people you emailed" set that powers Gmail autocomplete). | Sensitive (third-party classification, unverified against Google's master list) |
| `https://www.googleapis.com/auth/contacts.readonly` | People API `people.connections.list`, `people.searchContacts`. | Sensitive (unverified, as above) |
| `https://www.googleapis.com/auth/calendar.events` | Calendar `events.list`, `events.patch`, `events.import` for RSVP. | Sensitive (unverified) |
| `https://www.googleapis.com/auth/pubsub` | Only needed if the client itself pulls from a Pub/Sub subscription. See section 3. | Cloud scope, not a Gmail scope |

Minimum set for the product as described: `gmail.modify` (which covers `gmail.labels`),
`gmail.settings.basic`, `contacts.other.readonly`, `contacts.readonly`, and
`calendar.events` if you do RSVP. Add `https://mail.google.com/` only if you use IMAP
or want permanent delete. Any Gmail scope beyond `gmail.send` and `gmail.labels` is
restricted, so there is no scope choice that avoids restricted-scope verification for a
real mail client.

### What verification means in practice

Publishing status and user caps, quoted from Google's "Manage App Audience" page:

- **Testing**: "up to 100 test users listed in the OAuth consent screen". Every test
  user must be added by email address by the project owner. "Authorizations by a test
  user will expire seven days from the time of consent", and that includes the refresh
  token. The only exception is apps that request nothing beyond name, email and profile.
  A mail client in Testing therefore forces a full re-login every 7 days. Do not ship
  in Testing.
- **In production, unverified**: any Google account can be presented with the
  "unverified app" interstitial and click through. The project gets "100 new users in
  total" for its lifetime; the cap "cannot be reset". Refresh tokens do not have the
  7-day expiry in this state.
- **In production, verified**: no cap, no interstitial (after brand verification the
  app name and logo show on the consent screen).

Verification tiers and timelines:

- Brand verification (name and logo only): "typically takes 2-3 business days".
- Sensitive scope verification: "typically takes 3-5 business days". Requires a
  domain verified in Search Console, a public homepage on that domain, a privacy
  policy hosted on the same domain and linked from the consent screen, an unlisted
  YouTube demo showing the consent flow with the client ID visible in the address bar
  and each scope in use, and a written justification per scope.
- Restricted scope verification: all of the above plus Limited Use compliance, and the
  process "can potentially take several weeks". Re-verification is annual.
- Security assessment (CASA, run by the App Defense Alliance): Google's wording is
  "Every app that requests access to Google users' restricted data and has the ability
  to access data from or through a third-party server must go through a security
  assessment from Google-empanelled security assessors." Assessments are assigned an
  assurance level (AL1 or AL2) and "All applications must be revalidated every year."
  Third-party pricing for the common Tier 2 / lab-verified scan is roughly USD 540 to
  1,800 per year (TAC Security via switchlabs.dev, 2025 to 2026); older figures of USD
  15,000 to 75,000 (Nylas, 2021) refer to the original pentest regime and are stale.

The open question that decides Margin's cost: a desktop client whose only server is
Google, with all mail data on the user's disk, does not obviously "access data from or
through a third-party server". Google publishes no explicit "local-only apps are exempt"
clause, so treat this as unverified and ask the verification team in writing before
you build a push relay (a relay that sees only the user's email address and a
`historyId`, as Mimestream's does, may or may not count). Mimestream states it
"applied for and completed Google's Restricted scope verification process" and that
"Mimestream stores user email data locally on the user's device", but does not say
whether it was assessed.

Exemptions Google lists for verification itself: "personal use (fewer than 100
users)", apps in development/testing/staging, service-account-only apps, and internal
Workspace apps. None of these is a distribution strategy.

### Bring your own OAuth client

The pattern: each user creates their own Google Cloud project, enables the Gmail API,
configures an External consent screen, adds every scope, creates a "Desktop app" OAuth
client, and pastes the client ID and secret into Margin. Each user is then the
"developer" of a one-user app, which falls under the personal-use exemption and needs
no verification. It works, and several open-source tools ship this way.

It is not realistic for non-technical friends. It is fifteen to twenty clicks across a
console whose UI changes every few months, it needs the People API and Calendar API
enabled separately, the consent screen must either stay in Testing (7-day re-login)
or be pushed to production (scary interstitial, but stable tokens), and support
requests will all be "the Google page looks different". Offer it as an escape hatch
for the technical, not as the default.

### Installed-app OAuth flow

Use the loopback flow: OAuth client type "Desktop app", redirect to
`http://127.0.0.1:{port}` (or `http://[::1]:{port}`) served by a one-shot listener in the
Rust backend, PKCE mandatory (verifier 43 to 128 unreserved characters, `S256`
challenge). Google states "Custom URI schemes are no longer supported due to the risk
of app impersonation" and the out-of-band copy-paste flow is gone. The client secret
of a Desktop client is not a secret; Google's own doc says installed apps "cannot keep
secrets", so embedding it in the binary is expected.

Token facts to design around:

- "refresh tokens are always returned for installed applications".
- "limit of 100 refresh tokens per Google Account per OAuth 2.0 client ID". The oldest
  is silently invalidated. Per-account, per-client, so a user on many machines is fine.
- A refresh token dies if unused for six months, if the user revokes it, or if "The
  user changed passwords and the refresh token contains Gmail scopes". Password change
  logs Margin out. Handle `invalid_grant` by prompting re-auth, not by crashing sync.
- Access tokens last about an hour. IMAP sessions authenticated with OAuth are "limited
  to about the validity period of the access token used (usually 1 hour)".

Store refresh tokens in the OS keychain (macOS Keychain, Windows Credential Manager,
Secret Service on Linux), never in SQLite.

### What a small team shipping to friends should do

Create one Cloud project and one Desktop OAuth client. Push the consent screen to
production unverified straight away and eat the interstitial: it costs 100 lifetime
users, which is plenty for a friends release and avoids the 7-day token expiry that
Testing imposes. In parallel, buy a domain, publish a homepage and privacy policy on
it, record the demo video, and submit restricted scope verification with the explicit
statement that the app has no server and stores all Google user data on the user's
device; ask whether a security assessment is required. Budget for one anyway (order of
USD 1,000 to 2,000 per year at the cheap end). Do not request `https://mail.google.com/`
unless IMAP is in the plan; the narrower the scope list the easier the review.

Sources: https://developers.google.com/workspace/gmail/api/auth/scopes,
https://support.google.com/cloud/answer/15549945,
https://support.google.com/cloud/answer/7454865,
https://support.google.com/cloud/answer/13464323,
https://support.google.com/cloud/answer/13465431,
https://developers.google.com/identity/protocols/oauth2/production-readiness/restricted-scope-verification,
https://developers.google.com/identity/protocols/oauth2/production-readiness/sensitive-scope-verification,
https://developers.google.com/identity/protocols/oauth2/native-app,
https://developers.google.com/identity/protocols/oauth2 (refresh token expiration),
https://developers.google.com/terms/api-services-user-data-policy,
https://mimestream.com/trust/security-and-privacy,
https://www.switchlabs.dev/post/casa-tier-2-tier-3-security-review-providers-pricing-and-the-cheapest-option (third-party),
https://www.nylas.com/blog/google-oauth-app-verification/ (third-party, 2021).

## 2. Data model

Base URL is `https://gmail.googleapis.com/gmail/v1/users/{userId}/...`; use `me` for
`userId`. Uploads go to `https://gmail.googleapis.com/upload/gmail/v1/...`.

### Message

Fields: `id` ("The immutable ID of the message", a 16-hex-digit string), `threadId`,
`labelIds[]`, `snippet` ("A short part of the message text", HTML-escaped, roughly a
sentence), `historyId` ("The ID of the last history record that modified this
message"), `internalDate` ("The internal message creation timestamp (epoch ms)", the
time Gmail received or created it, not the `Date` header; for `insert`/`import` you
choose `internalDateSource=receivedTime|dateHeader`), `payload` (parsed MIME tree),
`sizeEstimate` (bytes, approximate), `raw` (base64url RFC 2822 bytes, only with
`format=raw`).

`format` on `messages.get` and `threads.get`:

| format | Returns | Notes |
|---|---|---|
| `minimal` | id, threadId, labelIds, snippet, historyId, internalDate, sizeEstimate | No headers. Same 20 units as `full`. |
| `metadata` | minimal plus `payload.headers` | Pass `metadataHeaders=From&metadataHeaders=Subject...` to restrict which headers come back. No body parts, no MIME tree. |
| `full` | Parsed MIME tree in `payload` | Body data is base64url in `payload.parts[].body.data`, or referenced by `attachmentId` when large. |
| `raw` | Whole RFC 2822 message base64url in `raw` | Best for archival and for feeding a real MIME parser (mail-parser or mailparse in Rust). |

`MessagePart`: `partId`, `mimeType`, `filename` (only present on attachments),
`headers[] {name, value}`, `body {attachmentId, size, data}`, `parts[]`. Body `data`
is base64url without padding issues in practice but always decode with a
URL-safe, padding-tolerant decoder. Large bodies and every real attachment arrive as
`attachmentId` and must be fetched separately with
`GET .../messages/{messageId}/attachments/{id}` (20 units, returns
`MessagePartBody {attachmentId, size, data}`). Inline images are ordinary parts whose
headers carry `Content-ID: <foo>` and `Content-Disposition: inline`; the HTML body
references them as `src="cid:foo"`, and the client maps cid to the part and serves the
bytes to the webview. Attachment IDs are widely reported to change between `get`
calls, so store the `partId`/`Content-ID` path, not the attachment ID (unverified in
the docs, consistently observed).

Sort and display by `internalDate`. The `Date` header is the sender's claim and can be
hours off or absent.

### Thread

`{id, snippet, historyId, messages[]}`. `threads.get` accepts the same `format` and
`metadataHeaders` and returns every message in the thread in one call for 40 units.
Threads cannot be created directly; Gmail assigns `threadId` on delivery, `send`,
`insert` or `import`. Threading rules are in section 4. Gmail's web UI splits a
conversation into a new thread with the same subject after 100 messages (third-party,
consistently reported, unverified in Google docs).

### Labels

System labels the guide lists, and whether `modify` may add or remove them:

| Label | Applicable via API |
|---|---|
| `INBOX` | Yes (remove to archive) |
| `SPAM` | Yes |
| `TRASH` | Yes (prefer `trash`/`untrash` methods) |
| `UNREAD` | Yes |
| `STARRED` | Yes |
| `IMPORTANT` | Yes |
| `SENT` | No |
| `DRAFT` | No |
| `CATEGORY_PERSONAL`, `CATEGORY_SOCIAL`, `CATEGORY_PROMOTIONS`, `CATEGORY_UPDATES`, `CATEGORY_FORUMS` | Yes |

"The preceding list isn't exhaustive and other reserved label names exist" (`CHAT` is
one). Creating a user label with a reserved name returns "HTTP 400 - Invalid label
name". System label IDs equal their names; user label IDs look like `Label_42` and
are immutable, while names can be changed with `labels.patch`, so key everything on
ID and re-list labels when history shows anything unexpected.

User label fields: `name` (a `/` in the name nests the label in the Gmail UI, so
`Margin/Snoozed` renders under `Margin`; the parent label should exist first),
`messageListVisibility` (`show`, `hide`), `labelListVisibility` (`labelShow`,
`labelShowIfUnread`, `labelHide`), `color {textColor, backgroundColor}` restricted to
a fixed palette of about a hundred hex values listed in the Label resource docs (any
other value is rejected), `type` (`system`, `user`), and read-only counts
`messagesTotal`, `messagesUnread`, `threadsTotal`, `threadsUnread`. Hard limit:
"Maximum labels per mailbox: 10,000". Name length limit is not documented (the web
UI enforces 225 characters, unverified). Labels cannot be applied to drafts.

### Drafts

`{id, message}`. The draft `id` is stable across `drafts.update`; the inner
`message.id` changes on every update. `drafts.send` deletes the draft and returns a new
message with a new `id` and the `SENT` label. Draft messages only ever carry the
`DRAFT` label.

### History and historyId

`historyId` appears on every message, thread, the profile (`users.getProfile`, 1
unit, returns `emailAddress`, `messagesTotal`, `threadsTotal`, `historyId`), and every
`history.list` and `watch` response. Google: "History IDs increase chronologically but
are not contiguous with random gaps in between valid IDs." A history record is
`{id, messages[], messagesAdded[], messagesDeleted[], labelsAdded[], labelsRemoved[]}`
where the added/removed entries carry `{message: {id, threadId, labelIds?}, labelIds[]}`.
"We recommend using the specific change-type fields instead of" `messages[]`.
`messagesDeleted` means permanently deleted, not trashed; trash shows up as
`labelsAdded` with `TRASH`.

Stable IDs: message id, thread id, label id, draft id (until sent), historyId (as a
cursor). Not stable: the message id behind a draft, attachment ids (reported),
`snippet` text (can be regenerated), `resultSizeEstimate` (an estimate).

Sources: https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/Format,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages.attachments/get,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.threads,
https://developers.google.com/workspace/gmail/api/guides/labels,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.labels,
https://developers.google.com/workspace/gmail/api/guides/drafts,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.history/list,
https://support.cloudhq.net/how-does-gmail-decide-to-group-emails-into-conversations/ (third-party).

## 3. Sync strategy

### Quota, the numbers that matter

"As of May 1, 2026, the usage limits for this API were updated."

| Limit | Value |
|---|---|
| Per minute per user per project | 6,000 quota units |
| Per minute per project | 1,200,000 quota units |
| Per day per project (billing threshold) | 80,000,000 quota units |
| Batch | 100 calls max per batch, "larger than 50 requests is not recommended", counted as n requests |
| Pricing | "All standard use of the Gmail API is available at no additional cost. Exceeding the quota request limits is planned to incur charges to your Google Cloud billing account later in 2026." |

Per-method costs (units):

| Method | Units | Method | Units |
|---|---|---|---|
| `messages.send`, `drafts.send`, `watch` | 100 | `messages.get`, `drafts.get`, `messages.attachments.get`, `messages.trash` | 20 |
| `threads.get` | 40 | `threads.trash`, `threads.delete` | 20 |
| `messages.batchModify`, `messages.batchDelete`, `stop` | 50 | `messages.insert`, `messages.import` | 25 |
| `drafts.update` | 15 | `messages.delete`, `drafts.create`, `drafts.delete`, `threads.list`, `threads.modify` | 10 |
| `messages.list`, `drafts.list`, `messages.modify`, `messages.untrash`, `labels.create/delete/update`, `settings.filters.create/delete`, `settings.updateVacation` | 5 | `history.list` | 2 |
| `getProfile`, `labels.get`, `labels.list`, `settings.filters.get/list`, `settings.getVacation`, `settings.getAutoForwarding` | 1 | `settings.sendAs.create/update/verify`, `forwardingAddresses.create`, `delegates.create`, `updatePop` | 100 |

Third-party pages dated 2026 still quote `messages.get` at 5 and `threads.get` at 10;
Google's page as read on 2026-09-03 says 20 and 40. There is also an undocumented
concurrency ceiling per mailbox (Unipile reports "50 concurrent in-flight requests
per mailbox"; Google's error page only says 429 can be "triggered by daily per-user
limits, bandwidth limits, or concurrent request limits" and "Per-user limits cannot
be increased").

What 6,000 units per minute buys: 300 `messages.get` per minute, or 150 `threads.get`,
or 60 sends. A mailbox with 20,000 messages costs 400,000 units to fetch metadata for
every message, which is 67 minutes at the ceiling; 100,000 messages is 5.6 hours.
Idle polling of `history.list` every 15 seconds is 8 units per minute, so polling is
free; hydration is what costs.

### Initial full sync

1. `getProfile` and record `historyId` before you list anything, so the first partial
   sync covers everything that changed during the crawl.
2. `messages.list?maxResults=500&includeSpamTrash=true` (5 units per page, ids and
   threadIds only, newest first) and page with `pageToken` until exhausted. 20,000
   messages is 40 pages, 200 units, a few seconds. Store ids, threadIds and a
   "needs hydration" flag.
3. Hydrate newest first in batches of 50 `messages.get?format=metadata` with
   `metadataHeaders` limited to what the UI and the classifier need: `From`, `To`,
   `Cc`, `Bcc`, `Reply-To`, `Subject`, `Date`, `Message-ID`, `In-Reply-To`,
   `References`, `List-Id`, `List-Unsubscribe`, `List-Unsubscribe-Post`, `Precedence`,
   `Authentication-Results`, `Content-Type`. Throttle to roughly 280 gets per
   minute, retry 429 and 403 `userRateLimitExceeded` with truncated exponential
   backoff starting at 1 second, and treat 404 as "deleted since listing".
4. For threads with three or more messages, `threads.get?format=metadata` (40 units)
   is cheaper than per-message gets. Group step 2's ids by threadId and route.
5. Bodies: fetch `format=full` (or `raw`) lazily on open and prefetch the last N days
   in the background. Cache decoded HTML and text in SQLite with an LRU cap.
6. Labels: `labels.list` (1 unit) once, then again whenever history mentions a label
   id you do not know.

Order of hydration is a product decision: newest 30 days first, then the rest, and the
UI must be usable while step 3 is running. Zero (Mail-0) reports initial sync "can take
hours for large inboxes (10,000+ emails)"; with the new unit costs that is the norm,
not the exception.

### Partial sync

`history.list?startHistoryId=X&maxResults=500` (2 units per page), optionally
`historyTypes=messageAdded|messageDeleted|labelAdded|labelRemoved` and `labelId=`.
Apply pages in order; the records are chronological by `id`. For each `messagesAdded`
run `messages.get` (the record includes `labelIds`, but not headers); for
`labelsAdded`/`labelsRemoved` update the local label set; for `messagesDeleted`
delete locally. A message can appear in `messagesAdded` and `messagesDeleted` in the
same window, and `messages.get` can 404 for a record you have not processed yet;
both are normal, not errors. When the final page has no `nextPageToken`, store the
response `historyId` as the new cursor. Never combine `q` with history; history is
mailbox-wide and has no query parameter, so every screening decision is made locally
on the record's `labelIds` and the fetched headers.

Google on validity: "A historyId is typically valid for at least a week, but in some
rare circumstances may be valid for only a few hours. If you receive an HTTP 404 error
response, your application should perform a full sync." The cheap recovery is: re-run
step 2 of the full sync (ids only, 5 units per 500), diff against the local set to
find adds and deletes, hydrate the adds, and refresh `labelIds` for the most recent N
days with `format=minimal` (still 20 units each; there is no cheaper label-only read
except `threads.list`, which returns only ids). Accept that label state for old mail
may be stale until the user opens it.

### Polling cadence

Poll `history.list` every 10 to 15 seconds while the window is focused, 60 seconds in
the background, and immediately after any local write. Cost at 15 seconds is 5,760
units per user per day; 1,000 users idle-polling this way is 5.8 million units per
day against the 80 million project threshold. The budget goes to hydration and full
syncs, not polling.

### Push via Pub/Sub, honestly

`users.watch` (100 units) with `topicName=projects/{project}/topics/{topic}` where the
project "must exactly match your Google developer project id (the one executing this
watch request)". You grant `roles/pubsub.publisher` on the topic to
`gmail-api-push@system.gserviceaccount.com`. Watches expire after 7 days and Google
says "We recommend calling watch once per day". The notification is only
`{"emailAddress": "...", "historyId": "..."}` and the rate is capped at "one event per
second" per user. On receipt you run the partial sync above.

Delivery needs a Pub/Sub subscription. A push subscription posts to an HTTPS endpoint,
which means a server. A pull subscription is possible from a desktop app in theory:
`projects.subscriptions.pull` needs the `pubsub.subscriptions.consume` permission
(`roles/pubsub.subscriber`) on the subscription and a token with
`https://www.googleapis.com/auth/pubsub` or `cloud-platform`. The problem is the
principal. The desktop app authenticates as the end user's Google account, which has
no IAM on your project. Your options are to grant every user individually (a server
or manual step), grant `allAuthenticatedUsers` (any Google account on earth could pull
and ack everyone's notifications from the shared subscription), or ship a service
account key in the binary (extractable, and Google will reject it in review). None is
acceptable. Conclusion: with no server there is no push. If you want push later, the
minimum is a small relay that owns the subscription and forwards `{emailAddress,
historyId}` to connected clients; Mimestream runs exactly that (`push.mimestream.com`)
and stores only "Email address, APNs device token, Device identifier". That relay may
also drag you into the security assessment. Ship with polling.

### IMAP as a complement

Gmail IMAP (`imap.gmail.com:993`, SMTP `smtp.gmail.com:465` or `587`) authenticates with
SASL XOAUTH2 (`base64("user=" user "^Aauth=Bearer " token "^A^A")`) using the
`https://mail.google.com/` scope, which is restricted like the rest. Capabilities
include `IDLE`, `CONDSTORE`, `MOVE`, `UIDPLUS`, `X-GM-EXT-1` and `APPENDLIMIT=35651584`;
there is no `QRESYNC`. The Gmail extensions expose `X-GM-MSGID` (64-bit message id),
`X-GM-THRID` (thread id), `X-GM-LABELS` (fetch, store and search labels) and
`X-GM-RAW` (full Gmail search syntax over IMAP). The REST `id` is the lowercase hex of
`X-GM-MSGID` and `threadId` the hex of `X-GM-THRID` (widely relied on, verify in a
spike before depending on it).

What IMAP does that REST cannot: bulk header fetch with no unit quota (one `UID FETCH
1:* (ENVELOPE BODYSTRUCTURE X-GM-LABELS X-GM-THRID X-GM-MSGID)` over `[Gmail]/All Mail`
returns tens of thousands of messages in minutes, limited only by the 2,500 MB per day
IMAP download bandwidth), and `IDLE` for near-instant new-mail notification without a
server (one connection per watched folder, sessions "limited to about 24 hours" and
OAuth sessions to the token lifetime, so reconnect hourly). What REST does that IMAP
cannot: `history.list` (IMAP has no mailbox-wide change log; `CONDSTORE` gives per-folder
`MODSEQ` only), filters, settings, send-as, vacation, drafts with stable ids, proper
`SENT` threading via `threadId`, snippets, `sizeEstimate`, category labels as first-class
ids, and batch HTTP. IMAP exceeding bandwidth suspends the account for "1 hour, but can
last up to 24 hours".

Reasonable hybrid: REST for everything the user touches and for the change log; IMAP
only as an optional accelerator for the initial backfill and for `IDLE` as a wake-up
signal that triggers `history.list`. That costs you the `https://mail.google.com/`
scope and a second protocol stack, so do it only if the initial sync time measured
with REST alone is unacceptable.

### Local mirror

Mimestream keeps everything in a Core Data SQLite store on the Mac and syncs "directly
with Google APIs, not through an intermediary service". Do the same: tables for
accounts (email, historyId cursor, profile), labels (id, name, type, visibility,
colour), messages (id, threadId, internalDate, sizeEstimate, snippet, labelIds as a
JSON array or a join table, parsed headers as columns, hydration state), threads
(id, latest internalDate, participant summary, derived flags), bodies (message id,
html, text, fetched_at), attachments (message id, partId, filename, mimeType, size,
content-id, cached path), an outbox, and an FTS5 table over subject, participants and
body text. Everything in section 5 marked "client-side" also lives here.

Sources: https://developers.google.com/workspace/gmail/api/reference/quota,
https://developers.google.com/workspace/gmail/api/guides/sync,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.history/list,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages/list,
https://developers.google.com/workspace/gmail/api/guides/batch,
https://developers.google.com/workspace/gmail/api/guides/handle-errors,
https://developers.google.com/workspace/gmail/api/guides/push,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users/watch,
https://docs.cloud.google.com/pubsub/docs/access-control,
https://docs.cloud.google.com/pubsub/docs/reference/rest/v1/projects.subscriptions/pull,
https://mimestream.com/trust/private-push,
https://developers.google.com/workspace/gmail/imap/imap-extensions,
https://developers.google.com/workspace/gmail/imap/xoauth2-protocol,
https://developers.google.com/workspace/gmail/imap/imap-smtp,
https://knowledge.workspace.google.com/admin/gmail/gmail-bandwidth-limits,
https://gist.github.com/emersion/2c769bc1ed60a7b7945910d35b606801 (third-party capability dump),
https://www.unipile.com/gmail-api-limits/ (third-party).

## 4. Writes

### Labels and state

- `messages.modify` (5 units): `{addLabelIds[], removeLabelIds[]}`.
- `messages.batchModify` (50 units): same body plus `ids[]`, "There is a limit of 1000
  ids per request", empty response, no per-id error report.
- `threads.modify` (10 units): applies to every message in the thread. Note that
  "Messages in a thread might have labels that other messages in the same thread
  don't have", so thread-level views must aggregate.
- Read/unread: remove/add `UNREAD`. Star: `STARRED`. Importance: `IMPORTANT`. Archive:
  remove `INBOX`. Spam: add `SPAM` (removes from inbox in Gmail's view). Category
  move: remove one `CATEGORY_*`, add another.
- Trash: `messages.trash` / `threads.trash` (20) and `untrash` (5). Gmail purges trash
  after 30 days. Permanent delete: `messages.delete` (10), `batchDelete` (50),
  `threads.delete` (20); these need `https://mail.google.com/` (long-standing,
  unverified in this pass).
- Mute: there is no mute label or method. `q=is:muted` does find muted threads, so
  the state is readable but not writable. Emulate with a `Margin/Muted` label and a
  client rule that strips `INBOX` from new messages in muted threads as they arrive in
  history. While Margin is not running, Gmail web will show them in the inbox.

### Sending

`messages.send` (100 units): body `{raw, threadId?}` where `raw` is the complete RFC
2822 message base64url-encoded. Simple JSON body is fine for small messages; for
anything with attachments use the upload URI with `uploadType=multipart` or
`resumable`. Discovery document `maxSize` for send is 36,700,160 bytes (35 MB) for the
whole encoded message; the Gmail web UI limit for attachments is 25 MB (unverified).
`insert` and `import` accept 157,286,400 bytes (150 MB). Consumer accounts are capped
at 500 messages per day and Workspace at 2,000 (Google's help page confirms the 500;
the 2,000 is third-party), counted across web, IMAP and API.

Threading rules for a reply, all three required: the `threadId` in the request, and
"The References and In-Reply-To headers must be set in compliance with the RFC 2822
standard", and "The Subject headers must match" (Gmail tolerates `Re:`, `Fwd:`, `R:`
and similar prefixes). Since the March 2019 change Gmail also requires that "an
incoming message's Reference header, if present, must reference IDs of previous
messages"; same subject and participants alone no longer thread. Practical recipe:
`In-Reply-To: <parent Message-ID>`, `References: <parent References> <parent
Message-ID>`, subject `Re: <original>`, `threadId` of the parent. Changing the subject
starts a new thread on the recipient's side.

Other send facts: the `From` header must be the account's primary address or a
verified send-as alias, otherwise Gmail rewrites it to the primary (documented
behaviour of aliases, rewriting itself reported by developers, unverified in the
reference). `labelIds` in the send body should be assumed ignored; apply labels with
`messages.modify` on the returned id (unverified). Gmail generates a `Message-ID` if
you omit one; supplying your own lets you correlate the outbox with the sent copy.
The response contains the new `id`, `threadId` and `labelIds` (`SENT`). There is no
scheduled send and no undo send in the API; Gmail web's Undo Send is a client-side
hold of 5, 10, 20 or 30 seconds before the message leaves, which is exactly what
Margin should implement.

### Drafts

`drafts.create` (10), `drafts.update` (15), `drafts.get` (20), `drafts.list` (5),
`drafts.delete` (10), `drafts.send` (100, "You can send as-is or provide updates by
including a new MIME message"). Put `threadId` inside `message` for reply drafts.
Drafts created via the API appear in Gmail web's Drafts folder, which is the only
roaming compose state you get.

### Settings

- `settings.sendAs.list/get`: enumerate the primary address and aliases with
  `displayName`, `replyToAddress`, `signature` (HTML, "added to new emails only",
  Gmail "will sanitize the HTML before saving it"), `isPrimary`, `isDefault`,
  `treatAsAlias`, `verificationStatus` (`accepted`, `pending`). `sendAs.update/patch`
  on the primary address works with `gmail.settings.basic`; "Addresses other than the
  primary address for the account can only be updated by service account clients that
  have been delegated domain-wide authority", and `create`, `delete`, `verify` are
  service-account-only too. So: read aliases, send from them, edit the primary
  signature; nothing else.
- `settings.getVacation/updateVacation` (`gmail.settings.basic`):
  `{enableAutoReply, responseSubject, responseBodyPlainText, responseBodyHtml,
  restrictToContacts, restrictToDomain, startTime, endTime}`.
- Filters (`gmail.settings.basic`): `settings.filters.create/get/list/delete`, no
  update ("filters must be deleted and recreated"), "you can only create a maximum of
  1,000 filters". Criteria: `from`, `to` (matches the local part, case-insensitive),
  `subject`, `query` (Gmail search syntax), `negatedQuery`, `hasAttachment`,
  `excludeChats`, `size` with `sizeComparison` (`smaller`, `larger`). Actions:
  `addLabelIds[]`, `removeLabelIds[]`, `forward` (needs a verified forwarding address,
  which you cannot create). The guide's examples use `INBOX` and `UNREAD` in
  `removeLabelIds` (skip inbox, mark read), `TRASH`, `STARRED`, `IMPORTANT` in
  `addLabelIds`, and one user label per filter; the exact accepted set is not spelled
  out. "Filters only apply to specific messages and not the entire email thread", and
  they act on mail as it arrives; the web UI's "also apply filter to matching
  conversations" has no API equivalent, so backfilling is your job with `batchModify`.
- Forwarding addresses, auto-forwarding, delegates, POP/IMAP toggles: all
  service-account-only or admin territory. Delegates are Workspace-only ("up to 25
  delegates and up to 10 delegators").
- `messages.import` (25 units, 150 MB) runs "standard email delivery scanning and
  classification similar to receiving via SMTP" including `processForCalendar`;
  `messages.insert` just stores. Neither sends. Useful for migration, not for the
  features here.

Sources: https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages/batchModify,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages/send,
https://gmail.googleapis.com/$discovery/rest?version=v1 (maxSize values),
https://developers.google.com/workspace/gmail/api/guides/sending,
https://developers.google.com/workspace/gmail/api/guides/threads,
https://workspaceupdates.googleblog.com/2019/03/threading-changes-in-gmail-conversation-view.html,
https://developers.google.com/workspace/gmail/api/guides/drafts,
https://developers.google.com/workspace/gmail/api/guides/alias_and_signature_settings,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.settings.sendAs/create,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.settings.sendAs/update,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.settings/updateVacation,
https://developers.google.com/workspace/gmail/api/guides/filter_settings,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.settings.filters,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.settings.filters/create,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.settings.forwardingAddresses/create,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.settings/updateAutoForwarding,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.settings.delegates/create,
https://developers.google.com/workspace/gmail/api/reference/rest/v1/users.messages/import,
https://support.google.com/mail/answer/22839, https://support.google.com/mail/answer/2819488.

## 5. Feature feasibility matrix

"Native" means the Gmail API (or the named Google API) does it as a first-class
operation. "Emulate" means Margin builds it from labels, filters, headers and local
state. Anything emulated with local state does not roam to another machine or to
Gmail web unless the label mirror carries it.

| Feature | Native in Gmail API | Emulate client-side (how) | Not possible |
|---|---|---|---|
| Screener (allow-list senders, hide unknown senders from inbox) | No. Filters cannot express "sender not in a list" beyond a single `negatedQuery` capped by query length. | On every `messagesAdded` with `INBOX`, look the sender up in a local allow-list (contacts, prior correspondents, user decisions). Unknown: `modify` remove `INBOX`, add `Margin/Screened out`. Screen-in decision re-adds `INBOX` for the thread and records the sender. Only runs while Margin is running; Gmail web shows unscreened mail in Inbox in between. | Server-side enforcement without a server. |
| Split inbox / Feed / Paper trail | Partial. `CATEGORY_PROMOTIONS`, `CATEGORY_UPDATES`, `CATEGORY_SOCIAL`, `CATEGORY_FORUMS`, `CATEGORY_PERSONAL` are applied by Gmail's classifier regardless of whether tabs are on (third-party), and are readable and settable. | Per-sender routing stored locally and mirrored as `Margin/Feed`, `Margin/Paper trail` labels via `modify`; optionally a Gmail filter per stable sender (`from:` add label, remove `INBOX`) so Gmail web matches and mail arriving while Margin is closed is routed. Seed the initial bucket from `CATEGORY_*` and `List-Id`/`Precedence: bulk` headers; let the user override per sender. | |
| Reply later pile | No. | Label `Margin/Reply later` plus local ordering and optional local reminders. Keep or drop `INBOX` as a product choice. | |
| Set aside pile | No. | Label `Margin/Set aside`, remove `INBOX`. | |
| Snooze | No. Gmail's own snooze is invisible except through `q=in:snoozed` and the eventual `INBOX` label add/remove in history; no snooze date is exposed (issue 287304309 is open). | Remove `INBOX`, add `Margin/Snoozed`, store wake time in SQLite; a scheduler re-adds `INBOX` (and `UNREAD` if wanted). Fires late if Margin was closed. Do not touch messages the user snoozed in Gmail web; Mimestream documents a Gmail server bug where archiving previously-snoozed mail via the API leaves it showing in Inbox. | Cross-device wake without your own sync. |
| Send later | No. Gmail web's Schedule send is not in the API. | Local outbox row with `send_at`; scheduler calls `messages.send` when due. Store the compose as a Gmail draft too so it survives a reinstall. App must be running at send time. | Sending while the app is closed. |
| Undo send | No. | Hold the message in the outbox for N seconds (Gmail web offers 5, 10, 20, 30) before calling `messages.send`. After send it is gone. | Recall after send. |
| Read receipts / open tracking | No. Gmail's read receipts are Workspace-only and admin-enabled (`Disposition-Notification-To`); consumer accounts cannot request or return them. | Adding `Disposition-Notification-To` to `raw` costs nothing but only some recipients' clients honour it (unverified). Pixel tracking needs a server you run and is the same practice Margin blocks on receipt; recommend not building it. | Open tracking without a server. |
| Spy pixel blocking | No. | Load bodies in the webview with remote content blocked by default, rewrite `<img src=http...>` to placeholders, allow per-sender or per-message, strip 1x1 and known tracker hosts, never send `Referer`. Optional proxy through your own server later. | |
| Unsubscribe | No (Gmail web's button is UI only). | Fetch `List-Unsubscribe` and `List-Unsubscribe-Post` via `metadataHeaders`. If `List-Unsubscribe-Post: List-Unsubscribe=One-Click` is present, POST to the HTTPS URI with body `List-Unsubscribe=One-Click` (RFC 8058, no GET, no confirmation page). Else `mailto:` via `messages.send`, else open the HTTPS link. Pair with a `from:` filter to `TRASH` or `Margin/Screened out` for stragglers. | |
| Block sender | Yes via filters: `criteria.from`, `action.addLabelIds=[TRASH]` (or `removeLabelIds=[INBOX]` plus a label). | Backfill existing mail with `messages.list?q=from:x` and `batchModify`. | |
| Snippets / templates | No. | Local SQLite; insert into compose. | |
| Sticky notes on threads | No. | Local SQLite keyed by threadId. Hidden drafts in the thread show up in Gmail web and labels cannot carry text, so there is no sane Gmail-side store. | Roaming without your own sync. |
| Rename thread subject | No; message headers are immutable. | Local display-name override keyed by threadId; replies still carry the real subject so threading holds. | Changing what Gmail web shows. |
| Merge threads | No; `threadId` is assigned by Gmail. | Local mapping of several threadIds to one virtual thread; writes fan out to every underlying thread. | Merging in Gmail itself. |
| Contacts autocomplete | Yes via People API: `otherContacts.list` (`contacts.other.readonly`, `readMask=names,emailAddresses`, pageSize up to 1000, sync tokens expire after 7 days), `people.connections.list` (`contacts.readonly`, up to 1000), `otherContacts.search` and `people.searchContacts` (max 30 results, send an empty-query warm-up first). | Also index From/To/Cc from synced headers locally; it is the best signal and needs no extra scope. | |
| Attachments browsing | Partial: `q=has:attachment` or `filename:pdf` returns ids. | Index `payload.parts[].filename/mimeType/size` from `format=full` fetches into SQLite; fetch bytes on demand with `attachments.get`. | |
| Calendar invite RSVP | Not in Gmail API. Calendar API: find the event with `events.list?iCalUID=<UID from the text/calendar part>` on `primary` (Gmail auto-adds invites depending on the user's "Add invitations to my calendar" setting, "From everyone" or "Only if the sender is known"), then `events.patch` with `attendees[self].responseStatus` in `accepted`, `tentative`, `declined` and `sendUpdates=all`; if absent, `events.import` with `iCalUID`, `start`, `end`. Needs `calendar.events`. | Parse the `text/calendar` part (`METHOD:REQUEST`) yourself and render the card. A standards-only fallback is to email a `METHOD:REPLY` iMIP message to the organiser via `messages.send`, which needs no Calendar scope but does not update the user's own calendar. | |
| Shared / team features | Out of scope. Delegation exists but is Workspace-only and service-account-only. | | |
| Multiple accounts | Yes: one OAuth grant per account with the same client ID; separate refresh tokens and sync cursors. | | |
| Unified inbox | No. | Merge locally across account databases; every write routes to the owning account. | |
| Search | Yes: `messages.list?q=` and `threads.list?q=` (5 or 10 units per page, full Gmail operator set: `from:`, `to:`, `subject:`, `label:`, `category:`, `has:attachment`, `filename:`, `list:`, `newer_than:`, `larger:`, `rfc822msgid:`, `in:snoozed`, `is:muted`, `deliveredto:`, `AROUND`, `OR`, `-`). Returns ids only; hydrate from the mirror. Not available under `gmail.metadata`. Cannot be combined with `history.list`. | SQLite FTS5 over synced subject, participants and cached bodies for instant results; fall back to server `q` for anything not yet hydrated and for operators you have not implemented. Server search is a network round trip and returns ids in relevance-ish order, so results feel slower than local. | |
| Keyboard triage | No. | Client. Batch consecutive label changes into `batchModify` calls. | |
| Push notifications | Yes with `users.watch` plus Pub/Sub, but delivery requires a server or an unacceptable IAM grant (section 3). | Poll `history.list`; optionally IMAP `IDLE` on `INBOX` as a wake-up. | Push with no server. |
| Offline compose and queue | No. | Local outbox; `drafts.create` when online so the draft roams; `messages.send` on reconnect with backoff. | |
| Categories tabs | Yes: read and set `CATEGORY_*`. Accuracy is good for big senders and mediocre for small newsletters; Promotions is a fair Feed seed, Updates is a fair Paper trail seed (receipts, notifications), Forums and Social less useful. Gmail may reclassify a sender after the user moves a few messages, which you observe as label churn in history. | Treat as a prior; sender-level rules win. | |
| Importance markers | Yes: `IMPORTANT` label readable and settable; filters can `removeLabelIds=[IMPORTANT]`. | | |
| Muted threads | Readable via `q=is:muted`, not settable. | `Margin/Muted` label plus client rule (section 4). | Muting that Gmail itself enforces. |
| Labels and folders, archive, star, read state, trash, spam | Yes. | | |
| Signature and vacation responder | Yes for the primary address (`sendAs.patch` signature, `updateVacation`). | | Creating or verifying send-as aliases (service-account-only). |
| Permanent delete | Yes, but only with `https://mail.google.com/`. | Trash instead. | |

Sources: https://developers.google.com/workspace/gmail/api/guides/labels,
https://developers.google.com/workspace/gmail/api/guides/filter_settings,
https://issuetracker.google.com/issues/287304309 (login required),
https://mimestream.com/help/common-issues/archiving-snoozed-messages,
https://support.google.com/mail/answer/9413651,
https://www.rfc-editor.org/rfc/rfc8058,
https://developers.google.com/people/api/rest/v1/otherContacts/list,
https://developers.google.com/people/api/rest/v1/otherContacts/search,
https://developers.google.com/people/api/rest/v1/people.connections/list,
https://developers.google.com/people/api/rest/v1/people/searchContacts,
https://developers.google.com/workspace/calendar/api/v3/reference/events,
https://developers.google.com/workspace/calendar/api/v3/reference/events/list,
https://developers.google.com/workspace/calendar/api/v3/reference/events/import,
https://support.google.com/calendar/answer/13159188,
https://support.google.com/mail/answer/7190 (search operators),
https://support.google.com/mail/thread/231661001 (categories applied with tabs off, community thread).

## 6. Gotchas and war stories

- Quota changed on 1 May 2026. Any library, blog post or mental model built on 250
  units per second per user and 5-unit `messages.get` is off by a factor of 4 to 10.
  Budget hydration at 300 messages per minute per user and expect billing for overage
  "later in 2026".
- Full sync is now the slow part. A 100,000 message mailbox is roughly 5 to 6 hours of
  metadata hydration at the ceiling. Hydrate newest first, show partial state, and
  consider IMAP for the backfill if that is unacceptable.
- `history.list` 404 is routine. A user who does not open Margin for a week or two
  will hit it; treat the ids-only re-list as a normal code path, not a disaster.
- History can reference messages that no longer exist. `messages.get` 404 on a
  `messagesAdded` id means it was deleted in the same window. Log and move on.
- Label IDs are not names. `Label_123` is the key; names change; system labels are
  their own IDs. Re-list labels when history contains an unknown id. Do not create a
  label whose name collides with a reserved name (400).
- `labelIds` on `messages.send` should be assumed ignored; `modify` afterwards.
- `raw` is base64url, not standard base64. `+`/`/` versus `-`/`_` mistakes produce
  corrupt MIME that Gmail rejects with a 400 that says nothing useful.
- 35 MB (36,700,160 bytes) is the hard `send` ceiling for the whole encoded message,
  which after base64 expansion is about 25 MB of attachments. Fail early in the
  composer, not at send.
- Messages are immutable. There is no way to change headers, subject, or body of a
  stored message. Every "edit" feature in section 5 is a local overlay.
- `messages.list` pages at most 500 and returns only ids; `resultSizeEstimate` is an
  estimate and can be wildly wrong. `threads.list` likewise.
- Batch: 100 max, 50 recommended, each call metered separately, and 429 inside a batch
  comes back per part, so parse every part's status. The batch path from the discovery
  document is `batch/gmail/v1` on `www.googleapis.com`.
- 429 and 403 `userRateLimitExceeded` / `rateLimitExceeded` both want truncated
  exponential backoff (`min((2^n) + random_ms, 32 to 64 s)`) and "Start retry periods
  at least one second after the error". Per-user limits "cannot be increased for any
  reason". Watch the undocumented concurrency ceiling: keep in-flight requests per
  mailbox low (single digits) and the batch size at 25 to 50.
- Server search is slow-ish and eventually consistent: a message you just labelled may
  not match `q=label:x` for a few seconds. Local FTS is the fast path.
- Header decoding: RFC 2047 encoded-words (`=?UTF-8?B?...?=`) in From, Subject and
  filenames, RFC 2231 parameter continuations in `Content-Disposition`, and
  charset-labelled bodies (`iso-8859-1`, `windows-1252`, `gb2312`) all show up daily.
  Use a real MIME parser on `raw` rather than trusting `payload` for anything beyond
  headers.
- HTML mail in a webview: sanitise (strip scripts, forms, `<meta http-equiv>`,
  external CSS, `javascript:` URLs), isolate in an iframe or Tauri child webview with
  CSP, block remote images by default, rewrite `cid:` to local resources, and expect
  broken layouts with dark mode. Practical dark-mode approach is per-message opt-in
  inversion or a light-background iframe; automatic colour inversion breaks a lot of
  newsletters.
- Newsletter and bulk detection: `List-Id` (RFC 2919), `List-Unsubscribe`,
  `Precedence: bulk|list`, `Auto-Submitted`, `X-Mailer` and a `CATEGORY_PROMOTIONS`
  or `CATEGORY_UPDATES` label together give a reliable "not a human" signal for the
  screener's default bucket.
- Authentication results are only in raw headers. Fetch `Authentication-Results` via
  `metadataHeaders` and parse `spf=`, `dkim=`, `dmarc=` from the `mx.google.com` stanza.
  There is no API field for spam score or phishing verdict; `SPAM` label is all you get.
- Gmail's classifier and your screener will disagree. A first-time human sender in
  Promotions is common (Gmail keys on content, you key on sender). Make the screener
  decision sender-based and use categories only to pick the default bucket.
- Threading edge cases: recipients and senders can see different threads for the same
  messages; a changed subject starts a new thread; Gmail web splits at 100 messages;
  mailing-list software that rewrites `Message-ID` or strips `References` produces
  orphan threads; a reply sent without `threadId` lands in a new thread even with
  correct headers.
- Snoozed-in-Gmail messages that Margin archives can reappear in Gmail web's Inbox
  (server bug, Mimestream help page). Leave Gmail-snoozed mail alone.
- Password change revokes every refresh token with Gmail scopes. Expect
  `invalid_grant` and re-prompt.
- Testing-mode tokens die after 7 days. If someone reports "it logs me out weekly", the
  consent screen is still in Testing.
- `resultSizeEstimate`, `messagesTotal` and label counts are estimates and lag. Never
  drive UI badges from them; count locally.
- People API sync tokens expire after 7 days and "The first page of a full sync request
  has an additional quota" that returns 429 if hammered; do one full contacts sync per
  install, then incremental.
- Sending limits are per account across all clients: 500 per day consumer. A bulk
  "unsubscribe from 200 lists via mailto" action can burn a meaningful chunk of it.

Sources: https://developers.google.com/workspace/gmail/api/guides/handle-errors,
https://developers.google.com/workspace/gmail/api/reference/quota,
https://developers.google.com/workspace/gmail/api/guides/sync,
https://developers.google.com/workspace/gmail/api/guides/batch,
https://developers.google.com/workspace/gmail/api/guides/threads,
https://mimestream.com/help/common-issues/archiving-snoozed-messages,
https://developers.google.com/identity/protocols/oauth2,
https://developers.google.com/people/api/rest/v1/otherContacts/list.

## 7. Recommendation

REST is the only transport for v1. IMAP comes later, if at all, for two narrow jobs:
bulk backfill when measured REST sync time is unacceptable, and `IDLE` as a wake-up
signal. `history.list` stays the single change log; IMAP has no equivalent.

Request `gmail.modify`, `gmail.settings.basic`, `contacts.other.readonly`,
`contacts.readonly` and `calendar.events`; no `https://mail.google.com/` until IMAP is
real. Push the consent screen to production unverified for the friends release (100
lifetime users, stable tokens) and file restricted-scope verification now, stating
there is no server and asking in writing whether the assessment applies.

Everything lives in SQLite on the device: message and thread mirror, bodies with an
LRU, FTS5 index, per-account history cursor, and an outbox with `send_at` and
`hold_until` (send later and undo send are one mechanism). Poll `history.list` every
10 to 15 seconds in the foreground. No server, no push; say so in the product.

Mirror every pile as a real label under one namespace so Gmail web stays coherent:
`Margin/Screened out`, `Margin/Feed`, `Margin/Paper trail`, `Margin/Reply later`,
`Margin/Set aside`, `Margin/Snoozed`, `Margin/Muted`. All but Reply later also remove
`INBOX`, so Gmail web's inbox matches Margin's and the rest is findable by label. For
senders the user has explicitly routed, also create a `from:` filter so routing works
while Margin is closed; stay well under the 1,000 filter cap.

What cannot live in Gmail, and so does not roam to another device or to Gmail web
until Margin ships its own sync: sticky notes, thread renames and merges, snooze wake
times, scheduled send times, the allow-list and per-sender rules (labels show the
outcome, not the rule), snippets. A second machine sees the labels, not the rules,
and a snooze set on the laptop will not wake the desktop. Call this "single device of
record" in the onboarding copy rather than discovering it in support.

Skip open tracking: it needs a server, contradicts the pixel blocking, and invites
scrutiny during review. Design against these numbers: 6,000 units per minute per
user, 20 units per metadata fetch, 100 per send, 35 MB per message, 500 sends per
day, 1,000 filters, 10,000 labels, 500 ids per page, 1,000 ids per `batchModify`, 100
calls per batch, history validity that can drop to hours, and no API for snooze,
schedule send, undo send, mute, rename, notes, or push without a server.
