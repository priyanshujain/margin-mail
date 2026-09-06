import { useEffect, useState } from "react";
import { Button, Sheet, Toggle } from "../ui";
import { askForNotifications, notifyTest } from "../api/notifications";
import { startFresh } from "../api/threads";
import { undoToken } from "../api/undo";
import type { Place } from "../ipc";
import { useAccounts } from "../store/useAccounts";
import { useMail } from "../store/useMail";
import { useOverlays } from "../store/useOverlays";
import { useSettings } from "../store/useSettings";
import { notify } from "../store/useToast";
import { NOTIFY_PLACES, placesSentence, withPlace } from "./notifyPlaces";
import "./onboarding.css";

const DAY_MS = 86_400_000;

/** A week by default, which is the age at which mail stops being something you are still on. */
const AGES = [
  { id: "day", label: "a day", days: 1 },
  { id: "week", label: "a week", days: 7 },
  { id: "month", label: "a month", days: 30 },
  { id: "quarter", label: "three months", days: 90 },
];

/**
 * Which accounts have already been shown this panel on this machine.
 *
 * A device fact rather than a roaming one, and deliberately not in the state database with the
 * decisions that follow a person around. The panel is about the sync that just finished here: it
 * counts what arrived on this disk and offers a bulk write against it. A second machine syncing
 * the same account has that story to tell for the first time too, and a flag that roamed would
 * leave its first run silent about a mailbox it has just filled from scratch.
 */
const KEY = "marginmail-onboarded";

function onboarded(): string[] {
  try {
    const stored = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(stored) ? stored.filter((id) => typeof id === "string") : [];
  } catch {
    return [];
  }
}

function remember(accountId: string): void {
  const all = onboarded();
  if (all.includes(accountId)) return;
  localStorage.setItem(KEY, JSON.stringify([...all, accountId]));
}

/**
 * The first run panel, over the Inbox it is talking about.
 *
 * It says what the pass over senders did, offers the one bulk write this app ever proposes, and
 * asks once whether new mail should say so. Skipping is Done: nothing here has to be answered, and
 * a panel that had to be dismissed in a particular way would be a wizard.
 */
export function Onboarding() {
  const onboarding = useAccounts((s) => s.onboarding);
  const dismiss = useAccounts((s) => s.dismissOnboarding);
  const [age, setAge] = useState("week");
  // `marking` is the bulk write on its way, which holds the panel open until it answers.
  const [phase, setPhase] = useState<"idle" | "marking" | "error">("idle");
  // Held here rather than read from the store, so the switches move the moment they are pressed
  // and stay put while the save is on its way.
  const [places, setPlaces] = useState<Place[]>(
    () => useSettings.getState().settings?.notifyPlaces ?? [],
  );
  const [asked, setAsked] = useState(false);

  const accountId = onboarding?.accountId ?? null;
  const shown = accountId !== null && !onboarded().includes(accountId);

  // An account that has already had its first run here has nothing to be told again, so the panel
  // never mounts rather than flashing and closing.
  useEffect(() => {
    if (accountId && onboarded().includes(accountId)) dismiss();
  }, [accountId, dismiss]);

  if (!onboarding || !shown) return null;

  // Whichever way the panel ends, the tour follows it: this is the one moment the app has somebody's
  // attention and almost nothing here works the way their last mail client did. It follows every
  // account rather than only the first, because the panel it follows is per account too and a
  // second mailbox on a shared machine is somebody else's first look at the app. Skip is the first
  // control on it and Escape is the same answer.
  const done = () => {
    remember(onboarding.accountId);
    dismiss();
    useOverlays.getState().show("tour");
  };

  const run = async () => {
    const days = AGES.find((a) => a.id === age)?.days ?? 7;
    setPhase("marking");
    try {
      const undo = await startFresh(onboarding.accountId, Date.now() - days * DAY_MS);
      void useMail.getState().load();
      notify(undo.label, {
        label: "Undo",
        keycap: "z",
        run: () => void undoToken(undo.token).then(() => useMail.getState().load()),
      });
      setPhase("idle");
      done();
    } catch (e) {
      setPhase("error");
      notify(`Could not mark that mail as seen: ${e}`);
    }
  };

  const marking = phase === "marking";

  const setPlace = async (place: Place, on: boolean) => {
    const next = withPlace(places, place, on);
    setPlaces(next);
    const store = useSettings.getState();
    // The settings may not have been read yet: this panel can be the first thing to want them.
    if (!store.settings) await store.load();
    await store.save(on ? { notifyPlaces: next, notifications: true } : { notifyPlaces: next });

    // The first time anything is turned on is the moment to find out whether the system will let
    // us: where that is a dialog, the sample follows once it is allowed. Once per panel: the answer
    // does not change between switches.
    if (on && !asked) {
      setAsked(true);
      if ((await askForNotifications()) === "granted") {
        notifyTest().catch(() => {});
      }
    }
  };

  return (
    <Sheet
      open
      title="You are set up"
      busy={marking}
      onClose={done}
      foot={
        <>
          <Button disabled={marking} onClick={() => void run()}>
            {marking ? "Marking" : "Start fresh"}
          </Button>
          <Button variant="primary" disabled={marking} onClick={done}>
            Done
          </Button>
        </>
      }
    >
      <div className="onboard" data-phase={phase}>
        <p className="onboard-lead">
          <b>{onboarding.screenedIn.toLocaleString()} senders</b> were screened in already, because
          you have written to them or they are in your contacts.
        </p>
        <p className="onboard-sub">
          From here on, anyone new waits in the Screener until you say where their mail goes. Nobody
          is told either way.
        </p>

        <div className="field">
          <span className="field-label">Start fresh</span>
          <p className="onboard-sub">
            Optional. Mark older mail as seen, so only what is recent reads as new. It is the only
            bulk write this app ever proposes, and it is reversible for seven days.
          </p>
          <div className="choice">
            <span className="choice-label">Older than</span>
            {AGES.map((option) => (
              <button
                key={option.id}
                type="button"
                className="choice-option"
                data-on={option.id === age ? "" : undefined}
                aria-pressed={option.id === age}
                disabled={marking}
                onClick={() => setAge(option.id)}
              >
                {option.label}
              </button>
            ))}
          </div>
        </div>

        <div className="field">
          <span className="field-label">Tell me when new mail arrives</span>
          <p className="onboard-sub">
            Optional, and off everywhere until you say. A thread or a person can be turned on later
            with ⇧N or from the contact card; this is what everything else falls back to.
          </p>
          <div className="onboard-toggles">
            {NOTIFY_PLACES.map((place) => (
              <Toggle
                key={place.id}
                checked={places.includes(place.id)}
                label={place.label}
                note={place.note}
                onChange={(on) => void setPlace(place.id, on)}
              />
            ))}
          </div>
          <p className="onboard-sub onboard-quiet">{placesSentence(places)}</p>
        </div>
      </div>
    </Sheet>
  );
}

export default Onboarding;
