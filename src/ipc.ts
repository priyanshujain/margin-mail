// The IPC contract. Every type here mirrors a struct in src-tauri/src/dto.rs. Both sides are frozen
// once written: implementation modules add bodies, not fields.
//
// Typed per-command wrappers live in src/api/, one module per domain. Nothing outside src/api may
// call `call` directly.

import { invoke } from "@tauri-apps/api/core";

/** A Tauri build of any shape, phone included. There is a Rust backend behind this. */
export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const isMobileOs =
  typeof navigator !== "undefined" &&
  (/android|iphone|ipod/i.test(navigator.userAgent) ||
    // iPadOS reports itself as a Mac and gives itself away only by having a touchscreen.
    (/ipad|macintosh/i.test(navigator.userAgent) && navigator.maxTouchPoints > 1));

/**
 * A Tauri build with a real window behind it: something to drag by its title bar, a close to
 * intercept before it happens. `core:window:*` sits in the desktop-only capability, so on a phone
 * those commands are refused rather than ignored.
 */
export const isDesktop = isTauri && !isMobileOs;

/** The one window whose title bar has the traffic lights inside the page. */
export const isMacDesktop =
  isDesktop && typeof navigator !== "undefined" && /mac/i.test(navigator.userAgent);

/**
 * True when there is a backend to answer a command: Tauri, or the dev fixture in a browser.
 * Data-loading actions gate on this. Anything touching a window API gates on `isDesktop`, and
 * anything listening for a Tauri event on `isTauri`, because a phone emits those too.
 */
export const live = (): boolean => isTauri || import.meta.env.DEV;

// -------------------------------------------------------------------------------------------
// People, accounts and places
// -------------------------------------------------------------------------------------------

export interface Person {
  name: string | null;
  address: string;
}

/**
 * Which mailbox is behind an account.
 *
 * Almost nothing branches on this. It exists because a few screens genuinely differ: an IMAP
 * account has no scopes to grant, no Google account page to revoke from, and a server and a port
 * to show in settings that a Google account does not have.
 */
export type AccountKind = "google" | "imap";

export interface Account {
  id: string;
  email: string;
  kind: AccountKind;
  name: string;
  /** A hue token name (`hue-1`), never a hex. The stylesheet owns the value. */
  color: string;
  connected: boolean;
  /** What Google actually granted. A user can untick a scope, so features check rather than assume. */
  grantedScopes: string[];
  /** How far back the mirror keeps this account's mail, in days. Zero means everything. */
  windowDays: number;
}

export type Place =
  | "inbox"
  | "feed"
  | "paper-trail"
  | "reply-later"
  | "set-aside"
  | "screener"
  | "snoozed"
  | "everything"
  | "sent"
  | "drafts"
  | "starred"
  | "screened-out"
  | "spam"
  | "trash"
  | "label"
  | "search";

/** Where a sender's mail goes. Exactly one per sender, which is the whole of the routing model. */
export type Destination = "inbox" | "feed" | "paper-trail" | "screened-out";

export type Pile = "reply-later" | "set-aside";

/** `accountId` of null means every account, which is the All accounts view. */
export interface ThreadQuery {
  accountId?: string | null;
  place: Place;
  labelId?: string | null;
  query?: string | null;
  limit: number;
  cursor?: string | null;
}

// -------------------------------------------------------------------------------------------
// Threads
// -------------------------------------------------------------------------------------------

/**
 * One row of a list, already grouped and already ordered.
 *
 * `key` is the portable thread key: the first entry of the message's `References` header, else its
 * `In-Reply-To`, else its own `Message-ID`. It does not depend on which messages happen to be
 * mirrored, so it survives the storage window changing, a second device, and another provider.
 */
export interface ThreadSummary {
  key: string;
  accountId: string;
  accountColor: string;
  /** What to print: the rename when there is one, the real subject otherwise. */
  subject: string;
  /** Set only when renamed, so the row can say "renamed · was …". */
  originalSubject: string | null;
  from: Person;
  participants: Person[];
  snippet: string;
  dateMs: number;
  messageCount: number;
  /** At least one message has not been seen. There is no count anywhere in this app. */
  unseen: boolean;
  starred: boolean;
  /**
   * In the provider's trash, and in its spam. Flags rather than places: a search result carries
   * them into a list that is neither, and the row draws the glyph from these.
   */
  trashed: boolean;
  spam: boolean;
  hasAttachment: boolean;
  hasDraft: boolean;
  pile: Pile | null;
  snoozedUntil: number | null;
  ignored: boolean;
  notify: boolean;
  merged: boolean;
  /** The one line under the row. The whole note is in the pane. */
  note: string | null;
  /**
   * Which group this row sits in: a key of `GROUPS` when the group has a head, or `new` and `seen`
   * in the Inbox, which have none. The view decides both the grouping and the order, so a list
   * renders a head whenever a headed group changes and never sorts again.
   */
  group: string;
  sending: boolean;
}

/**
 * Every group head, by the key the view sends, in the order a list puts them in. Rows arrive
 * already sorted into this order, so this is what a head is labelled with rather than what a list
 * sorts by; sorting twice is how Back ends up in the middle of the Inbox.
 *
 * `new` and `seen` are not here on purpose. The Inbox is one list in time order under Back, and
 * whether a row is new is the row's weight to carry, not a heading's: two groups over a dot over a
 * weight was three signals for one bit, and read as a bug whenever one of them lagged.
 */
export const GROUPS: Record<string, string> = {
  back: "Back",
  today: "Today",
  "this-week": "This week",
  "this-month": "This month",
  earlier: "Earlier",
  "sent-to": "Sent to",
};

export interface ThreadPage {
  threads: ThreadSummary[];
  nextCursor: string | null;
  /** The quiet line at the foot, such as "Showing the last month. Older mail is on Gmail." */
  footer: string | null;
}

export interface MergedSource {
  key: string;
  subject: string;
}

export interface ThreadView {
  key: string;
  accountId: string;
  subject: string;
  originalSubject: string | null;
  participants: Person[];
  messages: MessageView[];
  notes: Note[];
  mergedFrom: MergedSource[];
  pile: Pile | null;
  snoozedUntil: number | null;
  ignored: boolean;
  notify: boolean;
  starred: boolean;
  /** The same two flags the row carries, so the pane can offer the way back out. */
  trashed: boolean;
  spam: boolean;
  labels: string[];
}

// -------------------------------------------------------------------------------------------
// Messages
// -------------------------------------------------------------------------------------------

export interface Attachment {
  id: string;
  messageId: string;
  filename: string;
  mimeType: string;
  size: number;
  inline: boolean;
  contentId: string | null;
  cached: boolean;
}

export interface Tracker {
  vendor: string;
  url: string;
}

export interface Unsubscribe {
  oneClick: boolean;
  mailto: string | null;
  url: string | null;
}

export type InviteResponse = "accepted" | "tentative" | "declined" | "needs-action";

export interface Invite {
  uid: string;
  summary: string;
  startMs: number;
  endMs: number;
  allDay: boolean;
  location: string | null;
  organizer: Person | null;
  description: string | null;
  myResponse: InviteResponse;
  calendarLink: string | null;
}

/**
 * Which surface a message body renders on.
 *
 * `theme` is the app's own paper, light or dark with everything else. `paper` is the light page in
 * both palettes, for a sender who painted one. Rust decides it from what the message paints, not
 * from how it arrived, and stores the answer on the body row.
 */
export type Surface = "theme" | "paper";

/**
 * One message, sanitised and ready to render.
 *
 * `html` goes into the iframe's `srcdoc`. It has been through the sanitiser, so `cid:` images are
 * already `data:` URIs and every remote image is either removed or, once the user has asked for
 * them, fetched by Rust and inlined the same way. The frontend never fetches anything.
 */
export interface MessageView {
  /**
   * The provider's message id. Every command argument called `messageId` is this one, never the
   * header below it: the RFC `Message-ID` is what the state database keys on and it never crosses
   * this boundary as an argument.
   */
  id: string;
  /** The RFC `Message-ID`, which is what the state database keys on. */
  messageId: string;
  threadKey: string;
  from: Person;
  to: Person[];
  cc: Person[];
  bcc: Person[];
  replyTo: Person[];
  dateMs: number;
  subject: string;
  html: string;
  /**
   * The body has not been fetched yet, so `html` is empty because there is nothing to show rather
   * than because the message was. Opening a thread never waits on the network: what the mirror has
   * comes back at once and the rest arrives on a later `store-changed`.
   */
  bodyPending: boolean;
  quotedHtml: string | null;
  isHtml: boolean;
  /**
   * The surface the body reads on. Not the same question as `isHtml`: what decides it is whether
   * the sender painted a page, and most HTML mail paints nothing.
   */
  surface: Surface;
  attachments: Attachment[];
  trackers: Tracker[];
  blockedImages: number;
  imagesLoaded: boolean;
  seen: boolean;
  draft: boolean;
  sentByMe: boolean;
  invite: Invite | null;
  unsubscribe: Unsubscribe | null;
  listId: string | null;
}

// -------------------------------------------------------------------------------------------
// The decisions: rules, piles, snoozes, notes, clips
// -------------------------------------------------------------------------------------------

export interface SenderRule {
  accountId: string;
  /** An address, or a domain when `isDomain`. */
  subject: string;
  isDomain: boolean;
  destination: Destination;
  decidedAtMs: number;
  reason: string | null;
}

export interface ScreenerCard {
  accountId: string;
  sender: Person;
  threadKey: string;
  subject: string;
  snippet: string;
  dateMs: number;
  suggestion: Destination;
  /** The sentence the card prints, which is the row of the rules table that matched. */
  reason: string;
  waiting: number;
}

export type SnoozeKind =
  | "later-today"
  | "tomorrow"
  | "weekend"
  | "next-week"
  | "date"
  | "if-no-reply";

export interface Snooze {
  threadKey: string;
  returnAtMs: number;
  kind: SnoozeKind;
}

export interface Note {
  id: string;
  threadKey: string;
  body: string;
  createdAtMs: number;
  afterMessageId: string | null;
}

export interface Clip {
  id: string;
  accountId: string;
  threadKey: string;
  messageId: string;
  text: string;
  sender: Person;
  subject: string;
  createdAtMs: number;
}

export interface ContactCard {
  person: Person;
  accountId: string;
  destination: Destination;
  domainRule: boolean;
  /** A consumer domain cannot carry a domain rule, so the toggle is not offered. */
  domainRuleAllowed: boolean;
  notify: boolean;
  screenedAtMs: number | null;
  note: string | null;
  allowRemoteImages: boolean;
  autoTrashDays: number | null;
  bundle: boolean;
  recentThreads: ThreadSummary[];
  files: Attachment[];
  unsubscribe: Unsubscribe | null;
}

/** An absent field means unchanged. */
export interface ContactPatch {
  destination?: Destination;
  domainRule?: boolean;
  notify?: boolean;
  note?: string;
  allowRemoteImages?: boolean;
  autoTrashDays?: number | null;
  bundle?: boolean;
}

// -------------------------------------------------------------------------------------------
// Writing
// -------------------------------------------------------------------------------------------

export interface DraftAttachment {
  path?: string | null;
  attachmentId?: string | null;
  filename: string;
  mimeType: string;
  size: number;
}

export interface Draft {
  id?: string | null;
  accountId: string;
  threadKey?: string | null;
  inReplyTo?: string | null;
  fromAlias?: string | null;
  to: Person[];
  cc?: Person[];
  bcc?: Person[];
  subject: string;
  /** The editor's HTML. Rust inlines the stylesheet and builds the plain text alternative. */
  bodyHtml: string;
  attachments?: DraftAttachment[];
  remindAtMs?: number | null;
}

export interface DraftSaved {
  id: string;
  updatedAtMs: number;
  encodedSize: number;
  overLimit: boolean;
}

export interface Outgoing {
  id: string;
  accountId: string;
  threadKey: string | null;
  to: Person[];
  subject: string;
  holdUntilMs: number;
  attempts: number;
  lastError: string | null;
}

// -------------------------------------------------------------------------------------------
// Undo
// -------------------------------------------------------------------------------------------

/**
 * What a mutating command hands back so `z` can take it back. The token is a handle on the state
 * before the change, held in a bounded stack in Rust, because a bulk archive of forty threads with
 * mixed prior state cannot be reversed from what the frontend knew.
 */
export interface Undo {
  token: string;
  label: string;
  /** Zero for anything but a send. A send's toast counts down. */
  undoMs: number;
}

/** The flags a triage action sets. An absent field means unchanged. */
export interface FlagPatch {
  seen?: boolean;
  starred?: boolean;
  archived?: boolean;
  trashed?: boolean;
  spam?: boolean;
}

// -------------------------------------------------------------------------------------------
// Search, labels, files
// -------------------------------------------------------------------------------------------

export interface SearchResult {
  page: ThreadPage;
  providerSearched: boolean;
  note: string | null;
}

export interface LabelInfo {
  id: string;
  accountId: string;
  name: string;
  kind: string;
}

export interface FileCard {
  attachment: Attachment;
  threadKey: string;
  subject: string;
  sender: Person;
  dateMs: number;
  category: string;
}

// -------------------------------------------------------------------------------------------
// Settings
// -------------------------------------------------------------------------------------------

/** margin-shared's `FontRef`: one of the six bundled faces, or a family off the machine. */
export type FontRef = { kind: "bundled"; id: string } | { kind: "system"; family: string };

// -------------------------------------------------------------------------------------------
// IMAP and SMTP configuration
// -------------------------------------------------------------------------------------------

/**
 * How a socket is protected, in the vocabulary every published mail configuration is written in.
 *
 * `tls` is what those documents call SSL: TLS from the first byte, on 993 or 465. `starttls` is a
 * plaintext connection upgraded by a command, on 143 or 587.
 */
export type Security = "plain" | "start-tls" | "tls";

export type AuthKind = "password" | "o-auth2";

export interface ServerConfig {
  host: string;
  port: number;
  security: Security;
  auth: AuthKind;
  /** Already expanded: discovery substitutes the address placeholders before this is returned. */
  username: string;
}

export interface MailConfig {
  imap: ServerConfig;
  smtp: ServerConfig;
  /**
   * Which rung of the ladder answered: `autoconfig`, `ispdb`, `mx`, `probe` or `manual`. The
   * connect screen says where the settings came from, because "we found these" and "we guessed
   * these" are different promises and somebody about to type a password should be told which.
   */
  source: string;
  displayName: string | null;
}

/**
 * A certificate that has to be decided about before a connection can be made.
 *
 * Only ever raised for a host that is not the loopback. A local bridge listens on 127.0.0.1 with
 * a certificate it generated itself and there is nothing in between to impersonate anybody, so
 * loopback is trusted without asking.
 */
export interface CertQuestion {
  host: string;
  port: number;
  /** SHA-256 of the certificate, in the colon-separated form every other tool prints. */
  fingerprint: string;
  subject: string;
  issuer: string;
  expiresMs: number;
  /** `self-signed`, `expired` or `unknown-issuer`. A name mismatch is refused, never offered. */
  reason: string;
}

/**
 * What a connection test found.
 *
 * Not a thrown error, because the interesting outcomes are all things the screen draws. A wrong
 * password, a certificate to decide about and a host that does not answer are three panels.
 */
/**
 * What kind of refusal it was.
 *
 * A field rather than something the screen reads back out of the message, because real decisions
 * hang off it: a loopback that is unreachable means the bridge is not running, which is a
 * different sentence from a wrong password, and telling them apart by matching prose means a
 * regex over whatever the server happened to say that day.
 */
export type RefusalKind = "unreachable" | "auth" | "certificate" | "wrong-host" | "other";

export interface ConnectReport {
  ok: boolean;
  kind: RefusalKind | null;
  /** `imap` or `smtp`, when one leg failed and the other did not. */
  failed: string | null;
  message: string | null;
  cert: CertQuestion | null;
  /** A sentence naming what to go and do, when the server said enough to know. */
  advice: string | null;
}

/** The default port for a security. Both halves agree on the shape and differ on the numbers. */
export function defaultPort(leg: "imap" | "smtp", security: Security): number {
  if (leg === "imap") return security === "tls" ? 993 : 143;
  if (security === "tls") return 465;
  return security === "start-tls" ? 587 : 25;
}

export interface AccountSettings {
  accountId: string;
  name: string;
  color: string;
  /** 30, 90, 180, 365, or 0 for everything. */
  windowDays: number;
  signature: string;
  aliases: string[];
}

export interface SnoozeTimes {
  laterTodayHours: number;
  tomorrowAt: number;
  weekendAt: number;
  nextWeekAt: number;
}

export interface BackupSettings {
  store: "none" | "drive" | "r2";
  configured: boolean;
  lastBackupMs: number | null;
  hasPhrase: boolean;
  r2Bucket: string | null;
  r2Endpoint: string | null;
}

export interface Settings {
  theme: "light" | "dark" | "system";
  fontUi: FontRef;
  fontText: FontRef;
  textSize: number;
  readingPane: boolean;
  density: "comfortable" | "compact";

  accounts: AccountSettings[];
  attachmentCacheMb: number;
  prefetchBodies: boolean;

  /** The per sender allowances are not here: they live on the contact and roam with it. */
  remoteImages: "never" | "ask" | "always";
  linkCleaning: boolean;

  screenerEnabled: boolean;
  holdReplies: boolean;
  suggestions: boolean;

  snoozeTimes: SnoozeTimes;
  swipeRight: string;
  swipeLeft: string;
  feedAutoTrashDays: number;

  undoDelaySecs: number;
  replyAllDefault: boolean;
  instantIntro: string;

  badge: boolean;
  /** The switch over everything below: off, and nothing is posted whatever a thread, a person or a place says. */
  notifications: boolean;
  /** The places that notify without being asked thread by thread. Empty by default. */
  notifyPlaces: Place[];

  backup: BackupSettings;
}

/**
 * Whether the system will show this app's notifications: what System Settings says on macOS, and
 * "prompt" until the app has asked once. Everywhere else the answer is "granted".
 */
export type NotifyPermission = "granted" | "denied" | "prompt";

/**
 * Where a click on a notification goes: the account it was about, the place its thread shows in,
 * and the thread when the notification was about one. Several in one pass go to the place alone.
 */
export interface NotifyTarget {
  accountId: string;
  place: Place;
  threadKey: string | null;
}


export interface StorageUsed {
  accountId: string;
  messages: number;
  threads: number;
  mirrorBytes: number;
  bodiesBytes: number;
  attachmentsBytes: number;
  stateBytes: number;
  oldestMs: number | null;
}

// -------------------------------------------------------------------------------------------
// Sync, auth and events
// -------------------------------------------------------------------------------------------

export interface SyncStatus {
  accountId: string;
  phase:
    | "idle"
    | "syncing"
    | "hydrating"
    // Bodies being brought in behind a finished first sync, so a thread opens out of the mirror
    // rather than off the network. Quiet: the mail is already readable, this is only the wait
    // going away.
    | "caching"
    | "backfilling"
    | "offline"
    | "error"
    | "paused";
  lastSyncMs: number | null;
  error: string | null;
  pendingWrites: number;
  message: string | null;
  hydrated: number;
  total: number;
  oldestMs: number | null;
}

/** Payload of `sync-progress`. */
export type SyncProgress = SyncStatus;

/** Payload of the `auth` event. */
export interface AuthEvent {
  ok: boolean;
  error: string | null;
  accountId: string | null;
  email: string | null;
  cancelled: boolean;
  grantedScopes: string[];
  missingRequired: string[];
}

/**
 * Payload of `store-changed`: a scope naming what moved, so a note landing does not make the list
 * refetch its bodies. One of `threads`, `thread:<key>`, `accounts`, `settings`, `state`, `screener`,
 * `outbox`, or several separated by a space.
 */
export type StoreChanged = string;

/** The scopes the app asks for, in the order the consent screen lists them. */
export const SCOPES = [
  "openid",
  "email",
  "https://www.googleapis.com/auth/gmail.modify",
  "https://www.googleapis.com/auth/gmail.settings.basic",
  "https://www.googleapis.com/auth/contacts.readonly",
  "https://www.googleapis.com/auth/contacts.other.readonly",
] as const;

/** Answering an invite needs this one, and it is asked for by re-running the whole consent. */
export const CALENDAR_SCOPE = "https://www.googleapis.com/auth/calendar.events";

/** Backing up to Drive needs this one, asked for the same way. */
export const DRIVE_SCOPE = "https://www.googleapis.com/auth/drive.file";

/** Without this the account is not added at all, and the app says why. */
export const REQUIRED_SCOPE = "https://www.googleapis.com/auth/gmail.modify";

/**
 * In Tauri this is `invoke`. Opened in a browser during development it is served from the dev
 * fixture instead, so the real UI can be driven and looked at without a Google account. The branch
 * is compiled out of a production bundle, and `isTauri` means it can never shadow the real backend
 * inside the app, on a desktop or on a phone.
 */
export function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (import.meta.env.DEV && !isTauri) {
    return import("./dev/mockIpc").then((m) => m.mockCall<T>(command, args));
  }
  return invoke<T>(command, args).catch((e: unknown) => {
    // Written down before it becomes a toast, so the log holds what the person saw and not
    // only what the engine did on its own. Not for the note itself, which would loop.
    if (command !== "log_note") {
      void invoke("log_note", { who: "ui", line: `${command}: ${String(e)}` }).catch(() => {});
    }
    throw e;
  });
}
