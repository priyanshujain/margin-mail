import { useEffect, useMemo, useState, type ReactNode } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import {
  BUNDLED_FONTS,
  decodeRef,
  encodeRef,
  fontLabel,
  refsEqual,
} from "margin-shared/fonts";
import { Avatar, Button, Confirm, Icon, icons, NO_AUTOFILL, Segment, Sheet, Toggle } from "../ui";
import type { SegmentOption } from "../ui";
import { accountRemove, accountSetColor, accountSetName } from "../api/accounts";
import { backupConfigure, backupNow, backupPhrase, backupRestore } from "../api/backup";
import { contactUpdate, contactsList } from "../api/contacts";
import { imapServers } from "../api/imap";
import {
  askForNotifications,
  notifyPermission,
  notifyTest,
  openNotificationSettings,
} from "../api/notifications";
import {
  exportMbox,
  exportState,
  importState,
  keymapPath,
  keymapReset,
  mirrorClear,
  packagedBy,
} from "../api/settings";
import { threadsList } from "../api/threads";
import { applyFonts, applyTextSize, forgetThemeChoice, systemTheme } from "../appearance";
import { useEscapeLayer } from "../escape";
import { registerCommands } from "../keys/commands";
import {
  CALENDAR_SCOPE,
  DRIVE_SCOPE,
  REQUIRED_SCOPE,
  SCOPES,
  isDesktop,
  isTauri,
  type Account,
  type BackupSettings,
  type ContactCard,
  type FontRef,
  type MailConfig,
  type NotifyPermission,
  type Place,
  type Settings as SettingsDto,
  type ThreadSummary,
  type SyncStatus,
} from "../ipc";
import { useAccounts } from "../store/useAccounts";
import { useImapConnect } from "../store/useImapConnect";
import { useMail } from "../store/useMail";
import { useSettings } from "../store/useSettings";
import { useSync } from "../store/useSync";
import { useTheme } from "../store/useTheme";
import { notify } from "../store/useToast";
import { version as builtVersion } from "../../package.json";
import {
  ConnectMail,
  ConnectMailServers,
  connectMailTitle,
  serverLine,
} from "./ConnectMail";
import { accountHue, displayName, messageTime } from "./format";
import { NOTIFY_PLACES, withPlace } from "./notifyPlaces";
import "./settings.css";

/**
 * Settings is a place, not a panel: the whole stage, a rail of section names down the left and one
 * section on the right. Twelve sections is too much for an overlay that steals the window and too
 * much for a palette that shows one row at a time, and both shapes make somebody looking for the
 * storage window read the list twice.
 *
 * Every section here is real controls over real commands. Where docs/settings.md asks for
 * something no command answers, the screen says what it can stand behind and offers no button,
 * because a control that does nothing is worse than a sentence admitting the gap.
 */
const SECTIONS = [
  "Accounts",
  "Appearance",
  "Mail",
  "Privacy",
  "Screener",
  "Piles and snooze",
  "Writing",
  "Notifications",
  "Keyboard",
  "Backup",
  "Data",
  "About",
] as const;

type Section = (typeof SECTIONS)[number];

/**
 * The section on the right. Function declarations, so the map may sit beside the list it answers
 * rather than at the foot of the file behind everything it names.
 */
const VIEWS: Record<Section, () => ReactNode> = {
  Accounts: AccountsSection,
  Appearance: AppearanceSection,
  Mail: MailSection,
  Privacy: PrivacySection,
  Screener: ScreenerSection,
  "Piles and snooze": PilesSection,
  Writing: WritingSection,
  Notifications: NotificationsSection,
  Keyboard: KeyboardSection,
  Backup: BackupSection,
  Data: DataSection,
  About: AboutSection,
};


// -------------------------------------------------------------------------------------------
// Permissions
// -------------------------------------------------------------------------------------------

interface Permission {
  scope: string;
  /** What it lets the app do, in the words a person would use. Never the name of the scope. */
  label: string;
  /** What is lost without it, which is the only useful thing to say about a missing one. */
  cost: string;
}

/**
 * The six capabilities, in the order the consent screen lists them.
 *
 * `openid` and `email` are not here. They say who you are rather than what the app may do, and a
 * line reading "Know who you are: Granted" beside five real permissions is noise.
 */
const PERMISSIONS: Permission[] = [
  {
    scope: REQUIRED_SCOPE,
    label: "Read and change your mail",
    cost: "Without this there is no account: reading, archiving and sending are all this one permission.",
  },
  {
    scope: "https://www.googleapis.com/auth/gmail.settings.basic",
    label: "Read your signature and aliases",
    cost: "Signatures and verified aliases are not connected, so replies go out from the main address with nothing after them.",
  },
  {
    scope: "https://www.googleapis.com/auth/contacts.readonly",
    label: "Read your contacts",
    cost: "Contacts are not connected, so addressing a message only autocompletes people you have already written to here.",
  },
  {
    scope: "https://www.googleapis.com/auth/contacts.other.readonly",
    label: "Read the people you have written to",
    cost: "The Screener has less to go on when it screens people in, so more first messages wait than need to.",
  },
  {
    scope: DRIVE_SCOPE,
    label: "Keep a backup in your Drive",
    cost: "Backup to Drive is not connected, so your decisions live only on this machine.",
  },
  {
    scope: CALENDAR_SCOPE,
    label: "Answer calendar invitations",
    cost: "Calendar is not connected, so an invitation renders but Accept, Maybe and Decline do nothing. Granting reopens the Google consent page.",
  },
];

/**
 * Google normalises the two short scopes into their long forms in what it grants back, so a grant
 * is compared on the canonical spelling rather than on the string that was asked for.
 */
const CANONICAL: Record<string, string> = {
  email: "https://www.googleapis.com/auth/userinfo.email",
  profile: "https://www.googleapis.com/auth/userinfo.profile",
};

const canonical = (scope: string): string => CANONICAL[scope] ?? scope;

const holds = (granted: string[], scope: string): boolean =>
  granted.some((one) => canonical(one) === canonical(scope));

/** What a scope is called in plain English, for the refused screen as well as this one. */
export function permissionName(scope: string): string {
  return PERMISSIONS.find((p) => p.scope === scope)?.label ?? scope;
}

const BASE: readonly string[] = SCOPES;

/**
 * The extras a re-consent has to carry.
 *
 * Google has no incremental authorization for installed apps, so granting one permission re-runs
 * the whole consent and replaces the token. Asking only for the missing one would hand back a grant
 * without the optional scopes this account already had, which is a silent revocation.
 */
function extrasFor(account: Account, wanted: string): string[] {
  const optional = [DRIVE_SCOPE, CALENDAR_SCOPE];
  const keep = optional.filter((scope) => holds(account.grantedScopes, scope));
  const extras = new Set([...keep, ...(BASE.includes(wanted) ? [] : [wanted])]);
  return [...extras];
}

// -------------------------------------------------------------------------------------------
// The place
// -------------------------------------------------------------------------------------------

export function Settings() {
  const close = useSettings((s) => s.close);
  const load = useSettings((s) => s.load);
  const linked = useAccounts((s) => s.accounts.length);
  const [section, setSection] = useState<Section>("Accounts");
  const View = VIEWS[section];

  useEscapeLayer(true, close);

  // Again when an account is added or removed, because the settings file grows and loses a row with
  // the account list and nothing else would tell this screen that its copy is a row short.
  useEffect(() => {
    void load();
  }, [load, linked]);

  // The rail is the list while settings has the stage, so it answers to the same two keys. The
  // list column is not mounted, so its handlers are not in the way.
  useEffect(() => {
    const step = (delta: number) =>
      setSection((was) => {
        const at = SECTIONS.indexOf(was) + delta;
        return SECTIONS[Math.min(Math.max(at, 0), SECTIONS.length - 1)];
      });
    return registerCommands({
      "select-next": () => step(1),
      "select-prev": () => step(-1),
    });
  }, []);

  return (
    <>
      <main className="stage">
        <section className="settings">
          <nav className="settings-rail" aria-label="Settings">
            <h1>Settings</h1>
            {SECTIONS.map((name) => (
              <button
                key={name}
                type="button"
                className="settings-tab"
                data-active={name === section ? "" : undefined}
                aria-current={name === section ? "page" : undefined}
                onClick={() => setSection(name)}
              >
                {name}
              </button>
            ))}
          </nav>

          <div className="settings-panel">
            <div className="settings-inner">
              <View />
            </div>
          </div>
        </section>
      </main>

      {/* The servers and the certificate question take the window rather than a place in the
          section that opened them, so they hang off the stage the way they hang off the welcome
          screen. The panel above is its own scroller, and an overlay mounted inside a thing that
          scrolls is an overlay that can be scrolled away from. */}
      <ConnectMailServers />
    </>
  );
}

// -------------------------------------------------------------------------------------------
// Accounts
// -------------------------------------------------------------------------------------------

const COUNTS = ["No", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight"];

const HUES = [1, 2, 3, 4, 5, 6, 7, 8];

function AccountsSection() {
  const accounts = useAccounts((s) => s.accounts);
  const phase = useAccounts((s) => s.phase);
  const authUrl = useAccounts((s) => s.authUrl);
  const addAccount = useAccounts((s) => s.addAccount);
  const grant = useAccounts((s) => s.grant);
  const openAuthUrl = useAccounts((s) => s.openAuthUrl);
  const copyAuthUrl = useAccounts((s) => s.copyAuthUrl);
  const cancelConnect = useAccounts((s) => s.cancelConnect);
  const mailPhase = useImapConnect((s) => s.phase);
  const mailConfig = useImapConnect((s) => s.config);
  const mailEmail = useImapConnect((s) => s.email);
  const mailBlocked = useImapConnect((s) => s.blocked);
  const startMail = useImapConnect((s) => s.start);
  const backMail = useImapConnect((s) => s.back);
  const leaveMail = useImapConnect((s) => s.leave);
  const settings = useSettings((s) => s.settings);

  const [removing, setRemoving] = useState(false);
  const waiting = phase === "connecting";

  const rowFor = (accountId: string) =>
    settings?.accounts.find((row) => row.accountId === accountId) ?? null;

  // The same field as the one in Writing, because this is where somebody looking for it looks.
  const setSignature = (accountId: string, signature: string) => {
    if (!settings) return;
    void useSettings.getState().save({
      accounts: settings.accounts.map((row) =>
        row.accountId === accountId ? { ...row, signature } : row,
      ),
    });
  };

  const many = accounts.length;
  const said = many < COUNTS.length ? COUNTS[many] : String(many);

  return (
    <>
      <h2 className="settings-title">Accounts</h2>
      <p className="settings-note">
        {said} {many === 1 ? "account" : "accounts"}, each with its own places, rules and piles. The
        colour is the edge on a row in All accounts.
      </p>

      {accounts.map((account) => {
        const row = rowFor(account.id);
        // The only thing on this screen that reads the kind, and it reads it three times: what sits
        // under the card, what the aliases line can honestly say, and whether there is a Google
        // account here at all for the footnote at the foot to be about.
        const google = account.kind === "google";
        const missing = google ? PERMISSIONS.filter((p) => !holds(account.grantedScopes, p.scope)) : [];
        return (
          <div className="acct" key={account.id}>
            <div className="acct-head">
              <Avatar
                name={account.name}
                address={account.email}
                hue={accountHue(account.color)}
              />
              <div>
                <Draft
                  className="settings-input acct-name"
                  label={`Name for ${account.email}`}
                  value={account.name}
                  placeholder={account.email}
                  onCommit={(name) => {
                    // Written to the list first, or the field flips back to the old name for
                    // the frame between the command and the refresh that follows it.
                    const was = account.name;
                    useAccounts.getState().patch(account.id, { name });
                    void accountSetName(account.id, name)
                      .then(() => useAccounts.getState().refresh())
                      .catch((e) => {
                        useAccounts.getState().patch(account.id, { name: was });
                        notify(`Could not change that name: ${e}`);
                      });
                  }}
                />
                <div className="acct-addr">{account.email}</div>
              </div>
              <div className="swatches">
                {HUES.map((hue) => (
                  <button
                    key={hue}
                    type="button"
                    className="swatch"
                    data-hue={hue}
                    data-on={account.color === `hue-${hue}` ? "" : undefined}
                    title={`Colour ${hue}`}
                    aria-label={`Colour ${hue}`}
                    onClick={() => {
                      void accountSetColor(account.id, `hue-${hue}`)
                        .then(() => useAccounts.getState().refresh())
                        .catch((e) => notify(`Could not change that colour: ${e}`));
                    }}
                  />
                ))}
              </div>
            </div>

            <div className="set-rows">
              <div className="set-row">
                <span className="lab">Signature</span>
                <Draft
                  className="settings-input val"
                  label={`Signature for ${account.email}`}
                  value={row?.signature ?? ""}
                  placeholder="Not set"
                  onCommit={(signature) => setSignature(account.id, signature)}
                />
              </div>
              {/* Aliases are the provider's answer, so an account with no provider to ask has to
                  say that rather than report a count of nothing. There is no command in IMAP or in
                  SMTP for "which addresses may I send as", and inventing one is not on offer, so a
                  second address on a mailbox like this is a second account. The signature above is
                  not the same case: it is Margin's own field, held here and journalled to the other
                  devices, and it works the same on either kind. */}
              <div className="set-row" data-wrap={google ? undefined : ""}>
                <span className="lab">Aliases</span>
                <span className="val">
                  {google
                    ? row && row.aliases.length > 0
                      ? row.aliases.join(", ")
                      : "None on the provider"
                    : "Neither IMAP nor SMTP has a way to publish them, so this account only ever sends as its own address."}
                </span>
              </div>
              {google && missing.length === 0 ? (
                <div className="set-row">
                  <span className="lab">Permissions</span>
                  <span className="val">All granted</span>
                </div>
              ) : null}
            </div>

            {!google ? <Servers account={account} /> : missing.length > 0 ? (
              <div className="scopes">
                <div className="scopes-head">Permissions</div>
                {PERMISSIONS.map((permission) => {
                  const granted = holds(account.grantedScopes, permission.scope);
                  return (
                    <div key={permission.scope}>
                      <div className="scope" data-missing={granted ? undefined : ""}>
                        {/* There is no minus in the icon set and a screen does not invent one, so
                            an ungranted line carries a rule rather than a glyph. */}
                        {granted ? <Icon d={icons.CHECK} size={13} /> : <span className="scope-dash" aria-hidden="true" />}
                        <span className="what">{permission.label}</span>
                        {granted ? (
                          <span className="state">Granted</span>
                        ) : (
                          <Button
                            size="sm"
                            disabled={waiting}
                            onClick={() =>
                              void grant(account.id, extrasFor(account, permission.scope))
                            }
                          >
                            {waiting ? "Waiting" : "Grant"}
                          </Button>
                        )}
                      </div>
                      {granted ? null : <p className="scope-why">{permission.cost}</p>}
                    </div>
                  );
                })}
              </div>
            ) : null}
          </div>
        );
      })}

      <div className="acct-foot">
        {/* Not a Google button wearing a general name. This one asks which mailbox it is, because
            the answer decides whether the next thing that happens is a browser or a password
            field, and half the addresses this app was built for are not Google's. */}
        <Button icon={icons.PLUS} disabled={waiting} onClick={startMail}>
          Add account
        </Button>
        {accounts.length > 0 ? (
          <Button variant="ghost" onClick={() => setRemoving(true)}>
            Remove an account
          </Button>
        ) : null}
      </div>

      {waiting ? (
        <div className="settings-waiting">
          <span>Waiting for Google in your browser.</span>
          <Button size="sm" disabled={!authUrl} onClick={openAuthUrl}>
            Open link again
          </Button>
          <Button size="sm" disabled={!authUrl} onClick={() => void copyAuthUrl()}>
            Copy link
          </Button>
          <Button size="sm" variant="ghost" onClick={cancelConnect}>
            Cancel
          </Button>
        </div>
      ) : null}

      {/* True of a Google account and of nothing else. On a machine where every account is an IMAP
          one there is no Google account page for it to be about, and a footnote explaining the
          consequences of revoking a grant nobody made is a paragraph that reads as though the app
          has quietly signed you in to something. */}
      {accounts.some((account) => account.kind === "google") ? (
        <p className="settings-quiet">
          Margin Mail, Margin and Margin Calendar share one Google client, so your Google account
          lists them once, as Margin, and revoking it revokes all three.
        </p>
      ) : null}

      <AddSheet
        open={mailPhase === "address" || mailPhase === "password" || mailPhase === "unsupported"}
        title={connectMailTitle(mailPhase, mailConfig, mailEmail, mailBlocked)}
        onBack={mailPhase === "address" ? undefined : backMail}
        onClose={leaveMail}
      >
        <ConnectMail
          host="sheet"
          onGoogle={(email) => {
            // The browser has the flow from here, and the strip above the account list is what
            // waits for it: a sheet left open over that would be two things waiting for one answer.
            leaveMail();
            void addAccount(email);
          }}
        />
      </AddSheet>

      <RemoveSheet open={removing} accounts={accounts} onClose={() => setRemoving(false)} />
    </>
  );
}

/**
 * Add account, which is the welcome screen's flow in a panel.
 *
 * The same `ConnectMail` renders here with its head left to this panel, so a sentence reworded on
 * one screen is reworded on both, and the panel's own back control retraces the flow's steps. The
 * servers and the certificate hang off the stage rather than off this sheet, because they are a
 * wider panel than this one and going back from them comes back here: the sheet gives the window
 * up entirely while those are open, which is why `open` names the phases it draws rather than
 * every phase that is not off.
 */
function AddSheet({
  open,
  title,
  onBack,
  onClose,
  children,
}: {
  open: boolean;
  title: string;
  onBack?: () => void;
  onClose: () => void;
  children: ReactNode;
}) {
  return (
    <Sheet open={open} title={title} onBack={onBack} backLabel="the address" onClose={onClose}>
      {children}
    </Sheet>
  );
}

/**
 * What an IMAP account has where a Google account lists its permissions.
 *
 * Nothing was granted here and there is no consent page to send anybody back to, so a list of six
 * capabilities with Grant buttons beside them would be six offers to open a Google page for an
 * account that has no Google. What this kind of account is instead is two servers, a login and a
 * password, and those are the facts somebody opens this card to check when the mail stops arriving.
 *
 * `imap_servers` hands back what the account was added with rather than what discovery suggested at
 * the time, so what is on screen is what the next connection will actually use.
 *
 * No IMAP session in this app has yet run against a real server, so all three of the answers this
 * can come back with are said out loud, and the two that are not a configuration are said
 * differently. A call that threw and an account with no servers stored are not the same trouble:
 * one is a card that cannot report, and the other is an account that cannot connect. Undefined is
 * neither of them, it is the question still being asked.
 */
function Servers({ account }: { account: Account }) {
  const [config, setConfig] = useState<MailConfig | null | undefined>(undefined);
  const [refused, setRefused] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    setConfig(undefined);
    setRefused(null);
    imapServers(account.id)
      .then((found) => {
        if (live) setConfig(found);
      })
      .catch((e) => {
        if (live) setRefused(String(e));
      });
    return () => {
      live = false;
    };
  }, [account.id]);

  return (
    <div className="acct-servers">
      <div className="scopes-head">Servers</div>

      {refused !== null ? (
        <p className="scope-why">
          The servers for this account could not be read back: {refused}. The account itself is
          untouched, but this card cannot say what it connects to until that call answers.
        </p>
      ) : config === undefined ? (
        <p className="scope-why">Reading the servers this account was added with.</p>
      ) : config === null ? (
        <p className="scope-why">
          No servers are stored for this account, so nothing can reach its mailbox. Connecting the
          same address again from Add account is what puts them back.
        </p>
      ) : (
        <>
          <div className="set-row">
            <span className="lab">Incoming</span>
            <span className="val">{serverLine(config.imap)}</span>
          </div>
          <div className="set-row">
            <span className="lab">Outgoing</span>
            <span className="val">{serverLine(config.smtp)}</span>
          </div>
          <div className="set-row">
            <span className="lab">Username</span>
            <span className="val">{loginLine(config)}</span>
          </div>
          <div className="set-row" data-wrap="">
            <span className="lab">Password</span>
            <span className="val">
              Sealed on this device beside the app's other keys. It goes to these two servers and to
              nobody else, and it is replaced by connecting the account again.
            </span>
          </div>
        </>
      )}
    </div>
  );
}

/**
 * Who logs in, said twice only when the two halves genuinely differ. A gateway wanting its own
 * credentials is common enough that the connect sheet does not even insist on a username for it,
 * and an account set up that way has two logins rather than one.
 */
function loginLine(config: MailConfig): string {
  const incoming = config.imap.username;
  const outgoing = config.smtp.username;
  if (!outgoing || outgoing === incoming) return incoming;
  return `${incoming} incoming, ${outgoing} outgoing`;
}

/**
 * One button and one question. Removing revokes Margin's access at Google and forgets the account
 * here; it used to be two buttons with two reaches, and read as a choice nobody could make. The
 * only choice left is whether the mail and the decisions stay on this computer, and it is a box
 * ticked to delete them, because the person pressed Remove.
 */
function RemoveSheet({
  open,
  accounts,
  onClose,
}: {
  open: boolean;
  accounts: Account[];
  onClose: () => void;
}) {
  const [asking, setAsking] = useState<Account | null>(null);
  const [deleteData, setDeleteData] = useState(true);
  const [phase, setPhase] = useState<"idle" | "removing">("idle");
  const busy = phase !== "idle";

  const ask = (account: Account) => {
    setDeleteData(true);
    setAsking(account);
  };

  const run = async (account: Account) => {
    setPhase("removing");
    try {
      await accountRemove(account.id, !deleteData);
      await useAccounts.getState().refresh();
      notify(removedNote(account, deleteData));
      setAsking(null);
      onClose();
    } catch (e) {
      // A revoke that could not reach Google still removes the account here, and says so in its
      // own words. The list is the only thing that can tell that apart from a removal that failed.
      await useAccounts.getState().refresh();
      const gone = !useAccounts.getState().accounts.some((one) => one.id === account.id);
      notify(gone ? String(e) : `Could not remove that account: ${e}`);
      if (gone) {
        setAsking(null);
        onClose();
      }
    } finally {
      setPhase("idle");
    }
  };

  return (
    <Sheet open={open} title="Remove an account" busy={busy} onClose={onClose}>
      {asking ? (
        <Confirm
          title={`Remove ${asking.email}?`}
          body={
            <>
              {asking.kind === "google" ? (
                <p>
                  This hands Margin's grant back to Google for Margin Mail, Margin and Margin
                  Calendar, on every machine you have signed in on, because all three share one
                  client. Your mail is untouched on Gmail.
                </p>
              ) : (
                <p>
                  This forgets the account's password on this computer. Your mail is untouched on
                  the server.
                </p>
              )}
              <label className="confirm-option">
                <input
                  type="checkbox"
                  checked={deleteData}
                  disabled={busy}
                  onChange={(e) => setDeleteData(e.target.checked)}
                />
                <span>
                  Also delete this account's mail and decisions from this computer
                  <span className="confirm-option-note">
                    Left unticked, they stay on disk and come back if you add the account again.
                  </span>
                </span>
              </label>
            </>
          }
          confirmLabel="Remove"
          busy={busy}
          busyLabel="Removing"
          onConfirm={() => void run(asking)}
          onCancel={() => setAsking(null)}
        />
      ) : (
        accounts.map((account) => (
          <div className="remove-row" key={account.id}>
            <Avatar
              name={account.name}
              address={account.email}
              hue={accountHue(account.color)}
              size="sm"
            />
            <span className="remove-who">{account.email}</span>
            <Button size="sm" variant="danger" onClick={() => ask(account)}>
              Remove
            </Button>
          </div>
        ))
      )}
    </Sheet>
  );
}

/** What the toast says once the account has gone, which depends on what there was to revoke. */
function removedNote(account: Account, deleted: boolean): string {
  const gone =
    account.kind === "google"
      ? `${account.email} was removed and Margin's access at Google revoked`
      : `${account.email} was removed from this device`;
  return deleted ? gone : `${gone}. Its mail and decisions stay on this computer`;
}

// -------------------------------------------------------------------------------------------
// Appearance
// -------------------------------------------------------------------------------------------

const THEMES = [
  { id: "light", label: "Light" },
  { id: "dark", label: "Dark" },
  { id: "system", label: "System" },
];

/** The reading size, in the steps a person can actually tell apart. */
const SIZES = [13, 14, 15, 16, 17, 18];

function AppearanceSection() {
  const settings = useSettings((s) => s.settings);
  const fonts = useSettings((s) => s.fonts);
  const loadFonts = useSettings((s) => s.loadFonts);
  const save = useSettings((s) => s.save);
  const pane = useMail((s) => s.pane);
  const togglePane = useMail((s) => s.togglePane);

  useEffect(() => {
    void loadFonts();
  }, [loadFonts]);

  if (!settings) return <h2 className="settings-title">Appearance</h2>;

  const setTheme = (choice: string) => {
    useTheme.getState().set(choice === "system" ? systemTheme() : (choice as "light" | "dark"));
    if (choice === "system") forgetThemeChoice();
    void save({ theme: choice as SettingsDto["theme"] });
  };

  const setFont = (slot: "ui" | "text", ref: FontRef) => {
    applyFonts(slot === "ui" ? ref : settings.fontUi, slot === "text" ? ref : settings.fontText);
    void save(slot === "ui" ? { fontUi: ref } : { fontText: ref });
  };

  return (
    <>
      <h2 className="settings-title">Appearance</h2>
      <p className="settings-note">
        How the app is set and how much of the window the mail gets. All of it is this machine's
        rather than this account's.
      </p>

      <SettingRow label="Theme" note="System follows whatever the machine is doing at the time.">
        <Segment options={THEMES} value={settings.theme} label="Theme" onChange={setTheme} />
      </SettingRow>

      <SettingRow label="Interface font" note="The chrome: rows, buttons, the palette.">
        <FontPicker
          label="Interface font"
          value={settings.fontUi}
          families={fonts}
          onChange={(ref) => setFont("ui", ref)}
        />
      </SettingRow>

      <SettingRow label="Text font" note="Message bodies, subjects and every heading.">
        <FontPicker
          label="Text font"
          value={settings.fontText}
          families={fonts}
          onChange={(ref) => setFont("text", ref)}
        />
      </SettingRow>

      <SettingRow label="Text size" note="Message bodies only. The chrome is already the size it wants to be.">
        <Segment
          options={SIZES.map((size) => ({ id: String(size), label: String(size) }))}
          value={String(settings.textSize)}
          label="Text size"
          onChange={(id) => {
            applyTextSize(Number(id));
            void save({ textSize: Number(id) });
          }}
        />
      </SettingRow>

      {/* The one row whose label belongs to the control rather than beside it: the switch is a
          real label and htmlFor, and the primitive already lays the row out the same way. */}
      <div className="set-field">
        <Toggle
          checked={pane}
          label="Reading pane"
          note="The same switch as ⌘\. With it off, a thread opens in place of the list."
          onChange={(next) => {
            if (next !== pane) togglePane();
            void save({ readingPane: next });
          }}
        />
      </div>
    </>
  );
}

function SettingRow({
  label,
  note,
  children,
}: {
  label: string;
  note?: string;
  children: ReactNode;
}) {
  return (
    <div className="set-field">
      <div className="set-field-text">
        <span className="set-field-label">{label}</span>
        {note ? <span className="set-field-note">{note}</span> : null}
      </div>
      <div className="set-field-control">{children}</div>
    </div>
  );
}

function FontPicker({
  label,
  value,
  families,
  onChange,
}: {
  label: string;
  value: FontRef;
  families: string[];
  onChange: (ref: FontRef) => void;
}) {
  const current = encodeRef(value);
  const known = useMemo(
    () =>
      BUNDLED_FONTS.some((font) => refsEqual({ kind: "bundled", id: font.id }, value)) ||
      families.some((family) => refsEqual({ kind: "system", family }, value)),
    [families, value],
  );

  return (
    <select
      className="settings-select"
      aria-label={label}
      value={current}
      onChange={(e) => onChange(decodeRef(e.target.value))}
    >
      {/* A face chosen on another machine, or one uninstalled since, is still what this setting
          says. Naming it keeps the picker from showing a face nobody chose. */}
      {known ? null : <option value={current}>{fontLabel(value)}</option>}
      <optgroup label="Bundled">
        {BUNDLED_FONTS.map((font) => (
          <option key={font.id} value={encodeRef({ kind: "bundled", id: font.id })}>
            {font.label}
          </option>
        ))}
      </optgroup>
      {families.length > 0 ? (
        <optgroup label="On this machine">
          {families.map((family) => (
            <option key={family} value={encodeRef({ kind: "system", family })}>
              {family}
            </option>
          ))}
        </optgroup>
      ) : null}
    </select>
  );
}

// -------------------------------------------------------------------------------------------
// What every section below shares
// -------------------------------------------------------------------------------------------

const DAY_MS = 86_400_000;

const dayMonthYear = new Intl.DateTimeFormat(undefined, {
  day: "numeric",
  month: "long",
  year: "numeric",
});

const count = (n: number): string => n.toLocaleString();

/** Disk, in the units a settings screen talks about. `fileSize` stops at MB and a mirror does not. */
function space(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${Math.round(kb)} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`;
  return `${(mb / 1024).toFixed(1)} GB`;
}

/** A time of day, which `SnoozeTimes` carries as minutes from midnight. */
const clock = (minutes: number): string =>
  `${String(Math.floor(minutes / 60) % 24).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;

const numbered = (values: number[], label: (n: number) => string): SegmentOption[] =>
  values.map((value) => ({ id: String(value), label: label(value) }));

interface Asking {
  /** The panel's head. The question itself is the confirmation's, one line further in. */
  title: string;
  question: string;
  body: ReactNode;
  confirmLabel: string;
  /** What the button says while `run` is on its way: "Clearing". */
  busyLabel?: string;
  run: () => Promise<void>;
}

/**
 * The one confirmation shape the sections share. Every destructive thing on this screen goes
 * through it, so what a person is asked and how they are asked it are written once.
 */
function Ask({ asking, onClose }: { asking: Asking | null; onClose: () => void }) {
  const [phase, setPhase] = useState<"idle" | "running">("idle");
  if (!asking) return null;
  const running = phase === "running";
  return (
    <Sheet open title={asking.title} size="mini" busy={running} onClose={onClose}>
      <Confirm
        title={asking.question}
        body={asking.body}
        confirmLabel={asking.confirmLabel}
        busy={running}
        busyLabel={asking.busyLabel}
        onConfirm={() => {
          setPhase("running");
          void asking.run().finally(() => {
            setPhase("idle");
            onClose();
          });
        }}
        onCancel={onClose}
      />
    </Sheet>
  );
}

/**
 * A field that writes when it is left rather than on every keystroke, which is the shape the
 * contact card's note uses and the only one that does not send a command per letter.
 */
function Draft({
  className,
  label,
  value,
  placeholder,
  rows,
  onCommit,
}: {
  className: string;
  label: string;
  value: string;
  placeholder?: string;
  rows?: number;
  onCommit: (value: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  const [editing, setEditing] = useState(false);

  // What another device wrote arrives while nobody is typing, and overwrites nothing once somebody
  // is.
  useEffect(() => {
    if (!editing) setDraft(value);
  }, [value, editing]);

  const commit = () => {
    setEditing(false);
    if (draft !== value) onCommit(draft);
  };

  const type = (next: string) => {
    setEditing(true);
    setDraft(next);
  };

  if (rows) {
    return (
      <textarea
        className={className}
        rows={rows}
        aria-label={label}
        value={draft}
        placeholder={placeholder}
        onChange={(e) => type(e.target.value)}
        onBlur={commit}
      />
    );
  }

  return (
    <input
      {...NO_AUTOFILL}
      className={className}
      type="text"
      aria-label={label}
      value={draft}
      placeholder={placeholder}
      onChange={(e) => type(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
      }}
    />
  );
}

function TextSetting({
  label,
  /** When the printed label is not enough to tell two of these apart, such as one per account. */
  named,
  note,
  value,
  placeholder,
  rows,
  onCommit,
}: {
  label: string;
  named?: string;
  note?: string;
  value: string;
  placeholder?: string;
  rows?: number;
  onCommit: (value: string) => void;
}) {
  return (
    <div className="set-stack">
      <span className="set-field-label">{label}</span>
      {note ? <span className="set-field-note">{note}</span> : null}
      <Draft
        className={rows ? "settings-textarea" : "settings-input"}
        label={named ?? label}
        value={value}
        placeholder={placeholder}
        rows={rows}
        onCommit={onCommit}
      />
    </div>
  );
}

/** A time of day, offered as the hours somebody actually chooses between. */
function TimePicker({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number;
  onChange: (minutes: number) => void;
}) {
  const hours = [6, 7, 8, 9, 10, 11].map((hour) => hour * 60);
  return (
    <select
      className="settings-select"
      aria-label={label}
      value={String(value)}
      onChange={(e) => onChange(Number(e.target.value))}
    >
      {/* A time set on another device, or by hand in the file, is still what this setting says. */}
      {hours.includes(value) ? null : <option value={String(value)}>{clock(value)}</option>}
      {hours.map((minutes) => (
        <option key={minutes} value={String(minutes)}>
          {clock(minutes)}
        </option>
      ))}
    </select>
  );
}

/** The thin bar the first sync draws, reused rather than drawn again, because it is the same wait. */
function Filling({ status }: { status: SyncStatus }) {
  const done = status.total > 0 ? Math.min(status.hydrated, status.total) / status.total : 0;
  return (
    <div className="set-filling">
      <div
        className="welcome-bar"
        role="progressbar"
        aria-label="Filling in the older mail"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(done * 100)}
      >
        <span className="welcome-bar-fill" style={{ transform: `scaleX(${done})` }} />
      </div>
      <span className="set-field-note">
        {status.message ?? "Filling in the older mail, newest first."}
      </span>
    </div>
  );
}

/** The head of a card that is about one account: its colour, its address and nothing else. */
function AccountHead({ account }: { account: Account }) {
  return (
    <div className="set-card-head">
      <Avatar
        name={account.name}
        address={account.email}
        hue={accountHue(account.color)}
        size="sm"
      />
      <span className="set-card-who">{account.email}</span>
    </div>
  );
}

// -------------------------------------------------------------------------------------------
// Mail
// -------------------------------------------------------------------------------------------

const WINDOWS: SegmentOption[] = [
  { id: "30", label: "30 days" },
  { id: "90", label: "90 days" },
  { id: "180", label: "180 days" },
  { id: "365", label: "A year" },
  { id: "0", label: "Everything" },
];

/** Zero is the widest window rather than the narrowest, which is the whole of shrink against widen. */
const reach = (days: number): number => (days === 0 ? Number.POSITIVE_INFINITY : days);

const spanOf = (days: number): string =>
  days === 0 ? "everything" : days === 365 ? "the last year" : `the last ${days} days`;

const CACHE_CAPS = numbered([256, 512, 1024, 2048, 5120], (mb) =>
  mb < 1024 ? `${mb} MB` : `${mb / 1024} GB`,
);

/** A thread somebody decided something about, which is the kind eviction keeps whatever its age. */
const decided = (thread: ThreadSummary): boolean =>
  Boolean(thread.pile) ||
  thread.snoozedUntil !== null ||
  Boolean(thread.note) ||
  thread.ignored ||
  thread.notify ||
  thread.starred ||
  thread.hasDraft;

const COUNT_PAGE = 500;
const COUNT_PAGES = 20;

/**
 * How many threads a narrower window would take off this device.
 *
 * Counted from the list rather than asked of Rust, because there is no dry run behind eviction:
 * `window_set` evicts and reports the number to nobody. So this is exact for what a person can
 * see, which is Everything, and Everything is the whole device but the trash.
 */
async function evictionCount(
  accountId: string,
  cutoffMs: number,
): Promise<{ threads: number; capped: boolean }> {
  let cursor: string | null = null;
  let threads = 0;
  for (let page = 0; page < COUNT_PAGES; page += 1) {
    const got = await threadsList({
      accountId,
      place: "everything",
      limit: COUNT_PAGE,
      cursor,
    });
    for (const thread of got.threads) {
      if (thread.dateMs < cutoffMs && !decided(thread)) threads += 1;
    }
    cursor = got.nextCursor;
    if (!cursor) return { threads, capped: false };
  }
  return { threads, capped: true };
}

function Doomed({ accountId, days }: { accountId: string; days: number }) {
  const [phase, setPhase] = useState<"counting" | "counted" | "error">("counting");
  const [found, setFound] = useState({ threads: 0, capped: false });

  useEffect(() => {
    let alive = true;
    void evictionCount(accountId, Date.now() - days * DAY_MS)
      .then((got) => {
        if (!alive) return;
        setFound(got);
        setPhase("counted");
      })
      .catch(() => {
        if (alive) setPhase("error");
      });
    return () => {
      alive = false;
    };
  }, [accountId, days]);

  return (
    <>
      <p>
        {phase === "counting"
          ? "Counting what would go."
          : phase === "error"
            ? `Every thread older than ${days} days leaves this device. Counting them first did not work, so this is the shape of it rather than the number.`
            : found.threads === 0
              ? `Nothing on this device is older than ${days} days, so nothing would be removed.`
              : `${found.capped ? "At least " : ""}${count(found.threads)} ${
                  found.threads === 1 ? "thread" : "threads"
                } older than ${days} days would be removed from this device.`}
      </p>
      <p>
        Threads carrying a pile, a note, a snooze or any other decision are kept whatever their age,
        and mail already in the trash is not in the count above. Nothing is removed from Gmail, and
        widening the window again brings it back.
      </p>
    </>
  );
}

function MailSection() {
  const settings = useSettings((s) => s.settings);
  const storage = useSettings((s) => s.storage);
  const loadStorage = useSettings((s) => s.loadStorage);
  const save = useSettings((s) => s.save);
  const accounts = useAccounts((s) => s.accounts);
  const statuses = useSync((s) => s.statuses);
  const [asking, setAsking] = useState<Asking | null>(null);

  useEffect(() => {
    void loadStorage();
  }, [loadStorage]);

  if (!settings) return <h2 className="settings-title">Mail</h2>;

  const rowFor = (accountId: string) =>
    settings.accounts.find((row) => row.accountId === accountId) ?? null;

  const write = (accountId: string, days: number) =>
    void save({
      accounts: settings.accounts.map((row) =>
        row.accountId === accountId ? { ...row, windowDays: days } : row,
      ),
    }).then(() => loadStorage());

  const choose = (account: Account, days: number) => {
    const was = rowFor(account.id)?.windowDays ?? 0;
    if (days === was) return;
    // Widening only ever adds, and Rust queues the backfill off the same write, so it goes through.
    if (reach(days) >= reach(was)) {
      write(account.id, days);
      return;
    }
    setAsking({
      title: "Storage window",
      question: `Keep only ${spanOf(days)} of ${account.email} on this device?`,
      body: <Doomed accountId={account.id} days={days} />,
      confirmLabel: "Narrow the window",
      run: async () => write(account.id, days),
    });
  };

  const cached = storage.reduce((sum, used) => sum + used.attachmentsBytes, 0);

  return (
    <>
      <h2 className="settings-title">Mail</h2>
      <p className="settings-note">
        How much of each mailbox lives on this machine. All of it is this machine's: the window that
        suits a laptop with a small disk is not the window that suits a desktop.
      </p>

      {accounts.map((account) => {
        const row = rowFor(account.id);
        const used = storage.find((one) => one.accountId === account.id) ?? null;
        const status = statuses.find((one) => one.accountId === account.id) ?? null;
        const filling =
          status && (status.phase === "backfilling" || status.phase === "hydrating") ? status : null;

        return (
          <div className="set-card" key={account.id}>
            <AccountHead account={account} />
            <div className="set-field">
              <div className="set-field-text">
                <span className="set-field-label">Storage window</span>
                <span className="set-field-note">
                  {used
                    ? `${count(used.threads)} threads and ${count(used.messages)} messages${
                        used.oldestMs ? `, back to ${dayMonthYear.format(used.oldestMs)}` : ""
                      }.`
                    : "Measuring what is on this device."}
                </span>
              </div>
              <div className="set-field-control">
                <Segment
                  options={WINDOWS}
                  value={String(row?.windowDays ?? account.windowDays)}
                  label={`Storage window for ${account.email}`}
                  onChange={(id) => choose(account, Number(id))}
                />
              </div>
            </div>
            {filling ? <Filling status={filling} /> : null}
          </div>
        );
      })}

      <p className="settings-quiet">
        Threads carrying a pile, a note, a snooze or any other decision are kept whatever their age,
        so narrowing the window never takes away something you decided about.
      </p>

      <SettingRow
        label="Attachment cache"
        note={`Attachments are fetched when you open them and kept until the cap is reached. ${space(cached)} in use.`}
      >
        <Segment
          options={CACHE_CAPS}
          value={String(settings.attachmentCacheMb)}
          label="Attachment cache"
          onChange={(id) => void save({ attachmentCacheMb: Number(id) })}
        />
      </SettingRow>

      <div className="set-field">
        <Toggle
          checked={settings.prefetchBodies}
          label="Fetch message bodies while idle"
          note="Fills in the bodies inside the window when nothing else is happening, so a thread opens without waiting. Disk, not what is in a list."
          onChange={(next) => void save({ prefetchBodies: next })}
        />
      </div>

      <Ask asking={asking} onClose={() => setAsking(null)} />
    </>
  );
}

// -------------------------------------------------------------------------------------------
// Privacy
// -------------------------------------------------------------------------------------------

const IMAGES: SegmentOption[] = [
  { id: "never", label: "Never" },
  { id: "ask", label: "Ask per message" },
  { id: "always", label: "Always" },
];

function PrivacySection() {
  const settings = useSettings((s) => s.settings);
  const save = useSettings((s) => s.save);
  const [allowed, setAllowed] = useState<ContactCard[]>([]);
  const [phase, setPhase] = useState<"loading" | "idle" | "error">("loading");

  useEffect(() => {
    let alive = true;
    void contactsList(null, "")
      .then((cards) => {
        if (!alive) return;
        setAllowed(cards.filter((card) => card.allowRemoteImages));
        setPhase("idle");
      })
      .catch(() => {
        if (alive) setPhase("error");
      });
    return () => {
      alive = false;
    };
  }, []);

  if (!settings) return <h2 className="settings-title">Privacy</h2>;

  const forget = (card: ContactCard) => {
    setAllowed((were) => were.filter((one) => one !== card));
    void contactUpdate(card.accountId, card.person.address, { allowRemoteImages: false }).catch(
      (e) => {
        setAllowed((were) => [...were, card]);
        notify(`Could not change that: ${e}`);
      },
    );
  };

  return (
    <>
      <h2 className="settings-title">Privacy</h2>
      <p className="settings-note">
        What a message is allowed to do when you open it. Nothing here leaves the machine to be
        decided somewhere else.
      </p>

      <SettingRow
        label="Remote images"
        note="Loading a remote image tells the server that hosts it that you opened the message, from your IP address, at that moment. Nothing else in the app reveals that."
      >
        <Segment
          options={IMAGES}
          value={settings.remoteImages}
          label="Remote images"
          onChange={(id) => void save({ remoteImages: id as SettingsDto["remoteImages"] })}
        />
      </SettingRow>

      <div className="set-field">
        <Toggle
          checked={settings.linkCleaning}
          label="Clean the tracking out of links"
          note="Takes the campaign and click-identifier parameters off a link before it opens, so the page you land on is not told which mail you came from."
          onChange={(next) => void save({ linkCleaning: next })}
        />
      </div>

      <div className="set-stack">
        <span className="set-field-label">Senders allowed to load images</span>
        <span className="set-field-note">
          Added from a contact card, and taken back here. Everyone else is held to the setting above.
        </span>
        {phase === "loading" ? (
          <p className="settings-quiet">Reading your contacts.</p>
        ) : phase === "error" ? (
          <p className="settings-quiet">Could not read your contacts.</p>
        ) : allowed.length === 0 ? (
          <p className="settings-quiet">Nobody yet.</p>
        ) : (
          <ul className="set-list">
            {allowed.map((card) => (
              <li key={`${card.accountId}|${card.person.address}`}>
                <Avatar
                  name={displayName(card.person)}
                  address={card.person.address}
                  size="sm"
                />
                <span className="set-list-who">
                  <span className="set-list-name">{displayName(card.person)}</span>
                  <span className="set-list-note">{card.person.address}</span>
                </span>
                <Button
                  size="sm"
                  label={`Stop loading images from ${card.person.address}`}
                  onClick={() => forget(card)}
                >
                  Remove
                </Button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </>
  );
}

// -------------------------------------------------------------------------------------------
// Screener
// -------------------------------------------------------------------------------------------

function ScreenerSection() {
  const settings = useSettings((s) => s.settings);
  const save = useSettings((s) => s.save);

  if (!settings) return <h2 className="settings-title">Screener</h2>;

  return (
    <>
      <h2 className="settings-title">Screener</h2>
      <p className="settings-note">
        The gate in front of the Inbox. A sender is decided once and every message from them after
        that follows the decision.
      </p>

      <div className="set-field">
        <Toggle
          checked={settings.screenerEnabled}
          label="Screen first contact"
          note="With this off, a sender nobody has decided about is routed by the suggestion instead and nothing waits."
          onChange={(next) => void save({ screenerEnabled: next })}
        />
      </div>

      <div className="set-field">
        <Toggle
          checked={settings.holdReplies}
          label="Hold replies to threads you are in"
          note="Off, because somebody answering a message you sent is not first contact. On is for wanting absolutely everything screened."
          onChange={(next) => void save({ holdReplies: next })}
        />
      </div>

      <div className="set-field">
        <Toggle
          checked={settings.suggestions}
          label="Suggest where a sender belongs"
          note="With this off a Screener card shows the three destinations and no recommendation."
          onChange={(next) => void save({ suggestions: next })}
        />
      </div>
    </>
  );
}

// -------------------------------------------------------------------------------------------
// Piles and snooze
// -------------------------------------------------------------------------------------------

const LATER_TODAY = numbered([1, 2, 3, 4], (hours) => (hours === 1 ? "1 hour" : `${hours} hours`));

const FEED_AGES: SegmentOption[] = [
  { id: "0", label: "Off" },
  { id: "7", label: "A week" },
  { id: "14", label: "Two weeks" },
  { id: "30", label: "A month" },
  { id: "90", label: "Three months" },
];

/** The verbs a swipe can be, which is what the contract's two strings are allowed to hold. */
const SWIPES: { id: string; label: string }[] = [
  { id: "reply-later", label: "Reply later" },
  { id: "set-aside", label: "Set aside" },
  { id: "archive", label: "Archive" },
  { id: "trash", label: "Trash" },
  { id: "none", label: "Nothing" },
];

function SwipePicker({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (id: string) => void;
}) {
  return (
    <select
      className="settings-select"
      aria-label={label}
      value={value}
      onChange={(e) => onChange(e.target.value)}
    >
      {SWIPES.some((swipe) => swipe.id === value) ? null : <option value={value}>{value}</option>}
      {SWIPES.map((swipe) => (
        <option key={swipe.id} value={swipe.id}>
          {swipe.label}
        </option>
      ))}
    </select>
  );
}

function PilesSection() {
  const settings = useSettings((s) => s.settings);
  const save = useSettings((s) => s.save);

  if (!settings) return <h2 className="settings-title">Piles and snooze</h2>;

  const times = settings.snoozeTimes;
  const setTime = (patch: Partial<typeof times>) =>
    void save({ snoozeTimes: { ...times, ...patch } });

  return (
    <>
      <h2 className="settings-title">Piles and snooze</h2>
      <p className="settings-note">
        What the four snooze choices mean, so they can be set once and never thought about again.
      </p>

      <SettingRow label="Later today" note="How far ahead Later today puts a thread.">
        <Segment
          options={LATER_TODAY}
          value={String(times.laterTodayHours)}
          label="Later today"
          onChange={(id) => setTime({ laterTodayHours: Number(id) })}
        />
      </SettingRow>

      <SettingRow label="Tomorrow morning" note="The hour a thread comes back tomorrow.">
        <TimePicker
          label="Tomorrow morning"
          value={times.tomorrowAt}
          onChange={(minutes) => setTime({ tomorrowAt: minutes })}
        />
      </SettingRow>

      <SettingRow label="The weekend" note="Saturday, at this time.">
        <TimePicker
          label="The weekend"
          value={times.weekendAt}
          onChange={(minutes) => setTime({ weekendAt: minutes })}
        />
      </SettingRow>

      <SettingRow label="Next week" note="Monday, at this time.">
        <TimePicker
          label="Next week"
          value={times.nextWeekAt}
          onChange={(minutes) => setTime({ nextWeekAt: minutes })}
        />
      </SettingRow>

      <SettingRow label="Swipe right" note="What a swipe to the right does to a row on a phone.">
        <SwipePicker
          label="Swipe right"
          value={settings.swipeRight}
          onChange={(id) => void save({ swipeRight: id })}
        />
      </SettingRow>

      <SettingRow label="Swipe left" note="And a swipe to the left.">
        <SwipePicker
          label="Swipe left"
          value={settings.swipeLeft}
          onChange={(id) => void save({ swipeLeft: id })}
        />
      </SettingRow>

      <SettingRow
        label="Trash Feed mail after"
        note="Off by default. A contact card sets it for one sender; this is every Feed sender at once."
      >
        <Segment
          options={FEED_AGES}
          value={String(settings.feedAutoTrashDays)}
          label="Trash Feed mail after"
          onChange={(id) => void save({ feedAutoTrashDays: Number(id) })}
        />
      </SettingRow>
    </>
  );
}

// -------------------------------------------------------------------------------------------
// Writing
// -------------------------------------------------------------------------------------------

const UNDO_DELAYS = numbered([5, 10, 20, 30], (secs) => `${secs}s`);

const REPLY_DEFAULT: SegmentOption[] = [
  { id: "reply", label: "Reply" },
  { id: "reply-all", label: "Reply all" },
];

function WritingSection() {
  const settings = useSettings((s) => s.settings);
  const save = useSettings((s) => s.save);
  const accounts = useAccounts((s) => s.accounts);

  if (!settings) return <h2 className="settings-title">Writing</h2>;

  const setSignature = (accountId: string, signature: string) =>
    void save({
      accounts: settings.accounts.map((row) =>
        row.accountId === accountId ? { ...row, signature } : row,
      ),
    });

  return (
    <>
      <h2 className="settings-title">Writing</h2>
      <p className="settings-note">
        What happens when you send, and what goes out under your name.
      </p>

      <SettingRow
        label="Undo delay"
        note="How long a send waits with its toast up before it actually goes. z takes it back."
      >
        <Segment
          options={UNDO_DELAYS}
          value={String(settings.undoDelaySecs)}
          label="Undo delay"
          onChange={(id) => void save({ undoDelaySecs: Number(id) })}
        />
      </SettingRow>

      <SettingRow label="What r does" note="Reply all is always on ⇧R whichever way this is set.">
        <Segment
          options={REPLY_DEFAULT}
          value={settings.replyAllDefault ? "reply-all" : "reply"}
          label="What r does"
          onChange={(id) => void save({ replyAllDefault: id === "reply-all" })}
        />
      </SettingRow>

      <TextSetting
        label="Instant intro"
        note="⌘⇧I in a reply moves the introducer to Bcc and puts this in. Pressing it again takes both back."
        value={settings.instantIntro}
        rows={3}
        onCommit={(instantIntro) => void save({ instantIntro })}
      />

      {accounts.map((account) => {
        const row = settings.accounts.find((one) => one.accountId === account.id);
        return (
          <div className="set-card" key={account.id}>
            <AccountHead account={account} />
            <TextSetting
              label="Signature"
              named={`Signature for ${account.email}`}
              value={row?.signature ?? ""}
              placeholder="Nothing after your messages"
              rows={3}
              onCommit={(signature) => setSignature(account.id, signature)}
            />
          </div>
        );
      })}

      <p className="settings-quiet">
        The intro text and the signatures are per account rather than per machine, so they follow you
        to another device rather than being typed again.
      </p>
    </>
  );
}

// -------------------------------------------------------------------------------------------
// Notifications
// -------------------------------------------------------------------------------------------

function NotificationsSection() {
  const settings = useSettings((s) => s.settings);
  const save = useSettings((s) => s.save);
  // What the system says, null until it has answered. Read again whenever the window comes back,
  // because a refusal is undone in System Settings and not here.
  const [permission, setPermission] = useState<NotifyPermission | null>(null);
  const [asking, setAsking] = useState(false);
  const [testing, setTesting] = useState(false);

  useEffect(() => {
    let alive = true;
    const check = () => {
      notifyPermission()
        .then((state) => {
          if (alive) setPermission(state);
        })
        .catch(() => {});
    };
    check();
    window.addEventListener("focus", check);
    return () => {
      alive = false;
      window.removeEventListener("focus", check);
    };
  }, []);

  if (!settings) return <h2 className="settings-title">Notifications</h2>;

  // One switch: the system's permission and the app's own preference together, because nobody
  // cares which of the two is saying no. Off is off whichever it was.
  const denied = permission === "denied";
  const on = permission === "granted" && settings.notifications;
  const places = settings.notifyPlaces;

  const allow = async (next: boolean) => {
    if (!next) {
      await save({ notifications: false });
      return;
    }
    setAsking(true);
    try {
      const state = await askForNotifications();
      setPermission(state);
      if (state !== "granted") return;
      // A yes with no place chosen would turn on nothing, so the Inbox comes with it the first
      // time. The Feed and the Paper Trail stay a choice.
      await save({
        notifications: true,
        ...(places.length === 0 ? { notifyPlaces: ["inbox" as Place] } : {}),
      });
    } catch (e) {
      notify(`Could not turn on notifications: ${e}`);
    } finally {
      setAsking(false);
    }
  };

  const setPlace = (place: Place, next: boolean) =>
    void save({ notifyPlaces: withPlace(places, place, next) });

  const test = async () => {
    setTesting(true);
    try {
      await notifyTest();
    } catch (e) {
      notify(`Could not send a test notification: ${e}`);
    } finally {
      setTesting(false);
    }
  };

  const openSystem = () =>
    void openNotificationSettings().catch((e) => notify(`Could not open System Settings: ${e}`));

  return (
    <>
      <h2 className="settings-title">Notifications</h2>
      <p className="settings-note">
        New mail can arrive as a push notification: a banner from the system with the sender and
        the subject. Nothing notifies you until you say so.
      </p>

      <div className="set-field">
        <Toggle
          checked={on}
          disabled={asking || denied || permission === null}
          label={asking ? "Asking the system" : "Allow notifications"}
          onChange={(next) => void allow(next)}
        />
      </div>

      {denied ? (
        <div className="set-stack" data-permission="denied">
          <span className="set-field-note">
            Notifications are turned off for Margin Mail in System Settings. Turn them on there and
            this switch comes back.
          </span>
          <div className="set-actions">
            <Button onClick={openSystem}>Open System Settings</Button>
          </div>
        </div>
      ) : null}

      {on ? (
        <>
          <div className="set-stack">
            <span className="set-field-label">Which mail</span>
            <span className="set-field-note">
              ⇧N turns notifications on for one thread and a contact card turns them on for one
              person. These are what everything else falls back to.
            </span>
          </div>

          {NOTIFY_PLACES.map((place) => (
            <div className="set-field" key={place.id}>
              <Toggle
                checked={places.includes(place.id)}
                label={`Notify about the ${place.label}`}
                note={place.note}
                onChange={(next) => setPlace(place.id, next)}
              />
            </div>
          ))}

          <div className="set-stack">
            <span className="set-field-label">Try it</span>
            <span className="set-field-note">Posts one sample notification.</span>
            <div className="set-actions">
              <Button disabled={testing} onClick={() => void test()}>
                {testing ? "Sending" : "Send a test notification"}
              </Button>
            </div>
          </div>
        </>
      ) : null}

      <div className="set-field">
        <Toggle
          checked={settings.badge}
          label="Badge the dock icon"
          note="Not a notification, so it is on by default and needs no permission. Counts unseen Inbox threads across every account, plus anything a snooze has brought back. Not an unread count: what is in the Screener, the Feed or the Paper Trail is not waiting for you."
          onChange={(next) => void save({ badge: next })}
        />
      </div>

      <p className="settings-quiet">
        These are this machine's, so a phone can be quiet while a desktop is not.
      </p>
    </>
  );
}

// -------------------------------------------------------------------------------------------
// Keyboard
// -------------------------------------------------------------------------------------------

function KeyboardSection() {
  const [path, setPath] = useState<string | null>(null);
  const [phase, setPhase] = useState<"loading" | "idle" | "error">("loading");
  const [asking, setAsking] = useState<Asking | null>(null);

  useEffect(() => {
    let alive = true;
    void keymapPath()
      .then((found) => {
        if (!alive) return;
        setPath(found);
        setPhase("idle");
      })
      .catch(() => {
        if (alive) setPhase("error");
      });
    return () => {
      alive = false;
    };
  }, []);

  const open = () => {
    if (!path) return;
    // Opening it is the point; showing it where it lives is what a machine that will not hand a
    // file to an editor can still do.
    openPath(path).catch(() => revealItemInDir(path).catch(() => notify("Could not open that file")));
  };

  return (
    <>
      <h2 className="settings-title">Keyboard</h2>
      <p className="settings-note">
        The keymap is a file rather than a table of pickers, so it can be read, edited, copied to
        another machine and kept in version control.
      </p>

      <div className="set-stack">
        <span className="set-field-label">Where it is</span>
        <span className="settings-path">
          {phase === "loading" ? "Looking." : phase === "error" ? "Could not be read." : path}
        </span>
        <div className="set-actions">
          <Button disabled={!path} onClick={open}>
            Open the file
          </Button>
          <Button
            variant="ghost"
            onClick={() =>
              setAsking({
                title: "Keyboard",
                question: "Put the default keys back?",
                body: (
                  <p>
                    Every change in the keymap file goes, and the keys go back to what
                    docs/keyboard.md lists. Nothing else on this machine is touched.
                  </p>
                ),
                confirmLabel: "Reset the keymap",
                run: async () => {
                  try {
                    await keymapReset();
                    notify("The keymap is back to the defaults");
                  } catch (e) {
                    notify(`Could not reset the keymap: ${e}`);
                  }
                },
              })
            }
          >
            Put the defaults back
          </Button>
        </div>
      </div>

      <p className="settings-quiet">
        A file that will not parse leaves the defaults in force until it is fixed.
      </p>

      <Ask asking={asking} onClose={() => setAsking(null)} />
    </>
  );
}

// -------------------------------------------------------------------------------------------
// Backup
// -------------------------------------------------------------------------------------------

const STORES: SegmentOption[] = [
  { id: "none", label: "Off" },
  { id: "drive", label: "Google Drive" },
  { id: "r2", label: "Cloudflare R2" },
];

const STORE_NAMES: Record<string, string> = {
  none: "nowhere",
  drive: "Google Drive",
  r2: "your R2 bucket",
};

/** The four S3 fields, and which of them the settings file is allowed to have kept. */
const R2_FIELDS: {
  key: string;
  label: string;
  placeholder: string;
  secret?: boolean;
  was: (backup: BackupSettings) => string | null;
}[] = [
  {
    key: "endpoint",
    label: "Endpoint",
    placeholder: "https://<account>.r2.cloudflarestorage.com",
    was: (backup) => backup.r2Endpoint,
  },
  { key: "bucket", label: "Bucket", placeholder: "margin-backup", was: (backup) => backup.r2Bucket },
  { key: "accessKey", label: "Access key id", placeholder: "", secret: true, was: () => null },
  { key: "secret", label: "Secret access key", placeholder: "", secret: true, was: () => null },
];

function BackupSection() {
  const settings = useSettings((s) => s.settings);
  const applyBackup = useSettings((s) => s.applyBackup);
  const [choosing, setChoosing] = useState<string | null>(null);
  const [r2, setR2] = useState<Record<string, string>>({});
  const [phrase, setPhrase] = useState<string | null>(null);
  const [restoring, setRestoring] = useState("");
  // Three different waits on the same store, and each has its own button to say so on. One shared
  // word for all of them was how Restore went grey while a backup ran.
  const [phase, setPhase] = useState<"idle" | "configuring" | "backing-up" | "restoring">("idle");

  if (!settings) return <h2 className="settings-title">Backup</h2>;

  const backup = settings.backup;
  const store = choosing ?? backup.store;

  /**
   * The phrase is shown once, at setup, so it is asked for as part of turning backup on rather
   * than sitting behind a button somebody has to know to press.
   */
  const configure = async (which: string, config: Record<string, string>) => {
    // The segment moves the moment it is pressed. Deriving the key can take a second, and a switch
    // that sits on the old value for that long reads as one that did not take the press.
    setChoosing(which);
    setPhase("configuring");
    try {
      const next = await backupConfigure(which, config);
      applyBackup(next);
      setChoosing(null);
      if (which !== "none" && !next.hasPhrase) {
        const words = await backupPhrase();
        setPhrase(words);
        applyBackup({ ...next, hasPhrase: true });
      }
    } catch (e) {
      // The R2 fields stay open so what was typed can be corrected. Any other pick falls back to
      // what the settings file still says.
      if (which !== "r2") setChoosing(null);
      notify(`Could not set that up: ${e}`);
    } finally {
      setPhase("idle");
    }
  };

  const run = async (
    doing: "backing-up" | "restoring",
    what: string,
    task: () => Promise<void>,
  ) => {
    setPhase(doing);
    try {
      await task();
    } catch (e) {
      notify(`Could not ${what}: ${e}`);
    } finally {
      setPhase("idle");
    }
  };

  return (
    <div className="set-section" data-phase={phase}>
      <h2 className="settings-title">Backup</h2>
      <p className="settings-note">
        Every decision the app holds, journalled and encrypted on this machine before it leaves it.
        The store holds ciphertext and file names and nothing else.
      </p>

      <SettingRow label="Where it goes" note="Drive uses the account you are already signed in to.">
        <Segment
          options={STORES}
          value={store}
          label="Backup store"
          disabled={phase === "configuring"}
          onChange={(id) => {
            // R2 cannot be turned on by picking it: it needs four fields first, so picking it
            // opens them and the button below is what commits.
            if (id === "r2") setChoosing("r2");
            else void configure(id, {});
          }}
        />
      </SettingRow>

      {store === "r2" ? (
        <div className="set-card">
          {/* Held here and sent once, rather than a command per field: three quarters of a set of
              credentials is not a bucket anybody can be configured against. */}
          {R2_FIELDS.map((field) => (
            <div className="set-stack" key={field.key}>
              <span className="set-field-label">{field.label}</span>
              <input
                {...NO_AUTOFILL}
                className="settings-input"
                type={field.secret ? "password" : "text"}
                aria-label={field.label}
                placeholder={field.placeholder}
                value={r2[field.key] ?? field.was(backup) ?? ""}
                onChange={(e) => setR2((were) => ({ ...were, [field.key]: e.target.value }))}
              />
            </div>
          ))}
          <div className="set-actions">
            <Button
              variant="primary"
              disabled={phase === "configuring"}
              onClick={() =>
                void configure(
                  "r2",
                  Object.fromEntries(
                    R2_FIELDS.map((field) => [field.key, r2[field.key] ?? field.was(backup) ?? ""]),
                  ),
                )
              }
            >
              {phase === "configuring" ? "Setting up" : "Use this bucket"}
            </Button>
          </div>
          <p className="settings-quiet">
            The keys are encrypted on this device and never written beside the backup.
          </p>
        </div>
      ) : null}

      <div className="set-rows">
        <div className="set-row">
          <span className="lab">Status</span>
          <span className="val">
            {backup.configured
              ? `Backing up to ${STORE_NAMES[backup.store] ?? backup.store}.`
              : "Not set up, so nothing leaves this machine."}
          </span>
        </div>
        <div className="set-row">
          <span className="lab">Last backup</span>
          <span className="val">
            {backup.lastBackupMs ? messageTime(backup.lastBackupMs) : "Never"}
          </span>
        </div>
      </div>

      {backup.configured ? (
        <div className="set-actions">
          <Button
            disabled={phase === "backing-up"}
            onClick={() =>
              void run("backing-up", "back up now", async () => {
                applyBackup(await backupNow());
                notify("Backed up");
              })
            }
          >
            {phase === "backing-up" ? "Backing up" : "Back up now"}
          </Button>
        </div>
      ) : null}

      {phrase ? (
        <div className="set-card set-phrase">
          <span className="set-field-label">Your recovery phrase</span>
          <p className="phrase">{phrase}</p>
          <p className="set-field-note">
            Write these twelve words down now. They are the only way to attach a second device or to
            restore after a lost one, and this is the only time they will be on a screen: writing
            them down now is the whole of the arrangement.
          </p>
        </div>
      ) : backup.hasPhrase ? (
        <div className="set-stack">
          <span className="set-field-label">Your recovery phrase</span>
          <span className="set-field-note">
            It was shown once when backup was turned on and cannot be shown again. That is the
            mechanism rather than a rule: the phrase was generated, the key it derives was sealed,
            and the phrase itself was dropped, so there is nothing left on this device that could
            print it a second time. A device that could would be a device that had kept it, and then
            the phrase would protect nothing the disk did not already give away. Backup carries on
            working without it.
          </span>
        </div>
      ) : null}

      <div className="set-stack">
        <span className="set-field-label">Restore from a phrase</span>
        <span className="set-field-note">
          Attaches this device to a backup that already exists, which is also how a lost machine is
          replaced. Choose where the backup is kept first.
        </span>
        <input
          {...NO_AUTOFILL}
          className="settings-input"
          type="text"
          aria-label="Recovery phrase"
          placeholder="Twelve words, separated by spaces"
          value={restoring}
          onChange={(e) => setRestoring(e.target.value)}
        />
        <div className="set-actions">
          <Button
            disabled={restoring.trim().length === 0 || phase === "restoring"}
            onClick={() =>
              void run("restoring", "restore from that phrase", async () => {
                await backupRestore(restoring.trim());
                setRestoring("");
                notify("Restored from your backup");
              })
            }
          >
            {phase === "restoring" ? "Restoring" : "Restore"}
          </Button>
        </div>
      </div>
    </div>
  );
}

// -------------------------------------------------------------------------------------------
// Data
// -------------------------------------------------------------------------------------------

function DataSection() {
  const storage = useSettings((s) => s.storage);
  const loadStorage = useSettings((s) => s.loadStorage);
  const accounts = useAccounts((s) => s.accounts);
  const [importing, setImporting] = useState("");
  const [asking, setAsking] = useState<Asking | null>(null);
  // Every one of these holds the database lock for as long as it takes to walk the whole mirror,
  // so a second press before the first answers is a second file, not a second chance.
  const [phase, setPhase] = useState<"idle" | "exporting" | "importing" | "error">("idle");
  /** Which export is being written: an account id for an mbox, "state" for the app state. */
  const [writing, setWriting] = useState<string | null>(null);
  const busy = phase === "exporting" || phase === "importing";

  useEffect(() => {
    void loadStorage();
  }, [loadStorage]);

  const wrote = (path: string) => {
    notify(`Written to ${path}`, {
      label: "Show it",
      run: () => void revealItemInDir(path).catch(() => {}),
    });
  };

  const write = async (what: string, task: () => Promise<string>, failed: string) => {
    setPhase("exporting");
    setWriting(what);
    try {
      wrote(await task());
      setPhase("idle");
    } catch (e) {
      setPhase("error");
      notify(`${failed}: ${e}`);
    } finally {
      setWriting(null);
    }
  };

  const bring = async () => {
    setPhase("importing");
    try {
      await importState(importing.trim());
      setImporting("");
      setPhase("idle");
      notify("Your app state was imported");
    } catch (e) {
      setPhase("error");
      notify(`Could not import that file: ${e}`);
    }
  };

  const totals = storage.reduce(
    (sum, used) => ({
      mirror: sum.mirror + used.mirrorBytes + used.bodiesBytes,
      attachments: sum.attachments + used.attachmentsBytes,
      state: sum.state + used.stateBytes,
    }),
    { mirror: 0, attachments: 0, state: 0 },
  );

  return (
    <div className="set-section" data-phase={phase}>
      <h2 className="settings-title">Data</h2>
      <p className="settings-note">
        Everything here is a file you end up holding. Nothing is uploaded and nothing asks anybody
        else first.
      </p>

      <div className="set-stack">
        <span className="set-field-label">Export mail as mbox</span>
        <span className="set-field-note">
          The window that is on this device, in the format every other mail client reads.
        </span>
        <div className="set-actions">
          {accounts.map((account) => (
            <Button
              key={account.id}
              label={`Export ${account.email} as mbox`}
              disabled={busy}
              onClick={() =>
                void write(
                  account.id,
                  () => exportMbox(account.id),
                  "Could not export that mailbox",
                )
              }
            >
              {writing === account.id ? "Writing" : account.email}
            </Button>
          ))}
        </div>
      </div>

      <div className="set-stack">
        <span className="set-field-label">App state</span>
        <span className="set-field-note">
          Every decision the app holds, as JSON: the sender rules, the piles, the snoozes, the notes,
          the clips and the renames. It is the portable form of all of it.
        </span>
        <div className="set-actions">
          <Button
            disabled={busy}
            onClick={() => void write("state", exportState, "Could not export your app state")}
          >
            {writing === "state" ? "Writing" : "Export"}
          </Button>
        </div>
        <input
          {...NO_AUTOFILL}
          className="settings-input"
          type="text"
          aria-label="App state file to import"
          placeholder="The path of a file an export wrote"
          value={importing}
          onChange={(e) => setImporting(e.target.value)}
        />
        <div className="set-actions">
          <Button disabled={importing.trim().length === 0 || busy} onClick={() => void bring()}>
            {phase === "importing" ? "Importing" : "Import"}
          </Button>
        </div>
      </div>

      <div className="set-stack">
        <span className="set-field-label">Storage used</span>
        <div className="set-rows">
          <div className="set-row">
            <span className="lab">Mirror</span>
            <span className="val">{space(totals.mirror)}</span>
          </div>
          <div className="set-row">
            <span className="lab">Attachments</span>
            <span className="val">{space(totals.attachments)}</span>
          </div>
          <div className="set-row">
            <span className="lab">Decisions</span>
            <span className="val">{space(totals.state)}</span>
          </div>
        </div>
      </div>

      <div className="set-stack">
        <span className="set-field-label">Clear the mirror</span>
        <span className="set-field-note">
          Throws away the local copy of the mail and syncs the window again from scratch. It is the
          answer to a database that has gone wrong.
        </span>
        <div className="set-actions">
          {accounts.map((account) => (
            <Button
              key={account.id}
              variant="danger"
              label={`Clear the mirror for ${account.email}`}
              onClick={() =>
                setAsking({
                  title: "Clear the mirror",
                  question: `Clear the local copy of ${account.email}?`,
                  body: (
                    <>
                      <p>
                        This deletes the mail on this device and syncs {spanOf(account.windowDays)}{" "}
                        again from Gmail. Nothing is removed from Gmail.
                      </p>
                      <p>
                        Every decision you have made is kept: the sender rules, the piles, the
                        snoozes, the notes, the clips, the renames and the merges are in the state
                        database, and this does not touch it.
                      </p>
                    </>
                  ),
                  confirmLabel: "Clear it",
                  busyLabel: "Clearing",
                  run: async () => {
                    try {
                      await mirrorClear(account.id);
                      await loadStorage();
                      // The pass starts now rather than at the next tick, so the toast's "being
                      // fetched again" is true as it is read.
                      void useSync.getState().run(account.id);
                      notify(`The local copy of ${account.email} is being fetched again`);
                    } catch (e) {
                      notify(`Could not clear that mirror: ${e}`);
                    }
                  },
                })
              }
            >
              {account.email}
            </Button>
          ))}
        </div>
      </div>

      <Ask asking={asking} onClose={() => setAsking(null)} />
    </div>
  );
}

// -------------------------------------------------------------------------------------------
// About
// -------------------------------------------------------------------------------------------

function AboutSection() {
  const [version, setVersion] = useState(builtVersion);
  const [packager, setPackager] = useState<string | null>(null);
  const [phase, setPhase] = useState<"idle" | "checking" | "current" | "found" | "installing">(
    "idle",
  );
  const [offered, setOffered] = useState<Update | null>(null);

  useEffect(() => {
    // The binary's version rather than the bundle's, because this is the number a bug report
    // carries and the two only agree until somebody ships a hotfix.
    if (isTauri) void getVersion().then(setVersion).catch(() => {});
    void packagedBy().then(setPackager).catch(() => {});
  }, []);

  const look = async () => {
    setPhase("checking");
    try {
      const update = await check();
      setOffered(update);
      setPhase(update ? "found" : "current");
    } catch (e) {
      setPhase("idle");
      notify(`Could not check for updates: ${e}`);
    }
  };

  const install = async (update: Update) => {
    setPhase("installing");
    try {
      await update.downloadAndInstall();
      await relaunch();
    } catch (e) {
      setPhase("found");
      notify(`Could not install that update: ${e}`);
    }
  };

  return (
    <>
      <h2 className="settings-title">About</h2>

      <div className="set-rows">
        <div className="set-row">
          <span className="lab">Version</span>
          <span className="val settings-version">Margin Mail {version}</span>
        </div>
        <div className="set-row">
          <span className="lab">Licence</span>
          <span className="val">FSL-1.1-MIT</span>
        </div>
        <div className="set-row">
          <span className="lab">Packaged by</span>
          <span className="val">{packager ?? "Nobody: this is a build from source."}</span>
        </div>
      </div>

      <p className="settings-quiet">
        The Functional Source License: use it for anything except competing with it, and every
        release turns MIT two years after it ships.
      </p>

      {isDesktop ? (
        <div className="set-actions">
          <Button
            disabled={phase === "checking" || phase === "installing"}
            onClick={() => void look()}
          >
            {phase === "checking" ? "Looking" : "Check for updates"}
          </Button>
          {offered ? (
            <Button
              variant="primary"
              disabled={phase === "installing"}
              onClick={() => void install(offered)}
            >
              {phase === "installing" ? "Installing" : `Install ${offered.version}`}
            </Button>
          ) : null}
          {phase === "current" ? (
            <span className="set-field-note">This is the newest there is.</span>
          ) : null}
        </div>
      ) : null}
    </>
  );
}

export default Settings;
