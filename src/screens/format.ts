// What the screens print. Dates, sizes, names and the one guess this package has to make.
//
// None of it is state and none of it touches a store, so it is a module of functions rather than a
// hook: the list, the pane and the piles all want the same short time string and none of them
// should own it.

import { keyFor, keyLabel, type CommandId } from "../keys/bindings";
import type { Person } from "../ipc";

const DAY_MS = 86_400_000;

const clock = new Intl.DateTimeFormat(undefined, {
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
});
const weekday = new Intl.DateTimeFormat(undefined, { weekday: "short" });
const dayMonth = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" });

const midnight = (ms: number): number => {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
};

/** Whole days between two instants, counted by calendar day rather than by elapsed hours. */
const daysAgo = (ms: number, now: number): number =>
  Math.round((midnight(now) - midnight(ms)) / DAY_MS);

/**
 * The time on a row: the hour today, the word Yesterday, the weekday inside the week, the date
 * after that. A row has one line for it and no room to say the year.
 */
export function rowTime(ms: number, now = Date.now()): string {
  const days = daysAgo(ms, now);
  if (days <= 0) return clock.format(ms);
  if (days === 1) return "Yesterday";
  if (days < 7) return weekday.format(ms);
  return dayMonth.format(ms);
}

/** The time on a message, which has room to say both the day and the hour. */
export function messageTime(ms: number, now = Date.now()): string {
  const days = daysAgo(ms, now);
  if (days <= 0) return `Today ${clock.format(ms)}`;
  if (days === 1) return `Yesterday ${clock.format(ms)}`;
  if (days < 7) return `${weekday.format(ms)} ${clock.format(ms)}`;
  return dayMonth.format(ms);
}

/** Binary, because that is what a mail client and an operating system both mean by KB here. */
export function fileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** The mark on an attachment chip: the extension, or the subtype when there is no extension. */
export function fileKind(filename: string, mimeType: string): string {
  const dot = filename.lastIndexOf(".");
  const ext = dot > 0 ? filename.slice(dot + 1) : mimeType.slice(mimeType.indexOf("/") + 1);
  return ext.slice(0, 4).toUpperCase();
}

/**
 * The name to print. A header that says "Young, Russell" is a sorting key that escaped from an
 * address book, and reading it back the way it was written is the whole of the fix.
 */
export function displayName(person: Person): string {
  const name = person.name?.trim();
  if (!name) return person.address;
  const comma = name.indexOf(",");
  if (comma > 0 && name.indexOf(",", comma + 1) === -1) {
    const last = name.slice(0, comma).trim();
    const first = name.slice(comma + 1).trim();
    if (first && last) return `${first} ${last}`;
  }
  return name;
}

/** A hue token name (`hue-4`) as the number the primitives take. */
export function hueOf(color: string): number | undefined {
  const n = Number(color.replace(/^hue-/, ""));
  return Number.isFinite(n) && n >= 1 && n <= 8 ? n : undefined;
}

const CONSUMER_DOMAINS = new Set([
  "gmail.com",
  "outlook.com",
  "hotmail.com",
  "yahoo.com",
  "icloud.com",
  "me.com",
  "proton.me",
  "protonmail.com",
  "hey.com",
  "example.com",
]);

const domainOf = (address: string): string => address.slice(address.indexOf("@") + 1).toLowerCase();
const localOf = (address: string): string => address.slice(0, address.indexOf("@")).toLowerCase();

/**
 * Whether to draw a company's bordered mark rather than a person's coloured initials.
 *
 * This is a guess, and it is a guess because nothing on `ThreadSummary` says which senders are
 * companies: the contract carries a `Person` and a `Person` is a name and an address. The rule is
 * that a person's address is built out of their name, which is true of `maya.raghunathan@` and
 * false of `no-reply@` and `customerservice@`, and that a name of more than two words is a company
 * whatever its address says. It gets Northwind Payroll wrong, and a real answer needs a fact from
 * the contact record rather than a better regular expression.
 */
export function isBrand(person: Person): boolean {
  const name = person.name?.trim();
  if (!name) return false;
  if (CONSUMER_DOMAINS.has(domainOf(person.address))) return false;
  const words = displayName(person).split(/\s+/).filter(Boolean);
  if (words.length > 2) return true;
  const local = localOf(person.address).replace(/[^a-z]/g, "");
  // An address that is somebody's initials is still somebody's address: pj@ is a person and dse@
  // is DocuSign, and the only thing telling them apart is that one spells the name and one does not.
  if (local === words.map((w) => w[0].toLowerCase()).join("")) return false;
  return !words.some((word) => {
    const w = word.toLowerCase().replace(/[^a-z]/g, "");
    return w.length > 2 && local.includes(w);
  });
}

/** The rendered text of a sanitised body, for the one line a collapsed message shows. */
export function previewOf(html: string): string {
  // Parsed rather than stripped with a regular expression, so entities come back as characters.
  // A DOMParser document is inert: nothing in it runs and nothing in it is fetched.
  const doc = new DOMParser().parseFromString(html, "text/html");
  return (doc.body.textContent ?? "").replace(/\s+/g, " ").trim();
}

/**
 * How many people the line names before it starts counting them. Four are still printed in full,
 * because "and 1 other" is longer than the name it would stand in for.
 */
const NAMED = 3;

/**
 * The participants line: everyone in the thread, with you last and only when you wrote in it.
 *
 * A receipt from City Power is addressed to you and saying "City Power and you" about it is noise;
 * a thread you answered is a conversation and leaving yourself out of it reads as though somebody
 * else did the answering.
 *
 * Past four it counts instead. A calendar invite to a floor of forty is one of the commonest things
 * in a work mailbox, and forty names is not a line, it is a column: it takes the height of the
 * whole head and pushes the message under the fold. Nobody reads the fortieth name.
 */
export function participantLine(names: string[], includesYou: boolean): string {
  const parts = includesYou ? [...names, "you"] : names;
  if (parts.length === 0) return "";
  if (parts.length === 1) return parts[0];
  if (parts.length > NAMED + 1) {
    return `${parts.slice(0, NAMED).join(", ")} and ${parts.length - NAMED} others`;
  }
  return `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
}

/**
 * The key a button prints. An unmodified letter is printed as the letter, which is how the app
 * writes `r` on Reply; anything with a modifier is printed in real glyphs, which is how it writes
 * ⇧N on Notify. The binding table is the only source for either.
 */
export function cap(command: CommandId): string | undefined {
  const combo = keyFor(command);
  if (!combo) return undefined;
  return combo.includes("+") ? keyLabel(combo) : combo;
}

/**
 * The number in an account's hue token. `Account.color` is a token name (`hue-4`) rather than a hex,
 * because the stylesheet owns the value and the account only owns the choice.
 */
export function accountHue(color: string): number | undefined {
  const match = /^hue-([1-8])$/.exec(color);
  return match ? Number(match[1]) : undefined;
}
