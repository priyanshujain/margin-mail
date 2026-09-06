import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Virtuoso, type VirtuosoHandle } from "react-virtuoso";
import { Button, EmptyState, GroupHead, icons, Pill, Row } from "../ui";
import { registerCommands, runCommand } from "../keys/commands";
import { useEscapeLayer } from "../escape";
import { GROUPS, type Place, type ThreadSummary } from "../ipc";
import { useAccounts } from "../store/useAccounts";
import { useMail } from "../store/useMail";
import { usePiles } from "../store/usePiles";
import { useScreener } from "../store/useScreener";
import { useSearch } from "../store/useSearch";
import { useSelection } from "../store/useSelection";
import { useSnooze } from "../store/useSnooze";
import { filling, type Fill, useSync } from "../store/useSync";
import { cap, displayName, hueOf, isBrand, rowTime } from "./format";
import { LabelPicker, type PickerMode } from "./ActionBar";
import { Piles } from "./Piles";
import { mergeThreads, NoteSheet } from "./ReadingPane";
import { QueryTerms } from "./SearchBar";
import { returnTime, SnoozePicker, snoozeAnchor } from "./SnoozePicker";
import * as triage from "./triage";
import "./list.css";

/** The place's name, in the text face, at the head of its column. */
const TITLES: Record<Place, string> = {
  inbox: "Inbox",
  feed: "Feed",
  "paper-trail": "Paper Trail",
  "reply-later": "Reply later",
  "set-aside": "Set aside",
  screener: "Screener",
  snoozed: "Snoozed",
  everything: "Everything",
  sent: "Sent",
  drafts: "Drafts",
  starred: "Starred",
  "screened-out": "Screened out",
  spam: "Spam",
  trash: "Trash",
  label: "Label",
  search: "Search",
};

/** One quiet line and nothing else. No illustration, no button suggesting you go and make mail. */
const EMPTY: Partial<Record<Place, string>> = {
  screener: "No one is waiting",
  snoozed: "Nothing due",
  search: "Nothing on this device",
  "screened-out": "Nobody has been screened out",
  spam: "Nothing in spam",
  trash: "Nothing in the trash",
};

/**
 * What an empty place shows while its mailbox is still on its way: the engine's own sentence, a
 * thin bar that fills once there is a total to fill it against, and the count under it. The same
 * bar the welcome screen draws, on the paper the list is on rather than on the stage, because
 * from Settings there is an Inbox to sit in while the mail arrives and nothing should take the
 * window over twice.
 */
function Filling({ fill }: { fill: Fill }) {
  const counted = fill.total > 0;
  const done = counted ? fill.hydrated / fill.total : 0;
  return (
    <div className="list-filling" role="status" aria-live="polite">
      <p className="list-filling-line">{fill.message}</p>
      <div
        className="list-filling-bar"
        data-counting={counted ? undefined : ""}
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={counted ? Math.round(done * 100) : undefined}
      >
        <span className="list-filling-fill" style={counted ? { transform: `scaleX(${done})` } : undefined} />
      </div>
      <p className="list-filling-count">
        {counted
          ? `${fill.hydrated.toLocaleString()} of ${fill.total.toLocaleString()} messages`
          : "This becomes your mail as soon as it lands"}
      </p>
    </div>
  );
}

type Entry =
  | { kind: "head"; id: string; label: string }
  | { kind: "row"; id: string; thread: ThreadSummary; rule: boolean };

/**
 * Rows arrive already grouped and already ordered, so this walks them once and puts a head in
 * wherever a group with a head changed. It never sorts and it never decides what a group is
 * called: the two sides cannot disagree about where a row belongs if only one of them has an
 * opinion. A group `GROUPS` has no label for (`new` and `seen` in the Inbox) draws nothing: the
 * Inbox is one list, and a row says it is new by its weight. The first row after a headed group
 * ends carries a rule, so Back has a bottom as well as a top and the list under it does not read
 * as more of it.
 */
function entriesOf(threads: ThreadSummary[]): Entry[] {
  const out: Entry[] = [];
  let group = "";
  let headed = false;
  for (const thread of threads) {
    let rule = false;
    if (thread.group !== group) {
      group = thread.group;
      const label = GROUPS[group];
      if (label) out.push({ kind: "head", id: `head:${group}`, label });
      rule = headed && !label;
      headed = Boolean(label);
    }
    out.push({ kind: "row", id: thread.key, thread, rule });
  }
  return out;
}

/**
 * One small glyph before the time. A star is a state you set; a paperclip is one the mail has;
 * trash and spam are where something else put it, which is what a search result most needs to say
 * and why they come first. Not drawn in Trash or Spam themselves, where a glyph on every row is
 * noise saying what the title already says.
 */
const markOf = (thread: ThreadSummary, place: Place): { mark: string; title: string } | undefined =>
  thread.trashed && place !== "trash"
    ? { mark: icons.TRASH, title: "Trash" }
    : thread.spam && place !== "spam"
      ? { mark: icons.SPAM, title: "Spam" }
      : thread.starred
        ? { mark: icons.STAR, title: "Starred" }
        : thread.hasAttachment
          ? { mark: icons.PAPERCLIP, title: "Has an attachment" }
          : undefined;

/**
 * The time on a row: when the mail arrived, except where the row is about when it comes back.
 *
 * In Snoozed and in the Back group the thread's own date is not what the list is for, so the slot
 * says the return instead. Whether a moment in the past means late or means it has already come
 * back is the group's answer, not the time's: in Snoozed it is still waiting and says "Due
 * yesterday", and in Back it has arrived and says when it was due.
 */
const timeOf = (thread: ThreadSummary, place: Place): string =>
  thread.snoozedUntil !== null && (place === "snoozed" || thread.group === "back")
    ? returnTime(thread.snoozedUntil)
    : rowTime(thread.dateMs);

/**
 * The rows kept mounted above and below the window, and the key each one is remembered by.
 *
 * Both are out here because they are constants, and a constant written inline is a new object on
 * every render: react-virtuoso reads them as having changed and remeasures a list that did not.
 */
const OVERSCAN = { top: 300, bottom: 600 };
const keyOf = (_: number, entry: Entry): string => entry.id;

export function ListColumn() {
  const place = useMail((s) => s.place);
  const accountId = useMail((s) => s.accountId);
  const labelName = useMail((s) => s.labelName);
  const threads = useMail((s) => s.threads);
  const footer = useMail((s) => s.footer);
  const phase = useMail((s) => s.phase);
  const paging = useMail((s) => s.paging);
  const focused = useMail((s) => s.focused);
  const step = useMail((s) => s.step);
  const open = useMail((s) => s.open);
  const goTo = useMail((s) => s.goTo);
  const loadLabels = useMail((s) => s.loadLabels);
  const loadMore = useMail((s) => s.loadMore);
  const selected = useSelection((s) => s.keys);
  const toggleSelect = useSelection((s) => s.toggle);
  const clearSelection = useSelection((s) => s.clear);
  const query = useSearch((s) => s.query);
  const note = useSearch((s) => s.note);
  const providerSearched = useSearch((s) => s.providerSearched);
  const searchPhase = useSearch((s) => s.phase);
  const askProvider = useSearch((s) => s.askProvider);
  const more = useSearch((s) => s.more);

  const list = useRef<VirtuosoHandle | null>(null);
  const scroller = useRef<HTMLElement | null>(null);
  const [picker, setPicker] = useState<PickerMode | null>(null);
  const [noting, setNoting] = useState<string | null>(null);

  // The Screener's pill counts senders waiting rather than messages, so it is not part of the page
  // the list loaded. It is the only count anywhere in this app.
  const waiting = useScreener((s) => s.cards.length);
  const loadScreener = useScreener((s) => s.load);

  const entries = useMemo(() => entriesOf(threads), [threads]);
  const searching = place === "search";
  const searchOpen = searchPhase !== "off";

  // Whether the mailbox behind an empty list has actually arrived yet. One account when one is
  // chosen, every account under "All accounts": a fill on any of them is a list that is short.
  const accounts = useAccounts((s) => s.accounts);
  const statuses = useSync((s) => s.statuses);
  const fill = useMemo(
    () => filling(statuses, accountId ? [accountId] : accounts.map((a) => a.id)),
    [statuses, accountId, accounts],
  );

  // The verbs the list owns while it is on screen, acting on the selection when there is one and on
  // the focused row when there is not. What is not here is what nothing has registered: `r`, `c`
  // and the rest of writing belong to the next milestone, and an unregistered command does nothing
  // at all rather than doing half of it.
  useEffect(() => {
    const onTargets = (run: (keys: string[]) => void) => () => run(triage.targets());
    const extendBy = (delta: number) => {
      const rows = useMail.getState().threads;
      const at = rows.findIndex((t) => t.key === useMail.getState().focused);
      if (at === -1) return;
      const to = at + delta;
      if (to < 0 || to >= rows.length) return;
      // The first extension takes the row it started from with it, which is what makes the anchor.
      if (useSelection.getState().keys.length === 0) useSelection.getState().toggle(rows[at].key);
      useMail.getState().focus(rows[to].key);
      useSelection.getState().extend(
        rows.map((t) => t.key),
        rows[to].key,
      );
    };

    return registerCommands({
      "select-next": () => step(1),
      "select-prev": () => step(-1),
      "open-selection": () => void open(),

      archive: onTargets(triage.archive),
      "toggle-seen": onTargets(triage.toggleSeen),
      "toggle-star": onTargets(triage.toggleStar),
      trash: onTargets(triage.trash),
      spam: onTargets(triage.spam),
      "mark-all-seen": () => void triage.markEverythingSeen(),
      undo: () => void triage.undo(),
      label: () => setPicker(triage.targets().length > 0 ? "apply" : null),

      "reply-later": onTargets((keys) => void usePiles.getState().toggle(keys, "reply-later")),
      "set-aside": onTargets((keys) => void usePiles.getState().toggle(keys, "set-aside")),
      snooze: onTargets((keys) => useSnooze.getState().show(keys, snoozeAnchor())),
      // A note is one thread's. On a selection of several `y` has nothing to write on, and a key
      // that would act on nothing does nothing.
      note: () => {
        const keys = triage.targets();
        if (keys.length === 1) setNoting(keys[0]);
      },
      // And a merge is two or more, so it is the selection's verb and the focused row is not a
      // selection of one.
      merge: () => void mergeThreads(useSelection.getState().keys),

      // `v` is two verbs in docs/keyboard.md: move to a place, which sets the sender's rule, and
      // move to a label. The first is routing, it belongs to the next milestone and there is no
      // command behind it yet, so `v` is registered here only where it means the second one.
      ...(place === "label"
        ? { move: () => setPicker(triage.targets().length > 0 ? "move" : null) }
        : {}),

      select: () => {
        const key = useMail.getState().focused;
        if (key) toggleSelect(key);
      },
      "select-extend-down": () => extendBy(1),
      "select-extend-up": () => extendBy(-1),
      "select-all": () =>
        useSelection.getState().allFrom(
          useMail.getState().threads.map((t) => t.key),
          useMail.getState().focused,
        ),
    });
  }, [place, step, open, toggleSelect]);

  // Escape gives the selection back before it gives anything else back, which is the last rung of
  // the ladder in docs/keyboard.md.
  useEscapeLayer(selected.length > 0, clearSelection);

  useEffect(() => {
    void loadLabels();
  }, [accountId, loadLabels]);

  // The focused row has to be on screen, or `j` walks the list from behind the fold.
  useEffect(() => {
    if (!focused) return;
    const index = entries.findIndex((e) => e.kind === "row" && e.id === focused);
    if (index >= 0) list.current?.scrollIntoView({ index, behavior: "auto" });
  }, [focused, entries]);

  // Where the list was when search took the stage, and putting it back when search gives it up. The
  // offset is read the moment the field opens, because by the time the results are in the column
  // the list that had the scroll is already gone.
  const parked = useRef(0);
  const pending = useRef<number | null>(null);
  const was = useRef(false);

  useEffect(() => {
    if (searchOpen) parked.current = scroller.current?.scrollTop ?? 0;
  }, [searchOpen]);

  useEffect(() => {
    if (was.current && !searching) pending.current = parked.current;
    was.current = searching;
  }, [searching]);

  // The offset goes back on the list that comes back, which is a different scroller: the results
  // and the place each mount their own. It rides in as `initialScrollTop` on that mount rather than
  // being scrolled to afterwards, because a list that has not measured its rows yet has nowhere to
  // scroll to and would quietly land at the top.
  useEffect(() => {
    if (pending.current !== null && entries.length > 0) pending.current = null;
  }, [entries]);

  // Asked for again whenever the list changed under it, because deciding a sender in the Screener
  // and archiving a thread in the Inbox both arrive as the same invalidation.
  useEffect(() => {
    if (place !== "inbox") return;
    void loadScreener(accountId);
  }, [place, accountId, threads, loadScreener]);

  /** The foot of a result list: what the answer covers, and the way to ask for the rest of it. */
  const searchFoot = useCallback(
    () => (
      <div className="search-foot">
        {note ? <p className="search-note">{note}</p> : null}
        {/* Every account at once is every provider at once, and search is per account for now. */}
        {!providerSearched && accountId ? (
          <Button
            size="sm"
            icon={icons.SEARCH}
            disabled={searchPhase === "searching"}
            onClick={() => void askProvider()}
          >
            Search older mail on Gmail
          </Button>
        ) : null}
      </div>
    ),
    [note, providerSearched, accountId, searchPhase, askProvider],
  );

  const holdScroller = useCallback((el: HTMLElement | Window | null) => {
    scroller.current = el as HTMLElement;
  }, []);

  const endReached = useCallback(
    () => (searching ? void more() : void loadMore()),
    [searching, more, loadMore],
  );

  // The place's quiet line, or, when the next page did not come, the one line that says so and
  // the way to ask for it again. The rows above are still the rows, so nothing else changes.
  const listFoot = useCallback(
    () =>
      paging === "error" ? (
        <div className="list-foot" data-state="error">
          <span>The rest of the list did not arrive.</span>
          <Button size="sm" variant="ghost" onClick={() => void loadMore()}>
            Try again
          </Button>
        </div>
      ) : (
        <p className="list-foot">{footer}</p>
      ),
    [footer, paging, loadMore],
  );

  const components = useMemo(
    () => ({
      Footer: searching ? searchFoot : footer || paging === "error" ? listFoot : undefined,
    }),
    [searching, searchFoot, footer, paging, listFoot],
  );

  // Everything the rows are drawn from, and nothing else. The list is handed a fresh page of
  // summaries on every sync pass, so what keeps a row from redrawing is `Row` comparing what it
  // prints; what keeps this from being called for every row on every keystroke is the identity of
  // this function, which is why it is not written inline.
  const itemContent = useCallback(
    (_: number, entry: Entry) => {
      if (entry.kind === "head") {
        return (
          <div className="list-item">
            <GroupHead>{entry.label}</GroupHead>
          </div>
        );
      }
      const mark = markOf(entry.thread, place);
      return (
        <div className="list-item" data-rule={entry.rule ? "" : undefined}>
          <Row
            sender={displayName(entry.thread.from)}
            address={entry.thread.from.address}
            brand={isBrand(entry.thread.from)}
            time={timeOf(entry.thread, place)}
            subject={entry.thread.subject}
            snippet={entry.thread.snippet}
            count={entry.thread.messageCount}
            note={entry.thread.note ?? undefined}
            mark={mark?.mark}
            markTitle={mark?.title}
            accountHue={accountId === null ? hueOf(entry.thread.accountColor) : undefined}
            // An ignored thread is never new to you, however many replies it has taken.
            unread={entry.thread.unseen && !entry.thread.ignored}
            selected={entry.thread.key === focused}
            selecting={selected.length > 0}
            checked={selected.includes(entry.thread.key)}
            onClick={() => void open(entry.thread.key)}
            onToggleCheck={() => toggleSelect(entry.thread.key)}
          />
        </div>
      );
    },
    [place, accountId, focused, selected, open, toggleSelect],
  );

  return (
    <section className="list-col">
      <div className="list-head">
        <h1 className="list-title">{place === "label" ? (labelName ?? "Label") : TITLES[place]}</h1>
        {searching ? <QueryTerms query={query} /> : null}
        {/* Held back while the account is still arriving: until the crawl finishes and the seed
            has screened in everyone it already knows, the count is every sender it has met so far,
            and "Screen 200 new senders" beside a bar that is still filling is a fright about
            nothing. */}
        {place === "inbox" && waiting > 0 && fill === null ? (
          <Pill icon={icons.SHIELD} keycap="6" onClick={() => goTo("screener")}>
            {`Screen ${waiting} new sender${waiting === 1 ? "" : "s"}`}
          </Pill>
        ) : null}
        {/* The third way into Focus & Reply, beside the key and the palette, and the one that is
            in front of you at the moment you are looking at the pile it is a page over. */}
        {place === "reply-later" && threads.length > 0 ? (
          <Pill
            icon={icons.REPLY}
            keycap={cap("focus-reply")}
            onClick={() => runCommand("focus-reply")}
          >
            Focus &amp; Reply
          </Pill>
        ) : null}
      </div>

      {entries.length === 0 ? (
        <div className="list list-blank">
          {phase === "loading" ? null : fill && !searching ? (
            <Filling fill={fill} />
          ) : (
            <EmptyState>{EMPTY[place] ?? "Nothing here"}</EmptyState>
          )}
          {searching ? searchFoot() : null}
        </div>
      ) : (
        <Virtuoso
          className="list"
          key={searching ? "search" : place}
          initialScrollTop={pending.current ?? 0}
          ref={list}
          scrollerRef={holdScroller}
          data={entries}
          computeItemKey={keyOf}
          endReached={endReached}
          increaseViewportBy={OVERSCAN}
          components={components}
          itemContent={itemContent}
        />
      )}

      <Piles />
      <LabelPicker mode={picker} keys={triage.targets()} onClose={() => setPicker(null)} />
      <NoteSheet threadKey={noting} onClose={() => setNoting(null)} />
      <SnoozePicker />
    </section>
  );
}

export default ListColumn;
