import { call, type Account, type ConnectReport, type MailConfig } from "../ipc";

/**
 * Turns an address into a pair of servers, or nothing at all, which is not a failure: it means the
 * settings have to come from the person and the manual sheet is the next screen.
 */
export const imapDiscover = (email: string) => call<MailConfig | null>("imap_discover", { email });

/**
 * Opens both connections and logs in to each without keeping either.
 *
 * Deliberately not a thrown error on refusal. A wrong password, a certificate to decide about and
 * a host that does not answer are three panels the screen draws, so they come back as a report.
 * A blank `smtpPassword` means the IMAP one is used, which is what a gateway that shares
 * credentials with the mail store wants.
 */
export const imapTest = (config: MailConfig, imapPassword: string, smtpPassword: string | null) =>
  call<ConnectReport>("imap_test", { config, imapPassword, smtpPassword });

/** Adds the account. The configuration has already been tested by the screen that calls this. */
export const imapConnect = (
  email: string,
  name: string,
  config: MailConfig,
  imapPassword: string,
  smtpPassword: string | null,
) => call<Account>("imap_connect", { email, name, config, imapPassword, smtpPassword });

/** The servers an account was set up with, for the Settings section that shows them. */
export const imapServers = (accountId: string) =>
  call<MailConfig | null>("imap_servers", { accountId });

/** Accepts one certificate on one host and port, and remembers it across restarts. */
export const imapTrustCert = (host: string, port: number, fingerprint: string) =>
  call<void>("imap_trust_cert", { host, port, fingerprint });

/** Takes that acceptance back, which is the only way out of having trusted the wrong thing. */
export const imapForgetCert = (host: string, port: number) =>
  call<void>("imap_forget_cert", { host, port });
