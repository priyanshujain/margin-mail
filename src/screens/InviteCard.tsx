import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Button, Icon, icons } from "../ui";
import { registerCommands } from "../keys/commands";
import { useKeyContext } from "../keys/keymap";
import { inviteRespond } from "../api/write";
import { CALENDAR_SCOPE, isTauri, type Invite, type InviteResponse } from "../ipc";
import { useAccounts } from "../store/useAccounts";
import { useMail } from "../store/useMail";
import { notify } from "../store/useToast";
import { displayName } from "./format";
import "./invite.css";

/**
 * A `text/calendar` invitation, as a card in the pane.
 *
 * The interesting part is the permission. Answering needs the Calendar scope, the app does not ask
 * for it when an account is added, and Google has no incremental grant for an installed app: the
 * only way to get it is to run the whole consent again. So the card renders whatever the account
 * was given, read only when the scope is not there, with one sentence saying why and a Grant that
 * runs consent. docs/features.md promises exactly that, and a card that hid itself instead would
 * lose the date and the time as well as the buttons.
 *
 * `y`, `m` and `n` mean accept, maybe and decline while the card holds the focus, which is the one
 * exception docs/keyboard.md allows to keys never being reused. The card is the only thing that can
 * receive them, and it prints all three.
 */

const RESPONSES: { response: InviteResponse; label: string; keycap: string }[] = [
  { response: "accepted", label: "Accept", keycap: "y" },
  { response: "tentative", label: "Maybe", keycap: "m" },
  { response: "declined", label: "Decline", keycap: "n" },
];

const ANSWERED: Record<InviteResponse, string> = {
  accepted: "Going",
  tentative: "Maybe",
  declined: "Not going",
  "needs-action": "",
};

const month = new Intl.DateTimeFormat(undefined, { month: "short" });
const dayNumber = new Intl.DateTimeFormat(undefined, { day: "numeric" });
const longDay = new Intl.DateTimeFormat(undefined, {
  weekday: "long",
  day: "numeric",
  month: "long",
});
const clock = new Intl.DateTimeFormat(undefined, {
  hour: "2-digit",
  minute: "2-digit",
  hourCycle: "h23",
});

/** "Wednesday 10 September · 17:00 to 17:45 · Sunny Day Music, Bandra". */
function when(invite: Invite): string {
  const parts = [longDay.format(invite.startMs)];
  if (!invite.allDay) parts.push(`${clock.format(invite.startMs)} to ${clock.format(invite.endMs)}`);
  else parts.push("All day");
  if (invite.location) parts.push(invite.location);
  return parts.join(" · ");
}

/** Whether an error from `invite_respond` is the app being short a permission rather than a fault. */
const aboutTheScope = (message: string): boolean =>
  message.includes(CALENDAR_SCOPE) || /scope|permission|calendar/i.test(message);

interface InviteCardProps {
  invite: Invite;
  /** The provider's message id, which is what `invite_respond` takes. */
  messageId: string;
  accountId: string;
}

export function InviteCard({ invite, messageId, accountId }: InviteCardProps) {
  const accounts = useAccounts((s) => s.accounts);
  const grant = useAccounts((s) => s.grant);
  const [answer, setAnswer] = useState<InviteResponse>(invite.myResponse);
  const [phase, setPhase] = useState<"idle" | "sending" | "no-scope">("idle");
  const [focused, setFocused] = useState(false);

  useEffect(() => setAnswer(invite.myResponse), [invite.myResponse, invite.uid]);

  const account = accounts.find((a) => a.id === accountId);
  // What Google actually granted, because a person can untick a scope on the consent screen and a
  // feature that assumed otherwise would offer a button that always fails.
  const granted = account?.grantedScopes.includes(CALENDAR_SCOPE) ?? false;
  const readOnly = !granted || phase === "no-scope";

  const respond = (response: InviteResponse) => {
    if (readOnly || phase === "sending") return;
    const before = answer;
    setAnswer(response);
    setPhase("sending");
    void inviteRespond(messageId, response)
      .then(() => {
        setPhase("idle");
        notify(`${ANSWERED[response]} · ${invite.summary}`);
        void useMail.getState().open(useMail.getState().openKey ?? undefined);
      })
      .catch((e) => {
        setAnswer(before);
        const message = String(e);
        // The one failure that is not a failure: the app is short a scope and can ask for it.
        if (aboutTheScope(message)) setPhase("no-scope");
        else {
          setPhase("idle");
          notify(`That did not go through: ${message}`);
        }
      });
  };

  // `y`, `m` and `n` are the card's only while the card has the focus. The handler is reached
  // through a ref so that the registration is one push and one pop rather than one of each on every
  // keystroke that changes the answer.
  const latest = useRef(respond);
  latest.current = respond;
  useKeyContext("invite", focused && !readOnly);
  useEffect(() => {
    if (!focused || readOnly) return;
    return registerCommands({
      "invite-accept": () => latest.current("accepted"),
      "invite-maybe": () => latest.current("tentative"),
      "invite-decline": () => latest.current("declined"),
    });
  }, [focused, readOnly]);

  const open = () => {
    if (!invite.calendarLink) return;
    if (!isTauri) window.open(invite.calendarLink, "_blank", "noopener,noreferrer");
    else openUrl(invite.calendarLink).catch((e) => notify(`Could not open the calendar: ${e}`));
  };

  return (
    <div
      className="invite"
      tabIndex={0}
      role="group"
      aria-label={`Invitation: ${invite.summary}`}
      data-focus={focused ? "" : undefined}
      onFocus={() => setFocused(true)}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFocused(false);
      }}
    >
      <div className="invite-date">
        <span className="mon">{month.format(invite.startMs)}</span>
        <span className="day">{dayNumber.format(invite.startMs)}</span>
      </div>
      <div className="invite-main">
        <div className="invite-title">{invite.summary}</div>
        <div className="invite-when">{when(invite)}</div>
        {invite.organizer ? (
          <div className="invite-organizer">{`Organised by ${displayName(invite.organizer)}`}</div>
        ) : null}

        {readOnly ? (
          <p className="invite-scope">
            Answering an invitation needs Google Calendar, which this account has not given Margin
            yet.
          </p>
        ) : null}

        <div className="invite-actions">
          {readOnly ? (
            <Button variant="primary" onClick={() => void grant(accountId, [CALENDAR_SCOPE])}>
              Grant
            </Button>
          ) : (
            RESPONSES.map(({ response, label, keycap }) => (
              <Button
                key={response}
                variant={answer === response ? "primary" : "default"}
                keycap={keycap}
                active={answer === response}
                disabled={phase === "sending"}
                onClick={() => respond(response)}
              >
                {label}
              </Button>
            ))
          )}
          {answer !== "needs-action" && !readOnly ? (
            <span className="invite-answered">
              <Icon d={icons.CHECK} size={12} />
              {ANSWERED[answer]}
            </span>
          ) : null}
          {invite.calendarLink ? (
            <button type="button" className="invite-open" onClick={open}>
              Open in Margin Calendar
            </button>
          ) : null}
        </div>
      </div>
    </div>
  );
}

export default InviteCard;
