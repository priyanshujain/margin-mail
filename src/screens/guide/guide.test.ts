// The guide's content, checked the way a link checker checks a manual.
//
// Nothing here renders anything. What can go wrong in a library of forty-odd articles is a link to
// an article that was renamed, a picture nobody captured, and a title that says one thing while the
// rail says another, and all three are readable straight off the data.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { SECTIONS, articleOf, filterSections, titleOf } from "./content";
import { PICTURES, matches, textOf, type Article, type Piece } from "./types";

const ARTICLES: Article[] = SECTIONS.flatMap((section) => [...section.articles]);

const piecesOf = (article: Article): Piece[] =>
  article.blocks.flatMap((block) =>
    "p" in block
      ? block.p
      : "note" in block
        ? block.note
        : "steps" in block
          ? block.steps.flat()
          : [],
  );

describe("the shape of it", () => {
  it("is the how-to first and the questions last", () => {
    expect(SECTIONS.map((s) => s.title)).toEqual([
      "Getting started",
      "The Screener",
      "Reading",
      "Triage",
      "Writing",
      "Organising",
      "Accounts",
      "Questions",
    ]);
  });

  it("gives every article an id of its own", () => {
    const ids = ARTICLES.map((a) => a.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("gives every article a title and something to say", () => {
    for (const article of ARTICLES) {
      expect(article.title.length).toBeGreaterThan(0);
      expect(article.blocks.length).toBeGreaterThan(0);
    }
  });
});

describe("the links", () => {
  it("point at articles that exist", () => {
    for (const article of ARTICLES) {
      for (const piece of piecesOf(article)) {
        if (typeof piece === "object" && "see" in piece) {
          expect(articleOf(piece.see), `${article.id} links to ${piece.see}`).not.toBeNull();
        }
      }
    }
  });

  it("print the title the target carries, so renaming one renames its links", () => {
    expect(titleOf("screen-how")).toBe(articleOf("screen-how")?.title);
  });
});

describe("the pictures", () => {
  const used = ARTICLES.flatMap((article) =>
    article.blocks.flatMap((block) => ("picture" in block ? [block.picture] : [])),
  );

  it("are ones that were captured, and none is shown twice", () => {
    for (const name of used) expect(Object.keys(PICTURES)).toContain(name);
    expect(new Set(used).size).toBe(used.length);
  });

  it("carry an alt and a caption", () => {
    for (const article of ARTICLES) {
      for (const block of article.blocks) {
        if (!("picture" in block)) continue;
        expect(block.alt.length).toBeGreaterThan(0);
        expect(block.caption.length).toBeGreaterThan(0);
      }
    }
  });

  // The sizes are the guide's copy of a fact that lives in ten files somebody else captures, so
  // they are checked against the files rather than trusted. A recapture at a different crop fails
  // here, which is the only place it can fail before it is a picture drawn at the wrong size.
  it("are drawn at half the pixels they were captured at", () => {
    for (const [name, size] of Object.entries(PICTURES)) {
      const file = fileURLToPath(new URL(`../../../public/guide/${name}.png`, import.meta.url));
      const header = readFileSync(file);
      // The PNG header: an 8 byte signature, the IHDR length and type, then width and height.
      expect({ width: header.readUInt32BE(16) / 2, height: header.readUInt32BE(20) / 2 }).toEqual(
        size,
      );
    }
  });
});

describe("the figures", () => {
  const drawn = ARTICLES.flatMap((article) =>
    article.blocks.flatMap((block) => ("figure" in block ? [block] : [])),
  );

  it("carry the sentence they are making", () => {
    for (const block of drawn) expect(block.caption.length).toBeGreaterThan(0);
  });

  // A wall of prose is what this guide was built to stop being, so the rule is mechanical: an
  // article says its piece in a few short paragraphs, and anything longer earns a picture, a
  // diagram, a list of steps or a table of keys to break it up.
  // Both of these gather every offender before asserting rather than expecting inside the loop.
  // An expect in a loop stops at the first one, which hid a second failing article behind the
  // first for as long as it took somebody to fix the first.
  it("break up anything long enough to need it", () => {
    const bare = ARTICLES.filter((article) => {
      const words = textOf(article).split(/\s+/).filter(Boolean).length;
      if (words < 120) return false;
      return !article.blocks.some(
        (block) => "figure" in block || "picture" in block || "keys" in block || "steps" in block,
      );
    }).map((article) => `${article.id} (${textOf(article).split(/\s+/).filter(Boolean).length} words)`);
    expect(bare, "long articles with nothing to look at").toEqual([]);
  });

  it("keep a paragraph short enough to be read", () => {
    const long: string[] = [];
    for (const article of ARTICLES) {
      for (const block of article.blocks) {
        if (!("p" in block)) continue;
        const words = block.p
          .map((piece) => (typeof piece === "string" ? piece : ""))
          .join(" ")
          .split(/\s+/)
          .filter(Boolean).length;
        if (words >= 75) long.push(`${article.id} (${words} words)`);
      }
    }
    expect(long, "paragraphs nobody will read").toEqual([]);
  });
});

describe("the prose", () => {
  it("holds no dash the house does not write", () => {
    for (const article of ARTICLES) {
      const said = [
        textOf(article),
        ...article.blocks.flatMap((block) => ("picture" in block ? [block.alt] : [])),
      ].join(" ");
      expect(said, article.id).not.toMatch(/[–—]/);
    }
  });

  it("is what the filter reads, keycaps and cross-links aside", () => {
    const article = articleOf("read-images")!;
    expect(textOf(article)).toContain("tracker");
    expect(textOf(article)).toContain("Show images");
  });
});

describe("filtering", () => {
  it("is every section when nothing is typed", () => {
    expect(filterSections("").length).toBe(SECTIONS.length);
    expect(filterSections("   ").length).toBe(SECTIONS.length);
  });

  it("finds an article by a word in its body rather than in its title", () => {
    const found = filterSections("tracker").flatMap((s) => s.articles.map((a) => a.id));
    expect(found).toContain("read-images");
    expect(found).not.toContain("write-drafts");
  });

  it("answers nothing when nothing says it", () => {
    expect(filterSections("kryptonite")).toEqual([]);
  });

  it("wants every word rather than any letter of them", () => {
    expect(matches("Snooze, and if no reply by", "snooze reply")).toBe(true);
    expect(matches("Snooze, and if no reply by", "snooze label")).toBe(false);
  });
});
