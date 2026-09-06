import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Button, EmptyState, GroupHead, Icon, NO_AUTOFILL, Sheet, icons } from "../ui";
import { isTauri } from "../ipc";
import { useOverlays } from "../store/useOverlays";
import { notify } from "../store/useToast";
import { ArticleView } from "./guide/Article";
import { FIRST, articleOf, filterSections, orderOf } from "./guide/content";
import "./guide.css";

/**
 * The guide: how to do things, and the questions the app raises by being unlike the others.
 *
 * A panel over the whole window rather than a stage under the header. It is read about the app
 * rather than instead of it, so nothing behind it can be pressed while it is up and closing it puts
 * back exactly what was there: the place, the thread, the scroll position, all of it. As a stage it
 * sat under a header whose own controls did nothing, because a stage wins over a place and pressing
 * Inbox up there changed a place nobody could see.
 *
 * Inside, the shape is Settings': a rail of names on the left and one page on the right at a
 * measure prose reads at, for the same reason Settings has it. Fifty short articles is too many for
 * a list that shows one row at a time.
 *
 * Search is above both of them and across the whole panel, because it is what somebody opens this
 * for. A person with a question does not know which of eight sections owns it, and a field tucked
 * into the head of the rail reads as a way to tidy the rail rather than as the way in. It takes the
 * focus on open, so the guide can be opened and typed into in one motion.
 *
 * The search is over what the articles say and not only over their titles, because somebody looking
 * for the page about images is as likely to type "tracker" as "images", and a search that only knew
 * the headings would answer nothing.
 */
export function Guide() {
  const open = useOverlays((s) => s.open) === "guide";
  const close = useOverlays((s) => s.close);

  return (
    <Sheet open={open} title="Guide" size="full" onClose={close}>
      <Pages />
    </Sheet>
  );
}

/** A new question against the repository. The same one App.tsx reports an issue to. */
const ISSUES_URL = "https://github.com/priyanshujain/margin-mail/issues/new";

/** The issue that a failed search is: labelled a question, titled with what was searched for. */
function askUrl(query: string): string {
  const params = new URLSearchParams({ labels: "question", title: query.trim() });
  return `${ISSUES_URL}?${params.toString()}`;
}

/**
 * The rail and the page, as their own component because a closed sheet never mounts its children:
 * that is what makes the guide open on its first article every time without anything having to
 * reset it.
 */
function Pages() {
  const [query, setQuery] = useState("");
  const [openId, setOpenId] = useState(FIRST);
  const panel = useRef<HTMLDivElement | null>(null);

  const shown = useMemo(() => filterSections(query), [query]);
  const order = useMemo(() => orderOf(shown), [shown]);
  const article = articleOf(openId);

  // What the rail currently holds, for the keys that walk it. Held in a ref so the handler is bound
  // once: rebinding it on every keystroke in the field would be a listener added fifty times.
  const held = useRef(order);
  useEffect(() => {
    held.current = order;
  }, [order]);

  const show = (id: string) => {
    setOpenId(id);
    // A cross-link can land on an article the search is hiding, and a rail that no longer shows
    // what is on screen has stopped saying where you are.
    if (!held.current.includes(id)) setQuery("");
  };

  // The panel owns its own arrows while it is up, the way the palette does. It cannot use the app's
  // `j` and `k` commands any more: an open panel shadows the whole view keymap, which is what keeps
  // those two from walking the list behind it. A letter is only a key when the field does not have
  // the focus, or typing "just" into it would go looking through the rail instead.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.isComposing || e.metaKey || e.ctrlKey || e.altKey) return;
      const target = e.target as HTMLElement | null;
      const typing = target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement;
      const down = e.key === "ArrowDown" || (!typing && e.key === "j");
      const up = e.key === "ArrowUp" || (!typing && e.key === "k");
      if (!down && !up) return;
      e.preventDefault();
      setOpenId((was) => {
        const list = held.current;
        if (list.length === 0) return was;
        const at = list.indexOf(was);
        if (at === -1) return list[0];
        return list[Math.min(Math.max(at + (down ? 1 : -1), 0), list.length - 1)];
      });
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // A new article starts at its heading. Without this, opening a short article from the foot of a
  // long one lands halfway down a page that has already ended.
  useEffect(() => {
    panel.current?.scrollTo({ top: 0 });
  }, [openId]);

  return (
    <div className="guide">
      <label className="guide-search">
        <Icon d={icons.SEARCH} size={16} />
        <input
          type="search"
          value={query}
          placeholder="Search the guide"
          aria-label="Search"
          data-autofocus
          {...NO_AUTOFILL}
          onChange={(e) => setQuery(e.target.value)}
        />
      </label>

      {shown.length === 0 ? <Nothing query={query} /> : null}

      <div className="guide-body">
        <nav className="guide-rail" aria-label="Guide">
          {shown.map((section) => (
            <Fragment key={section.id}>
              <GroupHead>{section.title}</GroupHead>
              {section.articles.map((one) => (
                <button
                  key={one.id}
                  type="button"
                  className="guide-tab"
                  data-active={one.id === openId ? "" : undefined}
                  aria-current={one.id === openId ? "page" : undefined}
                  onClick={() => show(one.id)}
                >
                  {one.title}
                </button>
              ))}
            </Fragment>
          ))}
        </nav>

        <div className="guide-panel" ref={panel}>
          {article ? <ArticleView article={article} onOpen={show} /> : null}
        </div>
      </div>
    </div>
  );
}

/**
 * A search nothing answered, which is the one thing in a guide that is not a failure of the guide:
 * it is a question nobody has written the page for yet, and the person holding it is the only one
 * who can say what it was.
 *
 * So the offer is to ask rather than to try other words. What it costs is said before it is
 * pressed, because it leaves the app for somebody else's website and carries what was typed with
 * it, and neither of those is something to find out afterwards.
 */
function Nothing({ query }: { query: string }) {
  const ask = () => {
    const url = askUrl(query);
    if (!isTauri) window.open(url, "_blank", "noopener,noreferrer");
    else openUrl(url).catch((e) => notify(`Could not open the browser: ${e}`));
  };

  return (
    <div className="guide-nothing">
      <EmptyState>Nothing here answers that</EmptyState>
      <p className="guide-ask">
        Ask on GitHub opens the project's issues in your browser, as a new question titled with what
        you typed. That page is public.
      </p>
      <Button onClick={ask}>Ask on GitHub</Button>
    </div>
  );
}

export default Guide;
