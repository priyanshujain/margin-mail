// Serves the IPC surface from the dev fixture when the app is opened in a browser rather than in
// Tauri, so the real UI can be driven and looked at, by a person or by Playwright, without a Google
// account and without a build of the Rust side.
//
// Reachable only when `import.meta.env.DEV` is true and there is no Tauri, so it is absent from a
// production bundle and can never shadow the real backend.
//
// Reads answer from the fixture. Writes mutate the in-memory copy and hand back an `Undo` whose
// token really does reverse the change, so the toast, the undo stack and `z` can all be exercised.
// An unknown command throws with its name rather than returning undefined, because a command that
// silently answers nothing is a bug that surfaces three screens later.

import {
  CALENDAR_SCOPE,
  SCOPES,
  type Account,
  type SyncStatus,
  type CertQuestion,
  type Clip,
  type ConnectReport,
  type RefusalKind,
  type ContactPatch,
  type Destination,
  type Draft,
  type DraftSaved,
  type FlagPatch,
  type LabelInfo,
  type MailConfig,
  type Note,
  type Person,
  type Pile,
  type Place,
  type ScreenerCard,
  type SearchResult,
  type SenderRule,
  type ServerConfig,
  type Settings,
  type Snooze,
  type SnoozeKind,
  type ThreadPage,
  type ThreadQuery,
  type ThreadSummary,
  type ThreadView,
  type Undo,
} from "../ipc";
import {
  devAccounts,
  devCert,
  devClips,
  devContacts,
  devDiscover,
  devDrafts,
  devFiles,
  devLabels,
  devNotes,
  devOutbox,
  devRules,
  devScreener,
  devSettings,
  devSnoozes,
  devStorage,
  devSyncStatus,
  devThreads,
  groupOf,
  type DevThread,
} from "./fixture";

// -------------------------------------------------------------------------------------------
// The mutable copy
// -------------------------------------------------------------------------------------------

const accounts: Account[] = devAccounts.map((account) => ({ ...account }));
const threads: DevThread[] = devThreads.map((thread) => ({ ...thread }));
const rules: SenderRule[] = devRules.map((rule) => ({ ...rule }));
const screener: ScreenerCard[] = devScreener.map((card) => ({ ...card }));
const notes: Note[] = devNotes.map((note) => ({ ...note }));
const clips: Clip[] = devClips.map((clip) => ({ ...clip }));
const snoozes: Snooze[] = devSnoozes.map((snooze) => ({ ...snooze }));
const drafts: Draft[] = devDrafts.map((draft) => ({ ...draft }));
const outbox = devOutbox.map((item) => ({ ...item }));
const labels: LabelInfo[] = devLabels.map((label) => ({ ...label }));
const contactNotes = new Map<string, string>();
const contactPatches = new Map<string, ContactPatch>();
let settings: Settings = { ...devSettings };

// -------------------------------------------------------------------------------------------
// The crowded thread
//
// A calendar invite addressed to a floor of forty, which is the one shape of mail the pane had
// never been drawn against and the one that broke it: forty faces across the head and forty
// addresses down the side of them. Behind `marginmail-dev-crowd` rather than in the fixture
// proper, so the Inbox every other test reads is the Inbox it has always been.
// -------------------------------------------------------------------------------------------

const CROWD =
  "Melanie Brennan, Yanni Kyriacos, Aditi Rao, Ben Carter, Chen Wei, Dev Sharma, Elena Fischer, " +
  "Farid Haddad, Grace Okonjo, Henrik Sten, Ivy Lam, Jonas Alvarez, Kavya Menon, Liam Docherty, " +
  "Mira Kovac, Noor Rahman, Oskar Lindqvist, Priya Nair, Quentin Roy, Rhea Kapoor, Sam Whitfield, " +
  "Tara Iyer, Umar Siddiqui, Vera Novak, Wren Ashby, Xiulan Zhou, Yusuf Demir, Zoe Marchetti, " +
  "Anika Bose, Bruno Salas, Clara Nyberg, Daniel Osei, Esther Vance, Felix Trang, Gita Prasad, " +
  "Hugo Bellamy, Ingrid Solberg, Jae-won Park, Kofi Mensah";

const crowdPerson = (name: string): Person => ({
  name,
  address: `${name.toLowerCase().replace(/[^a-z]+/g, ".")}@antithesis.example`,
});

function crowdThread(): DevThread {
  const to = CROWD.split(", ").map(crowdPerson);
  const from = to[0];
  const me: Person = { name: "Priyanshu Jain", address: "pj@73ai.org" };
  const subject =
    "Q3 planning offsite, Friday 12 September, The Barn on Level 4: agenda, travel and the " +
    "dietary form, and please reply to the invitation rather than to all of us";
  const dateMs = Date.now();
  const key = "<crowd-offsite-01@antithesis.example>";
  return {
    key,
    accountId: "acct-1",
    accountColor: "hue-4",
    subject,
    originalSubject: null,
    from,
    participants: [from, ...to.slice(1), me],
    snippet: "The room is booked from ten. Agenda attached, and the dietary form closes Tuesday",
    dateMs,
    messageCount: 1,
    unseen: true,
    starred: false,
    trashed: false,
    spam: false,
    hasAttachment: false,
    hasDraft: false,
    pile: null,
    snoozedUntil: null,
    ignored: false,
    notify: false,
    merged: false,
    note: null,
    group: "",
    sending: false,
    place: "inbox",
    archived: false,
    labels: [],
    threadNotes: [],
    mergedFrom: [],
    back: false,
    messages: [
      {
        id: "msg-crowd-1",
        messageId: key,
        threadKey: key,
        from,
        to: [...to.slice(1), me],
        cc: [],
        bcc: [],
        replyTo: [],
        dateMs,
        subject,
        html:
          "<p>The room is booked from ten and we have it until four. Agenda is attached and the " +
          "dietary form closes on Tuesday.</p><p>Trains from the centre run every twenty minutes " +
          "and there is parking behind the building if you would rather drive.</p>",
        bodyPending: false,
        quotedHtml: null,
        isHtml: false,
        surface: "theme",
        attachments: [],
        trackers: [],
        blockedImages: 0,
        imagesLoaded: false,
        seen: false,
        draft: false,
        sentByMe: false,
        invite: null,
        unsubscribe: null,
        listId: null,
      },
    ],
  };
}

if (flagged("marginmail-dev-crowd")) threads.unshift(crowdThread());

/**
 * The threads whose bodies have been fetched, under `marginmail-dev-pending`.
 *
 * That flag is how the loading path is looked at without a Rust build behind it: `thread_view`
 * answers with what the mirror has and the bodies come back empty and `bodyPending`, exactly as a
 * thread nobody has opened before does. The delays are the point of the flag, not an accident of
 * it: a state that lasts a frame is a state nobody can see.
 */
const fetched = new Set<string>();

const beat = (ms: number): Promise<void> =>
  typeof window === "undefined"
    ? Promise.resolve()
    : new Promise((resolve) => window.setTimeout(resolve, ms));

/** What `thread_view` hands back before the bodies are in: the thread, without the prose. */
const withoutBodies = (view: ThreadView): ThreadView => ({
  ...view,
  messages: view.messages.map((message) => ({
    ...message,
    html: "",
    quotedHtml: null,
    bodyPending: true,
    // Both are read out of the body, so neither is known until the body is.
    trackers: [],
    blockedImages: 0,
  })),
});

/**
 * A first launch, which the fixture otherwise has no way to show: it is seeded with two connected
 * accounts, so the connect and onboarding screens were the ones nobody could look at. Set
 * `marginmail-dev-empty` to `"1"` in localStorage and every read comes back empty, the way it does
 * before anything is connected.
 */
const firstRun = (): boolean => {
  try {
    return localStorage.getItem("marginmail-dev-empty") === "1";
  } catch {
    return false;
  }
};

// -------------------------------------------------------------------------------------------
// Events
//
// `store-changed`, `sync-progress` and `auth` are Tauri events, and a browser has no source for
// them. `src/ipc.ts` owns the real listener; here the same payloads go out as window CustomEvents
// under the same names, so whoever wires the store subscribes to one of the two by whether
// `isTauri` is set.
// -------------------------------------------------------------------------------------------

function emit(name: string, detail: unknown, afterMs = 0): void {
  // A node-environment test can drive the whole switchboard, and there is no window there.
  if (typeof window === "undefined") return;
  after(afterMs, () => window.dispatchEvent(new CustomEvent(name, { detail })));
}

/** The same guard for the one thing the fixture does on a timer that is not an event. */
function after(ms: number, run: () => void): void {
  if (typeof window === "undefined") return;
  if (ms > 0) window.setTimeout(run, ms);
  else run();
}

const changed = (reason: string): void => emit("store-changed", reason);

/** A dev switch, read the way the empty install is read. */
function devNotifyPermission(): "granted" | "denied" | "prompt" {
  try {
    const held = localStorage.getItem("marginmail-dev-notify");
    return held === "denied" || held === "prompt" ? held : "granted";
  } catch {
    return "granted";
  }
}

function flagged(key: string): boolean {
  try {
    return localStorage.getItem(key) === "1";
  } catch {
    return false;
  }
}

/**
 * What Google says when the Gmail API is not enabled on the project behind the client.
 *
 * Kept verbatim because it is the shape of refusal the screen has to be readable against: long,
 * written for a developer rather than for the person holding the laptop, and carrying the one link
 * that fixes it. A sentence written here instead would be shorter and would not help anybody.
 */
const FIRST_PASS_REFUSAL =
  "Gmail profile failed (403): Gmail API has not been used in project 205537985128 before or it " +
  "is disabled. Enable it by visiting https://console.developers.google.com/apis/api/" +
  "gmail.googleapis.com/overview?project=205537985128 then retry.";

// -------------------------------------------------------------------------------------------
// Undo
// -------------------------------------------------------------------------------------------

interface UndoEntry {
  token: string;
  label: string;
  revert: () => void;
}

const undoStack: UndoEntry[] = [];
let undoCount = 0;

function undoable(label: string, revert: () => void, undoMs = 0): Undo {
  undoCount += 1;
  const token = `undo-${undoCount}`;
  undoStack.push({ token, label, revert });
  // Bounded in Rust for the same reason: a stack that grows for a session is a leak.
  if (undoStack.length > 25) undoStack.shift();
  return { token, label, undoMs };
}

function runUndo(entry: UndoEntry): string {
  entry.revert();
  const index = undoStack.indexOf(entry);
  if (index >= 0) undoStack.splice(index, 1);
  changed("threads state screener");
  return entry.label;
}

// -------------------------------------------------------------------------------------------
// Threads
// -------------------------------------------------------------------------------------------

const byKey = (key: string): DevThread | undefined =>
  threads.find((thread) => thread.key === key);

/** The note a `y` leaves sits after whatever was the latest message when it was written. */
function lastMessageId(threadKey: string): string | null {
  const messages = byKey(threadKey)?.messages ?? [];
  return messages.length > 0 ? messages[messages.length - 1].id : null;
}

function matchesQuery(thread: DevThread, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const haystack =
    `${thread.subject} ${thread.snippet} ${thread.from.name ?? ""} ${thread.from.address}`.toLowerCase();
  return words.every((word) => {
    if (word.startsWith("from:")) return thread.from.address.includes(word.slice(5));
    if (word === "has:attachment") return thread.hasAttachment;
    if (word.startsWith("subject:")) return thread.subject.toLowerCase().includes(word.slice(8));
    // `in:` names a place. Trash and spam are flags rather than destinations, so the two of them
    // are read off the thread and the rest are the sender's routing.
    if (word.startsWith("in:")) {
      const named = word.slice(3);
      if (named === "trash") return thread.trashed;
      if (named === "spam") return thread.spam;
      return !thread.trashed && !thread.spam && thread.place === named;
    }
    return haystack.includes(word);
  });
}

function inPlace(thread: DevThread, place: Place, query: ThreadQuery): boolean {
  // A piled thread is in its pile and a snoozed thread is away, unless it has come back, in which
  // case it is here and in the Back group. The SQL says the same thing as
  // `(sn.thread_key IS NULL OR rt.thread_key IS NOT NULL)`.
  const loose = !thread.pile && (thread.snoozedUntil === null || thread.back) && !thread.archived;
  // Trash and spam take a thread out of every list but their own, out of Everything in the case of
  // trash, and out of a search of neither. The SQL says `t.trashed = 0 AND t.spam = 0` in each of
  // those clauses, which is this.
  const filed = thread.trashed || thread.spam;
  switch (place) {
    case "inbox":
    case "feed":
    case "paper-trail":
      return thread.place === place && loose && !filed;
    case "reply-later":
    case "set-aside":
      return thread.pile === place && !filed;
    case "snoozed":
      // A snooze that has come back is spent: the thread is in Back, not still waiting. The moment
      // it was due stays on the row so it can say "Due yesterday", which is why this asks whether
      // the thread has returned rather than whether the field is set. Rust draws the same line, by
      // keying the place on a live row in `state.snoozes`.
      return thread.snoozedUntil !== null && !thread.back && !filed;
    case "everything":
      // Everything holds spam, because a message Gmail junked is still on the device and this is
      // the one place with no filter in it. Trash is the exception it makes.
      return !thread.trashed;
    case "sent":
      return thread.messages.some((message) => message.sentByMe) && !thread.trashed;
    case "drafts":
      return thread.hasDraft && !thread.trashed;
    case "starred":
      return thread.starred && !thread.trashed;
    case "screened-out":
      return thread.place === place && !filed;
    case "spam":
      return thread.spam;
    case "trash":
      return thread.trashed;
    case "label":
      return thread.labels.includes(query.labelId ?? "") && !filed;
    case "search":
      return matchesQuery(thread, query.query ?? "");
    case "screener":
      // The Screener is a list of senders, not of threads, and `screener_list` serves it.
      return false;
  }
}

function summaryOf(thread: DevThread, place: Place): ThreadSummary {
  // The rest sibling is what strips the dev-only fields; a summary is exactly the contract's shape.
  const { place: _p, archived: _a, labels: _l, messages: _m, threadNotes: _n, mergedFrom: _f, back: _b, ...rest } =
    thread;
  return { ...rest, group: groupOf(thread, place) };
}

/** Groups print in a fixed order, so the list is ordered by group first and by date inside it. */
const GROUP_ORDER = ["back", "new", "seen", "this-week", "earlier", ""];

function page(query: ThreadQuery): ThreadPage {
  const rank = (thread: DevThread): number => {
    const at = GROUP_ORDER.indexOf(groupOf(thread, query.place));
    return at < 0 ? GROUP_ORDER.length : at;
  };
  const matching = threads
    .filter((thread) => !query.accountId || thread.accountId === query.accountId)
    .filter((thread) => inPlace(thread, query.place, query))
    .sort((left, right) => rank(left) - rank(right) || right.dateMs - left.dateMs);

  const start = query.cursor ? Number(query.cursor) : 0;
  const limit = query.limit > 0 ? query.limit : 50;
  const slice = matching.slice(start, start + limit);
  const next = start + limit < matching.length ? String(start + limit) : null;

  return {
    threads: slice.map((thread) => summaryOf(thread, query.place)),
    nextCursor: next,
    footer:
      query.place === "trash" || query.place === "spam"
        ? "Gmail empties this after 30 days."
        : query.place === "everything"
          ? "Showing the last month. Older mail is on Gmail."
          : null,
  };
}

function viewOf(thread: DevThread): ThreadView {
  return {
    key: thread.key,
    accountId: thread.accountId,
    subject: thread.subject,
    originalSubject: thread.originalSubject,
    participants: thread.participants,
    messages: thread.messages,
    notes: notes.filter((note) => note.threadKey === thread.key),
    mergedFrom: thread.mergedFrom,
    pile: thread.pile,
    snoozedUntil: thread.snoozedUntil,
    ignored: thread.ignored,
    notify: thread.notify,
    starred: thread.starred,
    trashed: thread.trashed,
    spam: thread.spam,
    labels: thread.labels,
  };
}

/** Takes a copy of the fields a triage action touches, so the undo can put them all back. */
function snapshot(keys: string[]): Array<[DevThread, Partial<DevThread>]> {
  return keys.flatMap((key) => {
    const thread = byKey(key);
    if (!thread) return [];
    const { unseen, starred, trashed, spam, archived, place, pile, snoozedUntil, ignored, notify, subject, originalSubject, merged, mergedFrom, labels: labelIds } = thread;
    return [
      [
        thread,
        { unseen, starred, trashed, spam, archived, place, pile, snoozedUntil, ignored, notify, subject, originalSubject, merged, mergedFrom, labels: labelIds },
      ] as [DevThread, Partial<DevThread>],
    ];
  });
}

const restore = (taken: Array<[DevThread, Partial<DevThread>]>) => () => {
  for (const [thread, before] of taken) Object.assign(thread, before);
};

const naming = (keys: string[], one: string, many: string): string =>
  keys.length === 1 ? one : `${keys.length} ${many}`;

// -------------------------------------------------------------------------------------------
// IMAP and SMTP
//
// The mail servers this fixture pretends to have, and what each of them does when somebody tries
// to log in. The interesting outcomes are all refusals rather than errors, so they come back as a
// report the way Rust hands one back, and the screen draws a different panel for each.
// -------------------------------------------------------------------------------------------

/** The servers each IMAP account was set up with, which is what the Settings section reads back. */
const servers = new Map<string, MailConfig>();

/**
 * An account that is already connected over IMAP, under `marginmail-dev-imap`.
 *
 * The fixture's two accounts are both Google, so the Settings card for a mailbox with servers
 * instead of permissions was the one shape nobody could look at without running the whole connect
 * flow first. Behind a flag rather than in the fixture proper, the way the crowded thread is, so
 * the Accounts section every other test reads is the one it has always been.
 *
 * Two settings rather than one, because two different things have to be looked at. `1` adds it
 * beside the Google accounts, which is the mixed install where the section draws both kinds of
 * card. `only` is the install of somebody who never signed in to Google at all, which is the only
 * way to see the section without the footnote about the shared Google client, and it takes the
 * first account's place rather than arriving as a third so that the threads that account already
 * owns stay owned and the Inbox is still an Inbox.
 *
 * Its servers come out of the same provider directory the connect flow discovers from, because an
 * account added through that flow is the account this is standing in for, and two sets of Fastmail
 * settings in one fixture would eventually disagree.
 */
function imapFixture(): string | null {
  try {
    return localStorage.getItem("marginmail-dev-imap");
  } catch {
    return null;
  }
}

const imapMode = imapFixture();
if (imapMode !== null) {
  const email = "pj@fastmail.example";
  const only = imapMode === "only";
  // Standing in for acct-1 means keeping its id, which is what the threads, the sync statuses and
  // the settings rows are all keyed on.
  const id = only ? "acct-1" : email;
  if (only) accounts.length = 0;
  accounts.push({
    id,
    email,
    kind: "imap",
    name: "Fastmail",
    color: only ? "hue-4" : "hue-6",
    connected: true,
    // Nobody granted anything. An IMAP account has a password, not permissions, which is the whole
    // reason its card in Settings is a different card.
    grantedScopes: [],
    windowDays: 30,
  });
  const found = devDiscover(email);
  if (found) servers.set(id, found);
}

/** Certificates accepted here, keyed `host:port`, the way Rust keys the trust file it writes. */
const trustedCerts = new Map<string, string>();

const target = (host: string, port: number): string => `${host}:${port}`;

/** The loopback, where a bridge listens and where no certificate is ever a question. */
const loopback = (host: string): boolean =>
  /^(localhost|127(\.\d{1,3}){3}|\[?::1\]?)$/i.test(host.trim());

/**
 * The sentence Rust builds from what a server said, ported here so the fixture cannot drift from
 * it. Only an authentication refusal ever carries one, and only when the server named something to
 * go and do; a socket that never opened has nothing to add.
 */
function adviceFrom(said: string): string | null {
  const lower = said.toLowerCase();
  if (
    lower.includes("application-specific") ||
    lower.includes("app password") ||
    lower.includes("app-specific")
  ) {
    return "This account wants an app password rather than the one you sign in with.";
  }
  if (lower.includes("bridge")) {
    return "Proton accounts connect through Bridge, using the password Bridge shows you.";
  }
  return null;
}

const refusal = (
  leg: string,
  kind: RefusalKind,
  message: string,
  cert: CertQuestion | null = null,
): ConnectReport => ({
  ok: false,
  kind,
  failed: leg,
  message,
  cert,
  advice: cert ? null : adviceFrom(message),
});

/**
 * One leg of a test, in the order the real thing finds out: the socket, then the certificate, then
 * the password. Null means this half was fine.
 *
 * Proton Bridge is not running unless `marginmail-dev-bridge` says it is, because that is the
 * state a person is actually in when they first put their Proton address in: the account exists,
 * the settings are right, and the program that serves them has not been started.
 */
function tryLeg(leg: "imap" | "smtp", server: ServerConfig, password: string): ConnectReport | null {
  if (loopback(server.host) && !flagged("marginmail-dev-bridge")) {
    return refusal(leg, "unreachable", `Connection refused (${target(server.host, server.port)})`);
  }
  const question = devCert(server.host, server.port);
  if (question && trustedCerts.get(target(server.host, server.port)) !== question.fingerprint) {
    return refusal(
      leg,
      "certificate",
      `the certificate for ${server.host} is ${question.reason}`,
      question,
    );
  }
  // The school's mail is behind Google Workspace, which refuses an ordinary password and says so
  // in the one sentence worth showing a person verbatim.
  if (server.host.endsWith("oakridge-school.example")) {
    return refusal(
      leg,
      "auth",
      "[ALERT] Application-specific password required: https://support.example.com/apppasswords",
    );
  }
  if (password.trim().length === 0 || password === "wrong") {
    return refusal(leg, "auth", "Invalid credentials (Failure)");
  }
  return null;
}

/** Rust's `next_hue`: the first of the eight nobody is using. */
/** An account a connect from inside the app adds, keyed and named off the address typed. */
function addedAccount(email: string): Account {
  const account: Account = {
    id: `acct-${accounts.length + 1}`,
    email,
    kind: "google",
    name: email.split("@")[0] ?? email,
    color: nextHue(),
    connected: true,
    grantedScopes: [...SCOPES],
    windowDays: 30,
  };
  accounts.push(account);
  return account;
}

/**
 * A first sync as the engine narrates one: listing, then the crawl with a count climbing to its
 * total, then a clean end. The statuses on the way carry no last sync, because the engine stamps
 * one only when a pass ends, and that stamp is what the arriving panel hands over on.
 */
function narrateFirstSync(status: SyncStatus, fromMs: number): void {
  const working = { ...status, lastSyncMs: null };
  const arriving = (hydrated: number) => ({
    ...working,
    phase: "hydrating" as const,
    message: "Fetching the newest mail first",
    hydrated,
  });
  emit("sync-progress", { ...working, phase: "syncing", message: "Listing your mail", hydrated: 0, total: 0 }, fromMs);
  emit("sync-progress", arriving(0), fromMs + 150);
  emit("sync-progress", arriving(1_204), fromMs + 550);
  emit("sync-progress", arriving(3_380), fromMs + 950);
  emit("store-changed", "threads accounts", fromMs + 1_150);
  emit("sync-progress", { ...status, phase: "idle", hydrated: 0, total: 0 }, fromMs + 1_250);
}

function nextHue(): string {
  for (let n = 1; n <= 8; n += 1) {
    const hue = `hue-${n}`;
    if (!accounts.some((account) => account.color === hue)) return hue;
  }
  return `hue-${(accounts.length % 8) + 1}`;
}

// -------------------------------------------------------------------------------------------
// Attachments and files
// -------------------------------------------------------------------------------------------

/** A one pixel PNG, which is enough for a preview to render rather than break. */
const PIXEL_PNG =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

// -------------------------------------------------------------------------------------------
// The switchboard
// -------------------------------------------------------------------------------------------

export async function mockCall<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const raw = args ?? {};
  const arg = <V>(name: string): V => raw[name] as V;
  const done = <V>(value: V): Promise<T> => Promise.resolve(value as unknown as T);
  const empty = firstRun();

  switch (command) {
    // ---------------------------------------------------------------------------------------
    // Accounts
    // ---------------------------------------------------------------------------------------
    case "accounts_list":
      // A copy rather than the array itself. A real IPC hands back fresh objects every time, and a
      // store that is handed its own array back cannot see that a colour or a name has changed.
      return done(empty ? [] : accounts.map((account) => ({ ...account })));

    case "account_connect":
    case "account_grant": {
      const extras = arg<string[] | undefined>("extraScopes") ?? [];
      // `account_grant` names the account it is for; `account_connect` does not, because there is
      // not one yet. On an install that already has accounts a connect adds one, keyed and named
      // off the address that was typed, which is what the arriving panel and the chip then show.
      // On the empty install the first account of the fixture stands in for it.
      const wanted = arg<string | undefined>("accountId");
      const adding = command === "account_connect" && !empty;
      const account = adding
        ? addedAccount(arg<string | null | undefined>("loginHint") ?? "new@example.test")
        : (accounts.find((held) => held.id === wanted) ?? accounts[0]);
      // The connect screen waits for `auth` before it moves on, and in a browser nothing else
      // would ever send it.
      emit(
        "auth",
        {
          ok: true,
          error: null,
          accountId: account?.id ?? "acct-1",
          email: account?.email ?? "pj@73ai.org",
          cancelled: false,
          grantedScopes: [...(account?.grantedScopes ?? []), ...extras],
          missingRequired: [],
        },
        900,
      );
      // And the account really does gain them, which is what the screen that asked for a scope
      // reads back. Rust replaces the whole stored token with what the consent returned; here it is
      // a union, which is the same answer for a flow that only ever adds.
      if (account && extras.length > 0) {
        account.grantedScopes = [...new Set([...account.grantedScopes, ...extras])];
      }
      // A first connect is also what ends the empty install: `marginmail-dev-empty` is this
      // fixture's version of having no accounts, and an account has just arrived. The first sync is
      // then narrated the way the real one narrates itself, so the progress screen and the moment
      // it hands over to the Inbox can both be driven and watched here.
      if (command === "account_connect") {
        // A first connect is also what ends the empty install: the account is written, and the
        // first sync waits on `account_start`, once the window has been chosen.
        after(880, () => {
          try {
            localStorage.removeItem("marginmail-dev-empty");
          } catch {
            /* a context without storage never had the flag set */
          }
        });
      }
      return done("https://accounts.google.com/o/oauth2/v2/auth?dev=1");
    }

    case "account_start": {
      const accountId = arg<string>("accountId");
      const days = arg<number>("windowDays");
      const row = settings.accounts.find((held) => held.accountId === accountId);
      if (row) row.windowDays = days;
      const account = accounts.find((held) => held.id === accountId);
      if (account) account.windowDays = days;
      const status = { ...devSyncStatus()[0], accountId };
      // A first pass that fails, which is the other thing that really happens: the account is
      // connected and the mailbox itself refuses. Both statuses go out inside one timer on
      // purpose, because that is what makes the bug this reproduces a bug. React batches them
      // into a single render, so a screen watching for the phase to change from working to
      // stopped never sees the working half and waits for ever.
      if (flagged("marginmail-dev-sync-fails")) {
        after(150, () => {
          window.dispatchEvent(new CustomEvent("sync-progress", {
            detail: { ...status, lastSyncMs: null, phase: "syncing", message: "Listing your mail", hydrated: 0, total: 0 },
          }));
          window.dispatchEvent(new CustomEvent("sync-progress", {
            detail: {
              ...status,
              lastSyncMs: null,
              phase: "error",
              hydrated: 0,
              total: 0,
              message: "Paused after repeated failures. Sync now to try again.",
              error: FIRST_PASS_REFUSAL,
            },
          }));
        });
        return done(undefined);
      }
      narrateFirstSync(status, 150);
      changed("settings accounts");
      return done(undefined);
    }

    // Whether the data was kept makes no difference here: the account is gone from the list either
    // way, and a kept pair is a directory nothing in the browser can see.
    case "account_remove": {
      const id = arg<string>("accountId");
      // Long enough for the confirmation's busy state to be seen, which is what the flag is for.
      if (flagged("marginmail-dev-pending")) await beat(900);
      const index = accounts.findIndex((account) => account.id === id);
      if (index >= 0) accounts.splice(index, 1);
      changed("accounts threads");
      return done(undefined);
    }

    case "account_set_color": {
      const account = accounts.find((candidate) => candidate.id === arg<string>("accountId"));
      if (account) account.color = arg<string>("color");
      changed("accounts");
      return done(undefined);
    }

    case "account_set_name": {
      const account = accounts.find((candidate) => candidate.id === arg<string>("accountId"));
      if (account) account.name = arg<string>("name");
      changed("accounts");
      return done(undefined);
    }

    // ---------------------------------------------------------------------------------------
    // IMAP and SMTP
    // ---------------------------------------------------------------------------------------
    case "imap_discover":
      return done(devDiscover(arg<string>("email")));

    case "imap_test": {
      const config = arg<MailConfig>("config");
      const imapPassword = arg<string>("imapPassword") ?? "";
      // A blank outgoing password means the incoming one, which is what a gateway sharing
      // credentials with the mail store wants and what the sheet promises.
      const smtpPassword = arg<string | null>("smtpPassword") ?? imapPassword;
      const report =
        tryLeg("imap", config.imap, imapPassword) ??
        tryLeg("smtp", config.smtp, smtpPassword) ??
        ({ ok: true, kind: null, failed: null, message: null, cert: null, advice: null } satisfies ConnectReport);
      return done(report);
    }

    case "imap_connect": {
      // The address is the identity: IMAP has nothing like Gmail's stable numeric id, so two
      // accounts on one address are one account.
      const email = arg<string>("email").trim().toLowerCase();
      const config = arg<MailConfig>("config");
      const existing = accounts.find((account) => account.id === email);
      const account: Account = {
        id: email,
        email,
        kind: "imap",
        name: arg<string>("name")?.trim() || email,
        color: existing?.color ?? nextHue(),
        connected: true,
        // Nothing was granted by anybody: an IMAP account has a password, not permissions, which
        // is why the Settings card for one has no list of them.
        grantedScopes: [],
        windowDays: 30,
      };
      if (existing) Object.assign(existing, account);
      else accounts.push(account);
      servers.set(email, config);
      // An account has arrived, so the empty install is over. Synchronous, because the screen asks
      // for the account list the moment this resolves.
      try {
        localStorage.removeItem("marginmail-dev-empty");
      } catch {
        /* a context without storage never had the flag set */
      }
      changed("accounts threads");
      return done({ ...account });
    }

    case "imap_servers":
      return done(servers.get(arg<string>("accountId")) ?? null);

    case "imap_trust_cert": {
      trustedCerts.set(
        target(arg<string>("host"), arg<number>("port")),
        arg<string>("fingerprint"),
      );
      return done(undefined);
    }

    case "imap_forget_cert": {
      trustedCerts.delete(target(arg<string>("host"), arg<number>("port")));
      return done(undefined);
    }

    // ---------------------------------------------------------------------------------------
    // Threads
    // ---------------------------------------------------------------------------------------
    case "threads_list": {
      const query = arg<ThreadQuery>("query");
      if (empty) return done({ threads: [], nextCursor: null, footer: null });
      return done(page(query));
    }

    case "thread_view": {
      const thread = byKey(arg<string>("key"));
      if (!thread) throw new Error(`dev mock has no thread ${arg<string>("key")}`);
      // Opening a thread marks it seen, and takes it out of Back.
      thread.unseen = false;
      thread.back = false;
      for (const message of thread.messages) message.seen = true;
      const view = viewOf(thread);
      if (!flagged("marginmail-dev-pending") || fetched.has(thread.key)) return done(view);
      // A local read, so it comes back whatever the network is doing. The beat is only so the head
      // the pane draws from the row is a thing a person, and a test on a loaded machine, can both
      // watch happen: a state that lasts a frame is a state nobody can catch.
      await beat(900);
      return done(withoutBodies(view));
    }

    case "thread_hydrate": {
      const thread = byKey(arg<string>("key"));
      if (!thread) throw new Error(`dev mock has no thread ${arg<string>("key")}`);
      await beat(400);
      // Every body refused, which is a count of zero and not an error: the shape Rust answers with
      // when the provider will not hand the bodies over, and when there is no provider attached at
      // all. Under `marginmail-dev-hydrate-fails`, read on every call, so taking the flag away is
      // how a Try again is made to land.
      if (flagged("marginmail-dev-hydrate-fails")) return done(0);
      fetched.add(thread.key);
      // The count is the answer; what to draw is read again. The event goes out as well, because
      // a body that arrives without anybody having asked has to reach the pane too.
      changed("thread");
      return done(thread.messages.length);
    }

    case "thread_opened": {
      const thread = byKey(arg<string>("key"));
      if (!thread) throw new Error(`dev mock has no thread ${arg<string>("key")}`);
      // Opened means seen, as `thread_opened` in Rust has it. No event goes out: the list keeps the
      // row where it is until its next reload, and the badge is the one thing that moves.
      thread.unseen = false;
      for (const message of thread.messages) message.seen = true;
      return done(undefined);
    }

    case "flags_set": {
      const keys = arg<string[]>("keys");
      const patch = arg<FlagPatch>("patch");
      const taken = snapshot(keys);
      for (const [thread] of taken) {
        if (patch.seen !== undefined) thread.unseen = !patch.seen;
        if (patch.starred !== undefined) thread.starred = patch.starred;
        if (patch.archived !== undefined) thread.archived = patch.archived;
        // Both are flags and neither is a destination: a thread put back lands where its sender's
        // rule says it goes, which is what `place` has held all along.
        if (patch.trashed !== undefined) thread.trashed = patch.trashed;
        if (patch.spam !== undefined) thread.spam = patch.spam;
      }
      changed("threads");
      const label = patch.archived
        ? naming(keys, "Archived", "threads archived")
        : patch.trashed !== undefined
          ? patch.trashed
            ? naming(keys, "Trashed", "threads trashed")
            : naming(keys, "Restored", "threads restored")
          : patch.spam !== undefined
            ? patch.spam
              ? naming(keys, "Marked as spam", "threads marked as spam")
              : naming(keys, "Marked as not spam", "threads marked as not spam")
            : naming(keys, "Marked", "threads marked");
      return done(undoable(label, restore(taken)));
    }

    case "mark_all_seen": {
      const accountId = arg<string | null>("accountId");
      const place = arg<Place>("place");
      const taken = snapshot(
        threads
          .filter((thread) => !accountId || thread.accountId === accountId)
          .filter((thread) => inPlace(thread, place, { place, limit: 0 }))
          .map((thread) => thread.key),
      );
      for (const [thread] of taken) thread.unseen = false;
      changed("threads");
      return done(undoable("Marked everything as seen", restore(taken)));
    }

    case "start_fresh": {
      const olderThanMs = arg<number>("olderThanMs");
      const accountId = arg<string>("accountId");
      if (flagged("marginmail-dev-pending")) await beat(900);
      const taken = snapshot(
        threads
          .filter((thread) => thread.accountId === accountId && thread.dateMs < olderThanMs)
          .map((thread) => thread.key),
      );
      for (const [thread] of taken) thread.unseen = false;
      changed("threads");
      return done(undoable(`Marked ${taken.length} older threads as seen`, restore(taken)));
    }

    case "pile_toggle": {
      const keys = arg<string[]>("keys");
      const pile = arg<Pile>("pile");
      const taken = snapshot(keys);
      const name = pile === "reply-later" ? "Reply later" : "Set aside";
      const adding = taken.some(([thread]) => thread.pile !== pile);
      for (const [thread] of taken) thread.pile = adding ? pile : null;
      changed("threads state");
      return done(
        undoable(adding ? `Moved to ${name}` : `Taken out of ${name}`, restore(taken)),
      );
    }

    case "snooze_set": {
      const keys = arg<string[]>("keys");
      const kind = arg<SnoozeKind>("kind");
      const returnAtMs = arg<number>("returnAtMs");
      const taken = snapshot(keys);
      for (const [thread] of taken) {
        thread.snoozedUntil = returnAtMs;
        // Snoozing a thread that is sitting in Back sends it away again, so it is no longer waiting
        // to be opened. `snooze::set` in Rust clears the returned row for the same reason.
        thread.back = false;
        snoozes.push({ threadKey: thread.key, returnAtMs, kind });
      }
      changed("threads state");
      return done(
        undoable(naming(keys, "Snoozed", "threads snoozed"), () => {
          restore(taken)();
          for (const key of keys) {
            const index = snoozes.findIndex((snooze) => snooze.threadKey === key);
            if (index >= 0) snoozes.splice(index, 1);
          }
        }),
      );
    }

    case "snooze_clear": {
      const keys = arg<string[]>("keys");
      const taken = snapshot(keys);
      const removed = snoozes.filter((snooze) => keys.includes(snooze.threadKey));
      for (const [thread] of taken) thread.snoozedUntil = null;
      for (const snooze of removed) snoozes.splice(snoozes.indexOf(snooze), 1);
      changed("threads state");
      return done(
        undoable("Snooze cleared", () => {
          restore(taken)();
          snoozes.push(...removed);
        }),
      );
    }

    case "snooze_evaluate": {
      const now = Date.now();
      const due = snoozes.filter((snooze) => snooze.returnAtMs <= now);
      for (const snooze of due) {
        const thread = byKey(snooze.threadKey);
        if (thread) {
          // Kept rather than cleared, and in the past from here on. Rust reads it as
          // `COALESCE(sn.return_at, rt.due_ms)` for the same reason: a thread that came back late
          // says "Due yesterday" rather than pretending, and the group is what tells a waiting
          // thread from a returned one.
          thread.snoozedUntil = snooze.returnAtMs;
          thread.back = true;
          thread.unseen = true;
        }
        snoozes.splice(snoozes.indexOf(snooze), 1);
      }
      if (due.length > 0) changed("threads");
      return done(due.map((snooze) => snooze.threadKey));
    }

    case "thread_rename": {
      const key = arg<string>("key");
      const name = arg<string | null>("name");
      const taken = snapshot([key]);
      const thread = byKey(key);
      if (thread) {
        if (name === null) {
          thread.subject = thread.originalSubject ?? thread.subject;
          thread.originalSubject = null;
        } else {
          thread.originalSubject = thread.originalSubject ?? thread.subject;
          thread.subject = name;
        }
      }
      changed(`threads thread:${key}`);
      return done(undoable(name === null ? "Name removed" : "Thread renamed", restore(taken)));
    }

    case "thread_merge": {
      const keys = arg<string[]>("keys");
      const name = arg<string | null>("name");
      const taken = snapshot(keys);
      const [head, ...rest] = keys.map(byKey).filter((thread): thread is DevThread => !!thread);
      if (head) {
        head.merged = true;
        head.mergedFrom = [head, ...rest].map((thread) => ({
          key: thread.key,
          subject: thread.originalSubject ?? thread.subject,
        }));
        if (name) {
          head.originalSubject = head.originalSubject ?? head.subject;
          head.subject = name;
        }
        head.messages = [...head.messages, ...rest.flatMap((thread) => thread.messages)].sort(
          (left, right) => left.dateMs - right.dateMs,
        );
        head.messageCount = head.messages.length;
        for (const thread of rest) threads.splice(threads.indexOf(thread), 1);
      }
      changed("threads");
      return done(
        undoable(`${keys.length} threads merged`, () => {
          restore(taken)();
          for (const thread of rest) if (!threads.includes(thread)) threads.push(thread);
        }),
      );
    }

    case "thread_unmerge": {
      const key = arg<string>("key");
      const taken = snapshot([key]);
      const thread = byKey(key);
      if (thread) {
        thread.merged = false;
        thread.mergedFrom = [];
      }
      changed("threads");
      return done(undoable("Unmerged", restore(taken)));
    }

    case "thread_ignore": {
      const keys = arg<string[]>("keys");
      const on = arg<boolean>("on");
      const taken = snapshot(keys);
      for (const [thread] of taken) thread.ignored = on;
      changed("threads state");
      return done(undoable(on ? "Ignoring this thread" : "No longer ignoring", restore(taken)));
    }

    case "thread_notify": {
      const keys = arg<string[]>("keys");
      const on = arg<boolean>("on");
      const taken = snapshot(keys);
      for (const [thread] of taken) thread.notify = on;
      changed("threads state");
      return done(undoable(on ? "Notifications on" : "Notifications off", restore(taken)));
    }

    // ---------------------------------------------------------------------------------------
    // Messages, attachments
    // ---------------------------------------------------------------------------------------
    case "message_show_images": {
      const id = arg<string>("messageId");
      const message = threads
        .flatMap((thread) => thread.messages)
        .find((candidate) => candidate.id === id || candidate.messageId === id);
      if (!message) throw new Error(`dev mock has no message ${id}`);
      // The same beat as the bodies: a fetch that lands in a frame is a busy state nobody can see.
      if (flagged("marginmail-dev-pending")) await beat(900);
      message.imagesLoaded = true;
      message.blockedImages = 0;
      return done(message);
    }

    case "attachment_data_url":
      return done(PIXEL_PNG);

    case "attachment_save":
      return done(`/Users/you/Downloads/${arg<string>("attachmentId")}`);

    case "attachment_open":
      // The same beat as the bodies: the bytes are a fetch when nobody has opened this file
      // before, and a chip that says so for a frame is a state nobody can look at.
      if (flagged("marginmail-dev-pending")) await beat(900);
      return done(undefined);

    // ---------------------------------------------------------------------------------------
    // The Screener
    // ---------------------------------------------------------------------------------------
    case "screener_list": {
      const accountId = arg<string | null>("accountId");
      if (empty) return done([]);
      return done(screener.filter((card) => !accountId || card.accountId === accountId));
    }

    case "screener_decide": {
      const accountId = arg<string>("accountId");
      const address = arg<string>("address");
      const destination = arg<Destination>("destination");
      const wholeDomain = arg<boolean>("wholeDomain");
      // Before the card leaves the list, so the card whose decision is out is still a card a
      // second press can land on, which is the state the double-click test is about.
      if (flagged("marginmail-dev-pending")) await beat(600);
      const index = screener.findIndex(
        (card) => card.accountId === accountId && card.sender.address === address,
      );
      const [card] = index >= 0 ? screener.splice(index, 1) : [undefined];
      const rule: SenderRule = {
        accountId,
        subject: wholeDomain ? address.slice(address.indexOf("@") + 1) : address,
        isDomain: wholeDomain,
        destination,
        decidedAtMs: Date.now(),
        reason: card?.reason ?? null,
      };
      rules.push(rule);
      changed("screener state threads");
      const where =
        destination === "screened-out"
          ? "Screened out"
          : `Screened in to ${destination === "paper-trail" ? "Paper Trail" : destination === "feed" ? "Feed" : "Inbox"}`;
      return done(
        undoable(where, () => {
          rules.splice(rules.indexOf(rule), 1);
          if (card) screener.splice(index, 0, card);
        }),
      );
    }

    case "screener_clear_all": {
      const accountId = arg<string | null>("accountId");
      const cleared = screener.filter((card) => !accountId || card.accountId === accountId);
      for (const card of cleared) screener.splice(screener.indexOf(card), 1);
      changed("screener state");
      return done(
        undoable(`${cleared.length} senders screened out`, () => {
          screener.push(...cleared);
        }),
      );
    }

    case "screener_seed":
      return done(163);

    // ---------------------------------------------------------------------------------------
    // Notes, clips, files
    // ---------------------------------------------------------------------------------------
    case "note_add": {
      const threadKey = arg<string>("threadKey");
      const note: Note = {
        id: `note-${notes.length + 1}`,
        threadKey,
        body: arg<string>("body"),
        createdAtMs: Date.now(),
        afterMessageId: lastMessageId(threadKey),
      };
      notes.push(note);
      const thread = byKey(threadKey);
      if (thread) thread.note = note.body.split("\n")[0] ?? null;
      changed(`threads state thread:${threadKey}`);
      return done(note);
    }

    case "note_update": {
      const note = notes.find((candidate) => candidate.id === arg<string>("id"));
      if (note) {
        note.body = arg<string>("body");
        const thread = byKey(note.threadKey);
        if (thread) thread.note = note.body.split("\n")[0] ?? null;
      }
      changed("state");
      return done(undefined);
    }

    case "note_delete": {
      const id = arg<string>("id");
      const index = notes.findIndex((note) => note.id === id);
      const [gone] = index >= 0 ? notes.splice(index, 1) : [undefined];
      if (gone) {
        const thread = byKey(gone.threadKey);
        if (thread) thread.note = null;
      }
      changed("state");
      return done(
        undoable("Note deleted", () => {
          if (gone) notes.splice(index, 0, gone);
        }),
      );
    }

    case "clip_save": {
      const clip: Clip = {
        id: `clip-${clips.length + 1}`,
        accountId: arg<string>("accountId"),
        threadKey: arg<string>("threadKey"),
        messageId: arg<string>("messageId"),
        text: arg<string>("text"),
        sender: byKey(arg<string>("threadKey"))?.from ?? { name: null, address: "" },
        subject: byKey(arg<string>("threadKey"))?.subject ?? "",
        createdAtMs: Date.now(),
      };
      clips.push(clip);
      changed("state");
      return done(clip);
    }

    case "clips_list": {
      const accountId = arg<string | null>("accountId");
      if (empty) return done([]);
      return done(clips.filter((clip) => !accountId || clip.accountId === accountId));
    }

    case "clip_delete": {
      const id = arg<string>("id");
      const index = clips.findIndex((clip) => clip.id === id);
      const [gone] = index >= 0 ? clips.splice(index, 1) : [undefined];
      changed("state");
      return done(
        undoable("Clip deleted", () => {
          if (gone) clips.splice(index, 0, gone);
        }),
      );
    }

    case "files_list": {
      if (empty) return done([]);
      const accountId = arg<string | null>("accountId");
      const category = arg<string>("category");
      const sender = arg<string>("sender");
      const mine = threads.filter((thread) => !accountId || thread.accountId === accountId);
      return done(
        devFiles(mine)
          .filter((card) => !category || card.category === category)
          .filter((card) => !sender || card.sender.address === sender),
      );
    }

    // ---------------------------------------------------------------------------------------
    // Contacts
    // ---------------------------------------------------------------------------------------
    case "contact_card": {
      const accountId = arg<string>("accountId");
      const address = arg<string>("address");
      const card = devContacts(threads, rules).find(
        (candidate) => candidate.accountId === accountId && candidate.person.address === address,
      );
      if (!card) throw new Error(`dev mock has no contact ${address}`);
      const slot = `${accountId}|${address}`;
      const note = contactNotes.get(slot);
      // The same beat as the bodies, so the card's waiting state is a thing that can be looked at.
      if (flagged("marginmail-dev-pending")) await beat(900);
      return done({ ...card, ...(contactPatches.get(slot) ?? {}), note: note ?? card.note });
    }

    case "contact_update": {
      const accountId = arg<string>("accountId");
      const address = arg<string>("address");
      const patch = arg<ContactPatch>("patch");
      const slot = `${accountId}|${address}`;
      contactPatches.set(slot, { ...(contactPatches.get(slot) ?? {}), ...patch });
      if (patch.note !== undefined) contactNotes.set(slot, patch.note);
      if (patch.destination) {
        const rule = rules.find(
          (candidate) => candidate.accountId === accountId && candidate.subject === address,
        );
        if (rule) rule.destination = patch.destination;
        for (const thread of threads) {
          if (thread.accountId === accountId && thread.from.address === address) {
            thread.place = patch.destination;
          }
        }
      }
      changed("state threads");
      return done(undefined);
    }

    case "contacts_list": {
      if (empty) return done([]);
      const accountId = arg<string | null>("accountId");
      const query = arg<string>("query").toLowerCase();
      if (flagged("marginmail-dev-pending")) await beat(900);
      return done(
        devContacts(threads, rules)
          .filter((card) => !accountId || card.accountId === accountId)
          .filter(
            (card) =>
              !query ||
              card.person.address.toLowerCase().includes(query) ||
              (card.person.name ?? "").toLowerCase().includes(query),
          ),
      );
    }

    case "contacts_suggest": {
      const prefix = arg<string>("prefix").toLowerCase();
      const seen = new Map<string, Person>();
      for (const thread of threads) {
        for (const who of thread.participants) {
          if (!seen.has(who.address)) seen.set(who.address, who);
        }
      }
      return done(
        [...seen.values()]
          .filter(
            (who) =>
              !prefix ||
              who.address.toLowerCase().includes(prefix) ||
              (who.name ?? "").toLowerCase().includes(prefix),
          )
          .slice(0, 8),
      );
    }

    // ---------------------------------------------------------------------------------------
    // Labels
    // ---------------------------------------------------------------------------------------
    case "labels_list": {
      const accountId = arg<string | null>("accountId");
      if (empty) return done([]);
      // The system ones are places already, so `read::labels` does not hand them over and neither
      // does this. The fixture keeps one so that staying out is something a test can see.
      return done(
        labels
          .filter((label) => label.kind !== "system")
          .filter((label) => !accountId || label.accountId === accountId),
      );
    }

    case "label_apply": {
      const keys = arg<string[]>("keys");
      const labelId = arg<string>("labelId");
      const on = arg<boolean>("on");
      const taken = snapshot(keys);
      for (const [thread] of taken) {
        thread.labels = on
          ? [...new Set([...thread.labels, labelId])]
          : thread.labels.filter((id) => id !== labelId);
      }
      changed("threads");
      const name = labels.find((label) => label.id === labelId)?.name ?? labelId;
      return done(undoable(on ? `Labelled ${name}` : `Label ${name} removed`, restore(taken)));
    }

    case "label_move": {
      const keys = arg<string[]>("keys");
      const labelId = arg<string>("labelId");
      const taken = snapshot(keys);
      for (const [thread] of taken) {
        thread.labels = [...new Set([...thread.labels, labelId])];
        thread.archived = true;
      }
      changed("threads");
      const name = labels.find((label) => label.id === labelId)?.name ?? labelId;
      return done(undoable(`Moved to ${name}`, restore(taken)));
    }

    // ---------------------------------------------------------------------------------------
    // Search
    // ---------------------------------------------------------------------------------------
    case "search": {
      const accountId = arg<string | null>("accountId");
      const query = arg<string>("query");
      const cursor = arg<string | null>("cursor");
      const found = page({ accountId, place: "search", query, limit: 50, cursor });
      if (flagged("marginmail-dev-pending")) await beat(900);
      return done({
        page: found,
        providerSearched: false,
        note: "Searching what is on this device. Older mail is on Gmail.",
      } satisfies SearchResult);
    }

    case "search_provider": {
      const accountId = arg<string>("accountId");
      const query = arg<string>("query");
      const found = page({ accountId, place: "search", query, limit: 50, cursor: null });
      return done({
        page: { ...found, footer: null },
        providerSearched: true,
        note: `${found.threads.length} from Gmail`,
      } satisfies SearchResult);
    }

    // ---------------------------------------------------------------------------------------
    // Writing
    // ---------------------------------------------------------------------------------------
    case "draft_save": {
      const draft = arg<Draft>("draft");
      const id = draft.id ?? `draft-${drafts.length + 1}`;
      const index = drafts.findIndex((candidate) => candidate.id === id);
      const saved = { ...draft, id };
      if (index >= 0) drafts[index] = saved;
      else drafts.push(saved);
      if (draft.threadKey) {
        const thread = byKey(draft.threadKey);
        if (thread) thread.hasDraft = true;
      }
      changed("threads");
      const encodedSize = Math.round(draft.bodyHtml.length * 1.37) + 2_048;
      return done({
        id,
        updatedAtMs: Date.now(),
        encodedSize,
        overLimit: encodedSize > 35_000_000,
      } satisfies DraftSaved);
    }

    case "draft_get": {
      const draft = drafts.find((candidate) => candidate.id === arg<string>("id"));
      if (!draft) throw new Error(`dev mock has no draft ${arg<string>("id")}`);
      return done(draft);
    }

    case "draft_delete": {
      const id = arg<string>("id");
      const index = drafts.findIndex((draft) => draft.id === id);
      if (index >= 0) {
        const [gone] = drafts.splice(index, 1);
        const thread = gone?.threadKey ? byKey(gone.threadKey) : undefined;
        if (thread) thread.hasDraft = false;
      }
      changed("threads");
      return done(undefined);
    }

    case "send": {
      const draft = arg<Draft>("draft");
      const id = `out-${outbox.length + 1}`;
      const holdUntilMs = Date.now() + settings.undoDelaySecs * 1_000;
      outbox.push({
        id,
        accountId: draft.accountId,
        threadKey: draft.threadKey ?? null,
        to: draft.to,
        subject: draft.subject,
        holdUntilMs,
        attempts: 0,
        lastError: null,
      });
      const thread = draft.threadKey ? byKey(draft.threadKey) : undefined;
      if (thread) {
        thread.sending = true;
        thread.hasDraft = false;
        thread.pile = null;
      }
      changed("outbox threads");
      const to = draft.to[0]?.name ?? draft.to[0]?.address ?? "nobody";
      return done(
        undoable(
          `Sent to ${to}`,
          () => {
            const index = outbox.findIndex((item) => item.id === id);
            if (index >= 0) outbox.splice(index, 1);
            if (thread) {
              thread.sending = false;
              thread.hasDraft = true;
            }
          },
          settings.undoDelaySecs * 1_000,
        ),
      );
    }

    case "send_now": {
      const id = arg<string>("outgoingId");
      // Rust pushes the message to the provider before it answers, and the line has a busy state
      // for that wait: the same beat as the bodies, so it can be looked at.
      if (flagged("marginmail-dev-pending")) await beat(900);
      const index = outbox.findIndex((item) => item.id === id);
      if (index >= 0) {
        const [gone] = outbox.splice(index, 1);
        const thread = gone?.threadKey ? byKey(gone.threadKey) : undefined;
        if (thread) thread.sending = false;
      }
      changed("outbox threads");
      return done(undefined);
    }

    case "outbox_list":
      return done(empty ? [] : outbox);

    case "invite_respond": {
      const id = arg<string>("messageId");
      const response = arg<"accepted" | "tentative" | "declined" | "needs-action">("response");
      const message = threads
        .flatMap((thread) => thread.messages)
        .find((candidate) => candidate.id === id || candidate.messageId === id);
      // Answering needs a scope the app does not ask for at connect, and Rust refuses before it
      // calls anything when the account never granted it. The card turns that sentence into a
      // Grant button, so the fixture has to be able to produce it.
      const thread = threads.find((held) => held.messages.some((m) => m === message));
      const owner = accounts.find((held) => held.id === thread?.accountId) ?? accounts[0];
      if (owner && !owner.grantedScopes.includes(CALENDAR_SCOPE)) {
        throw new Error(
          `answering an invitation needs permission this account has not given: ${CALENDAR_SCOPE}`,
        );
      }
      if (message?.invite) message.invite = { ...message.invite, myResponse: response };
      changed("threads");
      return done(undefined);
    }

    case "unsubscribe": {
      const accountId = arg<string>("accountId");
      const address = arg<string>("address");
      const alsoTrash = arg<boolean>("alsoTrash");
      const alsoScreenOut = arg<boolean>("alsoScreenOut");
      const mine = threads.filter(
        (thread) => thread.accountId === accountId && thread.from.address === address,
      );
      // A POST to a stranger's server, which is the one call here that really does take a while.
      if (flagged("marginmail-dev-pending")) await beat(900);
      const taken = snapshot(mine.map((thread) => thread.key));
      if (alsoTrash) for (const [thread] of taken) thread.trashed = true;
      if (alsoScreenOut) {
        for (const [thread] of taken) thread.place = "screened-out";
        rules.push({
          accountId,
          subject: address,
          isDomain: false,
          destination: "screened-out",
          decidedAtMs: Date.now(),
          reason: "Unsubscribed",
        });
      }
      changed("threads state");
      return done(undoable(`Unsubscribed from ${address}`, restore(taken)));
    }

    // ---------------------------------------------------------------------------------------
    // Undo
    // ---------------------------------------------------------------------------------------
    case "undo_last": {
      const entry = undoStack[undoStack.length - 1];
      if (!entry) return done(null);
      return done(runUndo(entry));
    }

    case "undo_token": {
      const token = arg<string>("token");
      // Long enough to see the toast go down before the undo lands, rather than the two happening
      // in one frame.
      if (flagged("marginmail-dev-pending")) await beat(700);
      const entry = undoStack.find((candidate) => candidate.token === token);
      if (entry) runUndo(entry);
      return done(undefined);
    }

    // ---------------------------------------------------------------------------------------
    // Sync
    // ---------------------------------------------------------------------------------------
    case "sync_now": {
      const accountId = arg<string | undefined>("accountId");
      const statuses = devSyncStatus().filter(
        (status) => !accountId || status.accountId === accountId,
      );
      // A pass with nothing to say for itself first, which is what an ordinary one looks like: the
      // header has only the fact that it was asked to go on until this comes back.
      if (flagged("marginmail-dev-pending")) await beat(900);
      for (const status of statuses) {
        emit("sync-progress", { ...status, phase: "syncing", message: "Checking for new mail" }, 100);
        emit("sync-progress", { ...status, phase: "idle", lastSyncMs: Date.now() }, 900);
      }
      emit("store-changed", "threads accounts", 1_000);
      return done(statuses.map((status) => ({ ...status, phase: "syncing" as const })));
    }

    case "sync_status":
      return done(empty ? [] : devSyncStatus());

    case "sync_flush":
    case "sync_backfill":
      return done(undefined);

    case "log_note":
      console.warn(`[log] ${arg<string>("who")}: ${arg<string>("line")}`);
      return done(undefined);

    // ---------------------------------------------------------------------------------------
    // Settings and the libraries behind them
    // ---------------------------------------------------------------------------------------
    case "settings_get":
      // Nothing reads settings at boot, so this is the wait the snooze picker comes up into.
      if (flagged("marginmail-dev-pending")) await beat(700);
      return done(settings);

    case "settings_set": {
      settings = { ...settings, ...arg<Partial<Settings>>("patch") };
      changed("settings");
      return done(settings);
    }

    case "system_fonts":
      return done([
        "Avenir Next",
        "Charter",
        "Georgia",
        "Helvetica Neue",
        "Iowan Old Style",
        "New York",
        "SF Pro Text",
        "Verdana",
      ]);

    case "keymap_path":
      return done("/Users/you/Library/Application Support/org.margin.mail/keymap.json");

    case "keymap_reset":
      return done(undefined);

    case "storage_used":
      return done(empty ? [] : devStorage);

    // Each of these walks the whole mirror under the database lock, which is seconds on a real
    // one. The beat is the only way the busy state on the button is a state and not a frame.
    case "mirror_clear":
      if (flagged("marginmail-dev-pending")) await beat(900);
      return done(undefined);

    case "export_mbox":
      if (flagged("marginmail-dev-pending")) await beat(900);
      return done(`/Users/you/Downloads/${arg<string>("accountId")}.mbox`);

    case "export_state":
      if (flagged("marginmail-dev-pending")) await beat(900);
      return done("/Users/you/Downloads/margin-mail-state.json");

    case "import_state":
      if (flagged("marginmail-dev-pending")) await beat(900);
      return done(undefined);

    case "packaged_by":
      return done(null);

    // A browser has no dock to post to, and the button that calls this only needs it to answer.
    case "notify_test":
      return done(undefined);
    // The system's answer is "granted" unless `marginmail-dev-notify` holds "denied" or "prompt",
    // which is how the two states the settings section has to handle are looked at in a browser.
    // Asking clears "prompt", the way one dialog does.
    case "notify_permission":
      return done(devNotifyPermission());
    case "notify_request": {
      if (devNotifyPermission() === "prompt") localStorage.removeItem("marginmail-dev-notify");
      return done(devNotifyPermission());
    }
    case "notify_open_settings":
      return done(undefined);
    // Nothing in a browser was clicked in a notification centre.
    case "notify_take":
      return done(null);


    // ---------------------------------------------------------------------------------------
    // Backup
    // ---------------------------------------------------------------------------------------
    case "backup_status":
      return done(settings.backup);

    case "backup_configure": {
      const store = arg<string>("store");
      const config = arg<Record<string, string>>("config");
      if (flagged("marginmail-dev-pending")) await beat(900);
      settings = {
        ...settings,
        backup: {
          ...settings.backup,
          store: store === "r2" ? "r2" : store === "drive" ? "drive" : "none",
          configured: true,
          r2Bucket: config.bucket ?? null,
          r2Endpoint: config.endpoint ?? null,
        },
      };
      changed("settings");
      return done(settings.backup);
    }

    case "backup_now": {
      if (flagged("marginmail-dev-pending")) await beat(900);
      settings = {
        ...settings,
        backup: { ...settings.backup, lastBackupMs: Date.now() },
      };
      changed("settings");
      return done(settings.backup);
    }

    // Argon2 over the phrase is most of a second on a real machine.
    case "backup_phrase":
      if (flagged("marginmail-dev-pending")) await beat(900);
      return done(
        "candle harbour ribbon meadow anchor lantern pepper thicket marble orchard signal walnut",
      );

    case "backup_restore":
      if (flagged("marginmail-dev-pending")) await beat(900);
      return done(undefined);

    default:
      throw new Error(`dev mock has no handler for ${command}`);
  }
}
