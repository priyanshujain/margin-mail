// What an article is made of.
//
// The prose is data rather than JSX, which is the one thing this screen needs that the tour does
// not: the filter searches what an article says, and a body that only exists as rendered elements
// can only be searched by rendering it. A block model keeps the words in one place, the markup in
// `Article.tsx`, and the search over the words themselves.
//
// It also settles the keycap rule mechanically. There is no way to type a letter into a sentence
// where a cap belongs, because a cap is a command id and the keymap answers it.

import { labelFor, type CommandId } from "../../keys/bindings";
import type { Place } from "../../ipc";

/**
 * Every picture the guide may show, with the size it is drawn at, and the whole list of them.
 *
 * They are captured from the real app by another package and land in `public/guide`, so a name here
 * that nobody captures is a broken picture and a capture nobody names is a file for nothing.
 *
 * The size is half the file's pixels, because every capture is at twice the size. That is the size
 * the thing in the picture actually was, and drawing it at anything less is a screenshot of an app
 * whose type is too small to read: at two thirds, the app's own body text lands at 10px. The
 * numbers ride here rather than in the stylesheet so they can go on the `img` itself, which is what
 * keeps the page from jumping as a picture arrives, and `guide.test.ts` reads the files to check
 * that they are still true.
 */
export const PICTURES = {
  inbox: { width: 1440, height: 900 },
  screener: { width: 1440, height: 854 },
  feed: { width: 1440, height: 854 },
  piles: { width: 419, height: 115 },
  palette: { width: 620, height: 479 },
  compose: { width: 600, height: 421 },
  "contact-card": { width: 320, height: 447 },
  snooze: { width: 264, height: 188 },
  shortcuts: { width: 620, height: 820 },
  settings: { width: 1440, height: 854 },
} as const;

export type PictureName = keyof typeof PICTURES;

/**
 * A piece of a sentence. Prose is a string, and everything else is a thing only the app can fill
 * in: the key a verb answers to today, another article's title, a way to the place being described.
 *
 * A cap always follows the name of the verb it belongs to, and the sentence has to read with the
 * cap taken out, because on a phone it is taken out: `<Key>` hides itself there.
 */
export type Piece =
  | string
  | { cap: CommandId }
  /** Another article, printed with that article's own title. */
  | { see: string }
  | { place: Place; label: string }
  | { settings: string };

/**
 * Every diagram the guide may draw, and what each one is about.
 *
 * A picture is the app photographed; a figure is an idea drawn. Where mail goes, what a pile does,
 * how long a send is held: none of those is a thing you can point a camera at, and all of them are
 * the thing somebody actually came to understand. They are built from the same tokens as the app,
 * in `Figures.tsx`, so they cannot drift from what they describe the way an exported drawing would.
 */
export type FigureId =
  /** A message arriving, the gate, and the four places it can end up. */
  | "routing"
  /** Inbox, Feed and Paper Trail beside each other, with what each one holds. */
  | "boxes"
  /** The shape of the window: header, list column, reading pane, the two piles at the foot. */
  | "window"
  /** The verbs of triage as the keys they answer to. */
  | "keyboard"
  /** One Screener card, labelled: who, what, the reason, and the three answers. */
  | "screener-card"
  /** A thread in the pane: the latest message open, the older ones collapsed to a line. */
  | "thread"
  /** A message with its remote images held back and its trackers counted. */
  | "trackers"
  /** A thread leaving the list for a pile, and the same key bringing it back. */
  | "piles"
  /** The snooze choices along a line of time. */
  | "snooze"
  /** Rows selected, and the piles replaced by the bar of verbs. */
  | "selection"
  /** Focus and Reply: every thread you owe, each with its box. */
  | "focus"
  /** A send held for its ten seconds, and what the toast offers while it waits. */
  | "undo"
  /** Two threads becoming one, and the banner that says so. */
  | "merge"
  /** Where a search looks: this device first, the provider on request. */
  | "search-reach"
  /** The window of mail this device holds, and the rest of the mailbox behind it. */
  | "storage"
  /** Several mailboxes in one window, each with its own everything. */
  | "accounts"
  /** The three switches a notification has to pass, all of them off to begin with. */
  | "notify"
  /** What stays on the device and what leaves it. */
  | "privacy";

export type Block =
  | { p: Piece[] }
  | { steps: Piece[][] }
  | { picture: PictureName; alt: string; caption: string }
  /** A drawn idea, with the sentence it is making underneath it. */
  | { figure: FigureId; caption: string }
  /**
   * The verbs of a thing as a table of the key and what it does, generated from the binding table.
   * A paragraph that lists six keys is a paragraph nobody reads; the same six as rows are read at a
   * glance, and they cannot go stale because none of the words in them are written here.
   */
  | { keys: readonly CommandId[] }
  /** One line set apart: the thing people get wrong, or the promise worth saying twice. */
  | { note: Piece[] };

export interface Article {
  id: string;
  title: string;
  blocks: readonly Block[];
}

export interface Section {
  id: string;
  title: string;
  articles: readonly Article[];
}

const wordsOf = (pieces: readonly Piece[]): string[] =>
  pieces.map((piece) => {
    if (typeof piece === "string") return piece;
    if ("place" in piece) return piece.label;
    if ("settings" in piece) return piece.settings;
    return "";
  });

/**
 * Everything an article says in words, which is what the filter reads.
 *
 * A keycap is not in it and neither is a cross-link, because both print something this article did
 * not write: a search for `e` that turned up every article with an archive key in it would be a
 * search over the keymap wearing a search over the prose.
 */
export function textOf(article: Article): string {
  const parts: string[] = [article.title];
  for (const block of article.blocks) {
    if ("p" in block) parts.push(...wordsOf(block.p));
    else if ("note" in block) parts.push(...wordsOf(block.note));
    else if ("steps" in block) for (const step of block.steps) parts.push(...wordsOf(step));
    // A key table's words are whole verbs printed on the page, which is not the case a cap makes:
    // somebody searching for "unsubscribe" should find the article that lists it in a row.
    else if ("keys" in block) parts.push(...block.keys.map(labelFor));
    else parts.push(block.caption);
  }
  return parts.join(" ");
}

/**
 * Whether a body answers a query: every word of it is somewhere in the text.
 *
 * Not the palette's subsequence match, which is right for a row of three words and wrong for a
 * page of them: any three letters are a subsequence of any paragraph, so a filter over bodies
 * built that way narrows nothing.
 */
export function matches(text: string, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return true;
  const hay = text.toLowerCase();
  return words.every((word) => hay.includes(word));
}
