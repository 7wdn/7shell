// What can be selected, and what refuses to be.
//
// The interface is a desktop application rather than a document: text is selected to take something
// away, and the shell — the frame, the lists, the strips, the menus, the handles — has nothing worth
// taking. Neither has a control, a heading or a field's name. That is written out in
// `docs/ui-conventions.md`, and this guard is what keeps it from drifting, because the failure is a
// quiet one: a rewrite drops a class or a base rule, the chrome becomes selectable again, and nobody
// notices until a drag across a toolbar smears a selection over it.
//
// Both halves are checked. A surface that stopped refusing is the obvious half; a page that started
// refusing is the worse one, because those are the surfaces people read *in order to copy from* —
// an address, a path, a command line — and losing that is losing a feature nobody would report as a
// bug.
//
// What it looks at is the file, not each element: a component that draws two strips (the workspace)
// or a header and a page (the network page) passes while either one of them is unselectable, and
// mutation-testing it is how that was found. The element-level half lives in that component's own
// spec, where the element can be named — `WorkspaceTabs.spec.ts`, `ContextMenu.spec.ts` and
// `NetworkReport.spec.ts` do exactly that.

import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import test from "node:test";

import { REPO_ROOT } from "../lib/descriptors.mjs";

const COMPONENTS = path.join(REPO_ROOT, "apps", "desktop", "src", "components");
const STYLES = path.join(REPO_ROOT, "apps", "desktop", "src", "styles", "globals.css");

/** The surfaces that must refuse selection, each with what it is. */
const REFUSING = [
  { file: "shell/TitleBar.vue", what: "the window's own bar" },
  { file: "shell/AppSidebar.vue", what: "the session library" },
  { file: "shell/ContextMenu.vue", what: "the menus, both levels" },
  { file: "shell/SidebarResizeHandle.vue", what: "the handle that widens the sidebar" },
  { file: "ui/ColumnResizeHandle.vue", what: "the edge between two columns, which is dragged to resize" },
  { file: "shell/TransferPanel.vue", what: "the list of transfers, whose rows are pressed" },
  { file: "workspace/WorkspaceTabs.vue", what: "the tab strip and the terminal's strip" },
  { file: "workspace/SplitDivider.vue", what: "the divider above the file panel" },
  { file: "files/FileBrowser.vue", what: "the directory listing" },
  { file: "files/DirectoryTree.vue", what: "the tree beside the listing" },
  { file: "monitor/MonitorPanel.vue", what: "the monitor panel" },
  { file: "settings/SettingsPage.vue", what: "the settings page, which is a page of controls" },
];

/** The surfaces that hold readings, which must stay selectable. */
const READINGS = [
  { file: "inspect/ProcessTable.vue", what: "the process page and its detail pane" },
  { file: "inspect/InterfaceTable.vue", what: "the interface table" },
  { file: "batch/BatchPanel.vue", what: "the batch panel, whose output is copied out of" },
  // The readings a page of settings holds, which are exempted from the rule above: fingerprints and
  // host names are checked by eye against another machine and pasted into a terminal. That they are
  // exempted *and* that the page refuses selection is the element-level case in `SettingsPage.spec.ts`
  // — a file-level check cannot see which half of one file is which.
  { file: "settings/KnownKeys.vue", what: "the fingerprints of the keys this device holds" },
  { file: "settings/TrustedKeys.vue", what: "the hosts this device trusts" },
];

const source = ({ file }) => readFileSync(path.join(COMPONENTS, file), "utf8");

function templates(directory = path.join(REPO_ROOT, "apps", "desktop", "src")) {
  const out = [];
  const walk = (current) => {
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) {
        walk(full);
      } else if (entry.name.endsWith(".vue")) {
        out.push(full);
      }
    }
  };
  walk(directory);
  return out;
}

test("the surfaces that must refuse selection still do", () => {
  const offenders = REFUSING.filter((entry) => !source(entry).includes("select-none")).map(
    (entry) => `${entry.file} (${entry.what})`,
  );

  assert.deepEqual(offenders, [], `a drag in the shell means moving, resizing or scrolling: ${offenders.join(", ")}`);
});

test("the surfaces that hold readings stay selectable", () => {
  const offenders = READINGS.filter((entry) => source(entry).includes("select-none")).map(
    (entry) => `${entry.file} (${entry.what})`,
  );

  assert.deepEqual(offenders, [], `what a page reads is what people copy: ${offenders.join(", ")}`);
});

/**
 * No table refuses to be selected.
 *
 * A table is where the readings are, and the failure this guard exists for is somebody putting
 * `select-none` on one — which reads as tidy in a screenshot and takes away the only reason those
 * pages exist. It is checked per table rather than per file because two of the pages also own a
 * chrome strip of their own (the network page's connections header, the system page's caption), and
 * a file-level rule cannot see the difference.
 */
test("no table refuses to be selected", () => {
  const offenders = [];

  for (const file of templates()) {
    const source = readFileSync(file, "utf8");
    const template = source.slice(source.indexOf("<template>"));
    for (const tag of template.matchAll(/<table\b[^>]*>/g)) {
      if (tag[0].includes("select-none")) {
        offenders.push(path.relative(REPO_ROOT, file).split(path.sep).join("/"));
      }
    }
  }

  assert.deepEqual(offenders, [], `readings are what people copy: ${offenders.join(", ")}`);
});

/**
 * The rule that covers the several hundred controls nobody wrote a class on.
 *
 * A button, a heading, a column heading and a field's name say what the interface does rather than
 * what it read, and the base layer is where that is said once. It is a stylesheet rather than markup,
 * so the two cases above cannot see it: this one reads the file.
 */
test("a control, a heading and a name are not text", () => {
  const sheet = readFileSync(STYLES, "utf8");
  const base = sheet.slice(sheet.indexOf("@layer base"));

  assert.match(base, /user-select:\s*none/, "the base layer says which elements are not text");

  for (const element of ["button", "th", "dt", "h1", "h2", "h3"]) {
    assert.match(
      base,
      new RegExp(`(^|[,\\s])${element}\\s*[,{]`, "m"),
      `${element} is one of the shapes this interface says things with`,
    );
  }
});
