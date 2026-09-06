import { create } from "zustand";
import { imapConnect, imapDiscover, imapTest, imapTrustCert } from "../api/imap";
import {
  defaultPort,
  type CertQuestion,
  type ConnectReport,
  type MailConfig,
  type Security,
  type ServerConfig,
} from "../ipc";
import { domainOf, routeFor, routeOf, type Route } from "../providers";
import { useAccounts } from "./useAccounts";
import { notify } from "./useToast";

/**
 * Connecting an account: an address, then whatever that address turns out to need.
 *
 * The address is the only question asked before anything is known. From it the flow works out
 * which of three things comes next: a Google sign-in in the browser, a password for servers that
 * discovery found, or a sentence saying the provider cannot be reached with either. There is no
 * fork before that and no provider to pick, because a person knows their address and does not
 * always know who runs the mailbox behind it.
 *
 * Google is not this store's business past recognising it: the screen hands a Google address to
 * `useAccounts`, which owns the browser flow. Everything else is here.
 *
 * `phase` is where the person is and `work` is what is running, because those are two questions. A
 * test in flight does not move anybody to another panel, and the panel stays put while it runs.
 */
type Phase = "off" | "address" | "password" | "manual" | "cert" | "unsupported";

type Work = "idle" | "looking" | "checking" | "adding";

/**
 * Which panel started a test. A refusal and a certificate question both go back to the panel that
 * asked, because that is where the thing to correct is: the password on one, the servers on the
 * other.
 */
type From = "password" | "manual";

/** `skipped` is a person who knows nobody publishes their settings and said so up front. */
type Discovery = "untried" | "empty" | "found" | "skipped";

interface ImapConnectState {
  phase: Phase;
  work: Work;
  /**
   * Whether the ladder has been climbed for this address and what it found. The password panel
   * reads it to know what it is asking for: a password for servers that are known, or a password
   * and then the servers themselves.
   */
  discovery: Discovery;
  name: string;
  email: string;
  password: string;
  /** Blank means the IMAP password is used, which is what most gateways want. */
  smtpPassword: string;
  config: MailConfig | null;
  /** Why the address cannot be opened, when it cannot: the refusal the unsupported panel draws. */
  blocked: "microsoft" | "closed" | null;
  /** The last test. Kept after a failure, because it is what the panel is explaining. */
  report: ConnectReport | null;
  cert: CertQuestion | null;
  from: From;
  /** A command that threw, as opposed to a server that refused. Those are different sentences. */
  error: string | null;

  start: () => void;
  leave: () => void;
  back: () => void;
  setName: (name: string) => void;
  setEmail: (email: string) => void;
  setPassword: (password: string) => void;
  setSmtpPassword: (password: string) => void;
  setServer: (leg: "imap" | "smtp", patch: Partial<ServerConfig>) => void;
  /**
   * The address, and where it leads. Resolves to the route so the screen can hand a Google
   * address to the browser flow, which is the one step this store does not own. Null means the
   * address was not one, and the panel says so.
   */
  lookup: () => Promise<Route | null>;
  /** A lookup that has gone on long enough. Whatever it finds is dropped, and the field comes back. */
  stopLookup: () => void;
  /** Straight to the sign-in step with the servers left to the person, for a domain they know
   *  publishes nothing. Thunderbird offers the same the moment the address is valid, because
   *  waiting out a lookup that was never going to answer is the wait people describe as hanging. */
  skipLookup: () => void;
  /** The password panel's button: test the servers and, if they answer, add the account. */
  submit: () => Promise<void>;
  /** The sheet's own button: test what is in it and, if it works, add the account. */
  apply: () => Promise<void>;
  openManual: () => void;
  trustCert: () => Promise<void>;
}

const LOOPBACK = /^(localhost|127(\.\d{1,3}){3}|\[?::1\]?)$/i;

/** The loopback, where a bridge listens and where a refused connection means it is not running. */
export const isLoopback = (host: string): boolean => LOOPBACK.test(host.trim());

/**
 * The sentence under a failed test, which is the difference between a screen that helps and one
 * that prints a socket error.
 *
 * The report's own advice comes first when there is any, because it is the server's own words that
 * earned it. The second case is Proton and it is worth the special reading: a Proton account's
 * published settings point at Bridge on 127.0.0.1, so nothing answering there is not a network
 * problem, it is a program on this machine that is not running, and "connection refused" would
 * send somebody to look at their router.
 */
export function adviceFor(config: MailConfig | null, report: ConnectReport | null): string | null {
  if (!report || report.ok) return null;
  if (report.advice) return report.advice;
  if (!config || report.cert) return null;
  const server = report.failed === "smtp" ? config.smtp : config.imap;
  if (!isLoopback(server.host)) return null;
  // The kind rather than the sentence. Rust already knows which refusal this was, so matching
  // prose here would only be a way for the two to disagree the next time a server rewords itself.
  if (report.kind !== "unreachable") return null;
  return (
    "Nothing is listening on the loopback, which means Proton Bridge is not running. Start Bridge " +
    "and try again. The password Bridge shows you is the one to use here, not your Proton password."
  );
}

/**
 * The sheet's starting point when nothing was discovered.
 *
 * The two hosts are the names most providers use and are there to be corrected rather than
 * trusted. TLS on 993 and 465 is the guess to make: a provider that only speaks STARTTLS is now
 * the exception, and starting at the encrypted end means a wrong guess fails rather than quietly
 * putting a password on the wire in the clear.
 */
function blankConfig(email: string): MailConfig {
  const domain = domainOf(email);
  const leg = (which: "imap" | "smtp"): ServerConfig => ({
    host: domain ? `${which}.${domain}` : "",
    port: defaultPort(which, "tls"),
    security: "tls",
    auth: "password",
    // The outgoing half starts empty rather than repeating the address, because empty is the
    // sheet's promise that the incoming credentials are used and a gateway wanting nothing at all
    // is a real configuration.
    username: which === "imap" ? email.trim() : "",
  });
  return { imap: leg("imap"), smtp: leg("smtp"), source: "manual", displayName: null };
}

/**
 * The configuration as the backend should receive it. An empty outgoing username means the
 * incoming one, which is the sheet's promise and has to be kept somewhere: the contract carries a
 * username on each half and no way to say "the same as the other one".
 */
function wire(config: MailConfig): MailConfig {
  if (config.smtp.username.trim().length > 0) return config;
  return { ...config, smtp: { ...config.smtp, username: config.imap.username } };
}

/** The ports the two halves come with, which is what tells a typed port from a defaulted one. */
const isDefaultPort = (leg: "imap" | "smtp", port: number): boolean =>
  (["plain", "start-tls", "tls"] as Security[]).some((s) => defaultPort(leg, s) === port);

/** An address has something on both sides of one @, and that is the whole of what is checked here.
 *  The servers are the ones who know whether it is a mailbox. */
const looksLikeAddress = (email: string): boolean => {
  const at = email.indexOf("@");
  return at > 0 && at === email.lastIndexOf("@") && at < email.length - 1 && !/\s/.test(email);
};

/** Everything a run of the flow holds, cleared on the way in and on the way out. */
const CLEAN = {
  work: "idle" as Work,
  discovery: "untried" as Discovery,
  name: "",
  email: "",
  password: "",
  smtpPassword: "",
  config: null,
  blocked: null,
  report: null,
  cert: null,
  from: "password" as From,
  error: null,
};

export const useImapConnect = create<ImapConnectState>((set, get) => {
  /**
   * Which run of the flow a call was made for. Leaving bumps it, and so does stepping back past
   * the panel that asked, because a test can take half a minute and its answer used to arrive
   * after the person had gone: it wrote the panel back over whatever they had gone back to. A
   * result that comes home to a different number is about a question nobody is asking any more.
   */
  let run = 0;
  const stale = (mine: number): boolean => run !== mine;

  const add = async (): Promise<void> => {
    const { email, name, config, password, smtpPassword, from } = get();
    if (!config) return;
    const mine = run;
    set({ work: "adding", error: null });
    try {
      const account = await imapConnect(
        email.trim(),
        name.trim() || email.trim(),
        wire(config),
        password,
        smtpPassword.trim() || null,
      );
      get().leave();
      // The account list is what the window watches: one more account in it and the welcome
      // screen is not the app any more.
      await useAccounts.getState().refresh();
      // The backend starts the first pass the moment the account is written. What waits on it is
      // the arriving panel, over whatever screen this was: it holds the window until the engine
      // says the mail is in, then opens the account's Inbox. Before it, a mailbox that had just
      // been connected was an empty Inbox with a line in the header and a toast saying it was
      // connected, which is not the same thing as having its mail.
      useAccounts.getState().arrive(account.id, account.email);
    } catch (e) {
      if (stale(mine)) return;
      set({ work: "idle", error: String(e), phase: from });
    }
  };

  /**
   * One test, and the four things it comes back as: it worked and the account is added, the
   * certificate has to be decided about, one of the two legs refused, or the call itself fell over.
   * The last two land back on the panel that asked.
   */
  const check = async (from: From): Promise<void> => {
    const { config, password, smtpPassword } = get();
    if (!config) return;
    const mine = run;
    set({ work: "checking", from, error: null });
    try {
      const report = await imapTest(wire(config), password, smtpPassword.trim() || null);
      if (stale(mine)) return;
      set({ report, work: "idle", cert: report.cert });
      if (report.cert) set({ phase: "cert" });
      else if (!report.ok) set({ phase: from });
      else await add();
    } catch (e) {
      if (stale(mine)) return;
      set({ work: "idle", error: String(e), phase: from });
    }
  };

  return {
    phase: "off",
    ...CLEAN,

    start: () => set({ phase: "address", ...CLEAN }),

    // Nothing typed here outlives the flow. The password is the reason: it sits in memory for as
    // long as it takes to try it, and leaving the screen is a decision not to.
    leave: () => {
      run += 1;
      set({ phase: "off", ...CLEAN });
    },

    // Escape retraces the way in rather than throwing the whole flow away, so a certificate goes
    // back to the panel whose test raised it, the servers go back to the password, and the password
    // goes back to the address. The address itself stays: it is what the person came back to fix.
    //
    // A call still out belongs to the panel being left, so the work ends here rather than when it
    // answers, or the panel arrived on would sit with its buttons held until an answer nobody is
    // going to read came home.
    back: () => {
      run += 1;
      const { phase, from } = get();
      if (phase === "cert") set({ phase: from, cert: null, work: "idle" });
      else if (phase === "manual") set({ phase: "password", work: "idle" });
      else if (phase === "password" || phase === "unsupported") {
        set({
          phase: "address",
          work: "idle",
          password: "",
          smtpPassword: "",
          config: null,
          blocked: null,
          report: null,
          cert: null,
          discovery: "untried",
          error: null,
        });
      } else get().leave();
    },

    setName: (name) => set({ name }),
    setEmail: (email) => set({ email, error: null }),
    setPassword: (password) => set({ password }),
    setSmtpPassword: (smtpPassword) => set({ smtpPassword }),

    /**
     * A field of one server, with the one correction the sheet makes on somebody's behalf.
     *
     * Changing the security moves the port with it, because the pairing is fixed and getting it
     * wrong is the single most common way a hand-typed configuration fails. A port that is not one
     * of the three defaults was typed on purpose, though, and Bridge's 1143 is exactly that, so it
     * survives the change rather than being helpfully replaced with 143.
     */
    setServer: (leg, patch) => {
      const config = get().config;
      if (!config) return;
      const before = config[leg];
      const after = { ...before, ...patch };
      if (patch.security && patch.security !== before.security && isDefaultPort(leg, before.port)) {
        after.port = defaultPort(leg, patch.security);
      }
      set({ config: { ...config, [leg]: after, source: "manual" } });
    },

    stopLookup: () => {
      run += 1;
      set({ work: "idle" });
    },

    skipLookup: () => {
      const email = get().email.trim();
      if (!looksLikeAddress(email)) {
        set({ error: "An email address is what this needs." });
        return;
      }
      run += 1;
      set({
        work: "idle",
        error: null,
        report: null,
        cert: null,
        config: blankConfig(email),
        discovery: "skipped",
        phase: "password",
      });
    },

    lookup: async () => {
      const email = get().email.trim();
      if (!looksLikeAddress(email)) {
        set({ error: "An email address is what this needs." });
        return null;
      }

      // A few providers are known from the domain alone, and none is worth a lookup: a gmail.com
      // address is going to the browser whatever the directory says, and an outlook.com or a
      // hey.com one is going nowhere this app can take it.
      const quick = routeFor(email);
      if (quick === "google") return "google";
      if (quick === "microsoft" || quick === "closed") {
        set({ phase: "unsupported", blocked: quick, error: null });
        return quick;
      }

      const mine = run;
      set({ work: "looking", error: null, report: null, cert: null, discovery: "untried" });
      let found: MailConfig | null;
      try {
        found = await imapDiscover(email);
      } catch (e) {
        // The ladder itself fell over, which is not the same as it finding nothing, but the next
        // step is the same: the servers come from the person, and the panel says why.
        if (stale(mine) || get().work !== "looking") return null;
        set({
          work: "idle",
          error: String(e),
          config: blankConfig(email),
          discovery: "empty",
          phase: "password",
        });
        return "password";
      }
      // Stop, Escape, or a different address while the lookup was out: whatever it found is about
      // a question nobody is asking any more.
      if (stale(mine) || get().work !== "looking" || get().email.trim() !== email) return null;

      if (!found) {
        // Not a failure. It means the settings have to come from a person, and the panel says so.
        set({ work: "idle", config: blankConfig(email), discovery: "empty", phase: "password" });
        return "password";
      }
      // A custom domain hosted by one of the two is only recognisable from its servers, which is
      // what the directory is for: a mailbox at work is very often a Google one under another name.
      const route = routeOf(found);
      if (route === "google") {
        set({ work: "idle" });
        return "google";
      }
      if (route === "microsoft") {
        set({ work: "idle", phase: "unsupported", blocked: "microsoft" });
        return "microsoft";
      }
      set({ work: "idle", config: found, discovery: "found", phase: "password" });
      return "password";
    },

    submit: async () => {
      const { discovery, password } = get();
      // With nothing discovered, the password panel's button opens the servers rather than testing
      // hosts that were only ever a guess at the usual names.
      if (discovery === "empty" || discovery === "skipped") {
        get().openManual();
        return;
      }
      if (password.length === 0) {
        set({ error: "The password is what this needs." });
        return;
      }
      await check("password");
    },

    apply: () => check("manual"),

    openManual: () => {
      const { config, email } = get();
      set({ phase: "manual", error: null, config: config ?? blankConfig(email) });
    },

    /**
     * Accepting one certificate on one host and port, and then carrying on with whatever the test
     * was for. Trusting it is only ever half an answer: the connection it was refused on still has
     * to be made.
     */
    trustCert: async () => {
      const cert = get().cert;
      if (!cert) return;
      const mine = run;
      set({ work: "checking", error: null });
      try {
        await imapTrustCert(cert.host, cert.port, cert.fingerprint);
        if (stale(mine)) return;
        set({ cert: null, report: null });
        await check(get().from);
      } catch (e) {
        if (stale(mine)) return;
        set({ work: "idle", error: String(e) });
        notify(`Could not remember that certificate: ${e}`);
      }
    },
  };
});
