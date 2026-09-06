// Which way in an address takes, and what the provider behind it is going to want.
//
// The connect screen asks for one thing, the address, and everything after it is worked out. Two
// providers cannot be reached with a password at all: Google, which this app talks to through its
// API after a sign-in in the browser, and Microsoft, which closed password sign-in and wants an app
// registration Margin does not have. Both are recognised twice. From the domain, when it is one of
// theirs, so a gmail.com address never waits on a lookup. And from the servers discovery hands
// back, when it is a custom domain they host, which is what a mailbox at work usually turns out
// to be.
//
// The hints are the other half. The single most common way an IMAP setup fails is a person typing
// the password they sign in to the website with into a provider that wants a password made for
// the purpose, and every one of those providers refuses in its own vocabulary. Saying which kind
// of password to type, before it is typed, is worth more than any sentence after the refusal.

import type { MailConfig } from "./ipc";

export type Route = "google" | "microsoft" | "closed" | "password";

/** The domain half of an address, lowercased. Empty when there is no @ to split on. */
export const domainOf = (email: string): string => {
  const at = email.lastIndexOf("@");
  return at < 0 ? "" : email.slice(at + 1).trim().toLowerCase();
};

const GOOGLE_DOMAINS = new Set(["gmail.com", "googlemail.com"]);

/** outlook.com, hotmail.com and live.com under any country's ending (hotmail.co.uk, live.com.au,
 *  outlook.de), and msn.com. The middle part is a country's own second level and nothing longer,
 *  so a domain that merely starts with one of the names is not one of theirs. */
const MICROSOFT_DOMAIN = /^(?:outlook|hotmail|live)\.(?:(?:co|com|ne)\.)?[a-z]{2,3}$|^msn\.com$/;

/**
 * The providers with no IMAP, no POP and no other way in for a mail client, by their own account.
 * HEY says so in its help pages and Tuta in its FAQ. Discovery would climb every rung for these and
 * land on the servers sheet, which is the one screen that cannot help.
 */
const CLOSED: Record<string, string> = {
  "hey.com": "HEY",
  "tuta.com": "Tuta",
  "tutanota.com": "Tuta",
  "tutamail.com": "Tuta",
  "tuta.io": "Tuta",
  "keemail.me": "Tuta",
};

/** From the address alone: the providers whose consumer domains say everything already. */
export function routeFor(email: string): Route | "lookup" {
  const domain = domainOf(email);
  if (GOOGLE_DOMAINS.has(domain)) return "google";
  if (MICROSOFT_DOMAIN.test(domain)) return "microsoft";
  if (domain in CLOSED) return "closed";
  return "lookup";
}

/** The name of a provider that keeps its mail to its own app, for the sentence that says so. */
export const closedName = (email: string): string => CLOSED[domainOf(email)] ?? domainOf(email);

const GOOGLE_HOST = /(?:^|\.)(?:gmail|googlemail|google)\.com$/;
const MICROSOFT_HOST = /(?:^|\.)(?:office365|outlook|hotmail|live)\.com$/;

/** From what discovery found: a custom domain is whoever its incoming server belongs to. */
export function routeOf(config: MailConfig): Route {
  const host = config.imap.host.trim().toLowerCase();
  if (GOOGLE_HOST.test(host)) return "google";
  if (MICROSOFT_HOST.test(host)) return "microsoft";
  return "password";
}

const LOOPBACK = /^(localhost|127(\.\d{1,3}){3}|\[?::1\]?)$/i;

/** What the provider calls itself, or the domain when it did not say. */
export function providerName(config: MailConfig | null, email: string): string {
  const said = config?.displayName?.trim();
  return said && said.length > 0 ? said : domainOf(email) || "this address";
}

interface Hint {
  /** Matched against the incoming host, lowercased. */
  host: RegExp;
  /** What to type, and where it comes from, in one or two sentences. */
  says: string;
}

/**
 * The providers known to want a password made for the purpose, keyed on the server they publish.
 *
 * The patterns match the provider's own domain under any ending rather than one exact host, so a
 * regional host or a renamed one still gets its sentence. The sentences name the place in the
 * provider's settings where the password is made, in the provider's own words, because "an app
 * password" on its own sends a person searching.
 */
const HINTS: Hint[] = [
  {
    host: /(?:^|\.)fastmail\.[a-z.]+$/,
    says:
      "Fastmail only accepts an app password here, never the one you sign in with. Make one in " +
      "Fastmail's settings under Privacy & Security, then Connected apps & API tokens.",
  },
  {
    host: /(?:^|\.)(?:mail\.me|icloud)\.com$/,
    says:
      "iCloud only accepts an app-specific password here, never your Apple Account password, and " +
      "needs two-factor authentication turned on. Make one at account.apple.com under Sign-In " +
      "and Security.",
  },
  {
    host: /(?:^|\.)yahoo\.[a-z.]+$/,
    says:
      "Yahoo only accepts an app password here, never the one you sign in with. Make one on the " +
      "Account Security page of your Yahoo account, under Generate app password.",
  },
  {
    host: /(?:^|\.)aol\.com$/,
    says:
      "AOL only accepts an app password here, never the one you sign in with. Make one on the " +
      "Account Security page of your AOL account, under Generate app password.",
  },
  {
    host: /(?:^|\.)zoho(?:mail)?\.[a-z.]+$/,
    says:
      "With multi-factor sign-in on, Zoho wants an app-specific password rather than the one you " +
      "sign in with, made under Security in Zoho Accounts. IMAP also has to be turned on under " +
      "Settings, then Mail Accounts.",
  },
  {
    host: /(?:^|\.)(?:gmx\.[a-z.]+|mail\.com)$/,
    says:
      "GMX and mail.com keep IMAP switched off until you turn it on under Settings, then POP3 & " +
      "IMAP, in the web mail. The password is the one you sign in with, unless two-factor is on, " +
      "when it is an app-specific password from Security Options.",
  },
  {
    host: /(?:^|\.)(?:yandex\.[a-z.]+|ya\.ru)$/,
    says:
      "Yandex wants an app password rather than the one you sign in with, made in Yandex ID under " +
      "Security, then App passwords. IMAP has to be turned on under Settings, then Email clients.",
  },
  {
    host: /(?:^|\.)mailbox\.org$/,
    says:
      "mailbox.org takes the password you sign in with, unless two-factor is on, when it wants an " +
      "app password made under Security, then Email app-passwords.",
  },
];

/**
 * The sentence under the password field, when there is something worth saying before it is typed.
 * A bridge on this machine is the one case that is about where the servers are rather than who
 * runs them: Proton's published settings point at Bridge, and Bridge has a password of its own.
 */
export function passwordHint(config: MailConfig | null): string | null {
  if (!config) return null;
  const host = config.imap.host.trim().toLowerCase();
  if (LOOPBACK.test(host)) {
    return (
      "These servers are Proton Bridge running on this machine, or another bridge like it. The " +
      "password is the one Bridge shows under Mailbox details, not your Proton password, and " +
      "Bridge has to be running."
    );
  }
  return HINTS.find((hint) => hint.host.test(host))?.says ?? null;
}
