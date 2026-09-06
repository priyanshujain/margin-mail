import { useEffect, useMemo, useState } from "react";
import { Palette, type PaletteGroup } from "../ui";
import {
  BINDINGS,
  keyLabel,
  PALETTE_COMMANDS,
  type CommandBinding,
  type CommandId,
} from "../keys/bindings";
import { commandMatches, runCommand } from "../keys/commands";
import { useMail } from "../store/useMail";
import { useOverlays } from "../store/useOverlays";
import type { Place } from "../ipc";

/**
 * The palette is the only menu in the app and the way every setting is reached, so what is in it
 * is generated rather than listed: Places are the binding table's place commands and Actions are
 * every command the table marks as belonging here. A command that exists is in the palette, and
 * one that is removed leaves it, without anybody remembering to do either.
 */
const PLACES: readonly CommandBinding[] = BINDINGS.filter(
  (b): b is CommandBinding => b.command !== null && b.group === "Places",
);

const ACTIONS: readonly CommandBinding[] = PALETTE_COMMANDS.filter((b) => b.group !== "Places");

/**
 * A command the palette lists that the keyboard does not reach, so the binding table has no row for
 * it. Mark all as seen is a row here and a key, and nothing else.
 */
/** The folders, which are places with no key of their own rather than commands. */
const FOLDERS: { id: Place; label: string }[] = [
  { id: "sent", label: "Sent" },
  { id: "drafts", label: "Drafts" },
  { id: "starred", label: "Starred" },
];

/**
 * The three you go looking in rather than read, in a group of their own under the labels.
 *
 * None of them has a number key and none of them is beside the Inbox, because where a place sits
 * is the honest statement of how often you should be in it. They are still one keystroke and three
 * letters away, which is what a rescue actually needs.
 */
const OTHER: { id: Place; label: string }[] = [
  { id: "screened-out", label: "Screened out" },
  { id: "spam", label: "Spam" },
  { id: "trash", label: "Trash" },
];

/**
 * The first combo a row prints: the bare letter when it is unmodified, glyphs when it is not.
 *
 * A command with no keys at all is a real case rather than an oversight: the palette is how a
 * setting with no verb behind it is reached, and such a row prints nothing on its right.
 */
const caps = (binding: CommandBinding): string[] => {
  const combo = binding.keys[0];
  if (!combo) return [];
  return [combo.includes("+") ? keyLabel(combo) : combo];
};

const placeOf = (id: string): Place => id.replace(/^place-/, "") as Place;

export function CommandPalette() {
  const open = useOverlays((s) => s.open) === "palette";
  const close = useOverlays((s) => s.close);
  const goTo = useMail((s) => s.goTo);
  const goToLabel = useMail((s) => s.goToLabel);
  const labels = useMail((s) => s.labels);
  const loadLabels = useMail((s) => s.loadLabels);

  const [query, setQuery] = useState("");
  const [active, setActive] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(null);
      // The provider's labels are places, and a place list that is a sync behind is a place list
      // that sends you somewhere that is not there any more.
      void loadLabels();
    }
  }, [open, loadLabels]);

  const groups = useMemo((): PaletteGroup[] => {
    const match = (label: string) => commandMatches(label, query);
    return [
      {
        id: "places",
        label: "Places",
        items: [
          ...PLACES.filter((b) => match(b.label)).map((b) => ({
            id: b.command,
            label: b.label,
            keys: caps(b),
          })),
          ...FOLDERS.filter((f) => match(f.label)).map((f) => ({
            id: `folder:${f.id}`,
            label: f.label,
          })),
        ],
      },
      // The provider's labels are places. They are the provider's, they roam with the mailbox, and
      // they are not how Margin organises anything, which is why they are a group of their own
      // under the places rather than mixed in with them.
      {
        id: "labels",
        label: "Labels",
        items: labels
          .filter((label) => match(label.name))
          .map((label) => ({ id: `label:${label.id}`, label: label.name })),
      },
      {
        id: "other",
        label: "Other",
        items: OTHER.filter((f) => match(f.label)).map((f) => ({
          id: `folder:${f.id}`,
          label: f.label,
        })),
      },
      {
        id: "actions",
        label: "Actions",
        items: [
          ...ACTIONS.filter((b) => match(b.label)).map((b) => ({
            id: b.command,
            label: b.label,
            keys: caps(b),
          })),
        ],
      },
      // People are the contacts package's, and Settings is the settings screen's: both fill their
      // group from here when they arrive. An empty group renders as nothing at all.
      { id: "people", label: "People", items: [] },
      { id: "settings", label: "Settings", items: [] },
    ];
  }, [query, labels]);

  const flat = useMemo(() => groups.flatMap((g) => g.items.map((i) => i.id)), [groups]);
  const activeId = active && flat.includes(active) ? active : flat[0];

  const choose = (id: string) => {
    close();
    if (id.startsWith("folder:")) goTo(id.slice("folder:".length) as Place);
    else if (id.startsWith("label:")) {
      const label = labels.find((candidate) => candidate.id === id.slice("label:".length));
      if (label) goToLabel(label);
    } else if (id.startsWith("place-")) goTo(placeOf(id));
    else runCommand(id as CommandId);
  };

  // The palette owns the arrow keys and Return while it is open. The field has the focus, so the
  // app's keymap is already standing back; this is the panel moving its own row.
  //
  // No dependency list on purpose: the handler closes over the filtered rows and the active one,
  // and both change on every keystroke into the field.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const at = flat.indexOf(activeId ?? "");
        const next = at + (e.key === "ArrowDown" ? 1 : -1);
        if (next >= 0 && next < flat.length) setActive(flat[next]);
      } else if (e.key === "Enter" && activeId) {
        e.preventDefault();
        choose(activeId);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  return (
    <Palette
      open={open}
      query={query}
      onQuery={(next) => {
        setQuery(next);
        setActive(null);
      }}
      groups={groups}
      activeId={activeId}
      onChoose={choose}
      onClose={close}
    />
  );
}

export default CommandPalette;
