// Where an address goes, decided twice: from the domain, and from the servers discovery found.
//
// These are the decisions with a right answer. A gmail.com address that ended up on a password
// panel would be a screen that fails on purpose, and a work domain hosted by Google that was not
// recognised would be the same failure one lookup later.

import { describe, expect, it } from "vitest";
import type { MailConfig } from "./ipc";
import { closedName, domainOf, passwordHint, providerName, routeFor, routeOf } from "./providers";

function found(imapHost: string, displayName: string | null = null): MailConfig {
  const leg = (host: string) => ({
    host,
    port: 993,
    security: "tls" as const,
    auth: "password" as const,
    username: "pj@example.test",
  });
  return { imap: leg(imapHost), smtp: leg(imapHost), source: "ispdb", displayName };
}

describe("routeFor", () => {
  it("sends Google's own domains to the browser without a lookup", () => {
    expect(routeFor("pj@gmail.com")).toBe("google");
    expect(routeFor("PJ@GoogleMail.com")).toBe("google");
  });

  it("recognises Microsoft's consumer domains under any country's ending", () => {
    for (const address of [
      "pj@outlook.com",
      "pj@hotmail.com",
      "pj@live.com",
      "pj@msn.com",
      "pj@outlook.co.uk",
      "pj@hotmail.fr",
      "pj@live.com.au",
    ]) {
      expect(routeFor(address), address).toBe("microsoft");
    }
  });

  it("knows the providers that have no IMAP at all, by name", () => {
    expect(routeFor("pj@hey.com")).toBe("closed");
    expect(routeFor("pj@tutanota.com")).toBe("closed");
    expect(closedName("pj@hey.com")).toBe("HEY");
    expect(closedName("pj@tuta.io")).toBe("Tuta");
  });

  it("looks everything else up, including domains that merely contain a provider's name", () => {
    expect(routeFor("pj@fastmail.com")).toBe("lookup");
    expect(routeFor("pj@northgate.example")).toBe("lookup");
    expect(routeFor("pj@notgmail.com")).toBe("lookup");
    expect(routeFor("pj@outlook.example.org")).toBe("lookup");
  });
});

describe("routeOf", () => {
  it("knows a custom domain by the servers it was found on", () => {
    expect(routeOf(found("imap.gmail.com"))).toBe("google");
    expect(routeOf(found("imap.googlemail.com"))).toBe("google");
    expect(routeOf(found("outlook.office365.com"))).toBe("microsoft");
    expect(routeOf(found("imap-mail.outlook.com"))).toBe("microsoft");
  });

  it("leaves everyone else on the password", () => {
    expect(routeOf(found("imap.fastmail.com"))).toBe("password");
    expect(routeOf(found("127.0.0.1"))).toBe("password");
    expect(routeOf(found("mail.gmail.example"))).toBe("password");
  });
});

describe("providerName", () => {
  it("is what the provider calls itself, or the domain when it said nothing", () => {
    expect(providerName(found("imap.fastmail.com", "Fastmail"), "pj@northgate.example")).toBe(
      "Fastmail",
    );
    expect(providerName(found("mail.northgate.example"), "pj@northgate.example")).toBe(
      "northgate.example",
    );
    expect(providerName(null, "nonsense")).toBe("this address");
  });
});

describe("passwordHint", () => {
  it("names the kind of password a provider wants before it is typed", () => {
    expect(passwordHint(found("imap.fastmail.com"))).toContain("app password");
    expect(passwordHint(found("imap.mail.me.com"))).toContain("app-specific password");
    expect(passwordHint(found("imap.mail.yahoo.com"))).toContain("Yahoo");
    expect(passwordHint(found("imap.aol.com"))).toContain("AOL");
  });

  it("says a loopback is a bridge with a password of its own", () => {
    expect(passwordHint(found("127.0.0.1"))).toContain("Bridge shows under Mailbox details");
  });

  it("has nothing to add for a server it knows nothing about", () => {
    expect(passwordHint(found("mail.northgate.example"))).toBeNull();
    expect(passwordHint(null)).toBeNull();
  });
});

describe("domainOf", () => {
  it("is the lowercased half after the last @", () => {
    expect(domainOf("PJ@Example.COM ")).toBe("example.com");
    expect(domainOf("nonsense")).toBe("");
  });
});
