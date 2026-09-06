import { useCallback, useEffect } from "react";
import { Button, Sheet } from "../ui";
import { useAccounts } from "../store/useAccounts";
import { useMail } from "../store/useMail";
import { useSettings } from "../store/useSettings";
import { arrival, useSync } from "../store/useSync";
import { spanOf, WindowChoice } from "./WindowChoice";
import "./arriving.css";

/**
 * The panel over the window while an account added from inside the app brings its mail in.
 *
 * The welcome screen has its own stage for this wait, because there is nothing behind it. Here
 * there is: Settings, or the Inbox of another account, and the account that has just been added
 * used to land under them as an empty Inbox with one line in the header saying why. This takes the
 * window instead. First the one question, how far back this device holds, because the first sync
 * reads the answer; then the engine's own sentence, a bar and the count, and nothing else to do:
 * the close control and Escape are off, and the scrim is not a way out. It ends when the engine
 * says the mail is in, and what is behind it then is that account's Inbox.
 *
 * A first pass that stopped is the one thing it has to offer a way out of. The provider's own
 * sentence, Try again, and Go in anyway: an account that is going to stay broken for a while is
 * still an account, and the Inbox says what it has.
 */
export function Arriving() {
  const arriving = useAccounts((s) => s.arriving);
  const arrived = useAccounts((s) => s.arrived);
  const seedScreener = useAccounts((s) => s.seedScreener);
  const starting = useAccounts((s) => s.starting);
  const startSync = useAccounts((s) => s.startSync);
  const statuses = useSync((s) => s.statuses);
  const retry = useSync((s) => s.run);

  const status = arriving
    ? (statuses.find((s) => s.accountId === arriving.accountId) ?? null)
    : null;
  // Nothing has started until the window is chosen, whatever an older status for the id says.
  const state = arriving?.started ? arrival(status) : "working";

  const handOver = useCallback(() => {
    if (!arriving) return;
    const { accountId } = arriving;
    arrived();
    useSettings.getState().close();
    useMail.getState().goTo("inbox");
    useMail.getState().setAccount(accountId);
    // Everyone the account already knows is screened in by the pass that finished, and this is
    // what puts the number on the first-run panel. Asked after the hand-over, so a mirror that
    // is not ready yet (Go in anyway) is asked again when its sync reports idle.
    void seedScreener(accountId);
  }, [arriving, arrived, seedScreener]);

  useEffect(() => {
    if (arriving?.started && state === "done") handOver();
  }, [arriving, state, handOver]);

  if (!arriving) return null;

  const total = status?.total ?? 0;
  const hydrated = Math.min(status?.hydrated ?? 0, total);
  const done = total > 0 ? hydrated / total : 0;
  const stalled = state === "stalled";
  const offline = status?.phase === "offline";

  return (
    <Sheet
      open
      title={`Bringing in ${arriving.email}`}
      busy
      onClose={() => {}}
      foot={
        stalled ? (
          <>
            <Button variant="ghost" onClick={handOver}>
              Go in anyway
            </Button>
            <Button
              variant="primary"
              data-autofocus
              onClick={() => void retry(arriving.accountId)}
            >
              Try again
            </Button>
          </>
        ) : undefined
      }
    >
      <div className="arrive" data-state={arriving.started ? state : "choosing"}>
        {!arriving.started ? (
          <>
            <p className="arrive-line">How far back should this device hold?</p>
            <WindowChoice
              initial={arriving.days}
              busy={starting}
              onStart={(days) => void startSync(days)}
            />
          </>
        ) : stalled ? (
          <>
            <p className="arrive-line">
              {offline
                ? "No connection. The account is connected and nothing was lost; the mail comes in as soon as there is a network."
                : "The account is connected and its sign-in is stored. The mailbox itself refused the first request."}
            </p>
            {status?.error ? <p className="arrive-trouble">{status.error}</p> : null}
          </>
        ) : (
          <>
            <p className="arrive-line">{status?.message ?? "Listing your mail"}</p>
            <div
              className="arrive-bar"
              data-counting={total > 0 ? undefined : ""}
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={total > 0 ? Math.round(done * 100) : undefined}
            >
              <span
                className="arrive-fill"
                style={total > 0 ? { transform: `scaleX(${done})` } : undefined}
              />
            </div>
            <p className="arrive-count">
              {total > 0
                ? `${hydrated.toLocaleString()} of ${total.toLocaleString()} messages`
                : "Counting what is there"}
            </p>
            <p className="arrive-quiet">
              {`Bringing in ${spanOf(arriving.days)}, newest first. Nothing to do here: this closes on its own and opens the account’s Inbox once the mail is in.`}
            </p>
          </>
        )}
      </div>
    </Sheet>
  );
}

export default Arriving;
