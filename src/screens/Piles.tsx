import { useEffect } from "react";
import { icons, Icon, Key } from "../ui";
import type { Pile, Place, ThreadSummary } from "../ipc";
import { useMail } from "../store/useMail";
import { usePiles } from "../store/usePiles";
import { useSelection } from "../store/useSelection";
import { ActionBar } from "./ActionBar";
import { displayName } from "./format";
import "./piles.css";

/**
 * The two stacks at the foot of the list: Reply later on the left, Set aside on the right.
 *
 * A pile is its own query rather than a slice of the list above it, because a piled thread is
 * exactly the thread the Inbox has stopped showing. Each stack prints its label, its key, the top
 * thread's subject and who it is from, and an edge or two of the cards underneath so it reads as a
 * stack rather than as a single card. An empty pile is a dashed outline with its label.
 *
 * The other thing in this footprint is the selection's action bar, which takes the whole of it
 * while a selection exists.
 */
const PILES: { pile: Pile; place: Place; label: string; keycap: string; icon: string }[] = [
  { pile: "reply-later", place: "reply-later", label: "Reply later", keycap: "4", icon: icons.CLOCK },
  { pile: "set-aside", place: "set-aside", label: "Set aside", keycap: "5", icon: icons.SET_ASIDE },
];

/**
 * Who the pile is from: the top thread's sender, and how many more are underneath it.
 *
 * The count is here rather than as a number in the corner because it is the only place in the app
 * where one would be right: a pile is a thing with a depth, and how deep it is is what the stack
 * is drawing.
 */
function whoOf(top: ThreadSummary, count: number): string {
  const name = displayName(top.from);
  if (count <= 1) return name;
  return `${name}, and ${count - 1} more`;
}

export function Piles() {
  const goTo = useMail((s) => s.goTo);
  const accountId = useMail((s) => s.accountId);
  const rows = useMail((s) => s.threads);
  const selected = useSelection((s) => s.keys);
  const threads = usePiles((s) => s.threads);
  const load = usePiles((s) => s.load);

  // Asked for again whenever the list changed under it, because piling a thread in the Inbox and
  // undoing an archive both arrive as the same invalidation.
  useEffect(() => {
    void load();
  }, [accountId, rows, load]);

  // The bar takes the whole footprint rather than sitting beside the piles. Two stacks and a row of
  // verbs in 420px is neither, and what a selection wants from this corner is the verbs.
  if (selected.length > 0) return <ActionBar />;

  return (
    <div className="piles">
      {PILES.map((pile) => {
        const held = threads[pile.pile];
        const top = held[0];
        return (
          <button
            key={pile.place}
            type="button"
            className="pile"
            data-empty={top ? undefined : ""}
            onClick={() => goTo(pile.place)}
          >
            {held.length > 2 ? <span className="pile-edge" data-depth="2" /> : null}
            {held.length > 1 ? <span className="pile-edge" data-depth="1" /> : null}
            <span className="pile-card">
              <span className="pile-top">
                <Icon d={pile.icon} size={11} />
                {pile.label}
                <Key size="sm">{pile.keycap}</Key>
              </span>
              {top ? (
                <>
                  <span className="pile-subject">{top.subject}</span>
                  <span className="pile-who">{whoOf(top, held.length)}</span>
                </>
              ) : null}
            </span>
          </button>
        );
      })}
    </div>
  );
}

export default Piles;
