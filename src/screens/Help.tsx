import { useEffect, useRef, useState } from "react";
import { Button, Icon, Key, Popover, icons } from "../ui";
import { runCommand } from "../keys/commands";
import { labelFor, type CommandId } from "../keys/bindings";
import { comboOf, useKeyContext } from "../keys/keymap";
import { useCompose } from "../store/useCompose";
import { useOverlays } from "../store/useOverlays";
import { cap } from "./format";
import "./help.css";

/**
 * The question mark in the bottom right corner, and the small menu behind it.
 *
 * It is the only permanent chrome the app has, so it is a ghost button on the paper rather than the
 * coloured bubble every support widget in the world is: somebody who never needs it should be able
 * to work all day without noticing it is there.
 *
 * Three rows and each of them is a command, so the menu, the palette and the keyboard are one code
 * path. The labels come off the binding table for the same reason the shortcut sheet does, which is
 * that a menu with its own copy of them is a menu that will one day disagree with the palette.
 */

interface HelpRow {
  command: CommandId;
  icon: string;
}

const ROWS: readonly HelpRow[] = [
  { command: "tour", icon: icons.COMPASS },
  { command: "guide", icon: icons.BOOK },
  { command: "shortcuts", icon: icons.HELP },
];

export function HelpButton() {
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement | null>(null);
  // Whether there is a card rather than which card, so a draft being typed into is not a render of
  // this button per keystroke.
  const writing = useCompose((s) => s.card !== null);
  const overlay = useOverlays((s) => s.open);

  // Two ways to be in the way. The compose card takes this exact corner and sits above it, and an
  // overlay has the window and would leave a button floating over its scrim, which covers the guide
  // and the tour as well since both are panels. The phone is the third: the tab bar owns the corner
  // there, and that one is a rule in the stylesheet rather than a branch here.
  const hidden = writing || overlay !== null;

  // A row opens something that hides the button, and the menu goes with it. Without this the menu
  // would be back, still open, the moment the thing it opened was closed.
  useEffect(() => {
    if (hidden) setOpen(false);
  }, [hidden]);

  const close = () => setOpen(false);

  if (hidden) return null;

  return (
    <>
      <div className="help-launcher">
        <Button
          ref={button}
          variant="ghost"
          iconOnly
          icon={icons.HELP}
          active={open}
          title="Help"
          label="Help"
          onClick={() => setOpen((was) => !was)}
        />
      </div>
      {/* Above the anchor, because the anchor is 20px off the bottom of the window and a menu that
          opened downwards from it would be a menu nobody can read. */}
      <Popover
        open={open}
        anchor={button.current}
        onClose={close}
        placement="top-end"
        width={240}
        label="Help"
      >
        <HelpList onClose={close} />
      </Popover>
    </>
  );
}

/**
 * The rows, as their own component because the popover draws its children a frame after it is asked
 * to open, once it knows where the anchor is. Focusing the first row belongs to the moment the rows
 * exist, which is this component's mount.
 */
function HelpList({ onClose }: { onClose: () => void }) {
  const list = useRef<HTMLUListElement | null>(null);

  // In front of the window, so an arrow key here does not also walk the list behind it.
  useKeyContext("overlay");

  useEffect(() => {
    list.current?.querySelector<HTMLButtonElement>(".help-option")?.focus();
  }, []);

  useEffect(() => {
    const step = (delta: number) => {
      const rows = [...(list.current?.querySelectorAll<HTMLButtonElement>(".help-option") ?? [])];
      if (rows.length === 0) return;
      const at = rows.indexOf(document.activeElement as HTMLButtonElement);
      const next =
        at === -1 ? (delta > 0 ? 0 : rows.length - 1) : (at + delta + rows.length) % rows.length;
      rows[next].focus();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.isComposing) return;
      const combo = comboOf(e);
      if (combo !== "ArrowDown" && combo !== "ArrowUp") return;
      e.preventDefault();
      step(combo === "ArrowDown" ? 1 : -1);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  const choose = (command: CommandId) => {
    onClose();
    runCommand(command);
  };

  return (
    <div className="help-menu">
      <ul className="help-list" role="menu" ref={list}>
        {ROWS.map((row) => {
          const key = cap(row.command);
          return (
            <li key={row.command} role="none">
              <button
                type="button"
                role="menuitem"
                className="help-option"
                onClick={() => choose(row.command)}
              >
                <Icon d={row.icon} size={15} />
                <span className="help-label">{labelFor(row.command)}</span>
                {key ? <Key size="sm">{key}</Key> : null}
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

export default HelpButton;
