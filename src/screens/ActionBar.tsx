import { useEffect, useMemo } from "react";
import { Button, Icon, icons, Key, Sheet } from "../ui";
import { runCommand } from "../keys/commands";
import { useKeyContext } from "../keys/keymap";
import type { CommandId } from "../keys/bindings";
import type { ThreadSummary } from "../ipc";
import { useMail } from "../store/useMail";
import { useSelection } from "../store/useSelection";
import { cap } from "./format";
import * as triage from "./triage";
import "./actionbar.css";

/**
 * The bar that takes the piles' place while a selection exists, carrying the same verbs as the
 * keyboard and printing the same keys, because this is where the mouse teaches the keyboard.
 *
 * Every button runs the command rather than the function behind it, so the button and the key are
 * one code path and both act on the selection the same way.
 *
 * `l`, `s`, `b` and `g` all act on a selection now and none of them has a button here yet, which
 * is a gap rather than a decision: docs/features.md section 13 lists them, and six verbs is what
 * fits in three across and two down inside the piles' height. Adding four more is a taller
 * footprint for both this and the piles, and that measurement is asserted in two places.
 */
interface Verb {
  command: CommandId;
  /**
   * A toggle says which way it is about to go, read off the rows it would act on, the same way the
   * More menu's rows do. Over a mixed selection that is the direction most of it has not gone yet.
   */
  label: string | ((rows: ThreadSummary[]) => string);
  icon: string;
}

/**
 * The verbs, in the order docs/features.md section 13 lists them.
 *
 * Six of them, in the piles' footprint, which is two rows of three. The section lists ten, and the
 * width is what decides: the list column is 420 pixels and a button here is an icon, a word and a
 * keycap, which is four to a row before the last one is clipped. So the six that lead the section
 * get a button and the other four keep their keys, which work on a selection exactly the same way.
 * A bar that clipped its own labels would be worse than a bar that shows fewer of them.
 *
 * Every button runs the command rather than a function of its own, so a button and a keystroke are
 * one path and cannot come to mean different things.
 */
const VERBS: Verb[] = [
  { command: "reply-later", label: "Reply later", icon: icons.CLOCK },
  { command: "set-aside", label: "Set aside", icon: icons.SET_ASIDE },
  { command: "snooze", label: "Snooze", icon: icons.SNOOZE },
  { command: "toggle-seen", label: "Mark seen", icon: icons.ENVELOPE },
  { command: "archive", label: "Archive", icon: icons.ARCHIVE },
  {
    command: "trash",
    label: (rows) => (rows.length > 0 && rows.every((t) => t.trashed) ? "Put back" : "Trash"),
    icon: icons.TRASH,
  },
];

export function ActionBar() {
  const selected = useSelection((s) => s.keys);
  const clear = useSelection((s) => s.clear);
  const threads = useMail((s) => s.threads);

  const rows = useMemo(() => {
    const wanted = new Set(selected);
    return threads.filter((thread) => wanted.has(thread.key));
  }, [threads, selected]);

  return (
    <div className="action-bar">
      <div className="action-head">
        <span className="action-count">{`${selected.length} selected`}</span>
        <button type="button" className="action-clear" onClick={clear}>
          Clear
          <Key size="sm">⎋</Key>
        </button>
      </div>

      <div className="action-verbs">
        {VERBS.map((verb) => (
          <Button
            key={verb.command}
            variant="ghost"
            size="sm"
            icon={verb.icon}
            keycap={cap(verb.command)}
            onClick={() => runCommand(verb.command)}
          >
            {typeof verb.label === "function" ? verb.label(rows) : verb.label}
          </Button>
        ))}
      </div>
    </div>
  );
}

export type PickerMode = "apply" | "move";

interface PickerProps {
  mode: PickerMode | null;
  /** The threads the picked label lands on. */
  keys: string[];
  onClose: () => void;
}

/**
 * The provider's labels, as a list to pick one from. `Shift+L` applies or removes; `v` in a label
 * list moves, which to a provider with labels means applying one and archiving.
 *
 * A row cannot say which labels it carries, so a tick is only shown when the thread it belongs to
 * is the one open in the pane, which is the only place the app knows. Picking an untold label
 * applies it, which is the direction somebody pressing `Shift+L` meant.
 */
export function LabelPicker({ mode, keys, onClose }: PickerProps) {
  const labels = useMail((s) => s.labels);
  const labelsPhase = useMail((s) => s.labelsPhase);
  const loadLabels = useMail((s) => s.loadLabels);
  const accountId = useMail((s) => s.accountId);
  const openKey = useMail((s) => s.openKey);
  const thread = useMail((s) => s.thread);

  useEffect(() => {
    if (mode) void loadLabels();
  }, [mode, accountId, loadLabels]);

  // The sheet is in front, so the view's keys stand back until it closes.
  useKeyContext("overlay", mode !== null);

  const applied = useMemo(() => {
    const one = keys.length === 1 && keys[0] === openKey;
    return new Set(one ? (thread?.labels ?? []) : []);
  }, [keys, openKey, thread]);

  if (!mode) return null;

  const choose = (id: string) => {
    onClose();
    if (mode === "move") void triage.move(keys, id);
    else void triage.label(keys, id, !applied.has(id));
  };

  return (
    <Sheet
      open
      size="mini"
      title={mode === "move" ? "Move to a label" : "Label"}
      onClose={onClose}
    >
      <ul className="label-list">
        {labels.map((label) => (
          <li key={label.id}>
            <button type="button" className="label-option" onClick={() => choose(label.id)}>
              <span className="label-name">{label.name}</span>
              {applied.has(label.id) ? <Icon d={icons.CHECK} size={14} /> : null}
            </button>
          </li>
        ))}
        {/* An empty list means none only once a read has come back. While one is out, or after
            one failed, saying "no labels" would be answering a question nobody has asked yet. */}
        {labels.length === 0 ? (
          labelsPhase === "loading" ? (
            <li className="label-none" data-state="loading">
              Reading your labels
            </li>
          ) : labelsPhase === "error" ? (
            <li className="label-none" data-state="error">
              <span>Could not read your labels.</span>
              <Button size="sm" variant="ghost" onClick={() => void loadLabels()}>
                Try again
              </Button>
            </li>
          ) : (
            <li className="label-none">No labels on this account</li>
          )
        ) : null}
      </ul>
    </Sheet>
  );
}

export default ActionBar;
