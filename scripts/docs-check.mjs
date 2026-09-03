#!/usr/bin/env node
// The prose gate. Three rules, checked over every markdown file in the repository:
//
//   1. No em dash and no en dash, anywhere, including inside code fences. This is a house rule and
//      it is absolute: a comma, a colon, a semicolon, brackets or a full stop, whichever fits.
//   2. No directory tree. A tree in a committed file is stale the day somebody adds a file, and
//      naming the one path that matters is what a reader actually needed.
//   3. No broken relative link. A link to a file that is not there is worse than no link, because
//      it reads as though the detail exists somewhere.
//
// Run by `just docs` and by CI, and worth running before any hand-off.

import { readdirSync, readFileSync, statSync, existsSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SKIP = new Set(["node_modules", "dist", "target", ".git", "gen", ".playwright-mcp"]);

function markdownFiles(dir) {
  const found = [];
  for (const entry of readdirSync(dir)) {
    if (SKIP.has(entry)) continue;
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) found.push(...markdownFiles(path));
    else if (entry.endsWith(".md")) found.push(path);
  }
  return found;
}

// A run of box-drawing characters, or two or more lines of the ASCII form. One stray `|--` in a
// table is not a tree; four lines of them under a fenced block is.
const TREE_GLYPHS = /[├└│┬─]{2,}/;
const ASCII_TREE = /^\s*[|`+]--\s/;

const problems = [];

for (const path of markdownFiles(root)) {
  const shown = relative(root, path);
  const lines = readFileSync(path, "utf8").split("\n");
  let asciiTreeRun = 0;

  lines.forEach((line, i) => {
    const at = `${shown}:${i + 1}`;

    const dash = line.match(/[—–]/);
    if (dash) problems.push(`${at}: ${dash[0] === "—" ? "em" : "en"} dash: ${line.trim()}`);

    if (TREE_GLYPHS.test(line)) problems.push(`${at}: directory tree: ${line.trim()}`);

    if (ASCII_TREE.test(line)) {
      asciiTreeRun += 1;
      if (asciiTreeRun === 3) problems.push(`${at}: directory tree`);
    } else {
      asciiTreeRun = 0;
    }

    for (const [, target] of line.matchAll(/\]\(([^)#\s]+)(?:#[^)\s]*)?\)/g)) {
      if (/^[a-z]+:/i.test(target) || target.startsWith("/")) continue;
      const resolved = resolve(dirname(path), decodeURIComponent(target));
      if (!existsSync(resolved)) problems.push(`${at}: link goes nowhere: ${target}`);
    }
  });
}

if (problems.length === 0) {
  console.log("docs: clean");
  process.exit(0);
}
console.error("docs: problems found");
for (const problem of problems) console.error(`  ${problem}`);
process.exit(1);
