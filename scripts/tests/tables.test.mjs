// Every table in the interface is drawn from the one vocabulary.
//
// Ten tables had drifted into three treatments of the headings, two of the rows and two hover
// shades: the same control learned twice, and a table that did not answer the pointer read as a
// picture of a table. `lib/table.ts` is where that vocabulary lives now, and this guard is what
// keeps it the only one — an extract that nothing enforces is a suggestion.
//
// It reads the markup rather than rendering it: the rules are about what a component writes down,
// and rendering ten tables would need a store, a window and a fixture per table to assert the same
// strings. The rules are the ones that file states:
//
// 1. A table carries a name, because the readings are read out of it and not looked at.
// 2. A table, its headings and its cells wear `TABLE.<surface>` for everything the vocabulary has an
//    opinion about — the surface, the padding, the rules. What is a column's own business (its
//    width, its alignment, whether it is monospaced) stays where the columns are.
// 3. A row of readings goes through `rowClass(...)`, which is what keeps the band and the states the
//    same everywhere.
// 4. A message row spans the table's columns — a heading over nothing is what a collapsed list
//    looks like.

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import test from "node:test";

import { REPO_ROOT } from "../lib/descriptors.mjs";

const SOURCE_ROOT = path.join(REPO_ROOT, "apps", "desktop", "src");

/** The class utilities that are the vocabulary's business rather than a column's. */
const VOCABULARY_CLASSES = /\b(?:py|px|pt|pb|pl|pr)-\d|border-b\b|border-collapse\b|sticky\b|bg-background\b/;

function templates() {
  const out = [];
  const walk = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const full = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        walk(full);
      } else if (entry.name.endsWith(".vue")) {
        out.push(full);
      }
    }
  };
  walk(SOURCE_ROOT);
  return out;
}

/**
 * Every opening tag of one kind, with all of its attributes.
 *
 * The `\b` is what keeps `<th` from matching `<thead`: the two have different rules. Quoted
 * attribute values are matched whole, so a `>` inside one — `v-if="rows.length > 0"` — does not end
 * the tag early, which is a bug this guard shipped with for exactly one run.
 */
function tags(source, name) {
  const pattern = new RegExp(`<${name}\\b(?:"[^"]*"|'[^']*'|[^>"'])*>`, "g");
  return [...source.matchAll(pattern)].map((match) => match[0]);
}

/** The rows, each with its opening tag and with the markup inside it. */
function rows(source) {
  const out = [];
  for (const match of source.matchAll(/<tr\b(?:"[^"]*"|'[^']*'|[^>"'])*>/g)) {
    const rest = source.slice(match.index);
    const end = rest.indexOf("</tr>");
    out.push({ tag: match[0], body: end < 0 ? rest : rest.slice(0, end) });
  }
  return out;
}

const hasOwnClasses = (tag) => /\s:?class="/.test(tag);
const wearsVocabulary = (tag) => tag.includes("TABLE.");

test("every table is drawn from the one vocabulary", () => {
  const problems = [];
  const note = (where, what) => problems.push(`${where}: ${what}`);

  for (const file of templates()) {
    const source = readFileSync(file, "utf8");
    const where = path.relative(REPO_ROOT, file).split(path.sep).join("/");

    for (const tag of tags(source, "table")) {
      if (!tag.includes("aria-label")) {
        note(where, "a table with no name");
      }
      if (!wearsVocabulary(tag)) {
        note(where, "a table wearing its own surface classes");
      }
    }

    for (const tag of [...tags(source, "thead"), ...tags(source, "th")]) {
      if (hasOwnClasses(tag) && !wearsVocabulary(tag)) {
        note(where, "a heading wearing its own classes");
      }
    }

    for (const tag of [...tags(source, "td"), ...tags(source, "th")]) {
      if (hasOwnClasses(tag) && VOCABULARY_CLASSES.test(tag) && !wearsVocabulary(tag)) {
        note(where, "a cell wearing its own padding or rules");
      }
    }

    for (const { tag, body } of rows(source)) {
      const readings = tag.includes("v-for");
      if (readings && !tag.includes("rowClass(")) {
        note(where, "a row of readings that does not go through rowClass");
      }
      if (!readings && tag.includes("v-if") && !body.includes("colspan")) {
        note(where, "a table with a heading over nothing");
      }
    }
  }

  assert.deepEqual(problems, [], `the table vocabulary is the one place a table is defined:\n${problems.join("\n")}`);
});
