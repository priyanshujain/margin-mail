import type { ReactNode } from "react";
import { Banner, Button, Field, Segment, Sheet, icons } from "../ui";
import { useEscapeLayer } from "../escape";
import type { MailConfig, Security, ServerConfig } from "../ipc";
import { closedName, domainOf, passwordHint, providerName } from "../providers";
import { adviceFor, useImapConnect } from "../store/useImapConnect";
import "./connectmail.css";

/**
 * Adding an account, from the address onwards.
 *
 * One question first, the address, and no provider to pick before it: a person knows their address
 * and does not always know who runs the mailbox behind it, and a screen that opens with a Google
 * button and an "anything else" button underneath has already decided who it was built for. From
 * the address the flow works out which of three things comes next. A Google mailbox, including a
 * work domain that turns out to be one, goes to the browser and this component is done with it. A
 * Microsoft mailbox gets a sentence saying why it cannot be reached. Everything else gets a sign-in
 * panel: the servers discovery found, said out loud before the password is typed into them, and
 * then a name and that password.
 *
 * There is no "pick your provider" step and no fork between an easy path and an expert one, which
 * is where both Thunderbird and Mailspring ended up after years of having one. The servers sheet is
 * the same sheet whether it was opened to correct what was found or to type what was not.
 *
 * The three questions this screen has to answer honestly are all about trust rather than about
 * mail: where these settings came from, what kind of password this provider wants, and whether a
 * certificate nobody vouches for should be accepted. Each of those is said in its place.
 */
export function ConnectMail({ host = "stage", onGoogle, trouble = null }: ConnectMailProps) {
  const phase = useImapConnect((s) => s.phase);
  const back = useImapConnect((s) => s.back);

  // The sheets register their own layers, and in the other host the panel around this one already
  // owns Escape. On the stage the address step is the floor: there is nothing under it to go back
  // to, so Escape there does nothing rather than clearing what was typed.
  useEscapeLayer(host === "stage" && (phase === "password" || phase === "unsupported"), back);

  if (phase === "unsupported") return <Unsupported host={host} />;
  // The servers and the certificate are sheets over this panel, so it stays where it was under
  // them, and Escape from either lands back on it with nothing to redraw.
  if (phase === "password" || phase === "manual" || phase === "cert") {
    return <SignIn host={host} />;
  }
  return <Address host={host} onGoogle={onGoogle} trouble={trouble} />;
}

export interface ConnectMailProps {
  host?: Host;
  /**
   * A Google address, recognised from its domain or from the servers it was found on. The browser
   * flow belongs to `useAccounts`, and which of its two entry points to use is the host's to say.
   */
  onGoogle: (email: string) => void;
  /** Something the host has to say on the address step: the consent URL not being buildable. */
  trouble?: string | null;
}

/**
 * Where the flow is being drawn. The welcome screen gives it the stage; Settings has a stage of its
 * own already, so there it is a sheet over the account list.
 */
export type Host = "stage" | "sheet";

/**
 * What the flow calls itself at each step.
 *
 * On the stage it is a heading and in a sheet it is the panel's head, so it is a function rather
 * than a literal: the words are the flow's either way, and a second copy of them written into
 * Settings is how the two would come to disagree the next time one of them is reworded. The
 * address step is the one place the two hosts differ, because on the stage it is the wordmark.
 */
export function connectMailTitle(
  phase: string,
  config: MailConfig | null,
  email: string,
  blocked: "microsoft" | "closed" | null = null,
): string {
  if (phase === "unsupported") {
    return blocked === "closed" ? `${closedName(email)} has no way in` : "Outlook is not here yet";
  }
  if (phase === "password" || phase === "manual" || phase === "cert") {
    return `Sign in to ${providerName(config, email)}`;
  }
  return "Add an account";
}

/** The step's name, drawn here on the stage and left to the panel's own head in a sheet. */
function Title({ host, children }: { host: Host; children: string }) {
  if (host === "sheet") return null;
  return <h1 className="welcome-title">{children}</h1>;
}

/** The one sentence that names what works, said where somebody is deciding whether to type. */
const WORKS_WITH =
  "Any mailbox works here: Google, Fastmail, iCloud, Yahoo, Proton through Bridge, a mailbox " +
  "where you work, or anything else that speaks IMAP.";

/**
 * The address, which is the whole of the first step.
 *
 * Nothing else is asked yet because nothing else is known yet: a Google account has no password to
 * give and brings its own name, and a mailbox on a password does not need its name until the
 * servers are known. The button says what it is doing while the lookup is out, because a press
 * that changes nothing on screen reads as a press that did nothing.
 */
function Address({ host, onGoogle, trouble }: { host: Host } & ConnectMailProps) {
  const email = useImapConnect((s) => s.email);
  const work = useImapConnect((s) => s.work);
  const error = useImapConnect((s) => s.error);
  const setEmail = useImapConnect((s) => s.setEmail);
  const lookup = useImapConnect((s) => s.lookup);
  const stopLookup = useImapConnect((s) => s.stopLookup);
  const skipLookup = useImapConnect((s) => s.skipLookup);

  const looking = work === "looking";
  const domain = domainOf(email);

  const go = async () => {
    const route = await lookup();
    if (route === "google") onGoogle(email.trim());
  };

  return (
    <>
      <Title host={host}>Margin Mail</Title>
      {host === "stage" ? (
        <p className="welcome-line">
          A quiet, keyboard-first client for your mail, where every decision you make stays on your
          own machine.
        </p>
      ) : null}

      <form
        className="imap-address"
        noValidate
        onSubmit={(e) => {
          e.preventDefault();
          void go();
        }}
      >
        <Field
          label="Email address"
          type="email"
          autoComplete="email"
          value={email}
          onChange={setEmail}
          disabled={looking}
          autoFocus
        />
        <div className="imap-go">
          <Button variant="primary" type="submit" disabled={looking}>
            {looking ? `Looking up ${domain}` : "Continue"}
          </Button>
          {/* A lookup asks four places and then tries the usual names, and a domain that publishes
              nothing makes every one of them wait out its timeout. The person is not held to that. */}
          {looking ? (
            <Button variant="ghost" onClick={stopLookup}>
              Stop
            </Button>
          ) : null}
        </div>
      </form>

      {error ? <p className="welcome-trouble">{error}</p> : null}
      {trouble ? <p className="welcome-trouble">{trouble}</p> : null}

      <p className="welcome-privacy">
        {WORKS_WITH} Margin works out the servers from the address, and nothing is stored until the
        account is added.
      </p>

      {!looking ? (
        <p className="imap-aside">
          <Button variant="ghost" onClick={skipLookup}>
            Enter the servers myself
          </Button>
        </p>
      ) : null}
    </>
  );
}

/**
 * The servers discovery found, and the two things they need: a name and a password.
 *
 * `source` is the whole reason the first sentence exists. A configuration the provider publishes
 * and one guessed by trying the usual names are different promises to somebody about to type a
 * password, and the only honest thing to do is say which of the two this is before they do.
 *
 * The hint under the password is the other honest thing. Half the providers on the internet want a
 * password made for the purpose rather than the one on the website, and every one of them refuses
 * the wrong one in its own vocabulary. Saying which to type, before it is typed, is worth more
 * than any sentence after the refusal.
 */
function SignIn({ host }: { host: Host }) {
  const phase = useImapConnect((s) => s.phase);
  const config = useImapConnect((s) => s.config);
  const email = useImapConnect((s) => s.email);
  const name = useImapConnect((s) => s.name);
  const password = useImapConnect((s) => s.password);
  const work = useImapConnect((s) => s.work);
  const discovery = useImapConnect((s) => s.discovery);
  const report = useImapConnect((s) => s.report);
  const error = useImapConnect((s) => s.error);
  const setName = useImapConnect((s) => s.setName);
  const setPassword = useImapConnect((s) => s.setPassword);
  const submit = useImapConnect((s) => s.submit);
  const openManual = useImapConnect((s) => s.openManual);
  const back = useImapConnect((s) => s.back);

  if (!config) return null;

  const busy = work !== "idle";
  const empty = discovery === "empty" || discovery === "skipped";
  // A refusal is explained on whichever panel is in front. With the servers sheet open over this
  // one it is the sheet's to explain, and the same sentence twice on one screen is not twice as
  // clear.
  const front = phase === "password";
  const refused = front && report !== null && !report.ok && report.cert === null;
  const leg = report?.failed === "smtp" ? config.smtp : config.imap;
  const advice = front ? adviceFor(config, report) : null;

  return (
    <>
      <Title host={host}>{connectMailTitle("password", config, email)}</Title>
      <p className="welcome-line">{provenance(config, email, discovery)}</p>

      {!empty ? (
        <dl className="imap-facts">
          <Fact label="Incoming" value={serverLineAs(config.imap, email)} />
          <Fact label="Outgoing" value={serverLineAs(config.smtp, email)} />
        </dl>
      ) : null}

      <form
        className="imap-form"
        noValidate
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <Field
          label="Your name"
          value={name}
          onChange={setName}
          disabled={busy}
          autoFocus
          hint="What people see beside your address on the mail you send."
        />
        <Field
          type="password"
          label="Password"
          value={password}
          onChange={setPassword}
          disabled={busy}
          hint={passwordHint(config) ?? undefined}
        />

        <div className="imap-actions">
          <Button variant="primary" type="submit" disabled={busy}>
            {empty
              ? "Enter the servers"
              : work === "checking"
                ? "Checking"
                : work === "adding"
                  ? "Adding"
                  : "Add account"}
          </Button>
          {!empty ? (
            <Button variant="ghost" disabled={busy} onClick={openManual}>
              Change the servers
            </Button>
          ) : null}
          <Button variant="ghost" disabled={busy} onClick={back}>
            Back
          </Button>
        </div>
      </form>

      {refused ? (
        <p className="welcome-trouble">
          {report.failed === "smtp"
            ? `Your mail came through. Sending, through ${leg.host}, did not.`
            : `Signing in to ${leg.host} did not work.`}
        </p>
      ) : null}
      {refused && report.message ? <p className="imap-said">{report.message}</p> : null}
      {advice ? <p className="imap-advice">{advice}</p> : null}
      {front && error ? <p className="welcome-trouble">{error}</p> : null}

      <p className="welcome-privacy">
        Nothing is stored until these servers accept the password. Then it is encrypted on this
        device beside the keys the app already holds, and it goes to them and to nobody else.
      </p>
    </>
  );
}

/**
 * A mailbox this app cannot open, said plainly rather than with a button that fails.
 *
 * Two kinds. Microsoft turned password sign-in off for its mailboxes, personal and business both,
 * app passwords included, and the sign-in it wants instead is an OAuth client registered with
 * Microsoft, which Margin does not have. HEY and Tuta have no IMAP at all, by their own account,
 * so no mail client can open them. A panel that took a password for either and reported a refusal
 * would be a panel that knew better.
 */
function Unsupported({ host }: { host: Host }) {
  const email = useImapConnect((s) => s.email);
  const blocked = useImapConnect((s) => s.blocked);
  const back = useImapConnect((s) => s.back);
  const domain = domainOf(email) || "This address";
  const closed = blocked === "closed";
  const brand = closedName(email);

  return (
    <>
      <Title host={host}>{connectMailTitle("unsupported", null, email, blocked)}</Title>
      <p className="welcome-line">
        {closed
          ? `${brand} does not let any mail client sign in: there is no IMAP, no POP and no other ` +
            `way in, so this mailbox can only be read in ${brand}'s own app. Nothing was stored.`
          : `${domain} is a Microsoft mailbox. Microsoft turned off password sign-in to Outlook, ` +
            "Hotmail and Microsoft 365 in 2024, app passwords included, and the sign-in it wants " +
            "instead needs an app registration with Microsoft that Margin does not have yet. " +
            "Nothing was stored."}
      </p>

      <div className="imap-actions">
        <Button variant="primary" onClick={back}>
          Try another address
        </Button>
      </div>

      <p className="welcome-privacy">{WORKS_WITH}</p>
    </>
  );
}

/**
 * The servers, in two columns, and the certificate question when there is one.
 *
 * One sheet rather than two, because the certificate is a thing one of these servers presented and
 * going back from it means going back to whoever asked. Everything the sheet says about why it is
 * open comes from the report: which leg refused, what that server said, and what to do about it.
 */
export function ConnectMailServers() {
  const phase = useImapConnect((s) => s.phase);
  const config = useImapConnect((s) => s.config);
  const report = useImapConnect((s) => s.report);
  const cert = useImapConnect((s) => s.cert);
  const email = useImapConnect((s) => s.email);
  const error = useImapConnect((s) => s.error);
  const work = useImapConnect((s) => s.work);
  const discovery = useImapConnect((s) => s.discovery);
  const password = useImapConnect((s) => s.password);
  const setPassword = useImapConnect((s) => s.setPassword);
  const smtpPassword = useImapConnect((s) => s.smtpPassword);
  const setSmtpPassword = useImapConnect((s) => s.setSmtpPassword);
  const setServer = useImapConnect((s) => s.setServer);
  const apply = useImapConnect((s) => s.apply);
  const trustCert = useImapConnect((s) => s.trustCert);
  const back = useImapConnect((s) => s.back);

  const open = phase === "manual" || phase === "cert";
  if (!open || !config) return null;

  if (phase === "cert" && cert) {
    return (
      <Sheet
        open
        title="The server's certificate"
        onClose={back}
        foot={
          <>
            <Button onClick={back}>Go back</Button>
            <Button variant="danger" disabled={work !== "idle"} onClick={() => void trustCert()}>
              Trust this certificate
            </Button>
          </>
        }
      >
        <p className="imap-lead">
          {cert.host} presented a certificate, and {wrongWith(cert.reason, cert.expiresMs)}
        </p>

        <dl className="imap-facts">
          <Fact label="Fingerprint" value={cert.fingerprint} wrap />
          <Fact label="Subject" value={cert.subject} />
          <Fact label="Issuer" value={cert.issuer} />
          <Fact label="Expires" value={certDay.format(cert.expiresMs)} />
        </dl>

        <p className="imap-note">
          Trusting it means Margin accepts this exact certificate on {cert.host}, port {cert.port},
          from now on, and nothing else. If the server is yours, the fingerprint above is the one it
          prints, and comparing them is the whole check. If it is not yours, an unexpected
          certificate is what somebody standing in the middle looks like, and going back costs
          nothing. A bridge running on this machine never raises this question, so this is a server
          out on the network.
        </p>
      </Sheet>
    );
  }

  const advice = adviceFor(config, report);
  const leg = report?.failed === "smtp" ? config.smtp : config.imap;

  return (
    <Sheet
      open
      title="Servers"
      size="wide"
      onClose={back}
      foot={
        <>
          <Button onClick={back}>Back</Button>
          <Button variant="primary" disabled={work !== "idle"} onClick={() => void apply()}>
            {work === "idle" ? "Test and add" : work === "checking" ? "Testing" : "Adding"}
          </Button>
        </>
      }
    >
      {report && !report.ok ? (
        <Banner icon={icons.SPAM}>
          {report.failed === "smtp"
            ? `Your mail came through. Sending, through ${leg.host}, did not.`
            : `Signing in to ${leg.host} did not work.`}
        </Banner>
      ) : discovery === "empty" ? (
        <Banner icon={icons.SPAM}>
          Nobody publishes settings for {domainOf(email) || "this address"}, so they have to come
          from you. Your provider calls them IMAP and SMTP settings.
        </Banner>
      ) : null}

      {report?.message && !report.ok ? <p className="imap-said">{report.message}</p> : null}
      {advice ? <p className="imap-advice">{advice}</p> : null}
      {error ? <p className="imap-said">{error}</p> : null}

      <form
        className="imap-legs"
        noValidate
        onSubmit={(e) => {
          e.preventDefault();
          void apply();
        }}
      >
        <Leg
          leg="imap"
          title="Incoming mail (IMAP)"
          server={config.imap}
          onChange={(patch) => setServer("imap", patch)}
          password={{ value: password, onChange: setPassword }}
          autoFocus
        />
        <Leg
          leg="smtp"
          title="Outgoing mail (SMTP)"
          server={config.smtp}
          onChange={(patch) => setServer("smtp", patch)}
          password={{ value: smtpPassword, onChange: setSmtpPassword }}
        />
        {/* The sheet's own button sits in the foot, outside this form. This is what Enter presses,
            so the flow can be finished without reaching for the mouse. */}
        <button type="submit" className="imap-enter" tabIndex={-1} aria-hidden="true" />
      </form>

      <p className="imap-note">
        Leave the outgoing username and password empty and the incoming ones are used. Some
        gateways want no credentials at all, and this is where that is said.
      </p>
    </Sheet>
  );
}

const SECURITIES = [
  { id: "plain", label: "None" },
  { id: "start-tls", label: "STARTTLS" },
  { id: "tls", label: "TLS" },
];

interface LegProps {
  leg: "imap" | "smtp";
  title: string;
  server: ServerConfig;
  onChange: (patch: Partial<ServerConfig>) => void;
  /** Only the outgoing half has a password of its own, and only because it is optional. */
  password?: { value: string; onChange: (value: string) => void };
  autoFocus?: boolean;
}

/** One half of the account: a host, a port, how the socket is protected, and who logs in. */
function Leg({ leg, title, server, onChange, password, autoFocus }: LegProps) {
  return (
    <section className="imap-leg">
      <h3 className="imap-leg-title">{title}</h3>

      <Field
        label="Server"
        value={server.host}
        onChange={(host) => onChange({ host })}
        autoFocus={autoFocus}
      />

      <Field
        label="Port"
        value={server.port === 0 ? "" : String(server.port)}
        onChange={(value) => onChange({ port: Number(value.replace(/\D/g, "").slice(0, 5)) })}
      />

      <div className="field">
        <span className="field-label">Security</span>
        <Segment
          options={SECURITIES}
          value={server.security}
          label={`${title} security`}
          onChange={(id) => onChange({ security: id as Security })}
        />
        {server.security === "plain" ? (
          <span className="field-hint" data-tone="error">
            Nothing on this connection is encrypted, including the password.
          </span>
        ) : null}
      </div>

      <Field
        label="Username"
        value={server.username}
        onChange={(username) => onChange({ username })}
        placeholder={leg === "smtp" ? "Same as incoming" : undefined}
      />

      {password ? (
        <Field
          type="password"
          label="Password"
          value={password.value}
          onChange={password.onChange}
          placeholder="Same as incoming"
        />
      ) : null}
    </section>
  );
}

/** A name and a value, for the four facts on a certificate and the two on a configuration. */
function Fact({ label, value, wrap }: { label: string; value: ReactNode; wrap?: boolean }) {
  return (
    <div className="imap-fact">
      <dt>{label}</dt>
      <dd data-wrap={wrap ? "" : undefined}>{value}</dd>
    </div>
  );
}

/** A certificate's expiry is the one date in this app that needs its year, because it is a fact
 *  about a document rather than about a message and "18 Jun" cannot be checked against anything. */
const certDay = new Intl.DateTimeFormat(undefined, {
  day: "numeric",
  month: "short",
  year: "numeric",
});

const SECURITY_WORDS: Record<Security, string> = {
  plain: "no encryption",
  "start-tls": "STARTTLS",
  tls: "TLS",
};

/** One server on one line: where it is, which port, and how it is protected. */
export function serverLine(server: ServerConfig): string {
  return `${server.host}, port ${server.port}, ${SECURITY_WORDS[server.security]}`;
}

/**
 * The same line with the login on the end of it, for the panel that has nowhere else to put one.
 * Only when it is not the address, because repeating the address back is not a fact about a server.
 */
function serverLineAs(server: ServerConfig, email: string): string {
  const line = serverLine(server);
  return server.username && server.username !== email.trim()
    ? `${line}, as ${server.username}`
    : line;
}

/**
 * Where the settings came from, said before the password is typed into them, which is a different
 * promise in each of the six cases.
 */
function provenance(config: MailConfig, email: string, discovery: string): string {
  const domain = domainOf(email) || "this address";
  const name = providerName(config, email);
  if (discovery === "skipped") {
    return (
      "The servers are yours to type, with the usual names filled in as a start. Your provider " +
      "calls them IMAP and SMTP settings."
    );
  }
  if (discovery === "empty") {
    return (
      `Nobody publishes settings for ${domain} and no server answered at the usual names, so they ` +
      "have to come from you. Your provider calls them IMAP and SMTP settings."
    );
  }
  switch (config.source) {
    case "autoconfig":
      return `${name} publishes its own settings, so the servers are already known.`;
    case "ispdb":
      return (
        `${name}'s settings are in the public directory of mail providers that Thunderbird keeps, ` +
        "so the servers are already known."
      );
    case "mx":
      return name === domain
        ? `The servers were worked out from where ${domain}'s mail is delivered.`
        : `${domain}'s mail is delivered to ${name}, which publishes its settings, so the servers are already known.`;
    case "probe":
      return (
        `Nobody publishes settings for ${domain}, so Margin tried the usual server names and ` +
        `${config.imap.host} answered. That is a guess rather than a fact: check the servers ` +
        "below before you type a password into them."
      );
    default:
      return "These are the servers you typed.";
  }
}

/** What is wrong with a certificate, in the words the reason is worth translating into. */
function wrongWith(reason: string, expiresMs: number): string {
  if (reason === "expired") {
    return `it expired on ${certDay.format(expiresMs)}, so nothing vouches for it any more.`;
  }
  if (reason === "unknown-issuer") {
    return "it was issued by somebody this machine has never heard of, so nothing independent vouches for it.";
  }
  if (reason === "self-signed") {
    return "it signed its own certificate, so the only thing saying this server is who it claims to be is the server.";
  }
  return "nothing independent vouches for it.";
}

export default ConnectMail;
