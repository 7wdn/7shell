// The buttons the platform draws into fields of its own accord, and why they are turned off.
//
// Two of them exist, and both have been seen in a rendered window: a clear button in every search
// field, and a reveal eye in every password field. The password one was reported as a bug — the
// vault gate's field showed it beside the gate's own eye, two controls over one masked value — and
// what it takes to see it is a screenshot: the button is the engine's own, rendered from a
// pseudo-element, standing in no markup a test can reach. So both are turned off in the stylesheet,
// said once for every field, and this guard is what keeps them off.
//
// The failure it watches for is a quiet one. The rules are a couple of lines in a file of hundreds,
// no component references them, and nothing else goes red when they are dropped: the duplicates
// exist only in a rendered window, while every suite asserts against the DOM, where there is only
// ever one control. What this check can say is that the decision is still written down and still
// hides the button; whether a future engine keeps obeying it is not something a file can see — a
// rendered window is where that would show.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";

import { REPO_ROOT } from "../lib/descriptors.mjs";

const STYLES = path.join(REPO_ROOT, "apps", "desktop", "src", "styles", "globals.css");

/** Each platform button, named by the selector that lands on it. */
const BUILT_IN = [
  { what: "the clear button in every search field", selector: 'input[type="search"]::-webkit-search-cancel-button' },
  { what: "the reveal button in every password field", selector: 'input[type="password"]::-ms-reveal' },
];

test("the buttons the platform draws into fields are hidden in the stylesheet", () => {
  const sheet = readFileSync(STYLES, "utf8");

  for (const { what, selector } of BUILT_IN) {
    // The selector is matched whole, up to its block's brace: a longer selector that merely starts
    // with this one must not stand in for it — as one did the first time this check was mutation-
    // tested, by keeping the target as a prefix.
    const rule = new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*\\{`);
    const match = rule.exec(sheet);
    assert.ok(match, `${what} is no longer hidden: the rule naming it left globals.css`);
    const block = sheet.slice(match.index, sheet.indexOf("}", match.index));
    assert.match(block, /display:\s*none/, `${what} is no longer hidden: its rule stopped turning it off`);
  }
});
