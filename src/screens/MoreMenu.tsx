import { useEffect, useMemo, useRef } from "react";
import { Key, Popover } from "../ui";
import { isRegistered, runCommand } from "../keys/commands";
import { keysFor, normalizeCombo, type CommandId } from "../keys/bindings";
import { comboOf, useKeyContext } from "../keys/keymap";
import { useMail } from "../store/useMail";
import { cap } from "./format";
import "./more.css";

/**
 * `.`. The rest of the thread's verbs, behind the last button on the pane's bar.
 *
 * The bar carries the five a hand reaches for and this carries the others, in the order
 * docs/keyboard.md lists them, each printed with its key. A row runs the command and nothing else,
 * so the row, the key and the palette are one code path and cannot come to mean different things.
 *
 * A verb nobody owns right now is left out rather than greyed: `v` means a label move only in a
 * label list, `Cmd+U` is the Feed's, and a menu that showed them dimmed everywhere else would be a
 * menu you had to read twice. The registry is asked when the menu opens, which is the moment the
 * answer is about.
 */

interface RowFlags {
  unseen: boolean;
  starred: boolean;
  trashed: boolean;
  spam: boolean;
}

interface MoreVerb {
  command: CommandId;
  label: string | ((flags: RowFlags) => string);
}

const VERBS: readonly MoreVerb[] = [
  { command: "reply-all", label: "Reply all" },
  { command: "forward", label: "Forward" },
  { command: "toggle-seen", label: (f) => (f.unseen ? "Mark seen" : "Mark unseen") },
  { command: "toggle-star", label: (f) => (f.starred ? "Unstar" : "Star") },
  { command: "note", label: "Add a note" },
  { command: "contact-card", label: "Contact card" },
  { command: "label", label: "Label" },
  { command: "move", label: "Move to a label" },
  { command: "trash", label: (f) => (f.trashed ? "Put back" : "Trash") },
  { command: "spam", label: (f) => (f.spam ? "Not spam" : "Mark as spam") },
];

export interface MoreMenuProps {
  open: boolean;
  /** The More button, which is what the menu hangs from. */
  anchor: HTMLElement | null;
  onClose: () => void;
}

export function MoreMenu({ open, anchor, onClose }: MoreMenuProps) {
  if (!open) return null;
  return (
    <Popover open anchor={anchor} onClose={onClose} placement="bottom-end" width={240} label="More">
      <MoreList onClose={onClose} />
    </Popover>
  );
}

/**
 * The list itself, as its own component because the popover draws its children a frame after it
 * is asked to open, once it knows where the anchor is. Focusing the first row and asking the
 * registry both belong to the moment the rows exist, which is this component's mount.
 */
function MoreList({ onClose }: { onClose: () => void }) {
  const list = useRef<HTMLUListElement | null>(null);
  const threads = useMail((s) => s.threads);
  const openKey = useMail((s) => s.openKey);
  const thread = useMail((s) => s.thread);

  // Every toggle here says which way it is about to go rather than "toggle". The row is asked
  // first because the verbs patch the row and not the view, so it is the row that is current; the
  // view answers for a thread opened from somewhere with no row behind it.
  const flags = useMemo<RowFlags>(() => {
    const row = threads.find((t) => t.key === openKey);
    return {
      unseen: row?.unseen ?? false,
      starred: row?.starred ?? thread?.starred ?? false,
      trashed: row?.trashed ?? thread?.trashed ?? false,
      spam: row?.spam ?? thread?.spam ?? false,
    };
  }, [threads, openKey, thread]);

  const shown = useMemo(() => VERBS.filter((v) => isRegistered(v.command)), []);

  // In front of the pane, so `u` below means the row here and not the list's key behind it.
  useKeyContext("overlay");

  useEffect(() => {
    list.current?.querySelector<HTMLButtonElement>(".more-option")?.focus();
  }, []);

  const choose = (command: CommandId) => {
    onClose();
    runCommand(command);
  };

  useEffect(() => {
    const step = (delta: number) => {
      const rows = [...(list.current?.querySelectorAll<HTMLButtonElement>(".more-option") ?? [])];
      if (rows.length === 0) return;
      const at = rows.indexOf(document.activeElement as HTMLButtonElement);
      const next = at === -1 ? (delta > 0 ? 0 : rows.length - 1) : (at + delta + rows.length) % rows.length;
      rows[next].focus();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.isComposing || e.key === "Escape") return;
      const combo = comboOf(e);
      if (combo === "ArrowDown" || combo === "ArrowUp") {
        e.preventDefault();
        step(combo === "ArrowDown" ? 1 : -1);
        return;
      }
      // The key that opened it closes it.
      if (combo === ".") {
        e.preventDefault();
        onClose();
        return;
      }
      // A verb's own key works from inside the menu, because the menu is where somebody learns it.
      const verb = shown.find((v) => keysFor(v.command).some((k) => normalizeCombo(k) === combo));
      if (!verb) return;
      e.preventDefault();
      e.stopPropagation();
      choose(verb.command);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });

  return (
    <div className="more-menu">
      {shown.length === 0 ? (
        <p className="more-empty">Nothing more to do here</p>
      ) : (
        <ul className="more-list" role="menu" ref={list}>
          {shown.map((verb) => {
            const key = cap(verb.command);
            return (
              <li key={verb.command} role="none">
                <button
                  type="button"
                  role="menuitem"
                  className="more-option"
                  onClick={() => choose(verb.command)}
                >
                  <span className="more-label">
                    {typeof verb.label === "function" ? verb.label(flags) : verb.label}
                  </span>
                  {key ? <Key size="sm">{key}</Key> : null}
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
