/**
 * What the running application does, driven the way a person drives it.
 *
 * The first case is deliberately a survey rather than an assertion: it opens the application,
 * records what is on screen and takes a screenshot, so the selectors the real cases use come from
 * the interface itself instead of from a guess.
 */
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium, expect, test, type Locator, type Page } from "@playwright/test";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ARTIFACTS = path.join(HERE, "artifacts");
/** Where the launcher points the application's settings, so a run starts from nothing. */
const SETTINGS_FILE = path.join(HERE, ".run", "config", "settings.json");
const CDP_URL = process.env.SEVENSHELL_E2E_CDP ?? "http://127.0.0.1:9222";

/** The application's own page, which is the only one in the webview. */
async function openApp(): Promise<{ page: Page; close: () => Promise<void> }> {
  const browser = await chromium.connectOverCDP(CDP_URL);
  const context = browser.contexts()[0];
  if (!context) {
    throw new Error("the application has no webview context");
  }

  const page = context.pages()[0] ?? (await context.waitForEvent("page"));
  return { page, close: () => browser.close() };
}

/** The settings file as the application last wrote it, which is what a restart would read. */
async function writtenSettings(): Promise<{ packedTransfer?: boolean }> {
  return JSON.parse(await readFile(SETTINGS_FILE, "utf8")) as { packedTransfer?: boolean };
}

test("the window says what it is", async () => {
  await mkdir(ARTIFACTS, { recursive: true });
  const { page, close } = await openApp();
  try {
    await page.waitForLoadState("domcontentloaded");
    // The interface paints after its first call to the backend; waiting for a title is the
    // cheapest way to know it did.
    await expect(page.locator("text=7Shell").first()).toBeVisible();

    const text = await page.locator("body").innerText();
    await writeFile(path.join(ARTIFACTS, "first-screen.txt"), text, "utf8");
    await page.screenshot({ path: path.join(ARTIFACTS, "first-screen.png"), fullPage: false });

    const buttons = await page.locator("button").evaluateAll((elements) =>
      elements.map((element) => ({
        text: (element.textContent ?? "").trim(),
        label: element.getAttribute("aria-label"),
        title: element.getAttribute("title"),
      })),
    );
    await writeFile(path.join(ARTIFACTS, "buttons.json"), JSON.stringify(buttons, null, 2), "utf8");

    const fields = await page.locator("input").evaluateAll((elements) =>
      elements.map((element) => ({
        type: element.getAttribute("type"),
        placeholder: element.getAttribute("placeholder"),
        id: element.getAttribute("id"),
      })),
    );
    await writeFile(path.join(ARTIFACTS, "inputs.json"), JSON.stringify(fields, null, 2), "utf8");

    console.log(`first screen:\n${text}`);
  } finally {
    await close();
  }
});

/**
 * Creating a vault, and the shell it opens onto.
 *
 * The first thing anybody does with this application, driven the way they do it: two passwords and
 * a button. It uses the throwaway data directory the launcher set up, so it starts from nothing
 * every run — which is also why it is a real test of the first-run path rather than of whatever
 * happened to be left behind.
 */
test("a vault can be created and the shell opens", async () => {
  const { page, close } = await openApp();
  try {
    await page.getByPlaceholder("At least 8 characters").fill("seven-shell-e2e");
    await page.getByPlaceholder("Type the password again").fill("seven-shell-e2e");
    await page.getByRole("button", { name: "Create vault" }).click();

    // The shell is up when the session list and the protocol list are on screen.
    await expect(page.getByText("Sessions", { exact: true })).toBeVisible();
    await expect(page.getByText("Protocols", { exact: true })).toBeVisible();
    await expect(page.getByText("SSH", { exact: true })).toBeVisible();

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "shell.png"), fullPage: false });
  } finally {
    await close();
  }
});

/**
 * The host monitor, and the switch that turns it off.
 *
 * What an end-to-end run can see of it: the panel is where it belongs, it says it is not connected
 * rather than drawing zeroes, and hiding it from the panel and hiding it from the settings are the
 * same switch — which is the part that would break silently if the two ever stopped sharing one
 * preference. The sampling itself needs a real server and is proved against the lab
 * (`crates/sevenshell-core/tests/live_monitor.rs`).
 */
test("the host monitor is on, and hiding it is one setting", async () => {
  const { page, close } = await openApp();
  try {
    const panel = page.getByRole("complementary", { name: "Host monitor" });
    await expect(panel).toBeVisible();
    await expect(panel.getByText("No connection", { exact: false })).toBeVisible();
    // The rows are drawn with dashes rather than with zeroes — an empty column is not a reading —
    // and with no host known there is nothing to copy either.
    await expect(panel.getByRole("button", { name: "Copy the host name" })).toBeDisabled();

    // The three cards are the way in to the host's pages. With nothing connected there is no host
    // to open a page about, so they are drawn and refuse to be used rather than opening an empty
    // page — which is the state a person meets them in the first time they see the panel.
    for (const name of ["System info", "Processes", "Network"]) {
      await expect(panel.getByRole("button", { name, exact: true })).toBeDisabled();
    }

    // The panel's own button writes the preference, which is what makes the next assertion true.
    await panel.getByRole("button", { name: "Hide the monitor" }).click();
    await expect(panel).toHaveCount(0);

    await page.getByRole("button", { name: "Settings" }).click();

    // The settings are a page in the workspace rather than a dialog over it, so it is asked for the
    // way the other pages are — by the region it draws — and the row it is about is found by name.
    const settings = page.getByRole("region", { name: "Settings" });
    await settings.getByRole("button", { name: "Workspace", exact: true }).click();
    const monitorSwitch = settings.getByRole("switch", { name: "Host monitor" });
    await expect(monitorSwitch).toHaveAttribute("aria-checked", "false");
    await monitorSwitch.click();
    await expect(monitorSwitch).toHaveAttribute("aria-checked", "true");

    // A page is left the way any tab is left; closing it is what brings the workspace back, and the
    // panel being there again is the same preference read from the other end.
    await page.getByRole("button", { name: "Close settings" }).click();
    await expect(settings).toHaveCount(0);
    await expect(panel).toBeVisible();
  } finally {
    await close();
  }
});

/**
 * The interface's own right-click menu, on the one surface a run without a server can reach: the
 * session list.
 *
 * Saving a session is enough to have a row; connecting is not needed and deliberately not done. What
 * is asserted is the hand-over a person would check by hand — right-click a row, see what can be done
 * to it, press Escape — and that the browser's own menu is what does *not* come up, which is the
 * other half of the same feature.
 */
test("a saved session offers its own menu on a right-click", async () => {
  const { page, close } = await openApp();
  try {
    await page.getByRole("button", { name: "New session" }).click();
    const editor = page.getByRole("dialog");
    await editor.getByPlaceholder("Production web server").fill("lab");
    await editor.getByPlaceholder("example.com").fill("127.0.0.1");
    await editor.getByPlaceholder("root").fill("labuser");
    await editor.getByRole("button", { name: "Save" }).click();
    await expect(editor).toHaveCount(0);

    // The row's name is its title as well, which is what makes it findable without connecting.
    const row = page.getByTitle("Open a terminal for lab");
    await expect(row).toBeVisible();

    await row.click({ button: "right" });
    const menu = page.getByRole("menu");
    await expect(menu).toBeVisible();
    await expect(menu.getByRole("menuitem")).toHaveText([
      "Open a terminal for lab",
      "Open files for lab",
      "Edit session",
      "Delete session",
      "Copy address",
    ]);

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "context-menu.png"), fullPage: false });

    await page.keyboard.press("Escape");
    await expect(menu).toHaveCount(0);
  } finally {
    await close();
  }
});

/**
 * The dropdown, driven the way it is used.
 *
 * A native control would pass any assertion about a value being chosen, so what is checked here is
 * the part that only this build can do: the list is a real listbox drawn by the interface, one of
 * its rows refuses to be chosen because the transport does not implement it, and the form
 * underneath reacts to the choice. It also proves the list is reachable from inside a dialog's
 * modal layer, where the body is not accepting pointers — the one thing about the component that
 * could not be settled by reading the source.
 */
test("the session editor's dropdown draws its own list, and choosing from it changes the form", async () => {
  const { page, close } = await openApp();
  try {
    await page.getByRole("button", { name: "New session" }).click();
    const editor = page.getByRole("dialog");

    const method = editor.getByRole("combobox", { name: "Authentication" });
    await expect(method).toBeVisible();
    await expect(method).toHaveText("Password");

    await method.click();
    const list = page.getByRole("listbox");
    await expect(list).toBeVisible();
    await expect(list.getByRole("option")).toHaveText([
      "Password",
      "Public key",
      "Keyboard interactive",
      "SSH agent",
      "Pageant",
    ]);
    await expect(list.getByRole("option", { name: "SSH agent" })).toHaveAttribute("aria-disabled", "true");
    await expect(list.getByRole("option", { name: "Password" })).toHaveAttribute("aria-selected", "true");

    // And it looks the part: a row nobody can choose has to be drawn differently from one they can,
    // or the only way to find out is to try it. The platform drew that for free; this does not.
    const [unavailable, available] = await Promise.all(
      ["SSH agent", "Public key"].map(async (name) =>
        Number(await list.getByRole("option", { name }).evaluate((row) => getComputedStyle(row).opacity)),
      ),
    );
    expect(unavailable, "a row that cannot be chosen is drawn dimmer than one that can").toBeLessThan(available);

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "dropdown.png"), fullPage: false });

    await list.getByRole("option", { name: "Public key" }).click();
    await expect(list).toHaveCount(0);
    await expect(method).toHaveText("Public key");
    // The half of the form that belongs to that choice is the proof it was the method that changed.
    await expect(editor.getByRole("button", { name: "Choose a key file…" })).toBeVisible();

    await page.keyboard.press("Escape");
  } finally {
    await close();
  }
});

/**
 * The browser's menu, which is cancelled everywhere except in a text field.
 *
 * It is asserted where it actually happens rather than by looking for a menu that is drawn by the
 * platform and appears in no screenshot: WebView2 draws its menu unless the page cancels the event,
 * so a cancelled event is the mechanism, and a field's uncancelled one is the exception that keeps
 * cut and paste working.
 */
test("the browser's own menu is cancelled, and left alone in a text field", async () => {
  const { page, close } = await openApp();
  try {
    await page.getByRole("button", { name: "New session" }).click();
    const field = page.getByPlaceholder("Production web server");
    await expect(field).toBeVisible();

    const onBody = await page.evaluate(() => {
      const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
      document.body.dispatchEvent(event);
      return event.defaultPrevented;
    });

    const onField = await field.evaluate((element) => {
      const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
      element.dispatchEvent(event);
      return event.defaultPrevented;
    });

    expect(onBody, "the browser's menu is not what a right-click anywhere brings up").toBe(true);
    expect(onField, "a text field keeps its own menu").toBe(false);

    await page.keyboard.press("Escape");
  } finally {
    await close();
  }
});

/**
 * The packed-transfer switch, and the file it is written to.
 *
 * The same switch also sits in the file panel's own menu, which a run without a server cannot open —
 * so what is checked here is the half that is always reachable, plus the one thing a screenshot could
 * never show: that flipping it changes what the application will do *next time*, because the choice
 * is on disk rather than in the window that made it.
 */
test("the packed-transfer switch is written down, not just remembered", async () => {
  const { page, close } = await openApp();
  try {
    await page.getByRole("button", { name: "Settings" }).click();
    const settings = page.getByRole("region", { name: "Settings" });
    await settings.getByRole("button", { name: "Files", exact: true }).click();
    const packedSwitch = settings.getByRole("switch", { name: "Packed transfer" });

    // Everything after a transfer leaves an archive on the disk, so the default is the plain one.
    await expect(packedSwitch).toHaveAttribute("aria-checked", "false");
    await packedSwitch.click();
    await expect(packedSwitch).toHaveAttribute("aria-checked", "true");

    // Polled rather than read once: the interface applies a choice at once and writes it afterwards,
    // so what is being checked is that the file catches up rather than that it was written in the
    // same instant as the click.
    await expect
      .poll(async () => (await writtenSettings()).packedTransfer, {
        message: "the choice reached the settings file",
      })
      .toBe(true);

    await packedSwitch.click();
    await page.getByRole("button", { name: "Close settings" }).click();
    await expect(settings).toHaveCount(0);

    await expect
      .poll(async () => (await writtenSettings()).packedTransfer, {
        message: "and it is written back when it is turned off again",
      })
      .toBe(false);
  } finally {
    await close();
  }
});

/**
 * The two things a settings *page* has that a dialog of fixed rows did not: a search across every
 * category, and a way to put a category back the way it started.
 *
 * The search is the way in for somebody who knows the name of a setting but not the category it is
 * filed under — so it is checked from a category that does not hold the row it is looking for.
 * Restoring is checked on the disk rather than on the screen, because the file is what the next
 * run reads, and a page that only redrew itself would look identical.
 */
test("a setting can be found by name, and a category can be put back to its defaults", async () => {
  const { page, close } = await openApp();
  try {
    await page.getByRole("button", { name: "Settings" }).click();
    const settings = page.getByRole("region", { name: "Settings" });

    // The page as it opens: the first category, which is the one that is nearly all dropdowns —
    // a category with switches on it is the second shot below.
    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "settings-appearance.png"), fullPage: false });

    await settings.getByRole("button", { name: "Files", exact: true }).click();

    // Away from its default first, so that restoring has something to do.
    const packedSwitch = settings.getByRole("switch", { name: "Packed transfer" });
    await packedSwitch.click();
    await expect
      .poll(async () => (await writtenSettings()).packedTransfer, {
        message: "the choice is on disk before anything is restored",
      })
      .toBe(true);

    // The search looks through every category, not only the one on screen: the monitor row lives
    // under Workspace, and it is found from here.
    await settings.getByRole("searchbox").fill("monitor");
    await expect(settings.getByRole("switch", { name: "Host monitor" })).toBeVisible();
    await expect(settings.getByRole("switch", { name: "Packed transfer" })).toHaveCount(0);

    await page.screenshot({ path: path.join(ARTIFACTS, "settings-search.png"), fullPage: false });

    // Clearing the search is what goes back to the category that was on screen, and the last shot is
    // taken there: it is the page as somebody meets it, rather than as a query leaves it.
    await settings.getByRole("button", { name: "Clear the search" }).click();
    await expect(settings.getByRole("switch", { name: "Packed transfer" })).toBeVisible();

    await page.screenshot({ path: path.join(ARTIFACTS, "settings-page.png"), fullPage: false });

    await settings.getByRole("button", { name: "Restore the defaults for Files" }).click();
    await expect(packedSwitch).toHaveAttribute("aria-checked", "false");

    await expect
      .poll(async () => (await writtenSettings()).packedTransfer, {
        message: "and the default reached the settings file as well",
      })
      .toBe(false);
  } finally {
    await close();
  }
});

/**
 * The clipboard, through the backend, which is the only way this build can read one.
 *
 * A script's own clipboard read never settles in WebView2 — the permission request it raises is never
 * answered, which a probe of this window showed by hanging until the test timed out — so the
 * terminal's Paste asks Rust instead, and this is that call, driven the way the interface drives it.
 *
 * It reaches past the backend facade on purpose: the facade is not on the page, and the menu entry
 * that uses this needs a server to be reachable at all. Writing first is what makes the answer a fact
 * rather than a hope about whatever happened to be on the clipboard.
 */
test("the backend can read the clipboard the terminal pastes", async () => {
  const { page, close } = await openApp();
  try {
    const answer = await page.evaluate(async () => {
      const internals = (
        window as unknown as { __TAURI_INTERNALS__?: { invoke: (command: string) => Promise<unknown> } }
      ).__TAURI_INTERNALS__;
      if (!internals) {
        return { wrote: false, text: null };
      }
      await navigator.clipboard.writeText("seven-shell-e2e");
      return { wrote: true, text: await internals.invoke("clipboard_text") };
    });

    expect(answer.wrote, "the webview can still write a clipboard, which the copy entries rely on").toBe(true);
    expect(answer.text).toBe("seven-shell-e2e");
  } finally {
    await close();
  }
});

/** The rows the list is drawing, top to bottom, by the key each one carries. */
function rowKeys(page: Page): Promise<string[]> {
  return page
    .locator("[data-row]")
    .evaluateAll((elements) => elements.map((element) => element.getAttribute("data-row") ?? ""));
}

/** The key of the row an element is drawn in, which is how a heading or a row is named in a list. */
async function rowKeyOf(element: Locator): Promise<string> {
  const key = await element.evaluate((node) => node.closest("[data-row]")?.getAttribute("data-row") ?? null);
  if (key === null) {
    throw new Error("the element is not drawn in a row");
  }
  return key;
}

/** The group row a session row sits under: the nearest group row above it in the list. */
function groupOver(keys: readonly string[], at: number): string | null {
  const above = keys.slice(0, at).filter((key) => key.startsWith("group:"));
  return above.at(-1) ?? null;
}

/** A save dialog answered with a session that is only ever connected to by hand. */
async function saveSession(page: Page, name: string): Promise<void> {
  await page.getByRole("button", { name: "New session" }).click();
  const editor = page.getByRole("dialog");
  await editor.getByPlaceholder("Production web server").fill(name);
  await editor.getByPlaceholder("example.com").fill("127.0.0.1");
  await editor.getByPlaceholder("root").fill("labuser");
  await editor.getByRole("button", { name: "Save" }).click();
  await expect(editor).toHaveCount(0);
}

/** A group made where it is drawn: a name typed into the form under the list's header. */
async function saveGroup(page: Page, name: string): Promise<void> {
  await page.getByRole("button", { name: "New group" }).click();
  await page.getByPlaceholder("Group name").fill(name);
  await page.getByRole("button", { name: "Create", exact: true }).click();
  await expect(page.getByRole("heading", { name, exact: true })).toBeVisible();
}

/**
 * Groups, from making one to taking a session back out of it.
 *
 * The half of this that only a real window can show is the submenu: the list a session is moved
 * through is drawn against the row that opened it, and the choice that lands is the one that was
 * pointed at. What is asserted at the end is the sentence the confirmation says out loud — deleting
 * a group deletes the group, and what was inside it is still on screen.
 */
test("a session can be put in a group, renamed, and taken back out", async () => {
  const { page, close } = await openApp();
  try {
    await saveSession(page, "grouped-host");
    await saveGroup(page, "Lab");

    // The row's own menu, two levels deep, which is how a session is moved without a drag.
    const row = page.getByTitle("Open a terminal for grouped-host");
    await row.click({ button: "right" });
    await page.getByRole("menuitem", { name: "Move to group" }).hover();
    const intoLab = page.getByRole("menuitemcheckbox", { name: "Lab", exact: true });
    await expect(intoLab).toHaveAttribute("aria-checked", "false");
    await intoLab.click();

    // The row it sits under is the answer to where it went: the nearest group row above it.
    const sessionKey = await rowKeyOf(row);
    const groupKey = await rowKeyOf(page.getByRole("heading", { name: "Lab", exact: true }));
    await expect
      .poll(async () => {
        const keys = await rowKeys(page);
        return groupOver(keys, keys.indexOf(sessionKey));
      })
      .toBe(groupKey);

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "groups.png"), fullPage: false });

    // Renaming happens where the name is written: the label in the row becomes a field, through the
    // heading's own menu, and Enter writes it.
    await page.getByRole("heading", { name: "Lab", exact: true }).click({ button: "right" });
    await page.getByRole("menuitem", { name: "Rename group" }).click();
    const rename = page.getByRole("textbox", { name: "Group name" });
    await expect(rename).toHaveValue("Lab");
    await rename.fill("Lab (EU)");
    await rename.press("Enter");
    await expect(page.getByRole("heading", { name: "Lab (EU)", exact: true })).toBeVisible();

    // Deleting asks first, in the heading, and says what happens to what is inside.
    await page.getByRole("heading", { name: "Lab (EU)", exact: true }).click({ button: "right" });
    await page.getByRole("menuitem", { name: "Delete group" }).click();
    await expect(page.getByText(/Delete the group “Lab \(EU\)”\?/)).toBeVisible();
    await page.getByRole("button", { name: "Delete", exact: true }).click();

    // The group is gone and the session is not. With no group left there is no heading over it at
    // all — a flat list has nothing to set it apart from.
    await expect(page.getByRole("heading", { name: "Lab (EU)", exact: true })).toHaveCount(0);
    await expect(page.getByRole("heading", { name: "Ungrouped", exact: true })).toHaveCount(0);
    await expect(row).toBeVisible();
  } finally {
    await close();
  }
});

/**
 * The drag, driven the way a hand drives it.
 *
 * The pointer is real: press on the row, travel past the threshold, and the row is lifted — a ghost
 * follows the pointer and the row it would land on is drawn as the destination. Nothing about that
 * can be settled by reading the source, which is why it is asserted here: the release is the only
 * thing that says which of the two the interface settled on.
 *
 * The second half is nesting: one group dropped in the middle of another becomes its child, and is
 * drawn further in than the group it was dropped on.
 */
test("a session can be dragged into a group, and a group into another group", async () => {
  const { page, close } = await openApp();
  try {
    await saveSession(page, "dragged-host");
    await saveGroup(page, "Drop");
    await saveGroup(page, "Outer");

    const row = page.getByTitle("Open a terminal for dragged-host");
    const target = page.getByRole("heading", { name: "Drop", exact: true });
    const from = await row.boundingBox();
    const to = await target.boundingBox();
    if (!from || !to) {
      throw new Error("the rows have no place on screen");
    }

    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
    await page.mouse.down();
    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2 + 12, { steps: 2 });
    await page.mouse.move(to.x + to.width / 2, to.y + to.height / 2, { steps: 8 });

    // What is moving is drawn under the pointer, and the row it would land on says so.
    await expect(page.locator("[data-drag-ghost]")).toBeVisible();
    await expect(page.locator("[data-drag-ghost]")).toHaveText("dragged-host");
    const destination = target.locator("xpath=..");
    expect(await destination.evaluate((node) => node.className)).toContain("ring-primary");

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "drag.png"), fullPage: false });

    await page.mouse.up();
    await expect(page.locator("[data-drag-ghost]")).toHaveCount(0);

    // It is under "Drop" now, which is the only thing the release had to get right.
    const sessionKey = await rowKeyOf(row);
    const dropKey = await rowKeyOf(target);
    await expect
      .poll(async () => {
        const keys = await rowKeys(page);
        return groupOver(keys, keys.indexOf(sessionKey));
      })
      .toBe(dropKey);

    // A group dropped in the middle of another becomes its child: drawn further in than its parent.
    const outerRow = page.getByRole("heading", { name: "Outer", exact: true }).locator("xpath=..");
    const dropRow = target.locator("xpath=..");
    const dropBox = await dropRow.boundingBox();
    const outerBox = await outerRow.boundingBox();
    if (!dropBox || !outerBox) {
      throw new Error("the group rows have no place on screen");
    }
    await page.mouse.move(dropBox.x + dropBox.width / 2, dropBox.y + dropBox.height / 2);
    await page.mouse.down();
    await page.mouse.move(dropBox.x + dropBox.width / 2, dropBox.y + dropBox.height / 2 + 12, { steps: 2 });
    await page.mouse.move(outerBox.x + outerBox.width / 2, outerBox.y + outerBox.height / 2, { steps: 8 });
    await page.mouse.up();

    await expect
      .poll(async () => Number.parseInt(await dropRow.evaluate((node) => node.style.paddingLeft), 10))
      .toBeGreaterThan(Number.parseInt(await outerRow.evaluate((node) => node.style.paddingLeft), 10));

    await page.screenshot({ path: path.join(ARTIFACTS, "nested-groups.png"), fullPage: false });
  } finally {
    await close();
  }
});
