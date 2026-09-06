// Where a place was when you left it, which today means the Feed and nothing else.
//
// This belongs in the portable state database, in the `markers` table behind
// `state::write::set_marker`: a marker is a per place `seen_ms` that roams with the account through
// the backup store, so the hairline sits in the same spot on the laptop and on the phone. There is
// no command for it in the frozen contract, and the rule is that bodies get added to Rust rather
// than wrappers to `src/api/`, so it waits here until `markers` is reachable over IPC. Nothing else
// about this file changes when it moves: one number in, one number out.
//
// Out here rather than inside `useFeed` because a store may not touch disk, which is the same
// reason `src/pane.ts` and `src/theme.ts` are modules of their own.

const key = (place: string) => `marginmail-left-off-${place}`;

/** The instant the newest card on screen carried when the place was last left. */
export function readLeftOff(place: string): number | null {
  try {
    const held = Number(localStorage.getItem(key(place)));
    return Number.isFinite(held) && held > 0 ? held : null;
  } catch {
    return null;
  }
}

export function writeLeftOff(place: string, ms: number): void {
  try {
    localStorage.setItem(key(place), String(ms));
  } catch {
    /* a context without storage is a context that forgets, which is the worst of it */
  }
}
