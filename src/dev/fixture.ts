// The dev dataset: the same people, threads and words as the mockups in docs/mockups, so a
// screenshot of the running app can be put beside the PNG and compared.
//
// It mirrors src-tauri/fixtures, the `.eml` corpus the Rust side parses: Arun's lease with its
// HubSpot pixel, Sam Okafor's renamed and merged piano thread with its invite, City Power's bill
// with a PDF, The Browser's one-click unsubscribe. Anything the parser has an awkward case for has
// a thread here, so the two halves of the app are exercised on one story rather than two.
//
// Dates are computed at module load and anchored to the local day, so the list is always fresh:
// a few threads from today with times, a few from yesterday, then dates. Two absolute dates from
// the mockups (Hannah's "Sun" and Russell's "30 Aug") become relative offsets, so their printed
// dates drift from the PNG while their order does not.

import {
  CALENDAR_SCOPE,
  DRIVE_SCOPE,
  SCOPES,
  defaultPort,
  type Account,
  type Attachment,
  type AuthKind,
  type CertQuestion,
  type Clip,
  type ContactCard,
  type Draft,
  type FileCard,
  type Invite,
  type LabelInfo,
  type MailConfig,
  type MergedSource,
  type MessageView,
  type Note,
  type Outgoing,
  type Person,
  type Pile,
  type Place,
  type ScreenerCard,
  type Security,
  type SenderRule,
  type ServerConfig,
  type Settings,
  type Snooze,
  type StorageUsed,
  type Surface,
  type SyncStatus,
  type ThreadSummary,
  type Tracker,
  type Unsubscribe,
} from "../ipc";

// -------------------------------------------------------------------------------------------
// Time
// -------------------------------------------------------------------------------------------

function startOfDay(ms: number): number {
  const day = new Date(ms);
  day.setHours(0, 0, 0, 0);
  return day.getTime();
}

const TODAY = startOfDay(Date.now());

/** Days back from today, at a local wall-clock time. `setDate` rather than arithmetic, so a clock
 * change in the window does not move a message to the day before. */
function at(daysAgo: number, hour: number, minute = 0): number {
  const day = new Date(TODAY);
  day.setDate(day.getDate() - daysAgo);
  day.setHours(hour, minute, 0, 0);
  return day.getTime();
}

/** The next Wednesday, which is when Cooper's first piano lesson is. */
function nextWeekday(weekday: number, hour: number, minute = 0): number {
  const day = new Date(TODAY);
  const ahead = (weekday - day.getDay() + 7) % 7 || 7;
  day.setDate(day.getDate() + ahead);
  day.setHours(hour, minute, 0, 0);
  return day.getTime();
}

// -------------------------------------------------------------------------------------------
// People
// -------------------------------------------------------------------------------------------

const person = (name: string | null, address: string): Person => ({ name, address });

const ME = person("Priyanshu Jain", "pj@73ai.org");
const ME_PERSONAL = person("Priyanshu Jain", "priyanshujain@gmail.com");

const MAYA = person("Maya Raghunathan", "maya.raghunathan@example.com");
const ARUN = person("Arun Kulkarni", "arun@meridianproperties.in");
const AIRBNB = person("Airbnb", "automated@airbnb.example");
const SAM = person("Sam Okafor", "sam@sunnydaymusic.example");
const SUNNY = person("Sunny Day Music", "enrolments@sunnydaymusic.example");
const LENA = person("Lena Brandt", "lena@brandt-tischlerei.example");
const DEV = person("Dev Patel", "dev.patel@example.org");
const DOCUSIGN = person("DocuSign", "dse@docusign.example");
const HANNAH = person("Hannah Weiss", "hannah.weiss@example.net");
const RUSSELL = person("Young, Russell", "russell.young@example.net");
const JEFF = person("Jeff Wolfe", "jeff.wolfe@oakridge-school.example");
const CAROLINE = person("Caroline Bauhaus", "caroline@bauhaus-tickets.example");
const PRIYA = person("Priya Menon", "priya.menon@example.com");
const KARTHIK = person("Karthik Rao", "karthik.rao@example.com");
const BROWSER = person("The Browser", "hello@thebrowser.example");
const FIELD_NOTES = person("Field Notes Dispatch", "dispatch@fieldnotes.example");
const LEDGER = person("Ledger Lines", "weekly@ledgerlines.example");
const MEETUP = person("Bengaluru Systems Meetup", "meetup@antithesis.example");
const CRAFTSMAN = person("Craftsman Notes", "post@craftsmannotes.example");
const SPOTIFY = person("Spotify", "no-reply@spotify.example");
const CITY_POWER = person("City Power", "billing@citypower.example");
const BOOKSHOP = person("Bookshop.org", "orders@bookshop.example");
const GITHUB = person("GitHub", "notifications@github.example");
const NETFLIX = person("Netflix", "info@netflix.example");
const NATGEO = person("National Geographic Kids", "customerservice@ngkids.example");
const DISCOUNT_TIRE = person("Discount Tire", "specials@discounttire.example");
const MERIDIAN = person("Meridian Leasing", "leasing@meridianproperties.in");
const TODD = person("Todd Markham", "todd@harborlife.example");
const EVITE = person("Evite on behalf of Robyn Madison", "invitations@evite.example");
const ANITA = person("Anita Desai", "anita@northwind.example");
const PAYROLL = person("Northwind Payroll", "payroll@northwind.example");
const PRAGMATIC = person("The Pragmatic Engineer", "newsletter@pragmaticengineer.example");
const AMMA = person("Amma", "amma.jain@example.com");
// The Latin small c with acute in the display name is not a typo: a homoglyph in the sender
// name is what a real phishing message uses, and the row has to render it as it is.
const PHISH = person("Aćcount Security", "secure@accounts-verify.example");
const PARCEL = person("Parcel Notice", "delivery@parcel-notice.example");
const RECRUITER = person("Talent Partners", "reach@talentpartners.example");

// -------------------------------------------------------------------------------------------
// Accounts
// -------------------------------------------------------------------------------------------

/**
 * Two accounts, as the settings mockup has them. The work account was not granted the calendar
 * scope, which is what makes an invite render read-only with a Grant button on that account and
 * answerable on the other.
 */
export const devAccounts: Account[] = [
  {
    id: "acct-1",
    email: "pj@73ai.org",
    kind: "google",
    name: "Priyanshu Jain",
    color: "hue-4",
    connected: true,
    grantedScopes: [...SCOPES, DRIVE_SCOPE],
    windowDays: 30,
  },
  {
    id: "acct-2",
    email: "priyanshujain@gmail.com",
    kind: "google",
    name: "Personal",
    color: "hue-2",
    connected: true,
    grantedScopes: [...SCOPES, DRIVE_SCOPE, CALENDAR_SCOPE],
    windowDays: 90,
  },
];

const colorOf = (accountId: string): string =>
  devAccounts.find((account) => account.id === accountId)?.color ?? "hue-4";

// -------------------------------------------------------------------------------------------
// Threads
// -------------------------------------------------------------------------------------------

/**
 * A thread as the fixture holds it: the summary the list prints, plus the fields the pane and the
 * places need. `place` is the sender's destination, which is a routing fact rather than a view, so
 * it lives here and `threads_list` filters on it. Trash and spam are not destinations and are not
 * here: they are flags on the summary, which is how a thread put back knows where it came from.
 */
export interface DevThread extends ThreadSummary {
  place: Place;
  archived: boolean;
  labels: string[];
  messages: MessageView[];
  threadNotes: Note[];
  mergedFrom: MergedSource[];
  /** Returned from a snooze and not opened since, which is the Back group at the top of the Inbox. */
  back: boolean;
}

interface Seed {
  key: string;
  accountId?: string;
  place: Place;
  from: Person;
  to?: Person[];
  cc?: Person[];
  subject: string;
  originalSubject?: string;
  snippet: string;
  dateMs: number;
  messageCount?: number;
  unseen?: boolean;
  starred?: boolean;
  trashed?: boolean;
  spam?: boolean;
  pile?: Pile;
  snoozedUntil?: number;
  back?: boolean;
  ignored?: boolean;
  notify?: boolean;
  merged?: MergedSource[];
  note?: string;
  hasDraft?: boolean;
  sending?: boolean;
  archived?: boolean;
  labels?: string[];
  html?: string;
  isHtml?: boolean;
  /**
   * Which surface Rust decided this body onto. Set on the seeds whose markup paints a page, and
   * left at the default on the ones that do not, so the fixture says the same thing the sanitiser
   * would say about the same markup.
   */
  surface?: Surface;
  attachments?: Attachment[];
  trackers?: Tracker[];
  blockedImages?: number;
  invite?: Invite;
  unsubscribe?: Unsubscribe;
  listId?: string;
  quotedHtml?: string;
  /** Older messages, oldest first. The newest message is built from the seed itself. */
  earlier?: Earlier[];
}

interface Earlier {
  from: Person;
  dateMs: number;
  html: string;
  seen?: boolean;
  sentByMe?: boolean;
  attachments?: Attachment[];
}

const para = (...lines: string[]): string => lines.map((line) => `<p>${line}</p>`).join("\n");

const file = (
  id: string,
  messageId: string,
  filename: string,
  mimeType: string,
  size: number,
  extra: Partial<Attachment> = {},
): Attachment => ({
  id,
  messageId,
  filename,
  mimeType,
  size,
  inline: false,
  contentId: null,
  cached: true,
  ...extra,
});

let messageCounter = 0;

function buildThread(seed: Seed): DevThread {
  const accountId = seed.accountId ?? "acct-1";
  const to = seed.to ?? [accountId === "acct-1" ? ME : ME_PERSONAL];
  const earlier = seed.earlier ?? [];
  const messages: MessageView[] = [];

  const push = (
    from: Person,
    dateMs: number,
    html: string,
    over: Partial<MessageView> = {},
  ): MessageView => {
    messageCounter += 1;
    const view: MessageView = {
      id: `msg-${messageCounter}`,
      messageId: `<${seed.key.replace(/[<>]/g, "")}-${messageCounter}>`,
      threadKey: seed.key,
      from,
      to,
      cc: seed.cc ?? [],
      bcc: [],
      replyTo: [],
      dateMs,
      subject: seed.originalSubject ?? seed.subject,
      html,
      // The fixture is a mirror that has everything. `marginmail-dev-pending` in mockIpc.ts is
      // where a thread whose bodies have not been fetched yet comes from.
      bodyPending: false,
      quotedHtml: null,
      isHtml: seed.isHtml ?? false,
      surface: seed.surface ?? "theme",
      attachments: [],
      trackers: [],
      blockedImages: 0,
      imagesLoaded: false,
      seen: true,
      draft: false,
      sentByMe: false,
      invite: null,
      unsubscribe: seed.unsubscribe ?? null,
      listId: seed.listId ?? null,
      ...over,
    };
    messages.push(view);
    return view;
  };

  for (const older of earlier) {
    push(older.from, older.dateMs, older.html, {
      seen: older.seen ?? true,
      sentByMe: older.sentByMe ?? false,
      attachments: older.attachments ?? [],
    });
  }

  push(seed.from, seed.dateMs, seed.html ?? para(seed.snippet), {
    seen: !seed.unseen,
    attachments: seed.attachments ?? [],
    trackers: seed.trackers ?? [],
    blockedImages: seed.blockedImages ?? 0,
    invite: seed.invite ?? null,
    quotedHtml: seed.quotedHtml ?? null,
  });

  const participants = [seed.from, ...earlier.map((older) => older.from), ...to].filter(
    (candidate, index, list) =>
      list.findIndex((other) => other.address === candidate.address) === index,
  );

  return {
    key: seed.key,
    accountId,
    accountColor: colorOf(accountId),
    subject: seed.subject,
    originalSubject: seed.originalSubject ?? null,
    from: seed.from,
    participants,
    snippet: seed.snippet,
    dateMs: seed.dateMs,
    messageCount: seed.messageCount ?? messages.length,
    unseen: seed.unseen ?? false,
    starred: seed.starred ?? false,
    trashed: seed.trashed ?? false,
    spam: seed.spam ?? false,
    hasAttachment: messages.some((message) => message.attachments.length > 0),
    hasDraft: seed.hasDraft ?? false,
    pile: seed.pile ?? null,
    snoozedUntil: seed.snoozedUntil ?? null,
    ignored: seed.ignored ?? false,
    notify: seed.notify ?? false,
    merged: (seed.merged?.length ?? 0) > 0,
    note: seed.note ?? null,
    group: "",
    sending: seed.sending ?? false,
    place: seed.place,
    archived: seed.archived ?? false,
    labels: seed.labels ?? [],
    messages,
    threadNotes: [],
    mergedFrom: seed.merged ?? [],
    back: seed.back ?? false,
  };
}

const LEASE_PDF = file("att-lease", "msg-lease", "Studio-lease-2026-v2.pdf", "application/pdf", 421_888);
const STATEMENT_PDF = file(
  "att-statement",
  "msg-statement",
  "Statement-August.pdf",
  "application/pdf",
  90_112,
);
const SLIDES_PDF = file("att-slides", "msg-slides", "Cache-oblivious-layouts.pdf", "application/pdf", 2_310_144);
const SIGNED_PDF = file(
  "att-signed",
  "msg-signed",
  "Studio lease 2026 (signé).pdf",
  "application/pdf",
  518_144,
);
const RIDGE_1 = file("att-ridge-1", "msg-ridge", "ridge-walk-1.png", "image/png", 1_204_224, {
  inline: true,
  contentId: "ridge-walk-1@example.net",
});
const RIDGE_2 = file("att-ridge-2", "msg-ridge", "ridge-walk-2.png", "image/png", 1_118_208, {
  inline: true,
  contentId: "ridge-walk-2@example.net",
});
const ENROLMENT_PDF = file("att-enrol", "msg-enrol", "Enrolment-form.pdf", "application/pdf", 141_312);
const TERM_DATES_PDF = file("att-terms", "msg-enrol", "Term-dates.pdf", "application/pdf", 61_440);
const INVITE_ICS = file("att-invite", "msg-invite", "invite.ics", "text/calendar", 2_048);
const PAYSLIP_PDF = file("att-payslip", "msg-payslip", "Payslip-August-2026.pdf", "application/pdf", 74_752);

const ONE_CLICK = (url: string, mailto: string | null): Unsubscribe => ({
  oneClick: true,
  mailto,
  url,
});

const LINK_ONLY = (url: string): Unsubscribe => ({ oneClick: false, mailto: null, url });

const PIANO_INVITE: Invite = {
  uid: "4c9b2f7a-piano-cooper@sunnydaymusic.example",
  summary: "Piano lesson: Cooper",
  startMs: nextWeekday(3, 17, 0),
  endMs: nextWeekday(3, 17, 45),
  allDay: false,
  location: "Sunny Day Music, Bandra",
  organizer: SUNNY,
  description: "First lesson for Cooper. Nothing to prepare, and there is a piano here, so nothing to carry either.",
  myResponse: "needs-action",
  calendarLink: "margin-calendar://event/4c9b2f7a-piano-cooper",
};

const seeds: Seed[] = [
  // ---------------------------------------------------------------------------------------
  // Inbox, personal account
  // ---------------------------------------------------------------------------------------
  {
    key: "<CAF7q9maya-3f2a1c@mail.example.com>",
    place: "inbox",
    from: MAYA,
    subject: "Dinner on Thursday?",
    snippet: "Priya said the place on Church Street takes bookings now, want me to",
    dateMs: at(0, 11, 42),
    messageCount: 3,
    unseen: true,
    hasDraft: true,
    html: para(
      "Priya said the place on Church Street takes bookings now, want me to put us down for four at eight? Everyone can make Thursday except Karthik, who has the badminton thing and says he will come late.",
      "If you would rather somewhere quieter there is the Malleswaram place we went to in June. Say by tomorrow and I will call them.",
      "Maya",
    ),
    earlier: [
      {
        from: MAYA,
        dateMs: at(2, 19, 10),
        html: para("Are we still on for this week? I have not asked anybody yet."),
      },
      {
        from: ME,
        dateMs: at(1, 9, 2),
        sentByMe: true,
        html: para("Thursday suits me. Anywhere but the place with the loud ceiling."),
      },
    ],
  },
  {
    key: "<lease-2026-001@meridianproperties.in>",
    place: "inbox",
    from: ARUN,
    cc: [MERIDIAN],
    subject: "Lease renewal for the studio",
    snippet: "Attached the revised draft. The only change is clause 7, which now",
    dateMs: at(0, 10, 15),
    messageCount: 2,
    unseen: true,
    starred: true,
    hasDraft: true,
    isHtml: true,
    note: "Check the four percent cap against last year's index before signing.",
    attachments: [LEASE_PDF],
    trackers: [{ vendor: "HubSpot", url: "https://track.hubspot.com/__ptq.gif?k=8812" }],
    blockedImages: 1,
    labels: ["label-flat"],
    quotedHtml: para("On Mon, Priyanshu Jain wrote: Two things before I sign, the notice period in clause 7 and the rent review."),
    html: para(
      "Hi Priyanshu,",
      "Attached the revised draft. The only change is clause 7, which now reads three months either side rather than six. The rent review in clause 12 stays as it was, tied to the index and capped at four percent.",
      "If that works, sign when you get a moment and I will countersign the same day. Happy to walk through anything on a call, Thursday afternoon is open.",
      "Best,<br>Arun",
    ),
    earlier: [
      {
        from: ME,
        dateMs: at(3, 14, 20),
        sentByMe: true,
        html: para(
          "Hi Arun, thanks for sending the renewal over. Two things before I sign: the notice period in clause 7 and the rent review in clause 12.",
        ),
      },
    ],
  },
  {
    key: "<a1b2c3d4-e5f6-4a7b-8c9d-000000000001@airbnb.example>",
    place: "inbox",
    from: AIRBNB,
    subject: "Your reservation in Lisbon is confirmed",
    snippet: "Check-in Friday 12 September after 15:00. Your host Inês will send",
    dateMs: at(0, 9, 3),
    unseen: true,
    isHtml: true,
    blockedImages: 4,
    // The other half of the rule, and the reason this one is the confirmation rather than the
    // lease: it paints a page of its own, so it keeps it in both palettes.
    surface: "paper",
    html:
      `<div style="background-color:#f7f7f7;padding:24px">` +
      para(
        "Check-in Friday 12 September after 15:00. Your host Inês will send the door code the evening before.",
        "Rua da Bica de Duarte Belo 42, Lisboa. The tram stops at the top of the street and the walk down is five minutes.",
      ) +
      `</div>`,
  },
  {
    key: "<sunny-enrol-01@sunnydaymusic.example>",
    place: "inbox",
    from: SAM,
    subject: "Piano on Wednesdays",
    originalSubject: "Re: Fw: (no subject)",
    snippet: "Got it, thank you. Wednesdays at five work for us. Is there a",
    dateMs: at(1, 15, 40),
    messageCount: 3,
    unseen: true,
    notify: true,
    invite: PIANO_INVITE,
    attachments: [INVITE_ICS],
    merged: [
      { key: "<sunny-enrol-01@sunnydaymusic.example>", subject: "Enrolment received for Cooper" },
      { key: "<CAJ8pj-piano-06@mail.73ai.org>", subject: "Re: Fw: (no subject)" },
    ],
    quotedHtml: para(
      "On Mon, Priyanshu Jain wrote: Hi Sam, form signed and attached. Would a weekday after school suit Cooper?",
    ),
    html: para(
      "Got it, thank you. Wednesdays at five work for us.",
      "The first lesson is on the 9th, nothing to prepare.",
      "The invitation is attached, it should land in your calendar.<br>Sam",
    ),
    earlier: [
      {
        from: SUNNY,
        dateMs: at(22, 11, 2),
        attachments: [ENROLMENT_PDF, TERM_DATES_PDF],
        html: para(
          "Enrolment received for Cooper. Term starts the week of 8 September and your teacher will be in touch to fix a weekly slot.",
        ),
      },
      {
        from: ME,
        dateMs: at(3, 9, 12),
        sentByMe: true,
        html: para(
          "Hi Sam, form signed and attached. Would a weekday after school suit Cooper? He finishes at four and we are ten minutes away on foot.",
        ),
      },
    ],
  },
  {
    key: "<badminton-2026-09@example.com>",
    place: "inbox",
    from: KARTHIK,
    subject: "Badminton on Saturday",
    snippet: "Court is booked for seven. Bring the good shuttles, the cheap ones",
    dateMs: at(4, 19, 30),
    unseen: true,
    back: true,
    html: para(
      "Court is booked for seven. Bring the good shuttles, the cheap ones from last time went sideways.",
      "Sanjay is out, so it is three of us unless you can find a fourth.",
    ),
  },
  {
    key: "<20260902071200.2B7C4@brandt-tischlerei.example>",
    place: "inbox",
    from: LENA,
    subject: "Kitchen bench quote",
    snippet: "Sounds good, Julie. Any afternoon next week works for me.",
    dateMs: at(1, 11, 15),
    messageCount: 7,
    note: "Ask about the oak finish before confirming",
    html: para("Sounds good, Julie. Any afternoon next week works for me."),
  },
  {
    key: "<CAG5dev-slides-01@mail.example.org>",
    place: "inbox",
    from: DEV,
    subject: "Slides from the talk",
    snippet: "Here they are, plus the reading list I mentioned. The paper on",
    dateMs: at(3, 21, 14),
    messageCount: 3,
    isHtml: true,
    attachments: [SLIDES_PDF],
    quotedHtml: para("On Mon, Priyanshu Jain wrote: Good talk. Could you send the slides, and the reading list you put up at the end?"),
    html: para(
      "Here they are, plus the reading list I mentioned. The paper on cache oblivious layouts is the one to start with.",
    ),
  },
  {
    key: "<b0d1c4f2e3a54c8f9b7a1d2e3f405162@docusign.example>",
    place: "inbox",
    from: DOCUSIGN,
    subject: "Completed: Studio lease 2026",
    snippet: "All parties have completed the envelope. You can access the",
    dateMs: at(3, 18, 41),
    attachments: [SIGNED_PDF],
    html: para(
      "All parties have completed the envelope. You can access the signed copy from the link below or from the attached PDF.",
    ),
  },
  {
    key: "<CADhw-hawaii-04@mail.example.net>",
    place: "inbox",
    from: HANNAH,
    subject: "Photos from the Hawaii trip",
    snippet: "Finally went through them all. The ones from the ridge walk are",
    dateMs: at(4, 12, 19),
    messageCount: 2,
    starred: true,
    isHtml: true,
    attachments: [RIDGE_1, RIDGE_2],
    html: para(
      "Finally went through them all. The ones from the ridge walk are the best of the lot, two of them are below and the rest are in the folder.",
      "Hannah",
    ),
    earlier: [
      {
        from: HANNAH,
        dateMs: at(11, 20, 5),
        html: para("Back, sunburnt, and with nine hundred photographs. Give me a week."),
      },
    ],
  },
  {
    key: "<e7f1a9c2-pumpkin@example.net>",
    place: "inbox",
    from: RUSSELL,
    subject: "Pumpkin bread recipe",
    snippet: "From my mother's card, transcribed as best I could. Bake at 175",
    dateMs: at(10, 17, 55),
    html: para(
      "From my mother's card, transcribed as best I could. Bake at 175 for fifty minutes, and do not open the oven before forty.",
      "Two loaves, or one loaf and twelve muffins. The muffins take half the time and are better the next day.",
      "Russ",
    ),
  },
  {
    key: "<oak-ptc-2026-098@oakridge-school.example>",
    place: "inbox",
    from: JEFF,
    to: [ME, HANNAH, CAROLINE],
    subject: "Re: Cooper's parent-teacher conference",
    snippet: "Mr. and Mrs. Young, this is just a reminder to schedule your",
    dateMs: at(1, 16, 2),
    messageCount: 2,
    pile: "reply-later",
    html: para(
      "Mr. and Mrs. Young, this is just a reminder to schedule your parent-teacher conference with me. You can feel free to do that here.",
      "If you would prefer a video call instead of meeting at school, I am happy to do that as well. Please let me know what date and time works best for you.",
    ),
    earlier: [
      {
        from: JEFF,
        dateMs: at(9, 8, 30),
        html: para("Conference week is the first week of October. The booking sheet opens on Monday."),
      },
    ],
  },
  {
    key: "<recital-nov-2026@sunnydaymusic.example>",
    place: "inbox",
    from: SUNNY,
    subject: "Recital in November",
    snippet: "The autumn recital is on the 21st. Every pupil plays one piece and",
    dateMs: at(14, 10, 5),
    pile: "reply-later",
    html: para(
      "The autumn recital is on the 21st. Every pupil plays one piece and the hall takes two hundred, so tell us how many seats you need.",
    ),
  },
  {
    key: "<coorg-oct-2026@example.com>",
    place: "inbox",
    from: PRIYA,
    subject: "Re: Bengaluru in October",
    snippet: "I land on the 3rd and I am free the whole of that first weekend.",
    dateMs: at(5, 22, 40),
    messageCount: 4,
    pile: "reply-later",
    html: para(
      "I land on the 3rd and I am free the whole of that first weekend. Tell me what is worth doing and I will work around it.",
    ),
  },
  {
    key: "<9c11e0a4-tickets@bauhaus-tickets.example>",
    place: "inbox",
    from: CAROLINE,
    to: [ME, MAYA],
    subject: "Les Misérables tickets",
    snippet: "I got four for the second night rather than the first, the seats",
    dateMs: at(4, 21, 4),
    messageCount: 2,
    pile: "set-aside",
    html: para(
      "I got four for the second night rather than the first, the seats are better and it is ten pounds less each. Row H, slightly left of centre.",
      "Nobody has to pay me back until after, and if Karthik drops out I know two people who would take his.",
    ),
  },
  {
    key: "<coorg-stays@example.com>",
    place: "inbox",
    from: MAYA,
    subject: "Weekend in Coorg, places to stay",
    snippet: "Three of them take dogs and two have a kitchen. The last one is",
    dateMs: at(9, 13, 25),
    pile: "set-aside",
    html: para(
      "Three of them take dogs and two have a kitchen. The last one is the cheapest but it is an hour from anything.",
    ),
  },
  {
    key: "<deposit-refund-2026@meridianproperties.in>",
    place: "inbox",
    from: MERIDIAN,
    subject: "Deposit refund for the old flat",
    snippet: "The deduction for the carpet is the only item still open. We will",
    dateMs: at(12, 15, 12),
    snoozedUntil: at(-3, 8, 0),
    html: para(
      "The deduction for the carpet is the only item still open. We will write again once the cleaner has invoiced.",
    ),
  },

  // ---------------------------------------------------------------------------------------
  // Feed, personal account
  // ---------------------------------------------------------------------------------------
  {
    key: "<01000198a2f4c1b2-9b0c1d2e@thebrowser.example>",
    place: "feed",
    from: BROWSER,
    subject: "Five things worth reading this weekend",
    snippet: "A long piece on the history of the pencil, a short one on why bridges hum, and three more.",
    dateMs: at(0, 6, 0),
    isHtml: true,
    listId: "the-browser.list.thebrowser.example",
    unsubscribe: ONE_CLICK(
      "https://thebrowser.example/unsubscribe?u=91827&id=8f3a1c",
      "mailto:unsubscribe+91827@mail.thebrowser.example?subject=unsub",
    ),
    blockedImages: 2,
    trackers: [{ vendor: "Mailchimp", url: "https://thebrowser.example/o/open.gif?u=91827" }],
    html: para(
      "Good morning. It is a long weekend in some places and a wet one in most, so this edition leans toward pieces you can settle into. A long piece on the history of the pencil, a short one on why bridges hum, and three more.",
      "<b>The pencil, at length.</b> Henry Petroski wrote a whole book about the pencil and this essay is the argument for why that was a reasonable thing to do. Graphite, cedar, the Napoleonic wars and a factory in Nuremberg that still runs. Forty minutes, worth every one.",
      "<b>Why bridges hum.</b> A short explanation of vortex shedding from an engineer who spent a career listening to cables. Includes the story of a footbridge that was closed for two years because it sang in a particular wind.",
      "<b>The last typewriter repairman in Mumbai.</b> A profile that is really about what it means to keep a trade going after the trade has gone.",
    ),
  },
  {
    key: "<fn-2026-09-dispatch@fieldnotes.example>",
    place: "feed",
    from: FIELD_NOTES,
    subject: "September: the notebook that survived a washing machine",
    snippet: "A reader in Tromsø sent us a photo of a pocket notebook that went",
    dateMs: at(3, 9, 30),
    isHtml: true,
    listId: "dispatch.fieldnotes.example",
    unsubscribe: LINK_ONLY("https://fieldnotes.example/u/7712"),
    html: para(
      "A reader in Tromsø sent us a photo of a pocket notebook that went through a full cycle at forty degrees, in a coat, with the coat. Every page is still legible.",
      "Also this month: the autumn edition ships on the 15th. Three colours, the usual count.",
    ),
  },
  {
    key: "<ll-2026-08-30@ledgerlines.example>",
    place: "feed",
    from: LEDGER,
    subject: "The bond market, again",
    snippet: "Rates did the one thing nobody had written a note about, which is",
    dateMs: at(4, 5, 30),
    listId: "weekly.ledgerlines.example",
    unsubscribe: { oneClick: false, mailto: "mailto:stop@ledgerlines.example", url: null },
    html: para(
      "Rates did the one thing nobody had written a note about, which is nothing at all. Here is what that means for the long end.",
    ),
  },
  {
    key: "<meetup-sept-2026@antithesis.example>",
    place: "feed",
    from: MEETUP,
    subject: "September meetup: deterministic simulation testing",
    snippet: "Church Street, the usual room, doors at half six. Two talks and",
    dateMs: at(6, 17, 45),
    listId: "meetup.antithesis.example",
    unsubscribe: LINK_ONLY("https://antithesis.example/unsubscribe"),
    html: para(
      "Church Street, the usual room, doors at half six. Two talks and then the pub, as ever.",
    ),
  },
  {
    key: "<craftsman-2026-08@craftsmannotes.example>",
    place: "feed",
    from: CRAFTSMAN,
    subject: "Sharpening, part two",
    snippet: "The stone matters less than the angle, and the angle matters less",
    dateMs: at(11, 7, 15),
    listId: "post.craftsmannotes.example",
    unsubscribe: LINK_ONLY("https://craftsmannotes.example/u/221"),
    html: para(
      "The stone matters less than the angle, and the angle matters less than doing it at all. Part one is linked at the foot.",
    ),
  },

  // ---------------------------------------------------------------------------------------
  // Paper Trail, personal account
  // ---------------------------------------------------------------------------------------
  {
    key: "<20260903063018.4d2a1f9c@mail.spotify.example>",
    place: "paper-trail",
    from: SPOTIFY,
    subject: "Your receipt",
    snippet: "Thank you for purchasing Spotify Premium. Amount charged",
    dateMs: at(0, 6, 30),
    html: para(
      "Thank you for purchasing Spotify Premium. Amount charged 11.99, on the card ending 4417. Your next payment is due in a month.",
    ),
  },
  {
    key: "<116400046977232.1756846291@citypower.example>",
    place: "paper-trail",
    from: CITY_POWER,
    subject: "Bill payment pending",
    snippet: "Your one-time payment of 98.26 has been received and is pending",
    dateMs: at(1, 16, 4),
    attachments: [STATEMENT_PDF],
    html: para(
      "Dear Customer,",
      "Your one-time electronic payment of 98.26 for your City Power bill has been received and is pending. Your payment will be posted within two business days.",
      "Confirmation number 116400046977232",
      "Please keep this message for your records. If you did not make this payment, call us on the number printed on your statement.",
      "Thank you,<br>City Power Customer Care",
    ),
  },
  {
    key: "<bookshop-91221@bookshop.example>",
    place: "paper-trail",
    from: BOOKSHOP,
    subject: "Your order has shipped",
    snippet: "Two items are on their way. Track your parcel with the",
    dateMs: at(1, 9, 41),
    html: para("Two items are on their way. Track your parcel with the number below."),
  },
  {
    key: "<github-bundle-2026-08-31@github.example>",
    place: "paper-trail",
    from: GITHUB,
    subject: "Notifications, bundled",
    snippet: "margin-calendar: 3 new issues, 9 comments",
    dateMs: at(3, 20, 12),
    messageCount: 12,
    listId: "margin-calendar.github.example",
    unsubscribe: LINK_ONLY("https://github.example/notifications/unsubscribe/91"),
    html: para("margin-calendar: 3 new issues, 9 comments."),
  },
  {
    key: "<netflix-2026-08-28@netflix.example>",
    place: "paper-trail",
    from: NETFLIX,
    subject: "Receipt for your payment",
    snippet: "We received your payment of 15.49 for your membership. Your next",
    dateMs: at(8, 4, 20),
    html: para("We received your payment of 15.49 for your membership. Your next bill is in a month."),
  },
  {
    key: "<ngkids-gift-4471@ngkids.example>",
    place: "paper-trail",
    from: NATGEO,
    subject: "Gift order confirmation",
    snippet: "Thank you for your gift order. The first issue will arrive in",
    dateMs: at(9, 15, 2),
    html: para("Thank you for your gift order. The first issue will arrive in six to eight weeks."),
  },
  {
    key: "<camp-4471-8821@discounttire.example>",
    place: "paper-trail",
    from: DISCOUNT_TIRE,
    subject: "Your air pressure reminder",
    snippet: "It has been 30 days since your last check. Stop by any store for a",
    dateMs: at(11, 8, 0),
    isHtml: true,
    blockedImages: 6,
    trackers: [{ vendor: "Braze", url: "https://discounttire.example/o/open.png?id=4471" }],
    unsubscribe: LINK_ONLY("https://discounttire.example/u/4471"),
    html: para(
      "It has been 30 days since your last check. Stop by any store for a free air pressure check, no appointment needed.",
    ),
  },

  // ---------------------------------------------------------------------------------------
  // The work account, so All accounts and the switcher have something to merge
  // ---------------------------------------------------------------------------------------
  {
    key: "<northwind-roadmap-q3@northwind.example>",
    accountId: "acct-2",
    place: "inbox",
    from: ANITA,
    subject: "Re: Q3 roadmap review",
    snippet: "Moved it to Friday so the platform team can be there. Same room,",
    dateMs: at(0, 12, 5),
    messageCount: 5,
    unseen: true,
    html: para(
      "Moved it to Friday so the platform team can be there. Same room, same hour. The deck is in the drive folder if you want to read ahead.",
    ),
  },
  {
    key: "<amma-photos-2026@example.com>",
    accountId: "acct-2",
    place: "inbox",
    from: AMMA,
    subject: "Photos from Sunday",
    snippet: "Your cousin sent these. The one of you in the garden is the only",
    dateMs: at(6, 8, 55),
    html: para("Your cousin sent these. The one of you in the garden is the only good one, print it."),
  },
  {
    key: "<northwind-payslip-08@northwind.example>",
    accountId: "acct-2",
    place: "paper-trail",
    from: PAYROLL,
    subject: "Payslip for August",
    snippet: "Your payslip is attached and is also on the portal. Nothing has",
    dateMs: at(2, 6, 5),
    attachments: [PAYSLIP_PDF],
    html: para("Your payslip is attached and is also on the portal. Nothing has changed this month."),
  },
  {
    key: "<pragmatic-2026-09@pragmaticengineer.example>",
    accountId: "acct-2",
    place: "feed",
    from: PRAGMATIC,
    subject: "The scaling of small teams",
    snippet: "What happens to a five person team at fifteen, and why the answer",
    dateMs: at(2, 7, 30),
    listId: "newsletter.pragmaticengineer.example",
    unsubscribe: ONE_CLICK("https://pragmaticengineer.example/u/8812", null),
    html: para("What happens to a five person team at fifteen, and why the answer is never more process."),
  },

  // ---------------------------------------------------------------------------------------
  // The corners: spam, trash and a screened-out sender
  // ---------------------------------------------------------------------------------------
  {
    key: "<verify-8812@accounts-verify.example>",
    // Gmail junked it, and its sender has no rule, so taking the mark off owes them a decision.
    place: "inbox",
    spam: true,
    from: PHISH,
    subject: "Account verification required within 24 hours",
    snippet: "Your account will be suspended unless you verify your details",
    dateMs: at(2, 3, 12),
    isHtml: true,
    blockedImages: 3,
    html: para("Your account will be suspended unless you verify your details today."),
  },
  {
    key: "<parcel-991@parcel-notice.example>",
    // Trashed out of the Paper Trail rather than the Inbox, because where a thread came from is
    // the thing "put back" has to get right.
    place: "paper-trail",
    trashed: true,
    from: PARCEL,
    subject: "Your parcel could not be delivered",
    snippet: "A small fee is required to reschedule delivery of your item",
    dateMs: at(5, 11, 44),
    html: para("A small fee is required to reschedule delivery of your item."),
  },
  {
    key: "<talent-reach-2211@talentpartners.example>",
    place: "screened-out",
    from: RECRUITER,
    subject: "An exciting opportunity",
    snippet: "I came across your profile and thought of a role that could be a",
    dateMs: at(7, 10, 30),
    html: para("I came across your profile and thought of a role that could be a great fit."),
  },
];

export const devThreads: DevThread[] = seeds.map(buildThread);

// -------------------------------------------------------------------------------------------
// Notes, clips, snoozes, rules, screener, labels, drafts, outbox
// -------------------------------------------------------------------------------------------

export const devNotes: Note[] = [
  {
    id: "note-1",
    threadKey: "<lease-2026-001@meridianproperties.in>",
    body: "Check the four percent cap against last year's index before signing. Ask Maya if she still has the 2024 letter.",
    createdAtMs: at(0, 10, 40),
    afterMessageId: null,
  },
  {
    id: "note-2",
    threadKey: "<20260902071200.2B7C4@brandt-tischlerei.example>",
    body: "Ask about the oak finish before confirming",
    createdAtMs: at(1, 11, 30),
    afterMessageId: null,
  },
];

for (const note of devNotes) {
  const thread = devThreads.find((candidate) => candidate.key === note.threadKey);
  thread?.threadNotes.push(note);
}

export const devSnoozes: Snooze[] = [
  {
    threadKey: "<deposit-refund-2026@meridianproperties.in>",
    returnAtMs: at(-3, 8, 0),
    kind: "next-week",
  },
];

export const devClips: Clip[] = [
  {
    id: "clip-1",
    accountId: "acct-1",
    threadKey: "<01000198a2f4c1b2-9b0c1d2e@thebrowser.example>",
    messageId: "<01000198a2f4c1b2-9b0c1d2e@thebrowser.example>",
    text: "Graphite, cedar, the Napoleonic wars and a factory in Nuremberg that still runs.",
    sender: BROWSER,
    subject: "Five things worth reading this weekend",
    createdAtMs: at(0, 6, 12),
  },
  {
    id: "clip-2",
    accountId: "acct-1",
    threadKey: "<e7f1a9c2-pumpkin@example.net>",
    messageId: "<e7f1a9c2-pumpkin@example.net>",
    text: "Bake at 175 for fifty minutes, and do not open the oven before forty.",
    sender: RUSSELL,
    subject: "Pumpkin bread recipe",
    createdAtMs: at(10, 18, 20),
  },
];

/** Every sender with a decision. The Screener holds the ones that are not here yet. */
export const devRules: SenderRule[] = [
  ...[MAYA, ARUN, SAM, SUNNY, LENA, DEV, HANNAH, RUSSELL, JEFF, CAROLINE, PRIYA, KARTHIK, MERIDIAN].map(
    (who): SenderRule => ({
      accountId: "acct-1",
      subject: who.address,
      isDomain: false,
      destination: "inbox",
      decidedAtMs: at(22, 9, 0),
      reason: "Written by a person",
    }),
  ),
  ...[BROWSER, FIELD_NOTES, LEDGER, MEETUP, CRAFTSMAN].map(
    (who): SenderRule => ({
      accountId: "acct-1",
      subject: who.address,
      isDomain: false,
      destination: "feed",
      decidedAtMs: at(22, 9, 0),
      reason: "Has an unsubscribe header",
    }),
  ),
  ...[SPOTIFY, CITY_POWER, BOOKSHOP, GITHUB, NETFLIX, NATGEO, DISCOUNT_TIRE].map(
    (who): SenderRule => ({
      accountId: "acct-1",
      subject: who.address,
      isDomain: false,
      destination: "paper-trail",
      decidedAtMs: at(22, 9, 0),
      reason: "Sent by a service, or transactional",
    }),
  ),
  // The exception that proves the model: DocuSign would be suggested for the Paper Trail and was
  // moved to the Inbox by hand, which is what the contact card is for.
  {
    accountId: "acct-1",
    subject: DOCUSIGN.address,
    isDomain: false,
    destination: "inbox",
    decidedAtMs: at(3, 18, 45),
    reason: "Moved here from Paper Trail",
  },
  {
    accountId: "acct-1",
    subject: AIRBNB.address,
    isDomain: false,
    destination: "inbox",
    decidedAtMs: at(30, 9, 0),
    reason: "Moved here from Paper Trail",
  },
  {
    accountId: "acct-1",
    subject: "talentpartners.example",
    isDomain: true,
    destination: "screened-out",
    decidedAtMs: at(7, 10, 40),
    reason: "Screened out, everyone at this domain",
  },
  {
    accountId: "acct-2",
    subject: "northwind.example",
    isDomain: true,
    destination: "inbox",
    decidedAtMs: at(60, 9, 0),
    reason: "Written by a person",
  },
  {
    accountId: "acct-2",
    subject: PAYROLL.address,
    isDomain: false,
    destination: "paper-trail",
    decidedAtMs: at(60, 9, 0),
    reason: "Sent by a service, or transactional",
  },
  {
    accountId: "acct-2",
    subject: PRAGMATIC.address,
    isDomain: false,
    destination: "feed",
    decidedAtMs: at(60, 9, 0),
    reason: "Has an unsubscribe header",
  },
  {
    accountId: "acct-2",
    subject: AMMA.address,
    isDomain: false,
    destination: "inbox",
    decidedAtMs: at(60, 9, 0),
    reason: "Written by a person",
  },
];

/** The three cards the Screener mockup shows, with one suggestion of each kind. */
export const devScreener: ScreenerCard[] = [
  {
    accountId: "acct-1",
    sender: TODD,
    threadKey: "<c41e77a9-todd@harborlife.example>",
    subject: "Re: Life insurance quote",
    snippet:
      "Hello Ms. Young, I hope all is well with you. I wanted to touch base on our conversation from a few weeks ago.",
    dateMs: at(3, 9, 48),
    suggestion: "inbox",
    reason: "Written by a person",
    waiting: 1,
  },
  {
    accountId: "acct-1",
    sender: person("The Browser", "weekend@thebrowser.example"),
    threadKey: "<weekend-01@thebrowser.example>",
    subject: "Five things worth reading this weekend",
    snippet:
      "A long piece on the history of the pencil, a short one on why bridges hum, and three more.",
    dateMs: at(2, 6, 0),
    suggestion: "feed",
    reason: "Has an unsubscribe header",
    waiting: 2,
  },
  {
    accountId: "acct-1",
    sender: EVITE,
    threadKey: "<evite-99120-8f2@evite.example>",
    subject: "You have an invitation from Robyn Madison",
    snippet: "Join us for Jack's 8th birthday! Robyn Madison needs your RSVP.",
    dateMs: at(2, 19, 22),
    suggestion: "paper-trail",
    reason: "Sent by a service for a person",
    waiting: 1,
  },
  {
    accountId: "acct-2",
    sender: person("Notion", "team@notion.example"),
    threadKey: "<notion-welcome@notion.example>",
    subject: "Welcome to your new workspace",
    snippet: "Here is how to get the team started, plus three templates worth stealing.",
    dateMs: at(4, 11, 0),
    suggestion: "feed",
    reason: "Has an unsubscribe header",
    waiting: 3,
  },
];

export const devLabels: LabelInfo[] = [
  { id: "label-flat", accountId: "acct-1", name: "The flat", kind: "user" },
  { id: "label-school", accountId: "acct-1", name: "School", kind: "user" },
  { id: "label-travel", accountId: "acct-1", name: "Travel", kind: "user" },
  { id: "label-northwind", accountId: "acct-2", name: "Northwind", kind: "user" },
  // Gmail's own, as they arrive: a raw id for a name and a place in this app already. They are
  // stored and never offered, which is what keeps Spam out of the palette twice.
  { id: "IMPORTANT", accountId: "acct-1", name: "IMPORTANT", kind: "system" },
  { id: "SPAM", accountId: "acct-1", name: "SPAM", kind: "system" },
];

export const devDrafts: Draft[] = [
  {
    id: "draft-1",
    accountId: "acct-1",
    threadKey: null,
    inReplyTo: null,
    fromAlias: null,
    to: [MAYA],
    cc: [],
    bcc: [],
    subject: "Thursday",
    bodyHtml: para(
      "Maya, Thursday works. Church Street at eight?",
      "I will book it if you do not mind the walk from the station.",
    ),
    attachments: [],
    remindAtMs: null,
  },
  {
    id: "draft-2",
    accountId: "acct-1",
    threadKey: "<lease-2026-001@meridianproperties.in>",
    inReplyTo: "<lease-2026-001@meridianproperties.in>",
    fromAlias: null,
    to: [ARUN],
    cc: [],
    bcc: [],
    subject: "Re: Lease renewal for the studio",
    bodyHtml: para("Thanks Arun, three months works. One more question on clause 12"),
    attachments: [],
    remindAtMs: null,
  },
];

export const devOutbox: Outgoing[] = [
  {
    id: "out-1",
    accountId: "acct-1",
    threadKey: "<lease-2026-001@meridianproperties.in>",
    to: [ARUN],
    subject: "Re: Lease renewal for the studio",
    holdUntilMs: Date.now() + 8_000,
    attempts: 0,
    lastError: null,
  },
];

export const devSettings: Settings = {
  theme: "system",
  fontUi: { kind: "bundled", id: "hanken-grotesk" },
  fontText: { kind: "bundled", id: "literata" },
  textSize: 15,
  readingPane: true,
  density: "comfortable",

  accounts: [
    {
      accountId: "acct-1",
      name: "Priyanshu Jain",
      color: "hue-4",
      windowDays: 30,
      signature: "Priyanshu Jain · 73ai",
      aliases: ["priyanshu@73ai.org", "hello@73ai.org"],
    },
    {
      accountId: "acct-2",
      name: "Personal",
      color: "hue-2",
      windowDays: 90,
      signature: "",
      aliases: [],
    },
  ],
  attachmentCacheMb: 512,
  prefetchBodies: true,

  remoteImages: "ask",
  linkCleaning: true,

  screenerEnabled: true,
  holdReplies: false,
  suggestions: true,

  // Minutes from midnight, not hours: `SnoozeTimes` says so and Rust defaults to 8 * 60. Seeding
  // hours here made every Tomorrow eight minutes past midnight.
  snoozeTimes: { laterTodayHours: 3, tomorrowAt: 8 * 60, weekendAt: 9 * 60, nextWeekAt: 8 * 60 },
  swipeRight: "archive",
  swipeLeft: "reply-later",
  feedAutoTrashDays: 0,

  undoDelaySecs: 10,
  replyAllDefault: false,
  instantIntro: "Thank you for the introduction, moving you to bcc.",

  notifications: true,
  notifyPlaces: [],
  badge: true,

  backup: {
    store: "drive",
    configured: true,
    lastBackupMs: at(0, 4, 15),
    hasPhrase: true,
    r2Bucket: null,
    r2Endpoint: null,
  },
};

export const devStorage: StorageUsed[] = [
  {
    accountId: "acct-1",
    messages: 4_812,
    threads: 1_944,
    mirrorBytes: 71_303_168,
    bodiesBytes: 132_120_576,
    attachmentsBytes: 264_241_152,
    stateBytes: 1_048_576,
    oldestMs: at(30, 0, 0),
  },
  {
    accountId: "acct-2",
    messages: 1_207,
    threads: 588,
    mirrorBytes: 20_971_520,
    bodiesBytes: 41_943_040,
    attachmentsBytes: 88_080_384,
    stateBytes: 262_144,
    oldestMs: at(90, 0, 0),
  },
];

export const devSyncStatus = (): SyncStatus[] =>
  devAccounts.map((account) => ({
    accountId: account.id,
    phase: "idle" as const,
    lastSyncMs: Date.now() - 45_000,
    error: null,
    pendingWrites: 0,
    message: null,
    hydrated: account.id === "acct-1" ? 4_812 : 1_207,
    total: account.id === "acct-1" ? 4_812 : 1_207,
    oldestMs: account.id === "acct-1" ? at(30, 0, 0) : at(90, 0, 0),
  }));

// -------------------------------------------------------------------------------------------
// IMAP and SMTP
// -------------------------------------------------------------------------------------------

/**
 * A provider the fixture knows how to look up, in the terms a published configuration is written
 * in: two hosts, how each socket is protected, and which rung of the ladder answered.
 *
 * The set below is chosen so that every shape the connect screen has to draw is reachable by
 * typing an address: a provider that publishes its own settings, one only the public directory
 * knows, one worked out from the MX record, one nobody publishes at all, a server whose
 * certificate has to be decided about, and Proton, whose published settings point at Bridge on the
 * loopback and therefore fail when Bridge is not running.
 */
interface DevProvider {
  imapHost: string;
  smtpHost: string;
  imapSecurity: Security;
  smtpSecurity: Security;
  /** Only where the provider does not use the port its security comes with, which is Bridge. */
  imapPort?: number;
  smtpPort?: number;
  source: string;
  displayName: string | null;
  /** What the published document names first. Google and Fastmail both say OAuth2. */
  auth?: AuthKind;
}

const DEV_PROVIDERS: Record<string, DevProvider> = {
  "fastmail.example": {
    imapHost: "imap.fastmail.example",
    smtpHost: "smtp.fastmail.example",
    imapSecurity: "tls",
    smtpSecurity: "tls",
    source: "autoconfig",
    displayName: "Fastmail",
  },
  "oakridge-school.example": {
    imapHost: "imap.oakridge-school.example",
    smtpHost: "smtp.oakridge-school.example",
    imapSecurity: "tls",
    smtpSecurity: "start-tls",
    source: "ispdb",
    displayName: "Oakridge School Mail",
  },
  "northwind.example": {
    imapHost: "mail.northwind.example",
    smtpHost: "mail.northwind.example",
    imapSecurity: "tls",
    smtpSecurity: "start-tls",
    source: "mx",
    displayName: null,
  },
  "sunnydaymusic.example": {
    imapHost: "mail.sunnydaymusic.example",
    smtpHost: "mail.sunnydaymusic.example",
    imapSecurity: "tls",
    smtpSecurity: "tls",
    source: "probe",
    displayName: null,
  },
  // A school district on its own domain whose mail is delivered to Google: nothing about the
  // address says so, and the servers the directory hands back are the only way to tell. The
  // display name is what the directory's Google entry actually carries.
  "northgate.example": {
    imapHost: "imap.gmail.com",
    smtpHost: "smtp.gmail.com",
    imapSecurity: "tls",
    smtpSecurity: "tls",
    source: "mx",
    displayName: "Google Mail",
    auth: "o-auth2",
  },
  "proton.me": {
    imapHost: "127.0.0.1",
    smtpHost: "127.0.0.1",
    imapSecurity: "start-tls",
    smtpSecurity: "start-tls",
    imapPort: 1143,
    smtpPort: 1025,
    source: "autoconfig",
    displayName: "Proton Mail Bridge",
  },
};

/**
 * The discovery ladder, as far as a fixture can climb one. An address on a domain nobody publishes
 * settings for comes back with nothing, which is not a failure: it is the manual sheet.
 */
export function devDiscover(email: string): MailConfig | null {
  const address = email.trim();
  const provider = DEV_PROVIDERS[address.slice(address.indexOf("@") + 1).toLowerCase()];
  if (!provider) return null;
  const leg = (which: "imap" | "smtp"): ServerConfig => {
    const security = which === "imap" ? provider.imapSecurity : provider.smtpSecurity;
    const given = which === "imap" ? provider.imapPort : provider.smtpPort;
    return {
      host: which === "imap" ? provider.imapHost : provider.smtpHost,
      port: given ?? defaultPort(which, security),
      security,
      auth: provider.auth ?? "password",
      username: address,
    };
  };
  return {
    imap: leg("imap"),
    smtp: leg("smtp"),
    source: provider.source,
    displayName: provider.displayName,
  };
}

/**
 * The one host in the fixture that presents a certificate nobody vouches for: a small school of
 * music running its own mail server, which is exactly who this question is about. A bridge on the
 * loopback never asks it, because there is nothing between this process and that socket to
 * impersonate anybody.
 */
export function devCert(host: string, port: number): CertQuestion | null {
  if (host !== "mail.sunnydaymusic.example") return null;
  return {
    host,
    port,
    fingerprint:
      "9F:2C:41:8E:07:B3:D5:6A:14:CC:90:2F:E8:57:0B:39:AD:44:71:C2:6E:19:F0:85:3B:D7:62:AA:10:94:5C:E3",
    subject: "CN=mail.sunnydaymusic.example, O=Sunny Day Music",
    issuer: "CN=mail.sunnydaymusic.example, O=Sunny Day Music",
    expiresMs: at(-287, 9, 0),
    reason: "self-signed",
  };
}

// -------------------------------------------------------------------------------------------
// Derived views
// -------------------------------------------------------------------------------------------

const CONSUMER_DOMAINS = [
  "gmail.com",
  "outlook.com",
  "yahoo.com",
  "icloud.com",
  "proton.me",
  "hey.com",
  "example.com",
];

const domainOf = (address: string): string => address.slice(address.indexOf("@") + 1);

const CATEGORIES: Array<[string, RegExp]> = [
  ["images", /^image\//],
  ["pdfs", /^application\/pdf$/],
  ["invites", /^text\/calendar$/],
  ["documents", /wordprocessing|msword|^text\//],
  ["spreadsheets", /spreadsheet|excel|csv/],
];

export function categoryOf(mimeType: string): string {
  for (const [name, pattern] of CATEGORIES) {
    if (pattern.test(mimeType)) return name;
  }
  return "other";
}

/** Every attachment in the mirror as a card. Signature junk, inline images under 10 KB, is out. */
export function devFiles(threads: DevThread[]): FileCard[] {
  const cards: FileCard[] = [];
  for (const thread of threads) {
    for (const message of thread.messages) {
      for (const attachment of message.attachments) {
        if (attachment.inline && attachment.size < 10_240) continue;
        cards.push({
          attachment,
          threadKey: thread.key,
          subject: thread.subject,
          sender: message.from,
          dateMs: message.dateMs,
          category: categoryOf(attachment.mimeType),
        });
      }
    }
  }
  return cards.sort((left, right) => right.dateMs - left.dateMs);
}

/** The sender directory the Contacts place lists, built from the rules the way Rust would. */
export function devContacts(threads: DevThread[], rules: SenderRule[]): ContactCard[] {
  const everyone = new Map<string, Person>();
  for (const thread of threads) {
    for (const message of thread.messages) {
      if (!everyone.has(message.from.address)) everyone.set(message.from.address, message.from);
    }
  }

  return rules
    .filter((rule) => !rule.isDomain)
    .map((rule) => {
      const who = everyone.get(rule.subject) ?? person(null, rule.subject);
      const mine = threads.filter(
        (thread) => thread.accountId === rule.accountId && thread.from.address === who.address,
      );
      const unsubscribe =
        mine.flatMap((thread) => thread.messages).find((message) => message.unsubscribe)
          ?.unsubscribe ?? null;
      return {
        person: who,
        accountId: rule.accountId,
        destination: rule.destination,
        domainRule: false,
        domainRuleAllowed: !CONSUMER_DOMAINS.includes(domainOf(who.address)),
        notify: mine.some((thread) => thread.notify),
        screenedAtMs: rule.decidedAtMs,
        note: who.address === SAM.address ? "Cooper's teacher. Prefers email over calls." : null,
        allowRemoteImages: who.address === HANNAH.address,
        autoTrashDays: null,
        bundle: rule.destination === "paper-trail",
        recentThreads: mine.slice(0, 5),
        files: devFiles(mine).map((card) => card.attachment),
        unsubscribe,
      } satisfies ContactCard;
    });
}

/**
 * The group head a row sits under. The backend decides it so the two sides cannot disagree about
 * where a row belongs, which is why it is a field on the summary rather than a rule in the view.
 */
export function groupOf(thread: DevThread, place: Place): string {
  if (place === "inbox") {
    if (thread.back) return "back";
    return thread.unseen ? "new" : "seen";
  }
  if (place === "paper-trail") {
    return thread.dateMs >= at(7, 0, 0) ? "this-week" : "earlier";
  }
  return "";
}
