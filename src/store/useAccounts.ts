import { create } from "zustand";
import { openUrl } from "@tauri-apps/plugin-opener";
import { accountConnect, accountGrant, accountsList, accountStart } from "../api/accounts";
import { screenerSeed } from "../api/screener";
import { live, type Account, type AuthEvent } from "../ipc";
import { notify } from "./useToast";

/**
 * `connecting` is waiting for the browser, `choosing` is the account written and the window it is
 * to hold not yet picked, `syncing` is the first pass arriving behind the progress screen, and
 * `refused` is a consent page that came back without the one scope the app cannot work without.
 * Refusing is a decision somebody made, which is why it is not `error`.
 */
type Phase = "idle" | "connecting" | "choosing" | "syncing" | "refused" | "error";

/**
 * Which screen started the consent. Only the welcome screen's flow takes the stage: granting a
 * scope or adding a second account from Settings must not throw the first-run screen up over it.
 */
type Origin = "welcome" | "settings";

interface AccountsState {
  accounts: Account[];
  /**
   * True once a list has actually come back. An empty `accounts` before that is a page that has
   * not asked yet, and telling the two apart is the difference between "connect one" and a first
   * frame of the connect screen on every launch.
   */
  loaded: boolean;
  phase: Phase;
  error: string | null;
  /** The consent URL, kept so the two escape hatches have something to open and to copy. */
  authUrl: string | null;
  origin: Origin | null;
  /** The scopes the consent page came back without, which is what the refused screen names. */
  missingRequired: string[];
  /** The account a finished consent added, waiting for its first page of threads. */
  pendingAccountId: string | null;
  /** The window chosen for it, which is what the progress screen says it is bringing in. */
  pendingDays: number;
  /** `account_start` on its way: the window written, the engine handed the account. */
  starting: boolean;
  /**
   * An account added while the app was already up, whose first sync the window is waiting on.
   * `Arriving` holds a panel over everything until the engine says the mail is in, then opens
   * that account's Inbox. Set from Settings and from the IMAP sign-in; the welcome screen has
   * its own stage for the same wait and never sets this.
   */
  arriving: { accountId: string; email: string; days: number; started: boolean } | null;
  /** What the first-run panel narrates, set once the pass over senders has run. */
  onboarding: { accountId: string; screenedIn: number } | null;
  /**
   * An account whose seed was asked for before its first sync had finished. The mirror answers
   * "not yet" until the crawl is done, and this is who to ask for again when its sync reports idle.
   */
  seeding: string | null;
  resolveConnect: ((ok: boolean) => void) | null;

  refresh: () => Promise<void>;
  /**
   * Rewrites one account in place, ahead of the command that makes it so. The registry's answer
   * arrives on the next refresh; between the two the screen would otherwise show the old name.
   */
  patch: (accountId: string, changes: Partial<Account>) => void;
  /** The address, when the connect screen has one, opens the consent page on that account. */
  connect: (email?: string) => Promise<boolean>;
  /** The same flow from Settings, which leaves the Inbox where it is. */
  addAccount: (email?: string) => Promise<boolean>;
  /** Re-runs consent for a linked account to pick up a scope that was withheld. */
  grant: (accountId: string, extraScopes: string[]) => Promise<boolean>;
  cancelConnect: () => void;
  handleAuthEvent: (event: AuthEvent) => Promise<void>;
  openAuthUrl: () => void;
  copyAuthUrl: () => Promise<void>;
  /** Hands the window over from the progress screen to the Inbox and seeds the Screener. */
  finishConnect: () => Promise<void>;
  /** Puts the arriving panel up for an account that has just been added. */
  arrive: (accountId: string, email: string) => void;
  /**
   * The window chosen: writes it, hands the account to the engine and starts its first pass.
   * For the welcome screen this is what moves it from choosing to the progress; for the arriving
   * panel it is what moves the panel from the question to the bar.
   */
  startSync: (days: number) => Promise<void>;
  /** Takes it down again, whichever way the wait ended. */
  arrived: () => void;
  /**
   * Asks for the seed, and remembers to ask again if the mailbox is not ready for it. Reached
   * from the sync events as well as from the connect flow, because from Settings the flow is over
   * long before the first sync is.
   */
  seedScreener: (accountId: string) => Promise<void>;
  dismissOnboarding: () => void;
}

/**
 * The account list, the consent flow, and nothing about the query.
 *
 * Which account is being looked at is a field of the thread query, so it lives in `useMail` beside
 * the place rather than here; the header reads both and the switcher writes to that one.
 *
 * `connect()` returns a promise whose `resolve` is stashed in state for the later `auth` event to
 * settle, which is margin's pattern from `useBackup.ts` by way of the calendar's file of this name.
 */
export const useAccounts = create<AccountsState>((set, get) => {
  /** The three entry points differ only in which command builds the URL and who is watching. */
  const consent = (url: () => Promise<string>, origin: Origin): Promise<boolean> =>
    new Promise<boolean>((resolve) => {
      get().resolveConnect?.(false);
      set({
        phase: "connecting",
        origin,
        error: null,
        authUrl: null,
        missingRequired: [],
        resolveConnect: resolve,
      });
      url()
        .then((authUrl) => {
          set({ authUrl });
          // The browser that opened is not always the browser in front of you, which is what Open
          // link again and Copy link are for; this is the one that usually works.
          openUrl(authUrl).catch(() => {});
        })
        .catch((e) => {
          set({ phase: "error", error: String(e), resolveConnect: null });
          notify(`Could not connect your Google account: ${e}`);
          resolve(false);
        });
    });

  return {
    accounts: [],
    loaded: false,
    phase: "idle",
    error: null,
    authUrl: null,
    origin: null,
    missingRequired: [],
    pendingAccountId: null,
    pendingDays: 30,
    starting: false,
    arriving: null,
    onboarding: null,
    seeding: null,
    resolveConnect: null,

    // Deliberately does not touch `phase`: a background refresh from `store-changed` arriving while
    // the consent browser is open must not clear the screen that is waiting for it.
    refresh: async () => {
      if (!live()) return;
      try {
        set({ accounts: await accountsList(), loaded: true, error: null });
      } catch (e) {
        set({ phase: "error", error: String(e) });
      }
    },

    patch: (accountId, changes) =>
      set((s) =>
        s.accounts.some((account) => account.id === accountId)
          ? {
              accounts: s.accounts.map((account) =>
                account.id === accountId ? { ...account, ...changes } : account,
              ),
            }
          : {},
      ),

    connect: (email) => consent(() => accountConnect([], email), "welcome"),
    addAccount: (email) => consent(() => accountConnect([], email), "settings"),
    grant: (accountId, extraScopes) =>
      consent(() => accountGrant(accountId, extraScopes), "settings"),

    cancelConnect: () => {
      const resolve = get().resolveConnect;
      set({ phase: "idle", origin: null, authUrl: null, resolveConnect: null });
      resolve?.(false);
    },

    handleAuthEvent: async (event) => {
      if (get().phase !== "connecting") return;
      const resolve = get().resolveConnect;
      const origin = get().origin;

      if (event.ok) {
        // Who was here before the registry is read again, so that an account which has just been
        // added can be told from one that has just been granted a scope: the second changes its
        // permissions and nothing else, and no panel goes up for it.
        const known = new Set(get().accounts.map((account) => account.id));
        await get().refresh();
        const added = event.accountId !== null && !known.has(event.accountId);
        set({
          authUrl: null,
          error: null,
          missingRequired: [],
          resolveConnect: null,
          // From the welcome screen the progress screen holds the stage until the first page of
          // threads lands. From Settings the flow is over here, and the wait for the mail is the
          // arriving panel's: it takes the window until the engine says the account is in, then
          // opens that account's Inbox. It used to end at once, with the new account sat under
          // Settings on an empty Inbox and one line in the header saying why.
          pendingAccountId: origin === "welcome" ? event.accountId : null,
          pendingDays: 30,
          phase: origin === "welcome" ? "choosing" : "idle",
        });
        if (origin !== "welcome" && added && event.accountId) {
          get().arrive(event.accountId, event.email ?? "");
        }
      } else if (event.missingRequired.length > 0) {
        set({
          phase: "refused",
          authUrl: null,
          error: null,
          missingRequired: event.missingRequired,
          resolveConnect: null,
        });
      } else if (event.cancelled) {
        // Closing the consent page is an answer, not a fault. Back to where it started with nothing
        // said: the person knows what they did, and a red panel about it reads as though shutting a
        // browser tab broke something.
        set({ phase: "idle", origin: null, authUrl: null, error: null, resolveConnect: null });
      } else {
        const message = event.error ?? "authorization failed";
        set({ phase: "error", authUrl: null, error: message, resolveConnect: null });
        notify(`Could not connect your Google account: ${message}`);
      }
      resolve?.(event.ok);
    },

    openAuthUrl: () => {
      const url = get().authUrl;
      if (url) openUrl(url).catch(() => {});
    },

    copyAuthUrl: async () => {
      const url = get().authUrl;
      if (!url) return;
      try {
        await navigator.clipboard.writeText(url);
        notify("Sign-in link copied");
      } catch {
        notify("Could not copy the link");
      }
    },

    finishConnect: async () => {
      const accountId = get().pendingAccountId;
      set({ phase: "idle", origin: null, pendingAccountId: null });
      if (!accountId) return;
      await get().seedScreener(accountId);
    },

    seedScreener: async (accountId) => {
      try {
        // Everyone this account already knows is screened in silently. It is also the number the
        // first-run panel is about, which is why the panel waits for it. Asked from Settings this
        // lands seconds after consent, on a mirror with nothing in it yet; the mirror says so and
        // the sync's own idle is what asks again. It used to seed that empty mirror, mark the seed
        // done with a count of nobody, and leave every sender waiting in the Screener for good.
        const screenedIn = await screenerSeed(accountId);
        if (screenedIn === null) {
          set({ seeding: accountId });
          return;
        }
        set({ seeding: null, onboarding: { accountId, screenedIn } });
      } catch (e) {
        set({ seeding: null });
        notify(`Could not screen in the people you already know: ${e}`);
      }
    },

    arrive: (accountId, email) => {
      const days = get().accounts.find((account) => account.id === accountId)?.windowDays ?? 30;
      set({ arriving: { accountId, email, days, started: false } });
    },
    arrived: () => set({ arriving: null }),

    startSync: async (days) => {
      const arriving = get().arriving;
      const accountId = arriving?.accountId ?? get().pendingAccountId;
      if (!accountId || get().starting) return;
      set({ starting: true });
      try {
        await accountStart(accountId, days);
        if (arriving) {
          set({ starting: false, arriving: { ...arriving, days, started: true } });
        } else {
          set({ starting: false, pendingDays: days, phase: "syncing" });
        }
      } catch (e) {
        // The account is written and the question is still on the screen, so the choice stands
        // and the button is live again: nothing was lost except the attempt.
        set({ starting: false });
        notify(`Could not start bringing in the mail: ${e}`);
      }
    },

    dismissOnboarding: () => set({ onboarding: null }),
  };
});
