import { lazy, Suspense, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";

import { Connect } from "./screens/Connect";
import { Header } from "./screens/Header";
import { ListColumn } from "./screens/ListColumn";
import { Clips } from "./screens/Clips";
import { Compose } from "./screens/Compose";
import { ContactCards } from "./screens/ContactCards";
import { Contacts } from "./screens/Contacts";
import { Feed } from "./screens/Feed";
import { Files } from "./screens/Files";
import { FocusReply } from "./screens/FocusReply";
import { Guide } from "./screens/Guide";
import { HelpButton } from "./screens/Help";
import { Onboarding } from "./screens/Onboarding";
import { Tour } from "./screens/Tour";
import { Arriving } from "./screens/Arriving";
import { Screener } from "./screens/Screener";
import { ReadingPane } from "./screens/ReadingPane";
import { Settings } from "./screens/Settings";
import { CommandPalette } from "./screens/CommandPalette";
import { Toasts } from "./screens/Toasts";
import { ShortcutsSheet } from "./keys/Shortcuts";

import { notifyTake } from "./api/notifications";
import { useEscapeLayer } from "./escape";
import { registerCommands } from "./keys/commands";
import { setAccountSwitch, useKeyContext, useKeymap } from "./keys/keymap";
import { handleMenuAction } from "./keys/menu";
import { useAccounts } from "./store/useAccounts";
import { useMail } from "./store/useMail";
import { useStage } from "./store/useStage";
import { useOverlays } from "./store/useOverlays";
import { useScreener } from "./store/useScreener";
import { useSettings } from "./store/useSettings";
import { useSnooze } from "./store/useSnooze";
import { announce, useSync } from "./store/useSync";
import { useTheme } from "./store/useTheme";
import { notify } from "./store/useToast";
import { usePhone, useTouch } from "./useMedia";
import { isDesktop, isTauri, live, type AuthEvent, type Place, type SyncStatus } from "./ipc";

// The Kit page renders every primitive in every state, and it is how a restyle is reviewed: one
// page, light and dark, screenshotted by Playwright. `import.meta.env.DEV` is a compile-time
// constant, so the whole branch and the module behind it are absent from a production bundle.
const Kit = import.meta.env.DEV ? lazy(() => import("./screens/Kit")) : null;

function useHashRoute(): string {
  const [hash, setHash] = useState(() => window.location.hash);
  useEffect(() => {
    const onChange = () => setHash(window.location.hash);
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
  }, []);
  return hash;
}

/**
 * The backend's three events, from whichever side is serving them.
 *
 * In Tauri they arrive through the event bus. Opened in a browser `src/dev/mockIpc.ts` dispatches
 * the same three payloads as window CustomEvents under the same names, which is what lets the whole
 * connect flow be driven and watched without a Google account, by a person or by Playwright.
 */
function onAppEvent<T>(name: string, handler: (payload: T) => void): () => void {
  if (isTauri) {
    const stop = listen<T>(name, (event) => handler(event.payload));
    return () => void stop.then((off) => off());
  }
  const onCustom = (event: Event) => handler((event as CustomEvent<T>).detail);
  window.addEventListener(name, onCustom);
  return () => window.removeEventListener(name, onCustom);
}

/**
 * How long a burst of `store-changed` gathers before the list asks for a page again.
 *
 * The sync engine runs a pass every twelve seconds and reports on every pass that touched
 * anything, and a mailbox that is still coming in touches something every time; one pass can also
 * name several scopes in a row. 250ms is short enough that a list which really did change still
 * looks immediate (nothing here is a keystroke, and the eye reads under a third of a second as
 * "already there"), and long enough that a pass costs one query rather than one per event.
 */
const RELOAD_MS = 250;

/** Where the Help menu's Report an Issue goes. docs/release.md names the repository. */
const ISSUES_URL = "https://github.com/priyanshujain/margin-mail/issues/new";

/**
 * The Help menu's Check for Updates. It is the About section's button without the section: the
 * same call, answered in toasts because there is no card here to write the answer on, and every
 * one of them is an answer to something the person just asked for.
 */
async function checkForUpdates(): Promise<void> {
  if (!isDesktop) return;
  try {
    const update = await check();
    if (!update) {
      notify("This is the newest there is.");
      return;
    }
    notify(`Margin Mail ${update.version} is ready to install`, {
      label: "Install",
      run: () => {
        void update
          .downloadAndInstall()
          .then(() => relaunch())
          .catch((e) => notify(`Could not install that update: ${e}`));
      },
    });
  } catch (e) {
    notify(`Could not check for updates: ${e}`);
  }
}

/**
 * A click on a notification: the account it was about, the place its thread shows in, and the
 * thread when it named one. The backend holds where the click pointed until asked, because the
 * click that launches the app lands before this code is listening; so the launch and the event
 * both ask the same way, and whichever asks second finds nothing.
 */
async function openFromNotification(): Promise<void> {
  const target = await notifyTake().catch(() => null);
  if (!target) return;
  useSettings.getState().close();
  useStage.getState().close();
  const mail = useMail.getState();
  if (useAccounts.getState().accounts.some((a) => a.id === target.accountId)) {
    mail.setAccount(target.accountId);
  }
  mail.goTo(target.place);
  if (target.threadKey) await useMail.getState().open(target.threadKey);
}

const PLACE_COMMANDS: [string, Place][] = [
  ["place-inbox", "inbox"],
  ["place-feed", "feed"],
  ["place-paper-trail", "paper-trail"],
  ["place-reply-later", "reply-later"],
  ["place-set-aside", "set-aside"],
  ["place-screener", "screener"],
  ["place-snoozed", "snoozed"],
  ["place-everything", "everything"],
];

function Shell() {
  usePhone();
  useTouch();
  useKeymap();

  const overlay = useOverlays((s) => s.open);
  const closeOverlay = useOverlays((s) => s.close);
  const pane = useMail((s) => s.pane);
  const openKey = useMail((s) => s.openKey);
  const accounts = useAccounts((s) => s.accounts);
  const loaded = useAccounts((s) => s.loaded);
  const connect = useAccounts((s) => s.phase);
  const origin = useAccounts((s) => s.origin);
  const place = useMail((s) => s.place);
  const stage = useStage((s) => s.open);

  // Nothing runs in the background and nothing fires at an exact time, so a snooze comes back when
  // somebody looks: on open, and on every return to the foreground. It belongs to the window rather
  // than to the list, or a snooze would not come back while the Feed or a library was up.
  useEffect(() => {
    const run = () => void useSnooze.getState().evaluate();
    run();
    const onVisible = () => {
      if (document.visibilityState === "visible") run();
    };
    window.addEventListener("focus", run);
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.removeEventListener("focus", run);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, []);

  // The four screens that are a stage rather than a place. They are registered here rather than in
  // each screen because a screen that is not on cannot register the command that turns it on.
  useEffect(
    () =>
      registerCommands({
        "open-contacts": () => useStage.getState().toggle("contacts"),
        "open-clips": () => useStage.getState().toggle("clips"),
        "open-files": () => useStage.getState().toggle("files"),
        "focus-reply": () => useStage.getState().toggle("focus-reply"),
      }),
    [],
  );
  const settings = useSettings((s) => s.open);

  // The connect screen is the whole app until there is an account, and it keeps the stage through
  // the consent and the first sync it narrates. Adding an account or granting a scope from Settings
  // runs the same flow and must not take the window over, which is what the origin distinguishes.
  const welcome = loaded && (accounts.length === 0 || (origin === "welcome" && connect !== "idle"));

  // One column with a thread in it: the list gave up the width, so the thread is the page and
  // Escape is the way back to the list.
  const onePage = !pane && openKey !== null;
  useEscapeLayer(onePage, () => useMail.getState().close());
  // A stage is somewhere you went rather than something that opened over you, so Escape puts you
  // back on the place you left rather than closing anything underneath it.
  useEscapeLayer(stage !== null, () => useStage.getState().close());

  // An open panel shadows the whole view keymap while leaving the global one reachable, which is
  // what keeps `j` from walking the list behind the shortcuts sheet. Decided here rather than in
  // each panel, because whether something is in front is the window's question.
  useKeyContext("overlay", overlay !== null);

  useEffect(() => {
    if (!live()) return;
    void (async () => {
      await useAccounts.getState().refresh();
      const accounts = useAccounts.getState().accounts;
      // Every mailbox at once is a choice rather than a default, so the app opens on one account.
      if (accounts.length > 0) useMail.getState().setAccount(accounts[0].id);
      else await useMail.getState().load();
      void useSync.getState().refresh();
      // A notification clicked while the app was not running is what launched it.
      await openFromNotification();
    })();
  }, []);

  useEffect(() => {
    if (!isTauri) return;
    return onAppEvent<null>("notification-open", () => void openFromNotification());
  }, []);


  // The verbs that belong to the window rather than to a column. The triage and writing verbs are
  // deliberately absent: this milestone has nothing to act with, and an unregistered command does
  // nothing at all, which is what docs/keyboard.md asks for.
  useEffect(() => {
    // Going somewhere leaves settings, because settings is a place and you cannot be in two.
    const places = Object.fromEntries(
      PLACE_COMMANDS.map(([command, place]) => [
        command,
        () => {
          // Settings and a library are both in front of the place, and going somewhere means
          // leaving them: a stage wins over a place, so a place changed underneath one is a place
          // nobody can see.
          useSettings.getState().close();
          useStage.getState().close();
          useMail.getState().goTo(place);
        },
      ]),
    );
    return registerCommands({
      ...places,
      "toggle-pane": () => useMail.getState().togglePane(),
      "command-palette": () => useOverlays.getState().toggle("palette"),
      // Settings is a place inside the app, and the welcome screen is not inside the app yet.
      settings: () => {
        if (useAccounts.getState().accounts.length > 0) useSettings.getState().show();
      },
      shortcuts: () => useOverlays.getState().toggle("shortcuts"),
      // Always `show` rather than `toggle`: the tour is opened from the help menu, and a toggle
      // would close it again for anyone who reached the menu from the tour's own last slide.
      tour: () => useOverlays.getState().show("tour"),
      guide: () => useOverlays.getState().show("guide"),
      "sync-now": () => void useSync.getState().run(),
      "toggle-theme": () => useTheme.getState().toggle(),
      accounts: () => useMail.getState().setAccount(null),
      // The two Help menu items. Neither has a key, so nothing but the menu ever emits them.
      "check-updates": () => void checkForUpdates(),
      "report-issue": () => {
        if (!isTauri) window.open(ISSUES_URL, "_blank", "noopener,noreferrer");
        else openUrl(ISSUES_URL).catch((e) => notify(`Could not open the browser: ${e}`));
      },
    });
  }, []);

  // Nine keys and one idea, so it is not a command with nine bindings.
  useEffect(
    () =>
      setAccountSwitch((index) => {
        const account = useAccounts.getState().accounts[index];
        if (account) useMail.getState().setAccount(account.id);
      }),
    [],
  );

  useEffect(() => {
    if (!isTauri) return;
    return onAppEvent<string>("menu-action", handleMenuAction);
  }, []);

  useEffect(() => {
    // A burst of scopes costs one query. The timer is not reset by the events that arrive while it
    // is pending, so a run of them cannot hold the list off indefinitely: the first one decides
    // when the query goes out and the rest join it.
    let pending: number | undefined;
    const reloadList = () => {
      if (pending !== undefined) return;
      pending = window.setTimeout(() => {
        pending = undefined;
        void useMail.getState().load();
      }, RELOAD_MS);
    };

    // The reason is a scope and not a bell: `threads`, `thread` or `thread:<key>`, `accounts`,
    // `screener`, and the rest. Reloading the whole list on every one of them was the jank, because
    // a pass warming the body cache says so several times a second and each of those rebuilt every
    // row and made react-virtuoso measure the list again.
    const changed = onAppEvent<string>("store-changed", (reason) => {
      const scopes = (reason ?? "").split(" ").filter(Boolean);
      // An emitter that named no scope has changed something it could not name, and the list is the
      // only safe reading of that.
      if (scopes.length === 0 || scopes.includes("threads")) reloadList();
      // A body landing changes what the open thread shows and nothing at all about the row above
      // it, so it re-reads that thread in place rather than the page it sits in. That is the bare
      // scope, and it is the one the cache warmer emits several times a second.
      if (scopes.some((scope) => scope === "thread" || scope.startsWith("thread:"))) {
        void useMail.getState().refreshThread();
      }
      // A scope that names a thread is a decision somebody took about that thread: a note, a
      // rename, a merge. Those show on the row as well as in the pane, so this one does reach the
      // list, and joins whatever query is already pending.
      if (scopes.some((scope) => scope.startsWith("thread:"))) reloadList();
      if (scopes.includes("accounts")) void useAccounts.getState().refresh();
      // The Screener's pill counts senders waiting rather than rows, so it is not part of the page
      // the list loaded and it has to be asked for by name.
      if (scopes.includes("screener")) {
        void useScreener.getState().load(useMail.getState().accountId);
      }
    });

    // The consent flow's other half. `account_connect` hands back a URL and returns; what actually
    // happened arrives here, minutes later if the person went to make tea.
    const auth = onAppEvent<AuthEvent>("auth", (event) => {
      void useAccounts.getState().handleAuthEvent(event);
    });

    // A pass that fails in the background used to set the error and say nothing, so mail that never
    // arrived looked like mail you do not have. Then it said so on every pass, because the engine
    // reports at the start and the end of each one, the same trouble came back every twelve
    // seconds, and a transport failure carried a different URL each time so nothing matched.
    // `announce` decides: once per distinct trouble per account, nothing for offline or a rate
    // limit (the account chip has those), and it forgets what it said once a pass ends clean so
    // the trouble coming back is news.
    //
    // This is also the only thing that keeps the sync store current. Every pass reports its status
    // through the same sink that emits `store-changed`, and the payload here is the whole
    // `SyncStatus`, so asking `sync_status` again on every invalidation was a round trip for
    // something the app had already been handed.
    const said = new Map<string, string | null>();
    const progress = onAppEvent<SyncStatus>("sync-progress", (status) => {
      useSync.getState().apply(status);
      // A seed the mirror was not ready for is asked for again when the account's pass ends.
      // The engine has seeded by then if the crawl finished, and the answer is the count the
      // first-run panel wants; if it did not, the answer is "not yet" again and this waits on.
      if (status.phase === "idle" && useAccounts.getState().seeding === status.accountId) {
        void useAccounts.getState().seedScreener(status.accountId);
      }
      const { toast, last } = announce(status, said.get(status.accountId) ?? null);
      if (toast) notify(toast);
      said.set(status.accountId, last);
    });

    return () => {
      window.clearTimeout(pending);
      changed();
      auth();
      progress();
    };
  }, []);

  // Paper and nothing on it until the account list has come back, which is a few milliseconds
  // either way. Without it a first launch paints the header and an empty list for one frame before
  // the welcome screen replaces both, and that frame reads as a mailbox that lost your mail.
  if (!loaded && live() && connect !== "error") return <div className="app" />;

  // Nothing of the app is drawn behind the welcome screen. There is no account, so the header would
  // be a chip with nobody in it over a list of nothing.
  if (welcome) {
    return (
      <div className="app">
        <Connect />
        <Toasts />
      </div>
    );
  }

  return (
    // The place is on the root as well as in the store, because it decides which screen is up and
    // three of them are not the list column: a test, and a stylesheet, needs one place to ask.
    <div className="app" data-place={place} data-stage={stage ?? undefined}>
      <Header />
      {/* The Feed and the Screener take the whole stage rather than a list column beside a pane,
          because their content is inline: a Feed card is already open and a Screener card is the
          message itself. Every other place is the list and the pane. */}
      {settings ? (
        <Settings />
      ) : stage === "contacts" ? (
        <Contacts />
      ) : stage === "clips" ? (
        <Clips />
      ) : stage === "files" ? (
        <Files />
      ) : stage === "focus-reply" ? (
        <FocusReply />
      ) : place === "feed" ? (
        <Feed />
      ) : place === "screener" ? (
        <Screener />
      ) : (
        <main className="stage">
          {onePage ? null : <ListColumn />}
          {pane || onePage ? <ReadingPane /> : null}
        </main>
      )}
      {/* The compose card floats over the whole stage and belongs to none of the screens under it,
          so it is mounted once here. Anywhere else and `c` would only work where that screen was:
          the Feed, the Screener and the libraries would each have a Write button that did nothing. */}
      <Compose />
      <ContactCards />
      <CommandPalette />
      <ShortcutsSheet open={overlay === "shortcuts"} onClose={closeOverlay} />
      <Tour />
      <Guide />
      {/* The corner button is over the window rather than in any screen, for the same reason the
          compose card is: help that was only reachable from the Inbox would be missing from the
          places somebody actually gets stuck in. */}
      <HelpButton />
      <Onboarding />
      <Arriving />
      <Toasts />
    </div>
  );
}

function App() {
  const route = useHashRoute();

  if (Kit && route === "#/kit") {
    return (
      <Suspense fallback={null}>
        <Kit />
      </Suspense>
    );
  }

  return <Shell />;
}

export default App;
