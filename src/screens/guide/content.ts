// The guide's table of contents, assembled from the eight section files.
//
// The order is the order somebody meets the app: what it is, the gate in front of it, reading,
// triage, writing, the things kept beside the mail, the accounts, and then the questions all of it
// raises. How-to first and questions last, because a person with a question knows they have one and
// will find the section named after it, while a person who is new does not know what to ask.

import { ACCOUNTS } from "./accounts";
import { ORGANISING } from "./organising";
import { QUESTIONS } from "./questions";
import { READING } from "./reading";
import { SCREENER } from "./screener";
import { STARTED } from "./started";
import { TRIAGE } from "./triage";
import { WRITING } from "./writing";
import { matches, textOf, type Article, type Section } from "./types";

export const SECTIONS: readonly Section[] = [
  STARTED,
  SCREENER,
  READING,
  TRIAGE,
  WRITING,
  ORGANISING,
  ACCOUNTS,
  QUESTIONS,
];

interface Entry {
  article: Article;
  section: Section;
  /** Everything the article says, folded once at load rather than on every keystroke. */
  text: string;
}

const INDEX = new Map<string, Entry>();
for (const section of SECTIONS) {
  for (const article of section.articles) {
    INDEX.set(article.id, { article, section, text: textOf(article) });
  }
}

/** The article the guide opens on, which is the first thing in the first section. */
export const FIRST = SECTIONS[0].articles[0].id;

export function articleOf(id: string): Article | null {
  return INDEX.get(id)?.article ?? null;
}

export function sectionOf(id: string): Section | null {
  return INDEX.get(id)?.section ?? null;
}

/** What a cross-link prints: the target's own title, so a renamed article renames its links. */
export function titleOf(id: string): string {
  return INDEX.get(id)?.article.title ?? id;
}

/**
 * The rail, narrowed to what answers the query. A section with nothing left in it is not shown,
 * and an empty query is every section whole.
 */
export function filterSections(query: string): Section[] {
  if (query.trim() === "") return [...SECTIONS];
  const out: Section[] = [];
  for (const section of SECTIONS) {
    const articles = section.articles.filter((article) =>
      matches(INDEX.get(article.id)?.text ?? article.title, query),
    );
    if (articles.length > 0) out.push({ ...section, articles });
  }
  return out;
}

/** Every article in the rail as it currently stands, in order, which is what walks with the keys. */
export function orderOf(sections: readonly Section[]): string[] {
  return sections.flatMap((section) => section.articles.map((article) => article.id));
}
