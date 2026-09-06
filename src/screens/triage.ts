// The verbs, and the three things a store may not do: call the backend, decide what a toast says,
// and put a row back when the call did not go through.
//
// Every one of these is optimistic. The row changes in `useMail` first and the call goes behind it,
// because a mailbox that waits for a round trip before it moves a row is a mailbox you stop
// trusting with a keyboard. A failure puts the row back where it was and says so.
//
// Every call hands back an `Undo`, and every action that hides a row turns that into a toast
// carrying its own token. `z` is `undoLast`, which is the most recent action wherever it came
// from; the toast's button is `undoToken`, which is the action that toast is about. They are not
// the same thing once two actions have happened, and a toast that undid the wrong one would be
// worse than no toast at all.

import { flagsSet, markAllSeen } from "../api/threads";
import { labelApply, labelMove } from "../api/labels";
import { undoLast, undoToken } from "../api/undo";
import type { FlagPatch, Place, ThreadSummary, Undo } from "../ipc";
import { useMail } from "../store/useMail";
import { useSearch } from "../store/useSearch";
import { useSelection } from "../store/useSelection";
import { notify, useToast } from "../store/useToast";

/**
 * What a single key acts on: the selection when there is one, and the focused row otherwise.
 *
 * An empty list is an empty answer, which is what makes a key that would act on nothing do nothing
 * at all rather than reporting that it could not.
 */
export function targets(): string[] {
  const selected = useSelection.getState().keys;
  if (selected.length > 0) return selected;
  const focused = useMail.getState().focused;
  return focused ? [focused] : [];
}

/** Whatever list is on the stage, asked for again. Search owns its own; every place asks the mirror. */
export function refresh(): void {
  if (useMail.getState().place === "search") void useSearch.getState().run();
  else void useMail.getState().load();
}

const rowsOf = (keys: string[]): ThreadSummary[] => {
  const wanted = new Set(keys);
  return useMail.getState().threads.filter((t) => wanted.has(t.key));
};

/**
 * Whether a row this patch has just changed still belongs in the list it is in.
 *
 * The answer is the place's, not the verb's: archiving takes a thread out of the Inbox and leaves
 * it exactly where it was in Everything, and a row that vanished and came back on the next refresh
 * would read as a bug rather than as a rule.
 */
function hides(place: Place, patch: FlagPatch): boolean {
  // Search reaches trash, spam and screened out on purpose, so a result it found stays where it is
  // and changes its own glyph instead. A row that vanished from a search because you filed it
  // would be the app arguing with the query.
  if (place === "search") return false;
  if (patch.trashed === true) return place !== "trash";
  if (patch.trashed === false) return place === "trash";
  // Everything holds spam, so only the places that filter it out lose the row.
  if (patch.spam === true) return place !== "spam" && place !== "everything";
  if (patch.spam === false) return place === "spam";
  if (patch.archived) return place === "inbox" || place === "feed" || place === "paper-trail";
  if (patch.starred === false) return place === "starred";
  return false;
}

/**
 * The toast every hidden action leaves behind, carrying the token that reverses that one action.
 *
 * The piles, the snooze picker and the Screener all leave the same toast and each asks a different
 * list for itself afterwards, which is what `after` is. Pressing Undo takes the toast down before
 * the call goes out: a button that stayed up while it worked was pressed twice, and the second
 * press was answered with "that change can no longer be taken back" about an undo that had just
 * succeeded. What comes back is said the way `z` says it.
 */
export function acknowledge(undo: Undo, after: () => void = refresh): void {
  notify(undo.label, {
    label: "Undo",
    keycap: "z",
    run: () => {
      useToast.getState().dismiss();
      void undoToken(undo.token)
        .then(() => {
          after();
          notify(`Undone: ${undo.label}`);
        })
        .catch((e) => notify(`Could not undo that: ${e}`));
    },
  });
}

/**
 * `speaks` is the verb's own answer to docs/keyboard.md: archive, trash and spam are destructive
 * and always produce a toast, wherever the row ended up. Seen and star show themselves in the row
 * and say nothing, unless taking the star was what took the row out of Starred.
 */
async function act(
  keys: string[],
  patch: FlagPatch,
  changes: Partial<ThreadSummary>,
  speaks: boolean,
): Promise<void> {
  if (keys.length === 0) return;
  const mail = useMail.getState();
  const going = hides(mail.place, patch);
  const before = rowsOf(keys);
  const taken = going ? mail.take(keys) : [];
  if (!going) mail.patch(keys, changes);
  // A bulk action consumes the selection it acted on: the rows are gone and a checkbox over
  // nothing is a state you have to press Escape to get out of.
  if (going) useSelection.getState().clear();

  try {
    const undo = await flagsSet(keys, patch);
    // The pane draws trash and spam as a banner, and the view behind it is a different read from
    // the row. A banner still saying "In the trash" after Put back landed is the one thing the
    // mouse path must not do, so the open thread is asked for again when it was one of these.
    const openKey = useMail.getState().openKey;
    if ((patch.trashed !== undefined || patch.spam !== undefined) && openKey && keys.includes(openKey)) {
      void useMail.getState().refreshThread();
    }
    if (speaks || going) acknowledge(undo);
  } catch (e) {
    if (going) useMail.getState().untake(taken);
    else
      for (const row of before) {
        useMail.getState().patch([row.key], {
          unseen: row.unseen,
          starred: row.starred,
          trashed: row.trashed,
          spam: row.spam,
        });
      }
    notify(`That did not go through: ${e}`);
  }
}

export const archive = (keys: string[]): void => void act(keys, { archived: true }, {}, true);

/**
 * `#` and `!`, both toggles, because the verb that puts a thread somewhere is the verb that takes
 * it back out: that is how the piles already work and it is one fewer key to learn. Pressed over
 * rows that are all already there it means the other direction, which is the direction somebody
 * standing in Trash meant.
 *
 * The flag goes through as the optimistic change as well as the patch, so a row that stays on
 * screen, which is what happens in a search, redraws its own glyph rather than waiting for a page.
 */
export function trash(keys: string[]): void {
  const trashed = !filed(keys, "trashed");
  void act(keys, { trashed }, { trashed }, true);
}

export function spam(keys: string[]): void {
  const spam = !filed(keys, "spam");
  void act(keys, { spam }, { spam }, true);
}

/**
 * Whether every thread this key is about is already filed away. The rows are asked first because
 * the verbs patch rows; the open thread answers for one opened from somewhere with no row behind
 * it, which is a search result that has since been taken out of its list.
 */
function filed(keys: string[], flag: "trashed" | "spam"): boolean {
  const rows = rowsOf(keys);
  if (rows.length > 0) return rows.every((t) => t[flag]);
  return useMail.getState().thread?.[flag] ?? false;
}

/**
 * `u`, on a mixed selection as much as on one row: anything unseen makes the whole selection seen,
 * which is the direction a person means when they pressed it over a list with a dot in it.
 */
export function toggleSeen(keys: string[]): void {
  const rows = rowsOf(keys);
  // Not in the list means it is the open thread, which opening has already marked seen.
  const seen = rows.length === 0 ? true : !rows.some((t) => t.unseen);
  void act(keys, { seen: !seen }, { unseen: seen }, false);
}

export function toggleStar(keys: string[]): void {
  const rows = rowsOf(keys);
  const starred =
    rows.length > 0
      ? rows.every((t) => t.starred)
      : (useMail.getState().thread?.starred ?? false);
  void act(keys, { starred: !starred }, { starred: !starred }, false);
}

/** The palette row, and the key behind it. */
export async function markEverythingSeen(): Promise<void> {
  const { accountId, place, threads } = useMail.getState();
  const unseen = threads.filter((t) => t.unseen).map((t) => t.key);
  if (unseen.length === 0) return;
  useMail.getState().patch(unseen, { unseen: false });
  try {
    acknowledge(await markAllSeen(accountId, place));
  } catch (e) {
    useMail.getState().patch(unseen, { unseen: true });
    notify(`That did not go through: ${e}`);
  }
}

/**
 * `Shift+L`. A label is the provider's and it is never drawn on a row, so the toast is the only
 * thing that says it landed.
 */
export async function label(keys: string[], labelId: string, on: boolean): Promise<void> {
  if (keys.length === 0) return;
  try {
    acknowledge(await labelApply(keys, labelId, on));
  } catch (e) {
    notify(`That did not go through: ${e}`);
  }
}

/** `v` in a label list, which is the one meaning of `v` this milestone owns: apply and archive. */
export async function move(keys: string[], labelId: string): Promise<void> {
  if (keys.length === 0) return;
  const mail = useMail.getState();
  // A move archives, so the row leaves wherever archiving would have taken it out of.
  const going = hides(mail.place, { archived: true });
  const taken = going ? mail.take(keys) : [];
  useSelection.getState().clear();
  try {
    acknowledge(await labelMove(keys, labelId));
  } catch (e) {
    useMail.getState().untake(taken);
    notify(`That did not go through: ${e}`);
  }
}

/** `z`. The most recent reversible action, whichever screen it came from. */
export async function undo(): Promise<void> {
  try {
    const label = await undoLast();
    // Nothing to take back is not an error and does not get a toast: the key did nothing.
    if (!label) return;
    refresh();
    notify(`Undone: ${label}`);
  } catch (e) {
    notify(`Could not undo that: ${e}`);
  }
}
