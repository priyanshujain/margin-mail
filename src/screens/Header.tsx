import { useRef, useState, type MouseEvent } from "react";
import {
  Avatar,
  Button,
  Icon,
  icons,
  Key,
  Popover,
  Segment,
  type SegmentOption,
} from "../ui";
import { keyLabel } from "../keys/bindings";
import { useAccounts } from "../store/useAccounts";
import { countsFor, useCounts } from "../store/useCounts";
import { useMail } from "../store/useMail";
import { useOverlays } from "../store/useOverlays";
import { useSettings } from "../store/useSettings";
import { busy, trouble, useSync } from "../store/useSync";
import { runCommand } from "../keys/commands";
import { accountHue, displayName } from "./format";
import { SearchBar } from "./SearchBar";
import type { Place } from "../ipc";
import "./header.css";

/** The three boxes, and the only navigation that is on screen without being asked for. */
const BOX_LIST: SegmentOption[] = [
  { id: "inbox", label: "Inbox", keycap: "1" },
  { id: "feed", label: "Feed", keycap: "2" },
  { id: "paper-trail", label: "Paper Trail", keycap: "3" },
];

/**
 * On macOS Tauri decides the zoom on release rather than on the second press, so it leaves that
 * press alone and WebKit selects the word under it. Only a press that landed on the drag region
 * itself is quietened: a double-click in the search box still has a word to pick.
 */
export function quietDoubleClick(e: MouseEvent<HTMLElement>) {
  if (
    e.detail >= 2 &&
    e.target instanceof HTMLElement &&
    e.target.hasAttribute("data-tauri-drag-region")
  ) {
    e.preventDefault();
  }
}

function Screening({ count }: { count: number }) {
  if (count === 0) return null;
  return <span className="account-screening">{`${count} to screen`}</span>;
}

function Waiting({ counts }: { counts: ReturnType<typeof countsFor> }) {
  return (
    <span className="account-waiting">
      {counts.feed ? <span className="segment-dot" data-tone="quiet" title="New in the Feed" /> : null}
      {counts.paperTrail ? (
        <span className="segment-dot" data-tone="strong" title="New in the Paper Trail" />
      ) : null}
      {counts.inbox > 0 ? <span className="segment-count">{counts.inbox}</span> : null}
    </span>
  );
}

export function Header() {
  const place = useMail((s) => s.place);
  const accountId = useMail((s) => s.accountId);
  const setAccount = useMail((s) => s.setAccount);
  const goTo = useMail((s) => s.goTo);
  const accounts = useAccounts((s) => s.accounts);
  const statuses = useSync((s) => s.statuses);
  const syncing = useSync((s) => s.phase);
  const show = useOverlays((s) => s.show);
  const showSettings = useSettings((s) => s.show);
  const settingsOpen = useSettings((s) => s.open);
  const closeSettings = useSettings((s) => s.close);

  const [switcher, setSwitcher] = useState(false);
  const chip = useRef<HTMLButtonElement | null>(null);

  const counts = useCounts((s) => s.counts);
  const here = countsFor(counts, accountId);
  const boxes = BOX_LIST.map((box) =>
    box.id === "inbox"
      ? { ...box, count: here.inbox }
      : box.id === "feed"
        ? { ...box, dot: here.feed ? ("quiet" as const) : undefined }
        : { ...box, dot: here.paperTrail ? ("strong" as const) : undefined },
  );
  const elsewhere =
    accountId !== null &&
    counts.some((c) => c.accountId !== accountId && (c.inbox > 0 || c.screener > 0));

  const current = accounts.find((a) => a.id === accountId) ?? null;
  const note = trouble(statuses, accountId);
  // The engine's own sentence when it has one. A pass somebody asked for by hand mostly has none:
  // it lists, finds nothing new and stops, and for those seconds the only sign it is running at all
  // was the network light. So the ask itself gets a line, in the same slot, until it comes back.
  const working =
    busy(statuses, accountId) ?? (syncing === "syncing" ? "Checking for mail" : null);

  return (
    // The traffic lights float over the left end of this row, so the row is what drags the window
    // and zooms it on a double-click, and none of the controls in it do: a button that also drags
    // swallows its own click. The lanes and the status line carry the attribute as well, because
    // that is where the empty space actually is, and a bare region only answers to a press that
    // lands on it directly.
    <header className="titlebar" data-tauri-drag-region onMouseDown={quietDoubleClick}>
      <div className="titlebar-lead" data-tauri-drag-region>
        <button
          type="button"
          className="account-chip"
          ref={chip}
          aria-haspopup="dialog"
          aria-expanded={switcher}
          onClick={() => setSwitcher((was) => !was)}
        >
          {current ? (
            <Avatar name={current.name} address={current.email} hue={accountHue(current.color)} size="xs" />
          ) : (
            <Icon d={icons.PILE} size={16} />
          )}
          {current ? current.email : "All accounts"}
          {elsewhere ? <span className="account-elsewhere" aria-label="New mail in another account" /> : null}
          <Icon d={icons.CHEVRON_DOWN} size={12} />
        </button>

        {/* One line, never two, in the lane the account chip is already in: the boxes are centred
            in their own grid column and the buttons are right-aligned in theirs, so this can grow
            and shrink all day without moving anything a hand is aiming at. What is wrong wins over
            what is happening, and neither is ever a spinner, a bar or something to click. */}
        {note ? (
          <span className="sync-note" data-tauri-drag-region>
            {note}
          </span>
        ) : working ? (
          <span className="sync-busy" data-tauri-drag-region>
            {working}
          </span>
        ) : null}
      </div>

      {/* Going to a box leaves Settings, because Settings is a place and you cannot be in two.
          The keyboard already knew that: the place commands in App.tsx close it. This did not, so
          clicking Inbox from Settings changed the place underneath and left Settings on top of it,
          with no way out but the keyboard. While Settings is up nothing here is selected, because
          claiming Inbox is selected under a screen that is not the Inbox is the same lie. */}
      <Segment
        options={boxes}
        value={settingsOpen ? "" : place}
        label="Boxes"
        onChange={(id) => {
          closeSettings();
          goTo(id as Place);
        }}
      />

      <div className="titlebar-trail" data-tauri-drag-region>
        <SearchBar />
        <Button
          variant="ghost"
          iconOnly
          icon={icons.PLACES}
          title={`Places (${keyLabel("cmd+k")})`}
          onClick={() => show("palette")}
        />
        <Button variant="primary" icon={icons.PEN} keycap="c" onClick={() => runCommand("compose")}>
          Write
        </Button>
      </div>

      <Popover
        open={switcher}
        anchor={chip.current}
        onClose={() => setSwitcher(false)}
        label="Accounts"
        width={280}
      >
        <ul className="account-list">
          {accounts.map((account, index) => (
            <li key={account.id}>
              <button
                type="button"
                className="account-option"
                data-active={account.id === accountId ? "" : undefined}
                onClick={() => {
                  setAccount(account.id);
                  setSwitcher(false);
                }}
              >
                <Avatar name={account.name} address={account.email} hue={accountHue(account.color)} size="sm" />
                <span className="account-who">
                  <span className="account-name">{displayName({ name: account.name, address: account.email })}</span>
                  <span className="account-address">{account.email}</span>
                  <Screening count={countsFor(counts, account.id).screener} />
                </span>
                <Waiting counts={countsFor(counts, account.id)} />
                <Key>{keyLabel(`ctrl+${index + 1}`)}</Key>
              </button>
            </li>
          ))}
          <li>
            <button
              type="button"
              className="account-option"
              data-active={accountId === null ? "" : undefined}
              onClick={() => {
                setAccount(null);
                setSwitcher(false);
              }}
            >
              <Icon d={icons.PILE} size={20} />
              <span className="account-who">
                <span className="account-name">All accounts</span>
                <span className="account-address">Every mailbox in one list</span>
                <Screening count={countsFor(counts, null).screener} />
              </span>
              <Waiting counts={countsFor(counts, null)} />
              <Key>{keyLabel("ctrl+0")}</Key>
            </button>
          </li>
        </ul>

        {/* docs/settings.md names four ways in and this is one of them. Not an account option:
            it goes somewhere rather than switching what the list is showing. */}
        <button
          type="button"
          className="account-settings"
          onClick={() => {
            setSwitcher(false);
            showSettings();
          }}
        >
          <span>Settings</span>
          <Key>{keyLabel("cmd+,")}</Key>
        </button>
      </Popover>
    </header>
  );
}

export default Header;
