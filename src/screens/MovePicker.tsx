import { useEffect, useMemo, useRef, useState } from "react";
import { Key, Sheet } from "../ui";
import type { Destination } from "../ipc";
import { useKeyContext } from "../keys/keymap";
import { useMail } from "../store/useMail";
import { useMove } from "../store/useMove";
import * as triage from "./triage";

const PLACES: [Destination, string][] = [
  ["inbox", "Inbox"],
  ["feed", "Feed"],
  ["paper-trail", "Paper Trail"],
];

/**
 * `v`. Where to, and then whether that is this thread or everything from its sender. The place
 * the list is already showing is left out, because moving a thread to where it is moves nothing.
 */
export function MovePicker() {
  const keys = useMove((s) => s.keys);
  const hide = useMove((s) => s.hide);
  if (keys.length === 0) return null;
  return <Picker keys={keys} onClose={hide} />;
}

function Picker({ keys, onClose }: { keys: string[]; onClose: () => void }) {
  const place = useMail((s) => s.place);
  const threads = useMail((s) => s.threads);
  const open = useMail((s) => s.thread);
  const [to, setTo] = useState<[Destination, string] | null>(null);
  const list = useRef<HTMLUListElement | null>(null);

  useKeyContext("overlay");

  const places = PLACES.filter(([id]) => id !== place);

  const senders = useMemo(() => {
    const wanted = new Set(keys);
    const found = threads.filter((t) => wanted.has(t.key)).map((t) => t.from.address.toLowerCase());
    const last = open?.messages.at(-1);
    if (found.length === 0 && open && last && wanted.has(open.key)) found.push(last.from.address.toLowerCase());
    return [...new Set(found)];
  }, [keys, threads, open]);

  const scopes: [boolean, string][] = [
    [false, keys.length === 1 ? "This thread" : `These ${keys.length} threads`],
    [
      true,
      senders.length === 1
        ? `Everything from ${senders[0]}`
        : `Everything from these ${senders.length} senders`,
    ],
  ];

  const move = (destination: Destination, everything: boolean) => {
    onClose();
    void triage.moveTo(keys, destination, everything);
  };

  const rows: { label: string; run: () => void }[] = to
    ? scopes.map(([everything, label]) => ({ label, run: () => move(to[0], everything) }))
    : places.map((entry) => ({ label: entry[1], run: () => setTo(entry) }));

  useEffect(() => {
    list.current?.querySelector<HTMLButtonElement>(".label-option")?.focus();
  }, [to]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.isComposing || e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        const buttons = [...(list.current?.querySelectorAll<HTMLButtonElement>(".label-option") ?? [])];
        const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
        const delta = e.key === "ArrowDown" ? 1 : -1;
        buttons[(at + delta + buttons.length) % buttons.length]?.focus();
        e.preventDefault();
        return;
      }
      const row = rows[Number(e.key) - 1];
      if (!row) return;
      e.preventDefault();
      e.stopPropagation();
      row.run();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });

  return (
    <Sheet
      open
      size="mini"
      title={to ? `Move to ${to[1]}` : "Move to"}
      onClose={onClose}
      onBack={to ? () => setTo(null) : undefined}
      backLabel={to ? "the places" : undefined}
    >
      <ul className="label-list" ref={list}>
        {rows.map((row, at) => (
          <li key={row.label}>
            <button type="button" className="label-option" onClick={row.run}>
              <span className="label-name">{row.label}</span>
              <Key size="sm">{String(at + 1)}</Key>
            </button>
          </li>
        ))}
      </ul>
    </Sheet>
  );
}

export default MovePicker;
