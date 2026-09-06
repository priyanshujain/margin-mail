// The IPC contract. Every type here has a matching declaration in src/ipc.ts, and both sides are
// frozen once written: implementation modules add bodies, not fields.
//
// Two rules run through the whole file. Nothing that crosses this boundary carries a provider
// identifier the frontend could act on, because a view is the mirror joined to the state and the
// frontend is not entitled to know which of the two a value came from. And every list the frontend
// renders arrives already ordered and already grouped, because the grouping is part of the view
// and computing it twice in two languages is how the two drift.

use serde::{Deserialize, Serialize};

// -------------------------------------------------------------------------------------------
// People, accounts and places
// -------------------------------------------------------------------------------------------

/// One correspondent. `name` is whatever the header carried, which is often nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub name: Option<String>,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub email: String,
    /// Which mailbox is behind this account, and therefore which `Provider` drives it. Read by the
    /// frontend to know whether a scope list means anything here: an IMAP account has none.
    pub kind: AccountKind,
    /// The display name from the provider's profile, editable in settings.
    pub name: String,
    /// One of the eight hues, as a token name (`hue-1`), not a hex. The stylesheet owns the value.
    pub color: String,
    pub connected: bool,
    /// What Google actually granted. Granular consent means a user can untick a scope on the
    /// consent screen, so every feature that needs one checks this rather than assuming.
    pub granted_scopes: Vec<String>,
    /// How far back the mirror keeps this account's mail, in days. Zero means everything.
    pub window_days: u32,
}

/// The two kinds of mailbox, which is the one thing above the `Provider` trait that has to know.
///
/// Everything else in the app is written against the trait. This exists because a few screens
/// genuinely differ: an IMAP account has no scopes to grant, no Google account page to revoke
/// from, and a server and port to show in settings that a Google account does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccountKind {
    Google,
    Imap,
}

impl Default for AccountKind {
    /// Every account that existed before there was a second kind is a Google one.
    fn default() -> Self {
        AccountKind::Google
    }
}

// -------------------------------------------------------------------------------------------
// IMAP and SMTP configuration
// -------------------------------------------------------------------------------------------

/// How a socket is protected.
///
/// The three names Thunderbird's autoconfig format uses, because every published configuration in
/// the world is written in that vocabulary and translating it twice is how the meanings drift.
/// `Tls` is what the format calls SSL: TLS from the first byte, on 993 or 465. `StartTls` is a
/// plaintext connection upgraded by a command, on 143 or 587.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Security {
    Plain,
    StartTls,
    Tls,
}

/// Which family of SASL mechanism to authenticate with. The exact mechanism is chosen from what
/// the server advertises; this only says which list to choose from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthKind {
    Password,
    /// Reachable from a discovered configuration, which is why it can be represented. Nothing
    /// builds one yet: OAuth over IMAP needs a client registered with that provider.
    OAuth2,
}

/// One end of a mail account: a host, a port, how the socket is protected and who to log in as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub auth: AuthKind,
    /// Already expanded. `%EMAILADDRESS%` and `%EMAILLOCALPART%` are substituted inside discovery,
    /// so nothing downstream has to know that the format has placeholders in it.
    pub username: String,
}

/// Both ends, and where they came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailConfig {
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
    /// Which rung of the ladder answered: `autoconfig`, `ispdb`, `mx`, `probe` or `manual`. The
    /// connect screen says where the settings came from, because "we found these" and "we guessed
    /// these" are different promises and a person about to type a password should be told which.
    pub source: String,
    /// The provider's own name for itself, when the configuration carried one.
    pub display_name: Option<String>,
}

/// A certificate the user has to decide about before a connection can be made.
///
/// Only ever raised for a host that is not the loopback. Proton Bridge listens on 127.0.0.1 with a
/// certificate it generated itself, and there is nothing between this process and that socket to
/// impersonate anybody, so loopback is trusted without asking. Every other host is a question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertQuestion {
    pub host: String,
    pub port: u16,
    /// SHA-256 of the DER, uppercase hex in colon-separated pairs, which is the form every other
    /// tool prints so it can be compared against one.
    pub fingerprint: String,
    pub subject: String,
    pub issuer: String,
    pub expires_ms: i64,
    /// `self-signed`, `expired` or `unknown-issuer`. A hostname mismatch is never a question: it
    /// is the one failure that looks exactly like an interception, so it is refused outright.
    pub reason: String,
}

/// What a connection test found. Not a `Result`, because "the password was wrong" and "the
/// certificate needs a decision" are answers the screen renders rather than errors it reports.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectReport {
    pub ok: bool,
    /// What kind of refusal it was: `unreachable`, `auth`, `certificate`, `wrong-host` or `other`.
    ///
    /// A separate field rather than something the screen reads back out of `message`, because the
    /// screen has real decisions hanging off it (a loopback that is unreachable means the bridge
    /// is not running, and that is a different sentence from a wrong password) and reading them
    /// out of prose means a regex over whatever the server happened to say that day.
    pub kind: Option<String>,
    /// `imap` or `smtp`, when one leg failed and the other did not.
    pub failed: Option<String>,
    /// One sentence, already fit to read. The server's own words when they are usable.
    pub message: Option<String>,
    pub cert: Option<CertQuestion>,
    /// Set when the server refused the password but named a reason a person can act on, such as
    /// an app password being required.
    pub advice: Option<String>,
}

/// Every view the app can show. `Label` and `Search` carry an argument in `ThreadQuery`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Place {
    Inbox,
    Feed,
    PaperTrail,
    ReplyLater,
    SetAside,
    Screener,
    Snoozed,
    Everything,
    Sent,
    Drafts,
    Starred,
    ScreenedOut,
    Spam,
    Trash,
    Label,
    Search,
}

/// Where a sender's mail goes. Exactly one per sender, which is the whole of the routing model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Destination {
    Inbox,
    Feed,
    PaperTrail,
    ScreenedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Pile {
    ReplyLater,
    SetAside,
}

/// The surface a message body renders on, decided from what the sender painted rather than from
/// how the bytes arrived. `sanitize::surface` is where it is worked out and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Surface {
    /// The sender painted nothing, so the body takes the app's own paper and follows the theme.
    #[default]
    Theme,
    /// The sender painted an opaque light page across the layout, so the body keeps it in both
    /// palettes and the pane's chrome goes dark around it.
    Paper,
}

/// What a list asks for. `account_id` of None means every account, which is the All accounts view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadQuery {
    #[serde(default)]
    pub account_id: Option<String>,
    pub place: Place,
    /// The provider's label id, for `Place::Label`.
    #[serde(default)]
    pub label_id: Option<String>,
    /// The query text, for `Place::Search`.
    #[serde(default)]
    pub query: Option<String>,
    pub limit: u32,
    /// Opaque, from the previous page's `next_cursor`. None starts at the top.
    #[serde(default)]
    pub cursor: Option<String>,
}

// -------------------------------------------------------------------------------------------
// Threads
// -------------------------------------------------------------------------------------------

/// One row of a list, already grouped and already ordered.
///
/// `key` is the portable thread key: the first entry of the message's `References` header, else its
/// `In-Reply-To`, else its own `Message-ID`. It does not depend on which messages happen to be
/// mirrored, so it survives a storage window changing, a second device, and a move to another
/// provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub key: String,
    pub account_id: String,
    /// The account's hue token, for the coloured edge in All accounts.
    pub account_color: String,
    /// What to print. The rename when there is one, the real subject otherwise.
    pub subject: String,
    /// Set only when the thread was renamed, so the row can say "renamed · was …".
    pub original_subject: Option<String>,
    /// Whose name goes on the row: the latest correspondent who is not the account.
    pub from: Person,
    /// Everyone in the thread, for the stacked avatars in the pane.
    pub participants: Vec<Person>,
    pub snippet: String,
    /// Epoch milliseconds of the latest message.
    pub date_ms: i64,
    pub message_count: u32,
    /// At least one message the account has not seen. There is no count anywhere in this app.
    pub unseen: bool,
    pub starred: bool,
    /// In the provider's trash, and in its spam. Flags rather than places: a search result carries
    /// them into a list that is not Trash or Spam, and neither the row nor the pane can work them
    /// out from the place it is being shown in.
    pub trashed: bool,
    pub spam: bool,
    pub has_attachment: bool,
    pub has_draft: bool,
    pub pile: Option<Pile>,
    /// Epoch milliseconds: the moment this thread was due back.
    ///
    /// In the future while it is still waiting, in the past once it has come back and is sitting in
    /// the Back group, which is how a thread that returned late can say "Due yesterday" rather than
    /// pretending. The group tells the two apart, so a row never has to guess.
    pub snoozed_until: Option<i64>,
    pub ignored: bool,
    pub notify: bool,
    pub merged: bool,
    /// The one line under the row. The whole note is in the pane.
    pub note: Option<String>,
    /// Which group head this row sits under: "back", "new", "seen", "this-week", "earlier" or
    /// "sent-to". The view decides both the grouping and the order the groups come in, and the
    /// frontend renders a head whenever this changes and never sorts again. Sorting twice is how
    /// Back ends up in the middle of the Inbox.
    pub group: String,
    /// Waiting in the outbox, so the row can say "Waiting to send".
    pub sending: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadPage {
    pub threads: Vec<ThreadSummary>,
    /// Feed the next call. None when this is the end of the list.
    pub next_cursor: Option<String>,
    /// The quiet line at the foot: "Showing the last month. Older mail is on Gmail." or the
    /// search's "Search older mail on Gmail". None when there is nothing to say.
    pub footer: Option<String>,
}

/// A thread this one was merged from, for the banner that offers Unmerge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergedSource {
    pub key: String,
    pub subject: String,
}

/// The whole thread, for the reading pane.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadView {
    pub key: String,
    pub account_id: String,
    pub subject: String,
    pub original_subject: Option<String>,
    pub participants: Vec<Person>,
    pub messages: Vec<MessageView>,
    pub notes: Vec<Note>,
    pub merged_from: Vec<MergedSource>,
    pub pile: Option<Pile>,
    pub snoozed_until: Option<i64>,
    pub ignored: bool,
    pub notify: bool,
    pub starred: bool,
    /// The same two flags the row carries, so the pane can offer the way back out.
    pub trashed: bool,
    pub spam: bool,
    /// The provider's labels on this thread, for the label list. Never rendered in a row.
    pub labels: Vec<String>,
}

// -------------------------------------------------------------------------------------------
// Messages
// -------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub message_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
    /// Referenced from the body by `cid:` rather than listed as a chip.
    pub inline: bool,
    pub content_id: Option<String>,
    /// Already in the local cache, so opening it costs nothing.
    pub cached: bool,
}

/// A remote image that was removed before the body was rendered, and who it belonged to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tracker {
    /// The vendor from the shipped list, or the bare host when it is not a known one.
    pub vendor: String,
    pub url: String,
}

/// `List-Unsubscribe`, in the three forms it arrives in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Unsubscribe {
    /// RFC 8058: the app POSTs and the sender is required to honour it without a round trip.
    pub one_click: bool,
    pub mailto: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InviteResponse {
    Accepted,
    Tentative,
    Declined,
    NeedsAction,
}

/// A `text/calendar` part with `METHOD:REQUEST`, rendered as a card.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invite {
    pub uid: String,
    pub summary: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub all_day: bool,
    pub location: Option<String>,
    pub organizer: Option<Person>,
    pub description: Option<String>,
    /// What this account has already answered, when it has.
    pub my_response: InviteResponse,
    /// A deep link into Margin Calendar, when the event can be addressed there.
    pub calendar_link: Option<String>,
}

/// One message, sanitised and ready to render.
///
/// `html` is what goes into the iframe's `srcdoc`. It has been through the sanitiser, so `cid:`
/// images are already `data:` URIs and every remote image is either removed or, once the user has
/// asked for them, fetched by Rust and inlined the same way. The frontend never fetches anything.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageView {
    /// The provider's message id. Every command argument named `message_id` is this one, never the
    /// header below it: the RFC `Message-ID` is what the state database keys on and it never
    /// crosses this boundary as an argument.
    pub id: String,
    /// The RFC `Message-ID`, which is what the state database keys on.
    pub message_id: String,
    pub thread_key: String,
    pub from: Person,
    pub to: Vec<Person>,
    pub cc: Vec<Person>,
    pub bcc: Vec<Person>,
    pub reply_to: Vec<Person>,
    pub date_ms: i64,
    pub subject: String,
    pub html: String,
    /// True when this message's body has not been fetched yet, so `html` is empty because there is
    /// nothing to show rather than because the message was.
    ///
    /// Opening a thread never waits on the network. The rows come back from the mirror at once and
    /// anything still missing a body arrives on a later `store-changed`, which is the difference
    /// between a thread that opens and a thread that loads.
    pub body_pending: bool,
    /// The trailing quoted conversation, split off so it can sit behind a pill.
    pub quoted_html: Option<String>,
    /// True when the source was HTML. A plain text message has been converted, and the pane sets
    /// it on a 46em measure in the text face rather than leaving it to the sender's markup.
    pub is_html: bool,
    /// Which surface the body reads on. Not the same question as `is_html`: what decides it is
    /// whether the sender painted a page, and most HTML mail paints nothing.
    pub surface: Surface,
    pub attachments: Vec<Attachment>,
    pub trackers: Vec<Tracker>,
    /// Ordinary remote images that were blocked, which is a different count from the trackers.
    pub blocked_images: u32,
    /// Whether the body currently rendered has remote images loaded.
    pub images_loaded: bool,
    pub seen: bool,
    pub draft: bool,
    pub sent_by_me: bool,
    pub invite: Option<Invite>,
    pub unsubscribe: Option<Unsubscribe>,
    pub list_id: Option<String>,
}

// -------------------------------------------------------------------------------------------
// The decisions: rules, piles, snoozes, notes, clips
// -------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SenderRule {
    pub account_id: String,
    /// An address, or a domain when `is_domain`.
    pub subject: String,
    pub is_domain: bool,
    pub destination: Destination,
    pub decided_at_ms: i64,
    /// The suggestion rule that fired when this was set automatically, for the contact card.
    pub reason: Option<String>,
}

/// One waiting sender in the Screener.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenerCard {
    pub account_id: String,
    pub sender: Person,
    pub thread_key: String,
    pub subject: String,
    pub snippet: String,
    pub date_ms: i64,
    pub suggestion: Destination,
    /// The sentence the card prints, which is the row of the rules table that matched.
    pub reason: String,
    /// How many messages this sender has waiting, when it is more than one.
    pub waiting: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SnoozeKind {
    LaterToday,
    Tomorrow,
    Weekend,
    NextWeek,
    Date,
    /// Comes back only if nobody but the account has written since.
    IfNoReply,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snooze {
    pub thread_key: String,
    pub return_at_ms: i64,
    pub kind: SnoozeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub thread_key: String,
    pub body: String,
    pub created_at_ms: i64,
    /// The message that was latest when the note was written, so the pane can place it.
    pub after_message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    pub account_id: String,
    pub thread_key: String,
    pub message_id: String,
    pub text: String,
    pub sender: Person,
    pub subject: String,
    pub created_at_ms: i64,
}

/// Everything the app knows about one correspondent, for the popover and the Contacts place.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactCard {
    pub person: Person,
    pub account_id: String,
    pub destination: Destination,
    /// The rule that decides them is on their whole domain rather than their address.
    pub domain_rule: bool,
    /// A consumer domain cannot carry a domain rule, so the toggle is not offered.
    pub domain_rule_allowed: bool,
    pub notify: bool,
    pub screened_at_ms: Option<i64>,
    pub note: Option<String>,
    pub allow_remote_images: bool,
    /// Trash this sender's Feed mail after so many days. None is never.
    pub auto_trash_days: Option<u32>,
    /// Bundle this sender's Paper Trail rows into one.
    pub bundle: bool,
    pub recent_threads: Vec<ThreadSummary>,
    pub files: Vec<Attachment>,
    pub unsubscribe: Option<Unsubscribe>,
}

/// The patch the contact card writes back. An absent field means unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactPatch {
    #[serde(default)]
    pub destination: Option<Destination>,
    #[serde(default)]
    pub domain_rule: Option<bool>,
    #[serde(default)]
    pub notify: Option<bool>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub allow_remote_images: Option<bool>,
    #[serde(default)]
    pub auto_trash_days: Option<Option<u32>>,
    #[serde(default)]
    pub bundle: Option<bool>,
}

// -------------------------------------------------------------------------------------------
// Writing
// -------------------------------------------------------------------------------------------

/// A file on the way out, before it has been encoded into a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftAttachment {
    /// Absent for one that is already in the mirror, which is what a forward carries.
    #[serde(default)]
    pub path: Option<String>,
    /// The mirror's attachment id, for a forward.
    #[serde(default)]
    pub attachment_id: Option<String>,
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    /// The app's own draft id. Absent on the first save.
    #[serde(default)]
    pub id: Option<String>,
    pub account_id: String,
    /// The thread being replied to, which is what sets the threading headers.
    #[serde(default)]
    pub thread_key: Option<String>,
    /// The message being replied to or forwarded.
    #[serde(default)]
    pub in_reply_to: Option<String>,
    /// A verified alias to send as, when it is not the account's own address.
    #[serde(default)]
    pub from_alias: Option<String>,
    pub to: Vec<Person>,
    #[serde(default)]
    pub cc: Vec<Person>,
    #[serde(default)]
    pub bcc: Vec<Person>,
    pub subject: String,
    /// The editor's HTML. Rust inlines the stylesheet and builds the plain text alternative.
    pub body_html: String,
    #[serde(default)]
    pub attachments: Vec<DraftAttachment>,
    /// Set a reminder on the thread if nobody replies by then.
    #[serde(default)]
    pub remind_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftSaved {
    pub id: String,
    pub updated_at_ms: i64,
    /// The encoded size, so the composer can refuse an attachment before the send fails.
    pub encoded_size: u64,
    pub over_limit: bool,
}

/// A send that is waiting out its undo delay, or waiting for the network.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outgoing {
    pub id: String,
    pub account_id: String,
    pub thread_key: Option<String>,
    /// Who the toast names.
    pub to: Vec<Person>,
    pub subject: String,
    /// Epoch milliseconds. Until then the send can still be taken back.
    pub hold_until_ms: i64,
    pub attempts: u32,
    pub last_error: Option<String>,
}

// -------------------------------------------------------------------------------------------
// Undo
// -------------------------------------------------------------------------------------------

/// What a mutating command hands back so `z` can take it back.
///
/// The token is a handle on the state before the change, held in a bounded stack in Rust. It is
/// there rather than in the frontend because a bulk archive of forty threads with mixed prior
/// state cannot be reversed from what the frontend knew, and a reversal that guesses is worse than
/// no reversal at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Undo {
    pub token: String,
    /// What the toast says: "Archived 3 threads", "Sent to Ana".
    pub label: String,
    /// Zero for anything but a send. A send's toast counts down.
    pub undo_ms: u32,
}

// -------------------------------------------------------------------------------------------
// Search, labels, files
// -------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub page: ThreadPage,
    /// The query reached past the storage window, so the provider was asked as a second pass.
    pub provider_searched: bool,
    /// A note appended under the results when the two sources differ.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelInfo {
    pub id: String,
    pub account_id: String,
    pub name: String,
    /// system or user. The system ones are places already and are not offered for applying.
    pub kind: String,
}

/// One card in the All files place.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCard {
    pub attachment: Attachment,
    pub thread_key: String,
    pub subject: String,
    pub sender: Person,
    pub date_ms: i64,
    /// images, pdfs, documents, spreadsheets, presentations, invites, archives, other
    pub category: String,
}

// -------------------------------------------------------------------------------------------
// Settings
// -------------------------------------------------------------------------------------------

/// margin-shared's `FontRef`, which is either one of the six bundled faces or a family off the
/// machine. Stored rather than a bare family name, because a system font can be called "Literata"
/// and must not come back as the bundled one.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FontRef {
    #[serde(rename_all = "camelCase")]
    Bundled { id: String },
    #[serde(rename_all = "camelCase")]
    System { family: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSettings {
    pub account_id: String,
    pub name: String,
    pub color: String,
    /// 30, 90, 180, 365, or 0 for everything.
    pub window_days: u32,
    pub signature: String,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnoozeTimes {
    /// Hours from now for Later today.
    pub later_today_hours: u32,
    /// Local time of day, in minutes from midnight.
    pub tomorrow_at: u32,
    pub weekend_at: u32,
    pub next_week_at: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSettings {
    /// none, drive or r2
    pub store: String,
    pub configured: bool,
    pub last_backup_ms: Option<i64>,
    /// Shown once, at setup. Never returned again.
    pub has_phrase: bool,
    /// R2 only. The secret never crosses this boundary in the reading direction.
    pub r2_bucket: Option<String>,
    pub r2_endpoint: Option<String>,
}

/// Everything the settings screen edits. Device settings live in `settings.json` in the app data
/// directory, written atomically; the per account decisions that should roam live in the state
/// database and its journal instead, and are edited through the contact card and the account
/// sections rather than through this blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// light, dark or system
    pub theme: String,
    pub font_ui: FontRef,
    pub font_text: FontRef,
    pub text_size: u32,
    pub reading_pane: bool,
    /// comfortable or compact, on the phone
    pub density: String,

    pub accounts: Vec<AccountSettings>,
    pub attachment_cache_mb: u32,
    pub prefetch_bodies: bool,

    /// never, ask or always. The per sender allowances are not here: they live on the contact,
    /// because they are a decision about a person and they roam with the rest of those.
    pub remote_images: String,
    pub link_cleaning: bool,

    pub screener_enabled: bool,
    /// A reply to a thread you are in is never held. Turning this off holds it anyway.
    pub hold_replies: bool,
    pub suggestions: bool,

    pub snooze_times: SnoozeTimes,
    /// The pile a swipe reaches on the phone: reply-later, set-aside, archive, trash or none.
    pub swipe_right: String,
    pub swipe_left: String,
    pub feed_auto_trash_days: u32,

    pub undo_delay_secs: u32,
    pub reply_all_default: bool,
    pub instant_intro: String,

    pub badge: bool,
    /// The switch over everything below: off, and nothing is posted whatever a thread, a person or
    /// a place says. On by default, and absent from files written before it existed.
    #[serde(default = "on")]
    pub notifications: bool,
    /// The places that notify without being asked thread by thread. Empty is the default, which is
    /// the product's position: nothing tells you anything until you say so.
    pub notify_places: Vec<String>,

    pub backup: BackupSettings,
}

fn on() -> bool {
    true
}

/// Whether the system will show this app's notifications: what System Settings says on macOS, and
/// Prompt until the app has asked once. Everywhere else the answer is Granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyPermission {
    Granted,
    Denied,
    Prompt,
}

/// Where a click on a notification goes: the account it was about, the place its thread shows in,
/// and the thread itself when the notification was about one. Several messages in one pass are
/// one notification, and a click on that goes to the place alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyTarget {
    pub account_id: String,
    pub place: Place,
    pub thread_key: Option<String>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsed {
    pub account_id: String,
    pub messages: u64,
    pub threads: u64,
    pub mirror_bytes: u64,
    pub bodies_bytes: u64,
    pub attachments_bytes: u64,
    pub state_bytes: u64,
    /// The date of the oldest message on the device, which is what the window setting shows.
    pub oldest_ms: Option<i64>,
}

// -------------------------------------------------------------------------------------------
// Sync, auth and events
// -------------------------------------------------------------------------------------------

/// Per account, because one account being signed out must not be reported as the app being broken.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub account_id: String,
    /// idle | syncing | hydrating | caching | backfilling | offline | error | paused
    pub phase: String,
    pub last_sync_ms: Option<i64>,
    pub error: Option<String>,
    pub pending_writes: u32,
    /// The line the progress screen and the account chip show, when there is one.
    pub message: Option<String>,
    /// Whatever fill is running: the first sync's metadata crawl, and then the body cache behind
    /// it. Both zero when there is nothing left to bring in.
    pub hydrated: u32,
    pub total: u32,
    /// How far back the mirror actually reaches, which is not the same as the window setting until
    /// the first sync has finished.
    pub oldest_ms: Option<i64>,
}

impl SyncStatus {
    pub fn idle(account_id: &str) -> Self {
        SyncStatus {
            account_id: account_id.to_string(),
            phase: "idle".to_string(),
            last_sync_ms: None,
            error: None,
            pending_writes: 0,
            message: None,
            hydrated: 0,
            total: 0,
            oldest_ms: None,
        }
    }
}

/// Payload of the `auth` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthEvent {
    pub ok: bool,
    pub error: Option<String>,
    pub account_id: Option<String>,
    pub email: Option<String>,
    /// The user closed the consent browser rather than anything going wrong. Still not `ok`,
    /// because no account arrived, but changing your mind is not a failure.
    pub cancelled: bool,
    /// What was granted, which may be less than what was asked for.
    pub granted_scopes: Vec<String>,
    /// Set when the account was refused because a scope the app cannot work without was withheld.
    pub missing_required: Vec<String>,
}

/// The flags a triage action sets, and the only thing in this file the provider ever hears about.
/// An absent field means unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlagPatch {
    #[serde(default)]
    pub seen: Option<bool>,
    #[serde(default)]
    pub starred: Option<bool>,
    #[serde(default)]
    pub archived: Option<bool>,
    #[serde(default)]
    pub trashed: Option<bool>,
    #[serde(default)]
    pub spam: Option<bool>,
}
