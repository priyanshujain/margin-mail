import { useEffect } from "react";
import { Button } from "../ui";
import { useEscapeLayer } from "../escape";
import { useAccounts } from "../store/useAccounts";
import { useMail } from "../store/useMail";
import { useSync } from "../store/useSync";
import { ConnectMail, ConnectMailServers } from "./ConnectMail";
import { spanOf, WindowChoice } from "./WindowChoice";
import { quietDoubleClick } from "./Header";
import { permissionName } from "./Settings";
import "./connect.css";

/**
 * The first screen, and the only one that is not the app.
 *
 * Four states on one stage: the welcome, the wait for a browser that may not have come to the
 * front, a consent page that came back without the permission all of this is built on, and the
 * first sync arriving. Two of those are not failures. Closing the consent tab and withholding a
 * scope are both answers, and the screen takes them as answers rather than apologising.
 *
 * The welcome itself is one field, the address, and it belongs to `ConnectMail`: that flow reads
 * the domain and works out whether the browser, a password or a sentence comes next. What this file
 * owns is the Google half once it has been handed over, which is `useAccounts`, and the one
 * judgement about the sync: when to stop showing it and show the mail.
 */
/** Phases a first sync passes through on its way to mail. */
const WORKING = new Set(["syncing", "hydrating", "backfilling"]);
/** Phases it stops on without mail, each of which has something to say for itself. */
const STOPPED = new Set(["error", "offline", "paused"]);

export function Connect() {
  const phase = useAccounts((s) => s.phase);
  const error = useAccounts((s) => s.error);
  const authUrl = useAccounts((s) => s.authUrl);
  const missing = useAccounts((s) => s.missingRequired);
  const pendingAccountId = useAccounts((s) => s.pendingAccountId);
  const pendingDays = useAccounts((s) => s.pendingDays);
  const starting = useAccounts((s) => s.starting);
  const startSync = useAccounts((s) => s.startSync);
  const connect = useAccounts((s) => s.connect);
  const cancelConnect = useAccounts((s) => s.cancelConnect);
  const openAuthUrl = useAccounts((s) => s.openAuthUrl);
  const copyAuthUrl = useAccounts((s) => s.copyAuthUrl);
  const finishConnect = useAccounts((s) => s.finishConnect);
  const statuses = useSync((s) => s.statuses);
  const retry = useSync((s) => s.run);
  const threads = useMail((s) => s.threads);

  // While the browser has the flow, Escape belongs to the flow. Nothing to escape from once the
  // mail is on its way: the sync does not stop because somebody pressed a key at it.
  useEscapeLayer(phase === "connecting" || phase === "refused", cancelConnect);

  const status = statuses.find((s) => s.accountId === pendingAccountId) ?? statuses[0] ?? null;
  const failed = status !== null && STOPPED.has(status.phase);
  // No status at all means the first pass has not reported yet, which is a kind of working.
  const working = status === null || WORKING.has(status.phase);

  // The Inbox arrives when the first page of threads does, not when the sync finishes: the rest
  // fills in behind it and nobody should have to watch that. A mailbox whose first pass finds
  // nothing at all has no first page to wait for, so a pass that ended hands over too.
  //
  // The handover is read off the phase alone rather than off having watched the phase change.
  // A pass that fails in its first second emits "syncing" and then its failure inside one React
  // batch, so a flag set on the way past is never set, and the screen used to sit on "Listing
  // your mail" for ever with the reason two layers down in a status nobody rendered.
  useEffect(() => {
    if (phase !== "syncing" || failed) return;
    if (threads.length > 0 || !working) void finishConnect();
  }, [phase, working, failed, threads.length, finishConnect]);

  return (
    <>
      {/* Empty, but it is still the strip under the traffic lights, and the window has to be
          draggable by it before there is an account to put in it. */}
      <header className="titlebar" data-tauri-drag-region onMouseDown={quietDoubleClick} />
      <main className="stage">
        <div className="welcome">
          <svg
            className="welcome-mark"
            width={54}
            height={54}
            viewBox="0 0 54 54"
            aria-hidden="true"
          >
            <rect className="welcome-plate" width="54" height="54" rx="13" />
            <rect className="welcome-glyph" x="14" y="18.5" width="26" height="17" rx="3.2" />
            <path className="welcome-glyph" d="m14.8 20.5 10.5 7.6a3 3 0 0 0 3.4 0l10.5-7.6" />
          </svg>

          {phase === "choosing" ? (
            <Choosing initial={30} busy={starting} onStart={(days) => void startSync(days)} />
          ) : null}

          {phase === "syncing" ? (
            failed ? (
              <Stalled
                status={status}
                onRetry={() => void retry(status?.accountId)}
                onSkip={() => void finishConnect()}
              />
            ) : (
              <Progress status={status} days={pendingDays} />
            )
          ) : null}

          {phase === "connecting" ? (
            <Waiting
              ready={authUrl !== null}
              onOpen={openAuthUrl}
              onCopy={() => void copyAuthUrl()}
              onCancel={cancelConnect}
            />
          ) : null}

          {phase === "refused" ? (
            <Refused scopes={missing} onRetry={() => void connect()} onCancel={cancelConnect} />
          ) : null}

          {phase === "idle" || phase === "error" ? (
            <ConnectMail
              onGoogle={(email) => void connect(email)}
              trouble={phase === "error" ? error : null}
            />
          ) : null}
        </div>
      </main>

      {/* The servers and the certificate question take the window rather than a place in the
          column, so they hang off the screen rather than out of the panel that opened them. The
          welcome column centres its text, and an overlay inside it would inherit that. */}
      <ConnectMailServers />
    </>
  );
}

function Waiting({
  ready,
  onOpen,
  onCopy,
  onCancel,
}: {
  ready: boolean;
  onOpen: () => void;
  onCopy: () => void;
  onCancel: () => void;
}) {
  return (
    <>
      <h1 className="welcome-title">Waiting for Google</h1>
      <p className="welcome-line">
        {ready
          ? "Sign in there and this screen comes back on its own. The browser that opened is not always the one in front of you: open the link again, or copy it into the one you are signed in to Google in."
          : "Building the sign-in link."}
      </p>

      <div className="welcome-actions">
        <Button disabled={!ready} onClick={onOpen}>
          Open link again
        </Button>
        <Button disabled={!ready} onClick={onCopy}>
          Copy link
        </Button>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>

      <p className="welcome-privacy">{PRIVACY}</p>
    </>
  );
}

/** Quoted in docs/ui.md and in the browser suite. This sentence is the promise the sign-in is
 *  asking to be trusted on, and it is said once, on the screen that is waiting for it. */
const PRIVACY =
  "Sign-in happens in your browser with Google. Margin never sees your password. The key Google " +
  "hands back is stored only on this device, and you can revoke it any time from your Google account.";

function Refused({
  scopes,
  onRetry,
  onCancel,
}: {
  scopes: string[];
  onRetry: () => void;
  onCancel: () => void;
}) {
  const names = scopes.map(permissionName).join(", ");
  return (
    <>
      <h1 className="welcome-title">One permission is missing</h1>
      <p className="welcome-line">
        Google came back without “{names}”, which is the one thing a mail client cannot work
        without, so no account was added and nothing was stored.
      </p>

      <div className="welcome-actions">
        <Button variant="primary" onClick={onRetry}>
          Try again
        </Button>
        <Button variant="ghost" onClick={onCancel}>
          Not now
        </Button>
      </div>

      <p className="welcome-privacy">
        Every other permission is optional and can be granted later from Settings. This is the only
        one the app is built on.
      </p>
    </>
  );
}

interface Arrived {
  accountId?: string;
  phase?: string;
  error?: string | null;
  hydrated: number;
  total: number;
  message: string | null;
}

/**
 * A first sync that stopped without any mail, and the reason it gave.
 *
 * The account is connected and its token is on disk by the time this can appear, so the reason is
 * almost never about the sign-in: it is the provider refusing a call, a network that is not there,
 * or the breaker having given up. Whatever it was, the provider's own sentence is shown verbatim,
 * because Google's refusals name the thing to fix and a sentence written here would not.
 *
 * Two ways on. Trying again is the usual one. Going in anyway is for a mailbox that is going to
 * stay broken for a while: the app works, the mirror is empty, and the account chip keeps saying
 * so. Being held on a screen with one button is worse than an empty Inbox that explains itself.
 */
function Stalled({
  status,
  onRetry,
  onSkip,
}: {
  status: Arrived | null;
  onRetry: () => void;
  onSkip: () => void;
}) {
  const offline = status?.phase === "offline";
  const said = status?.error ?? status?.message ?? null;

  return (
    <>
      <h1 className="welcome-title">{offline ? "No connection" : "Your mail did not arrive"}</h1>
      <p className="welcome-line">
        {offline
          ? "The account is connected and nothing was lost. The mail comes in as soon as there is a network."
          : "The account is connected and its permissions are stored. The mailbox itself refused the first request."}
      </p>

      {said ? <p className="welcome-trouble">{said}</p> : null}

      <div className="welcome-actions">
        <Button variant="primary" onClick={onRetry}>
          Try again
        </Button>
        <Button variant="ghost" onClick={onSkip}>
          Go in anyway
        </Button>
      </div>

      <p className="welcome-privacy">
        Nothing here is lost by waiting. The sync picks up where it stopped, and the account chip in
        the corner says so until it does.
      </p>
    </>
  );
}

/**
 * The one question between consent and the mail: how far back this device holds. The account is
 * written by now, so there is nothing to cancel and no way back; the answer starts the sync.
 */
function Choosing({
  initial,
  busy,
  onStart,
}: {
  initial: number;
  busy: boolean;
  onStart: (days: number) => void;
}) {
  return (
    <>
      <h1 className="welcome-title">How far back?</h1>
      <p className="welcome-line">
        Choose how much of your mail this device holds. The first sync brings in that much, newest
        first, and older mail stays where it is until you search for it.
      </p>
      <WindowChoice initial={initial} busy={busy} onStart={onStart} />
    </>
  );
}

function Progress({ status, days }: { status: Arrived | null; days: number }) {
  const total = status?.total ?? 0;
  const hydrated = Math.min(status?.hydrated ?? 0, total);
  const done = total > 0 ? hydrated / total : 0;

  return (
    <>
      <h1 className="welcome-title">{`Bringing in ${spanOf(days)}`}</h1>
      <p className="welcome-line">{status?.message ?? "Listing your mail"}</p>

      <div
        className="welcome-bar"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(done * 100)}
      >
        <span className="welcome-bar-fill" style={{ transform: `scaleX(${done})` }} />
      </div>

      <p className="welcome-count">
        {total > 0
          ? `${hydrated.toLocaleString()} of ${total.toLocaleString()} messages`
          : "Counting what is there"}
      </p>

      <p className="welcome-privacy">
        This becomes your Inbox as soon as the first mail lands. The rest arrives behind it.
      </p>
    </>
  );
}

export default Connect;
