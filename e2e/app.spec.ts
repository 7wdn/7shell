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
async function writtenSettings(): Promise<{
  packedTransfer?: boolean;
  themePalette?: string;
  uiScale?: number;
  cornerStyle?: string;
  homeLayout?: string;
  uiFont?: string;
  terminalFont?: string;
  customColors?: { primary?: string };
  backgroundImage?: { fileName?: string; displayMode?: string; transparency?: number; blur?: number };
  terminalFontSize?: number;
  customColorsDark?: { primary?: string };
}> {
  return JSON.parse(await readFile(SETTINGS_FILE, "utf8")) as {
    packedTransfer?: boolean;
    themePalette?: string;
    uiScale?: number;
    cornerStyle?: string;
    homeLayout?: string;
    uiFont?: string;
    terminalFont?: string;
    customColors?: { primary?: string };
    backgroundImage?: { fileName?: string; displayMode?: string; transparency?: number; blur?: number };
    terminalFontSize?: number;
    customColorsDark?: { primary?: string };
  };
}

/**
 * Chooses which of the two home pages is drawn, through the settings page, and writes it.
 *
 * The cases either side of this one are about different pages, and neither should depend on which one a
 * fresh install happens to open on: each says which page it is about, out loud, through the interface a
 * person would use.
 */
async function chooseHomePage(page: Page, which: "Introduction" | "Workspace overview"): Promise<void> {
  await settingsButton(page).click();
  const settings = page.getByRole("region", { name: "Settings" });
  await settings.getByRole("button", { name: "Appearance", exact: true }).click();
  await settings.getByRole("radio", { name: new RegExp(`^${which}`) }).click();
  await applySettings(page);
  await page.getByRole("button", { name: "Close settings" }).click();
}

/**
 * The settings page's Apply: writes down everything chosen since the last one.
 *
 * A choice takes effect at once and the file is told when somebody says so — that is what the bar at
 * the foot of the page is for, and why every case that reads the file presses it first. It checks the
 * button itself on the way past: enabled while there is something to write, disabled after.
 */
async function applySettings(page: Page): Promise<void> {
  const button = page.getByRole("button", { name: "Apply", exact: true });
  await expect(button, "there is something to write").toBeEnabled();
  await button.click();
  await expect(button, "and now there is not").toBeDisabled();
}

/**
 * The three regions the same words appear in.
 *
 * The home page offers the sessions that were used last and buttons with the same names as the
 * sidebar's, and both are on screen at once — so an assertion has to say which of them it is about,
 * and a name that matches twice is a failure rather than a guess.
 */
function sidebar(page: Page): Locator {
  return page.locator("[data-session-list]").locator("xpath=ancestor::aside[1]");
}

/** The home page, which is what the workspace draws while no tab is open. */
function home(page: Page): Locator {
  return page.locator("[data-home]");
}

/** A saved session's row in the sidebar, by the title its own button carries. */
function sessionRow(page: Page, name: string): Locator {
  return sidebar(page).getByTitle(`Open a terminal for ${name}`);
}

/**
 * The same title, rooted at the document rather than at the sidebar.
 *
 * For `filter({ has })`, which re-roots its inner locator against each candidate element: a
 * locator that starts by naming the document — as {@link sessionRow} does — cannot be re-rooted,
 * and would ask for the sidebar *inside* the row. The rows on the home page carry the same title,
 * which is harmless here because a candidate is already a row of the sidebar's list.
 */
function anySessionRow(page: Page, name: string): Locator {
  return page.getByTitle(`Open a terminal for ${name}`);
}

/** The settings button the frame draws, which is the one the home page does not have. */
function settingsButton(page: Page): Locator {
  return page.getByRole("banner").getByRole("button", { name: "Settings" });
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

    // The shell is up when the sidebar — its own controls and the list it holds — is on screen.
    await expect(page.getByRole("button", { name: "Import / Export" })).toBeVisible();
    await expect(page.getByLabel("Search sessions")).toBeVisible();

    // A control is not text, and that is said once for every button in the base layer rather than on
    // each of them — so what is checked here is that the rule reaches a real one.
    await expect(page.getByRole("button", { name: "Import / Export" })).toHaveCSS("user-select", "none");

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "shell.png"), fullPage: false });
  } finally {
    await close();
  }
});

/**
 * The home page: what the workspace draws while nothing is open.
 *
 * Three things only a real window can show. That it is what a shell opens onto and what it comes
 * back to — the second half by opening a page over it and closing that page again. That its quick
 * start list is the same library the sidebar holds, reached without the sidebar, which is the whole
 * reason the page has shortcuts. And that a row there makes the *same* attempt the sidebar's row
 * makes: pointed at a closed port it fails, and the mark that failure leaves is on the home page's
 * own row, which is where somebody who clicked there is looking.
 */
test("the home page opens a session from its own list, and says so when it cannot", async () => {
  const { page, close } = await openApp();
  try {
    // The working page, asked for rather than assumed: a fresh install opens on the introduction.
    await chooseHomePage(page, "Workspace overview");

    const board = home(page);
    await expect(board.getByRole("heading", { name: "Quick start" })).toBeVisible();
    await expect(board.getByText("No sessions yet", { exact: false })).toBeVisible();
    await expect(board.getByRole("heading", { name: "Common operations" })).toBeVisible();
    // Nothing has been sent yet in a window this fresh, which is the other half of the panel.
    await expect(board.getByRole("heading", { name: "History" })).toBeVisible();
    await expect(board.getByText("Nothing sent yet.", { exact: false })).toBeVisible();
    // "Getting started" is the other home page's card, and which one is drawn is its own case.

    // Saved through the sidebar's own button, so what appears below is the same library read from
    // two places rather than a list the home page keeps for itself.
    await saveSession(page, "home-probe", "127.0.0.1", "1");
    const row = board.locator("[data-quick-session]").filter({ hasText: "home-probe" });
    await expect(row).toBeVisible();

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "home.png"), fullPage: false });

    // Nothing is listening on port 1, so this attempt fails. What matters is where the answer is
    // drawn: on this page's row, not only on the sidebar's row for the same session.
    await row.getByTitle("Open a terminal for home-probe").click();
    const mark = row.locator("[data-session-failure]");
    await expect(mark).toBeVisible({ timeout: 30_000 });
    await expect(page.getByText("Could not connect to home-probe", { exact: false }).first()).toBeVisible();

    await mark.click();
    await expect(page.locator("[data-session-failure-detail]")).toBeVisible();
    await page.getByRole("button", { name: "Clear error" }).click();
    await expect(mark).toHaveCount(0);

    // A shortcut that opens a page over this one, and the page being there again once it is closed.
    // The shortcut is found inside the home page, which is also the proof that the title bar's
    // button of the same name is not the one being pressed.
    await board.getByRole("button", { name: "Settings" }).click();
    await expect(page.getByRole("region", { name: "Settings" })).toBeVisible();
    await page.getByRole("button", { name: "Close settings" }).click();
    await expect(board.getByRole("heading", { name: "Quick start" })).toBeVisible();

    // Put back the page a fresh install has, since that is the state the cases after this one were
    // written against and this one chose the other page to work with.
    await chooseHomePage(page, "Introduction");
    await expect(board.getByRole("heading", { name: "Getting started" })).toBeVisible();
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

    // The panel is an instrument rather than a document: a drag across it selects nothing, and the
    // pointer does not turn into an I-beam over the readings. One class is what makes that happen,
    // and a class says nothing about how a browser treats the text inside it — the computed style is
    // the half of it that only a real window settles.
    await expect(panel).toHaveCSS("user-select", "none");

    // The three cards are the way in to the host's pages. With nothing connected there is no host
    // to open a page about, so they are drawn and refuse to be used rather than opening an empty
    // page — which is the state a person meets them in the first time they see the panel.
    for (const name of ["System info", "Processes", "Network"]) {
      await expect(panel.getByRole("button", { name, exact: true })).toBeDisabled();
    }

    // The panel's own button writes the preference, which is what makes the next assertion true.
    await panel.getByRole("button", { name: "Hide the monitor" }).click();
    await expect(panel).toHaveCount(0);

    await settingsButton(page).click();

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
    await sidebar(page).getByRole("button", { name: "New session" }).click();
    const editor = page.getByRole("dialog");
    await editor.getByPlaceholder("Production web server").fill("lab");
    await editor.getByPlaceholder("example.com").fill("127.0.0.1");
    await editor.getByPlaceholder("root").fill("labuser");
    await editor.getByRole("button", { name: "Save" }).click();
    await expect(editor).toHaveCount(0);

    // The row's name is its title as well, which is what makes it findable without connecting.
    const row = sessionRow(page, "lab");
    await expect(row).toBeVisible();

    await row.click({ button: "right" });
    const menu = page.getByRole("menu");
    await expect(menu).toBeVisible();
    // A menu is chrome: its rows are verbs, and a drag across one is somebody leaving in a hurry.
    // The class is in the markup; the computed style is what the browser actually does with it.
    await expect(menu).toHaveCSS("user-select", "none");
    // The row of its own rather than a submenu, because this library has no group to move into yet:
    // the entry that makes one is drawn where the list of destinations would have been.
    await expect(menu.getByRole("menuitem")).toHaveText([
      "Open a terminal for lab",
      "Open files for lab",
      "Edit session",
      "Move to a new group",
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
 * Deleting a session, and the question that stands in front of it.
 *
 * The question is a dialog over the window rather than a strip that opens inside the row — which is
 * what it was — and the difference a person meets first is the keyboard: Escape leaves it, the
 * focus starts on the button that changes nothing, and the row is untouched until the question is
 * answered. Both ways out are asserted, because "asks first" is only true if the other answer works.
 */
test("deleting a session asks first, and Escape leaves it alone", async () => {
  const { page, close } = await openApp();
  try {
    await saveSession(page, "doomed-host");

    const row = sessionRow(page, "doomed-host");
    await expect(row).toBeVisible();

    // The row carries no buttons of its own any more: what can be done to a session is in its menu,
    // which is where the same actions on any other kind of row already live.
    const ask = async () => {
      await row.click({ button: "right" });
      await page.getByRole("menuitem", { name: "Delete session" }).click();
    };
    await ask();
    const confirm = page.getByRole("dialog");
    await expect(confirm.getByText(/Delete “doomed-host”\?/)).toBeVisible();

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "confirm-dialog.png"), fullPage: false });

    await page.keyboard.press("Escape");
    await expect(confirm).toHaveCount(0);
    await expect(row).toBeVisible();

    // Answering it is what removes the row, and nothing else does.
    await ask();
    await confirm.getByRole("button", { name: "Delete session", exact: true }).click();

    await expect(confirm).toHaveCount(0);
    await expect(row).toHaveCount(0);
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
    await sidebar(page).getByRole("button", { name: "New session" }).click();
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
    // Pageant is the method this build refuses now: the machine's own agent became a real path, and
    // the row that says so moved with it.
    await expect(list.getByRole("option", { name: "Pageant" })).toHaveAttribute("aria-disabled", "true");
    await expect(list.getByRole("option", { name: "SSH agent" })).not.toHaveAttribute("aria-disabled", "true");
    await expect(list.getByRole("option", { name: "Password" })).toHaveAttribute("aria-selected", "true");

    // And it looks the part: a row nobody can choose has to be drawn differently from one they can,
    // or the only way to find out is to try it. The platform drew that for free; this does not.
    const [unavailable, available] = await Promise.all(
      ["Pageant", "Public key"].map(async (name) =>
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
    await sidebar(page).getByRole("button", { name: "New session" }).click();
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
    await settingsButton(page).click();
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
    await applySettings(page);

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
    await settingsButton(page).click();
    const settings = page.getByRole("region", { name: "Settings" });

    // The settings page is a tab, so the strip of tabs is on screen — and it is chrome rather than a
    // document: a drag along it is somebody scrolling or reaching for a tab. A class says nothing
    // about how a browser treats the text inside it, so the computed style is what is checked here;
    // the strip is drawn only while a tab is open, which is why this is the case that carries it.
    await expect(page.locator("[data-workspace-tabs]")).toHaveCSS("user-select", "none");

    // The page as it opens: the first category, which is the one that is nearly all dropdowns —
    // a category with switches on it is the second shot below.
    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "settings-appearance.png"), fullPage: false });

    // The appearance category opens with three settings drawn as fields rather than as cards — a
    // language, a colour and a size — because none of them needs a paragraph to be understood.
    await expect(settings.getByRole("combobox", { name: "Language" })).toBeVisible();
    const colour = settings.getByRole("combobox", { name: "Colour theme" });
    const scale = settings.getByRole("combobox", { name: "Interface scale" });
    await expect(colour).toBeVisible();
    await expect(scale).toBeVisible();

    // The page is chrome: a drag across it takes nothing away. The few readings it holds keep an
    // exemption of their own, and the search field is one of them — a surface that takes typing is a
    // document while the keyboard is in it, whatever the page around it says.
    await expect(settings).toHaveCSS("user-select", "none");
    await expect(settings.getByRole("searchbox")).toHaveCSS("user-select", "text");

    // The palette the list calls 洁白 / White is the base theme, and its two halves are the ones the
    // window is drawn in before anybody has chosen anything — white with near-black text, and the
    // near-black grey with light text. Named here as the colours they are rather than as "some dark
    // grey": a base palette that drifted by a shade is exactly the kind of thing nobody notices.
    //
    // Painted onto a canvas and read back rather than compared as text: these colours are written in
    // oklch, so the computed value is the spelling rather than the pixels it stands for, and
    // comparing the two would be comparing spellings. What is asked here is what the window is
    // actually painted with, which is the only form "the same colour as theirs" takes.
    const paintedBody = (): Promise<string> =>
      page.evaluate(() => {
        const canvas = document.createElement("canvas");
        canvas.width = 1;
        canvas.height = 1;
        const context = canvas.getContext("2d");
        if (!context) {
          throw new Error("no 2d context to read a colour through");
        }
        context.fillStyle = getComputedStyle(document.body).backgroundColor;
        context.fillRect(0, 0, 1, 1);
        const [r, g, b] = context.getImageData(0, 0, 1, 1).data;
        return `rgb(${r} ${g} ${b})`;
      });

    // The three modes are three buttons rather than a dropdown: the two somebody switches between
    // are one click each, and the one that hands the decision back is the third.
    const light = settings.getByRole("radio", { name: "Light", exact: true });
    const dark = settings.getByRole("radio", { name: "Dark", exact: true });
    await light.click();
    await expect.poll(paintedBody, { message: "the light base is the reference's white" }).toBe("rgb(255 255 255)");
    // Kept as a picture, because the light half is the one nobody will look at twice: the window is
    // white, its accent is near-black, and every panel edge is one flat grey.
    await page.screenshot({ path: path.join(ARTIFACTS, "settings-appearance-light.png"), fullPage: false });
    await dark.click();
    await expect.poll(paintedBody, { message: "and its dark base is the grey it declares" }).toBe("rgb(19 20 22)");

    // Looking at a colour is not choosing one. Resting the pointer on an option shows it at once and
    // writes nothing; letting go of the list puts back whatever was actually chosen. This is the
    // whole reason the list of colours is worth having: a colour says what it is by being worn.
    //
    // The option hovered is one that is on screen when the list opens, which is not a detail: the
    // list is longer than it is tall, and hovering something near the end scrolls it — and a scroll
    // moves the items under a pointer that has not moved, so what is previewed is whatever lands
    // there. That is the list behaving correctly and a case that would be measuring the wrong thing.
    const committed = (await page.locator("html").getAttribute("data-palette")) ?? "default";
    await colour.click();
    await page.getByRole("listbox").getByRole("option", { name: "Mist grey" }).hover();
    await expect(page.locator("html")).toHaveAttribute("data-palette", "mist");
    expect((await writtenSettings()).themePalette ?? "default", "a look is not a choice").toBe(committed);

    // The list while the pointer is on an option: the palettes with their own colours beside them,
    // and the window already wearing the one under the pointer rather than the one it has chosen.
    await page.screenshot({ path: path.join(ARTIFACTS, "settings-colour-themes.png"), fullPage: false });

    await page.keyboard.press("Escape");
    await expect(page.locator("html"), "letting go of the list puts the chosen colour back").toHaveAttribute(
      "data-palette",
      committed,
    );

    // A colour that is chosen is written down, worn by the document, and actually drawn: the file is
    // what the next run reads, the attribute is what the stylesheet selects on, and the background
    // is the proof that the second one reached the first. The attribute alone would pass while every
    // palette block in the stylesheet was misspelled.
    const backgroundBefore = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
    await colour.click();
    await page.getByRole("listbox").getByRole("option", { name: "Cobalt blue" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-palette", "cobalt");
    await applySettings(page);

    await expect
      .poll(async () => (await writtenSettings()).themePalette, { message: "the colour reached the file" })
      .toBe("cobalt");
    expect(
      await page.evaluate(() => getComputedStyle(document.body).backgroundColor),
      "the palette's own tokens are the ones being drawn",
    ).not.toBe(backgroundBefore);
    // And it is a tint over the theme rather than a third theme: choosing a colour says nothing
    // about which of the two the window is drawn in.
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

    // The pane is dressed from the same palette: a cobalt window with a grey terminal in the middle
    // of it was the one thing on screen that did not belong to the colour somebody chose. Read
    // through a probe carrying the pane's class rather than through an open terminal — what is being
    // checked is the stylesheet reaching the pane, and an open terminal needs a server to be reached.
    const pane = (): Promise<string> =>
      page.evaluate(() => {
        const probe = document.createElement("div");
        probe.className = "terminal-dark";
        document.body.append(probe);
        const colour = getComputedStyle(probe).getPropertyValue("--terminal-background").trim();
        probe.remove();
        return colour;
      });
    expect(await pane(), "the pane follows the colour the window is wearing").not.toBe(backgroundBefore);

    // Corner style: one attribute, and every radius in the window moves with it. Read off probe
    // elements rather than off a component, because the claim is about what a class means in the
    // built stylesheet — including the bare `rounded`, which Tailwind gives a fixed value of its own
    // and which several hundred elements carry.
    const radiusOf = (className: string): Promise<string> =>
      page.evaluate((name) => {
        const probe = document.createElement("div");
        probe.className = name;
        document.body.append(probe);
        const radius = getComputedStyle(probe).borderTopLeftRadius;
        probe.remove();
        return radius;
      }, className);

    expect(await radiusOf("rounded-lg"), "the largest step, as it has always been").toBe("10px");
    expect(await radiusOf("rounded"), "and the smallest, which was Tailwind's own 4px").toBe("6px");

    await settings.getByRole("radio", { name: "Square", exact: true }).click();

    await expect(page.locator("html")).toHaveAttribute("data-corner-style", "none");
    expect(await radiusOf("rounded-lg"), "square means square").toBe("0px");
    expect(await radiusOf("rounded"), "everywhere, not only where the token was used").toBe("0px");
    await applySettings(page);

    await expect.poll(async () => (await writtenSettings()).cornerStyle).toBe("none");

    await settings.getByRole("radio", { name: "Large", exact: true }).click();
    await expect(page.locator("html")).toHaveAttribute("data-corner-style", "large");

    // The two fonts. What a font setting is worth is a stack, and the family under the pointer is
    // shown before it is taken — through the token the stylesheet reads, which is why the whole
    // window follows a hover rather than only the row being looked at.
    const sans = (): Promise<string> =>
      page.evaluate(() => document.documentElement.style.getPropertyValue("--font-sans"));

    const interfaceFont = settings.getByRole("button", { name: "Interface font", exact: true });
    const interfaceBefore = await sans();
    const fontList = page.getByRole("listbox");

    // Opened with the keyboard rather than with a pointer. A panel opens where the pointer is, and the
    // click that opened it is still being delivered as it arrives: the release lands on the panel, the
    // panel reads that as a dismissal, and the list is gone before anybody can look at it. The case is
    // about the families in the list, and Enter is how somebody who cannot use a mouse opens it.
    await interfaceFont.press("Enter");
    await expect(fontList).toBeVisible();

    // Two rows answer to "Consolas": the preset, which carries the fallbacks this build gives it, and
    // the machine's own spelling of the same family. The first is the one a person finds first, and
    // the one a list led by the fixed-width families puts in front of them.
    await fontList.getByRole("option", { name: "Consolas" }).first().hover();
    await expect
      .poll(sans, { message: "the family under the pointer is the one being drawn" })
      .toBe("'Consolas', 'Courier New', monospace");
    // The list itself, kept as a picture: every row is drawn in the face it offers, which is what
    // makes a list of names worth reading — and what makes the pointer's row visibly the one worn.
    await page.screenshot({ path: path.join(ARTIFACTS, "settings-font-list.png"), fullPage: false });
    await page.keyboard.press("Escape");
    await expect.poll(sans, { message: "letting go of the list puts the chosen family back" }).toBe(interfaceBefore);

    await interfaceFont.press("Enter");
    await fontList.getByRole("option", { name: "Consolas" }).first().click();
    await applySettings(page);

    await expect.poll(async () => (await writtenSettings()).uiFont ?? "").toBe("'Consolas', 'Courier New', monospace");

    // The terminal's list is the other setting, and what it changes goes to the panes: a terminal is
    // a canvas with one font handed to it, so the family travels through the store rather than
    // through the stylesheet, and "the open pane follows" is asserted in that pane's own spec. What
    // is asked here is the half a page can see — hovering the terminal's list dresses nothing on
    // this page, which is the failure worth catching: a preview that dresses the wrong thing.
    await settings.getByRole("button", { name: "Appearance", exact: true }).click();

    // The terminal's own two rows live on the terminal's own page now, which is where a family for it
    // is chosen — so this case walks over there and back.
    await settings.getByRole("button", { name: "SSH terminal", exact: true }).click();
    const terminalFont = settings.getByRole("button", { name: "Terminal font", exact: true });
    await terminalFont.press("Enter");
    await expect(fontList).toBeVisible();
    await fontList.getByRole("option", { name: "JetBrains Mono" }).hover();
    await expect
      .poll(sans, { message: "the interface is not the terminal's list to dress" })
      .toBe("'Consolas', 'Courier New', monospace");
    await page.keyboard.press("Escape");
    await page.screenshot({ path: path.join(ARTIFACTS, "settings-fonts.png"), fullPage: false });

    await terminalFont.press("Enter");
    await fontList.getByRole("option", { name: "JetBrains Mono" }).click();
    await applySettings(page);

    await expect
      .poll(async () => (await writtenSettings()).terminalFont ?? "")
      .toBe("'JetBrains Mono', 'Fira Code', monospace");

    // The machine's own names, in the language this machine is in. An installation in Chinese has
    // 微软雅黑, 黑体 and 楷体, and the names come from the platform rather than from the font files —
    // a file carries both spellings and picking one here would be this build guessing what language
    // the computer is set to. Typed into the field rather than scrolled to, which is what the field
    // is for: a list of two hundred families is a list nobody scrolls.
    //
    // Which means walking back to the page the interface's own font lives on first: the app keeps no
    // memory of where somebody was, and the walk over to the terminal's page and back is this case's.
    await settings.getByRole("button", { name: "Appearance", exact: true }).click();
    await interfaceFont.press("Enter");
    // The panel's own search box, which is where the typing goes: the field beside the row shows the
    // chosen family and the search is inside the panel, which is the shape a list of two hundred
    // families asks for.
    await expect(page.getByRole("combobox", { name: "Search fonts…" })).toBeFocused();
    await page.keyboard.type("黑");
    await expect(fontList.getByRole("option", { name: /黑/ }).first()).toBeVisible();
    await page.screenshot({ path: path.join(ARTIFACTS, "settings-font-cjk.png"), fullPage: false });
    await page.keyboard.press("Escape");

    // Custom: five colours, and everything else derived from them — the panels, the hovered rows,
    // the text written on the accent, and the pane. Two halves of that are checked here, the one the
    // pickers write straight onto the document and the one it derives.
    await colour.click();
    await page.getByRole("listbox").getByRole("option", { name: "Custom" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-palette", "custom");

    const accent = settings.getByLabel("Accent");
    await expect(accent).toBeVisible();
    await accent.evaluate((input: HTMLInputElement) => {
      input.value = "#4f9cf0";
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await expect
      .poll(() => page.evaluate(() => document.documentElement.style.getPropertyValue("--primary")), {
        message: "the picker's colour is what the window is drawn from",
      })
      .toBe("rgb(79 156 240)");
    await applySettings(page);

    await expect
      .poll(async () => {
        // The pickers are about the set for the theme on screen; the other half of the palette is
        // not being edited, and reading it would be reading the wrong answer.
        const written = await writtenSettings();
        const dark = (await page.locator("html").getAttribute("data-theme")) === "dark";
        return dark ? written.customColorsDark?.primary : written.customColors?.primary;
      })
      .toBe("#4f9cf0");
    await page.screenshot({ path: path.join(ARTIFACTS, "settings-custom-colours.png"), fullPage: false });

    // The scale is the one preference here that changes the page every case after this one is driven
    // in, so it is checked by what it does to the window rather than by the file alone: the window
    // keeps its size and the interface inside it is drawn larger, which is the same window holding
    // fewer of its own pixels across.
    const widthBefore = await page.evaluate(() => window.innerWidth);
    await scale.click();
    await page.getByRole("listbox").getByRole("option", { name: "125%" }).click();
    await expect
      .poll(async () => (await page.evaluate(() => window.innerWidth)) / widthBefore, {
        message: "the interface was drawn larger inside the same window",
      })
      .toBeCloseTo(1 / 1.25, 2);
    await applySettings(page);

    await expect.poll(async () => (await writtenSettings()).uiScale).toBe(125);

    // Both put back, so that the rest of the run meets the page it expects. Leaving a hand-picked
    // palette would do it too, and it would be the wrong kind of leaving: its tokens are written on
    // the document rather than selected in the stylesheet, and every palette after it would be drawn
    // underneath them.
    await colour.click();
    await page.getByRole("listbox").getByRole("option", { name: "White" }).click();
    await scale.click();
    await page.getByRole("listbox").getByRole("option", { name: "100%" }).click();
    await interfaceFont.press("Enter");
    await page.getByRole("listbox").getByRole("option", { name: "App default" }).click();

    // The terminal's own family is on the terminal's own page, so putting it back means going there —
    // and coming back, since the rest of this case is about the appearance page.
    await settings.getByRole("button", { name: "SSH terminal", exact: true }).click();
    await settings.getByRole("button", { name: "Terminal font", exact: true }).press("Enter");
    await page.getByRole("listbox").getByRole("option", { name: "App default" }).click();
    await settings.getByRole("button", { name: "Appearance", exact: true }).click();

    await expect(page.locator("html")).toHaveAttribute("data-palette", "default");
    await expect.poll(() => page.evaluate(() => document.documentElement.style.getPropertyValue("--primary"))).toBe("");

    await settings.getByRole("button", { name: "Files", exact: true }).click();

    // Away from its default first, so that restoring has something to do.
    const packedSwitch = settings.getByRole("switch", { name: "Packed transfer" });
    await packedSwitch.click();
    await applySettings(page);

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

    await applySettings(page);

    await expect
      .poll(async () => (await writtenSettings()).packedTransfer, {
        message: "and the default reached the settings file as well",
      })
      .toBe(false);

    // Closed on the way out, so that the case after this one meets a page that has nothing pending —
    // it asserts that first, and a page left open with a choice in it would fail on somebody else's
    // behalf.
    await page.getByRole("button", { name: "Close settings" }).click();
  } finally {
    // Whatever this case left open, it is not left open for the next one. A listbox is an overlay
    // over the whole window: a case that failed with one standing does not fail on its own, it takes
    // every case after it down with it — eight of them, each waiting out its full timeout, which is
    // twelve minutes of run for one wrong assertion. Escape is what a person would press.
    await page.keyboard.press("Escape").catch(() => undefined);
    await close();
  }
});

/**
 * The bar at the foot of the settings page: apply, close, and the one way out that throws something
 * away.
 *
 * A choice takes effect the moment it is made, because these are settings whose whole content is what
 * they look like — and the *file* is written when somebody says so. So there are two things to check
 * and they are not the same thing: that choosing changes the window without touching the disk, and
 * that leaving with something unwritten asks rather than deciding on its own.
 */
test("a choice is applied at once, written on Apply, and dropped only when asked", async () => {
  const { page, close } = await openApp();
  try {
    await settingsButton(page).click();
    const settings = page.getByRole("region", { name: "Settings" });
    const apply = page.getByRole("button", { name: "Apply", exact: true });

    // The category the last case left behind is not this one: the theme is in the appearance page,
    // and the page opens on whichever category it was closed on.
    await settings.getByRole("button", { name: "Appearance", exact: true }).click();

    // Whatever the case before this one left pending is written down first: this case is about what
    // happens when nothing is, and asserting that on somebody else's leftovers is a case about the
    // order the file happens to run in.
    if (await apply.isEnabled()) {
      await apply.click();
      await expect(apply).toBeDisabled();
    }

    // Nothing has been chosen, so there is nothing to write and the button says so.
    await expect(apply).toBeDisabled();

    await settings.getByRole("radio", { name: "Light", exact: true }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(apply, "a choice is something to write").toBeEnabled();
    expect((await writtenSettings()).themeMode ?? "system", "the window is ahead of the file").not.toBe("light");

    // Closing is the way out that throws things away, so it asks — with three answers, because two
    // would have to leave one of "write it", "drop it" and "stay" unnamed.
    await settings.getByRole("button", { name: "Close", exact: true }).click();
    const question = page.getByRole("dialog");
    await expect(question).toBeVisible();
    await question.getByRole("button", { name: "Drop them" }).click();

    await expect(settings).toHaveCount(0);
    await expect(page.locator("html"), "the window goes back to what was written").toHaveAttribute(
      "data-theme",
      "dark",
    );
    expect((await writtenSettings()).themeMode ?? "system").not.toBe("light");

    // The other answer: write it on the way out.
    await settingsButton(page).click();
    const reopened = page.getByRole("region", { name: "Settings" });
    await reopened.getByRole("button", { name: "Appearance", exact: true }).click();
    await reopened.getByRole("radio", { name: "Light", exact: true }).click();
    await page.getByRole("region", { name: "Settings" }).getByRole("button", { name: "Close", exact: true }).click();
    await page.getByRole("dialog").getByRole("button", { name: "Write them and close" }).click();

    await expect.poll(async () => (await writtenSettings()).themeMode ?? "system").toBe("light");

    // Put back, so that the rest of the run meets a dark window: the choice is applied and then
    // written, which is the pair this case is about.
    await settingsButton(page).click();
    const again = page.getByRole("region", { name: "Settings" });
    await again.getByRole("radio", { name: "Dark", exact: true }).click();
    await applySettings(page);
    await expect.poll(async () => (await writtenSettings()).themeMode ?? "light").toBe("dark");

    // Closed, so that the case after this one meets the window it expects: nothing open is the home
    // page, and a case cannot assume it is there while a tab of somebody else's is.
    await page.getByRole("button", { name: "Close settings" }).click();
  } finally {
    await close();
  }
});

/**
 * The interface layout, which is the one appearance choice whose whole effect is on the chrome.
 *
 * What only a real window can settle: a tab takes its shape from the setting rather than only being
 * stored, and the headers give the row back — measured as the two computed numbers a person sees,
 * the corner of a tab and the room above the monitor's title, rather than as the class names that
 * happen to produce them. The other half is the file, which is where the next launch reads it from.
 */
test("the interface layout redraws the chrome, and the file hears about it", async () => {
  const { page, close } = await openApp();
  try {
    await settingsButton(page).click();
    const settings = page.getByRole("region", { name: "Settings" });
    await settings.getByRole("button", { name: "Appearance", exact: true }).click();

    // The settings page is a tab of its own, so the strip this is about is on screen while it is
    // open — and the monitor's header is beside it, which is the other half of the same choice.
    const tab = page.locator("[data-workspace-tabs] > div").first();
    const panel = page.getByRole("complementary", { name: "Host monitor" });

    // The corner is read as a number rather than as a length: how round a box is depends on the
    // corner style as well, and this case is about whether there is rounding at all.
    const radius = async (): Promise<number> =>
      tab.evaluate((el) => parseFloat(getComputedStyle(el).borderTopLeftRadius));

    // Separated is what a fresh install gets: boxes with air around them, and panels floating as cards.
    await expect(settings.getByRole("radio", { name: /^Modern separated/ })).toBeChecked();
    expect(await radius(), "a tab drawn as a box of its own").toBeGreaterThan(0);
    await expect(panel.locator("header").first(), "a header with room around its title").toHaveCSS(
      "padding-top",
      "8px",
    );
    await expect
      .poll(async () => panel.evaluate((el) => parseFloat(getComputedStyle(el).borderTopLeftRadius)), {
        message: "and every panel a floating card",
      })
      .toBeGreaterThan(0);

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "settings-layout-separated.png"), fullPage: false });

    await settings.getByRole("radio", { name: /^Classic compact/ }).click();
    await applySettings(page);

    await expect.poll(async () => (await writtenSettings()).appLayout).toBe("classic");
    await expect.poll(radius, { message: "no rounding, in a bar with no gaps" }).toBe(0);
    await expect(panel.locator("header").first(), "and a header that gave the row back").toHaveCSS(
      "padding-top",
      "4px",
    );
    await expect(panel, "panels flush against each other").toHaveCSS("border-top-left-radius", "0px");

    await page.screenshot({ path: path.join(ARTIFACTS, "settings-layout-classic.png"), fullPage: false });

    // Put back, so that the rest of the run meets the window it expects: the shape is a property of
    // the whole window, and every case after this one draws its own chrome in it.
    await settings.getByRole("radio", { name: /^Modern separated/ }).click();
    await applySettings(page);
    await expect.poll(async () => (await writtenSettings()).appLayout).toBe("separated");
    await page.getByRole("button", { name: "Close settings" }).click();
  } finally {
    await close();
  }
});

/**
 * What the bar does when the tabs do not fit in one row.
 *
 * The one chrome choice that is a measurement rather than a look: scrolling keeps the bar one row tall
 * and puts the tabs past the edge, while wrapping gives every tab a place and takes the height from
 * the panel below. What a run without a server can see is which rule the bar was drawn under — enough
 * tabs to make it wrap needs terminals, which need a host, and those cases are the skipped ones. The
 * files are what it reads for the choice itself.
 */
test("the tab bar is told what to do when the tabs do not fit", async () => {
  const { page, close } = await openApp();
  try {
    await settingsButton(page).click();
    const settings = page.getByRole("region", { name: "Settings" });
    await settings.getByRole("button", { name: "Appearance", exact: true }).click();

    const strip = page.locator("[data-workspace-tabs]");

    // One row that scrolls is what this build has always drawn, so it is where a fresh window is.
    await expect(settings.getByRole("radio", { name: /^Single row scroll/ })).toBeChecked();
    await expect(strip).toHaveCSS("flex-wrap", "nowrap");
    await expect(strip).toHaveCSS("overflow-x", "auto");

    await settings.getByRole("radio", { name: /^Multi-row wrap/ }).click();
    await applySettings(page);

    await expect.poll(async () => (await writtenSettings()).tabLayout).toBe("wrap");
    await expect(strip, "as many rows as there are tabs").toHaveCSS("flex-wrap", "wrap");
    await expect(strip, "and nothing past the edge").toHaveCSS("overflow-x", "visible");

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "settings-tab-layout.png"), fullPage: false });

    // Put back, so that the rest of the run meets the bar it expects.
    await settings.getByRole("radio", { name: /^Single row scroll/ }).click();
    await applySettings(page);
    await expect.poll(async () => (await writtenSettings()).tabLayout).toBe("scroll");
    await page.getByRole("button", { name: "Close settings" }).click();
  } finally {
    await close();
  }
});

/**
 * The picture behind the window, from the settings file to the pixels.
 *
 * The choosing half cannot be driven from here: picking a file opens the system's own dialog, which is
 * a window no page can press a button in — that half is checked by hand, and the shell's own cases hold
 * the copying and the limits. What this drives is everything after it: a picture in the place the shell
 * keeps its own copy, a settings file that names it, and a window that has read both.
 *
 * A one-pixel PNG, written where the shell would have copied it to, is enough for that: what is being
 * asked is whether the layer is there, how it is painted, and whether the surfaces in front of it were
 * told to let it through.
 */
test("the window draws the background picture the settings name, and lets it through", async () => {
  const { page, close } = await openApp();
  // A 1x1 PNG, which is a picture as far as every step of this is concerned.
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64",
  );
  const config = path.dirname(SETTINGS_FILE);
  const before = await writtenSettings();

  try {
    await mkdir(config, { recursive: true });
    await writeFile(path.join(config, "background-image.png"), png);
    await writeFile(
      SETTINGS_FILE,
      JSON.stringify({
        ...before,
        backgroundImage: { fileName: "wallpaper.png", displayMode: "fill", transparency: 25, blur: 6 },
      }),
      "utf8",
    );

    // The interface reads the settings when it starts, so the window has to be brought up again for a
    // file written from outside to be the one it is wearing — which is what a restart does, and what a
    // reload of the page is here.
    await page.reload();
    await expect(page.locator("[data-wallpaper]"), "the layer is there").toBeVisible();

    const painted = page.locator("[data-wallpaper] > div");
    await expect(painted, "filled into the window").toHaveCSS("background-size", "cover");
    await expect(painted, "with the blur the settings ask for").toHaveCSS("filter", "blur(6px)");

    // The surfaces between the picture and the work became translucent, by the transparency the
    // settings hold: what is left of an opaque colour is what shows through.
    const surface = await page.evaluate(() =>
      getComputedStyle(document.documentElement).getPropertyValue("--background").trim(),
    );
    expect(surface, "the window's own surface was told to let the picture through").toContain("/ 0.75");

    // And the block that chose it, in the page somebody meets it on: the picture's name, the five
    // ways of fitting it and the two sliders, drawn over the picture itself.
    await settingsButton(page).click();
    const settings = page.getByRole("region", { name: "Settings" });
    await settings.getByRole("button", { name: "Appearance", exact: true }).click();
    await expect(settings.getByText("wallpaper.png"), "the picture is named where it was chosen").toBeVisible();
    await expect(settings.getByRole("slider", { name: "Transparency" })).toHaveValue("25");
    await expect(settings.getByRole("slider", { name: "Blur" })).toHaveValue("6");

    // Scrolled to the block rather than left at the top of the page: the picture's row is the last of
    // the things a page is drawn *by*, and a screenshot of the top of the page would not have it in it.
    await settings.getByText("wallpaper.png").scrollIntoViewIfNeeded();
    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "settings-background.png"), fullPage: false });
    await page.getByRole("button", { name: "Close settings" }).click();
  } finally {
    // Put back, so that the rest of the run meets the window it expects: a settings file with a picture
    // in it would follow every case after this one around.
    if (before.backgroundImage === undefined) {
      delete (before as { backgroundImage?: unknown }).backgroundImage;
    }
    await writeFile(SETTINGS_FILE, JSON.stringify(before), "utf8");
    await page.reload().catch(() => undefined);
    await close();
  }
});

/**
 * The two switches about the window itself: the tray icon, and what closing does.
 *
 * The settings half of them is what a run like this can see — that they exist, that they start the way
 * a fresh install has them, and that what somebody chooses reaches the file the shell reads. What the
 * shell then *does* with them is a window and a tray icon, which no page can be asked about: that is
 * checked against the running binary itself, by hand, and the two predicates behind it have their own
 * unit cases.
 */
test("the tray icon and the close behaviour are choices, written where the shell reads them", async () => {
  const { page, close } = await openApp();
  try {
    await settingsButton(page).click();
    const settings = page.getByRole("region", { name: "Settings" });
    await settings.getByRole("button", { name: "Appearance", exact: true }).click();

    const tray = settings.getByRole("switch", { name: "Show tray/menu bar icon" });
    const quit = settings.getByRole("switch", { name: "Quit when closing window" });

    // What a fresh install has: an icon, and a window that puts itself away rather than ending the
    // application — which is the one of the two this build changed when it grew a tray.
    await expect(tray).toHaveAttribute("aria-checked", "true");
    await expect(quit).toHaveAttribute("aria-checked", "false");

    await tray.click();
    await quit.click();
    await applySettings(page);

    await expect.poll(async () => (await writtenSettings()).showTrayIcon).toBe(false);
    await expect.poll(async () => (await writtenSettings()).quitOnClose).toBe(true);

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "settings-window-switches.png"), fullPage: false });

    // Put back, so that the rest of the run meets the shell it expects.
    await tray.click();
    await quit.click();
    await applySettings(page);

    await expect.poll(async () => (await writtenSettings()).showTrayIcon).toBe(true);
    await expect.poll(async () => (await writtenSettings()).quitOnClose).toBe(false);
    await page.getByRole("button", { name: "Close settings" }).click();
  } finally {
    await close();
  }
});

/**
 * The tab bar's own four choices, which are the ones about the bar rather than about its tabs.
 *
 * What a real window is needed for: each of the four is a measurement rather than a colour — which way
 * the bar runs, whether the runs are named, what order the tabs are drawn in and how wide one may be —
 * and every one of them is read back as the computed style or the drawn element that produces it. The
 * file is read for the choice itself, which is the half a fresh window has to agree with.
 */
test("the tab bar is drawn where it was put, grouped, ordered and no wider than it was told", async () => {
  const { page, close } = await openApp();
  try {
    // A session in the library, because the cases after this one expect a window with something in it.
    await saveSession(page, "bar-probe", "127.0.0.1", "1");
    await settingsButton(page).click();
    const settings = page.getByRole("region", { name: "Settings" });
    await settings.getByRole("button", { name: "Appearance", exact: true }).click();

    const strip = page.locator("[data-workspace-tabs]");
    const section = page.locator("section").filter({ has: strip }).first();

    // Where this build has always put the bar: across the top, which is a row of tabs in a column of
    // two rows — the bar and the pane under it.
    await expect(strip).toHaveAttribute("data-tab-placement", "top");
    await expect(strip, "across the top means a row of tabs").toHaveCSS("flex-direction", "row");
    await expect(section, "with the pane under it").toHaveCSS("flex-direction", "column");

    // How wide a tab may be: a step of the horizontal bar, which is what a tab's own box answers for.
    await settings.getByRole("combobox", { name: "Maximum tab width" }).click();

    // The five steps, as they are offered: kept as a picture because the list is copied from the
    // interface this build follows, and a step that drifted by a word is what nobody would notice.
    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "settings-tab-width-steps.png"), fullPage: false });

    await page.getByRole("listbox").getByRole("option", { name: "Compact (160px)" }).click();
    await applySettings(page);

    await expect.poll(async () => (await writtenSettings()).tabMaxWidth).toBe("compact");
    await expect(page.locator("[data-tab]").first(), "the narrowest step, in pixels").toHaveCSS("max-width", "160px");

    // Down the left: the section becomes a row — the bar beside the panes — and the bar a column with
    // its line against the panes rather than against the window.
    await settings.getByRole("combobox", { name: "Tab bar position" }).click();
    await page.getByRole("listbox").getByRole("option", { name: "Left side" }).click();
    await applySettings(page);

    await expect.poll(async () => (await writtenSettings()).tabPlacement).toBe("left");
    await expect(section, "the panes beside the bar").toHaveCSS("flex-direction", "row");
    await expect(strip).toHaveCSS("flex-direction", "column");

    // Which side the line is on rather than how thick: the interface is scaled inside the window, so a
    // one-pixel rule measures as a fraction of one, and the edge it is drawn against is the fact this
    // case is about.
    const edges = await strip.evaluate((el) => {
      const style = getComputedStyle(el);
      return { right: parseFloat(style.borderRightWidth), left: parseFloat(style.borderLeftWidth) };
    });
    expect(edges.right, "a line against the left edge of the panes").toBeGreaterThan(0);
    expect(edges.left, "and none on the outside").toBe(0);

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "settings-tab-bar-left.png"), fullPage: false });

    // Grouped by session and ordered by name, which are two answers about the drawing alone. Nothing
    // in this window belongs to a session except the settings page, which belongs to none — so the
    // grouping has no run to name here, and the case checks that it says so rather than inventing one.
    await settings.getByRole("combobox", { name: "Group by" }).click();
    await page.getByRole("listbox").getByRole("option", { name: "Group by session" }).click();
    await settings.getByRole("combobox", { name: "Sort by" }).click();
    await page.getByRole("listbox").getByRole("option", { name: "Tab name" }).click();
    await applySettings(page);

    await expect.poll(async () => (await writtenSettings()).tabGroupMode).toBe("session");
    await expect.poll(async () => (await writtenSettings()).tabSortMode).toBe("title-asc");
    await expect(page.locator("[data-tab-group]"), "a heading only for tabs that belong to a session").toHaveCount(0);

    await page.screenshot({ path: path.join(ARTIFACTS, "settings-tab-bar-grouped.png"), fullPage: false });

    // Put back, so that the rest of the run meets the bar it expects.
    for (const [name, value] of [
      ["Tab bar position", "Top"],
      ["Group by", "No grouping"],
      ["Sort by", "Manual order"],
      ["Maximum tab width", "Unlimited (adaptive)"],
    ] as const) {
      await settings.getByRole("combobox", { name }).click();
      await page.getByRole("listbox").getByRole("option", { name: value }).click();
    }
    await applySettings(page);

    await expect.poll(async () => (await writtenSettings()).tabPlacement).toBe("top");
    await expect.poll(async () => (await writtenSettings()).tabMaxWidth).toBe("unlimited");
    await page.getByRole("button", { name: "Close settings" }).click();
  } finally {
    await close();
  }
});

/**
 * The home page, which is one of two.
 *
 * The page is the surface with nothing open on it, and there are two of them: the introduction, for
 * somebody who has not connected anything yet — what this build is and how a first connection goes,
 * with no host named and nothing counted — and the workspace overview, which is the library it
 * holds, the hosts used last, the actions and what was run. Which one is chosen in the settings
 * page's appearance tab, and the page itself is where it is seen, which means closing the settings
 * tab first: the home page is what is on screen when no tab is.
 */
test("the home page is the one somebody chose", async () => {
  const { page, close } = await openApp();
  try {
    // The page this case is about, asked for rather than inherited from whichever case ran last.
    await chooseHomePage(page, "Introduction");

    const board = home(page);
    await expect(board.getByRole("heading", { name: "Getting started" })).toBeVisible();
    await expect(board.getByText("Sessions saved"), "nothing here counts a session").toHaveCount(0);
    // Including the one thing this page promises about itself, and the way back to the other one.
    await expect(board.getByText("no host key and no history", { exact: false })).toBeVisible();
    await expect(board.getByRole("button", { name: "Home page settings" })).toBeVisible();

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "home-intro.png"), fullPage: false });

    await settingsButton(page).click();
    const settings = page.getByRole("region", { name: "Settings" });
    await settings.getByRole("button", { name: "Appearance", exact: true }).click();

    // The cards say what each page is, so the radio a reader finds is the card whose line begins with
    // its name — and the one that is lit is the introduction, which is where a fresh install is.
    const intro = settings.getByRole("radio", { name: /^Introduction/ });
    const workspace = settings.getByRole("radio", { name: /^Workspace overview/ });
    await expect(intro).toBeChecked();

    // Kept as a picture: two cards, each with the line that tells them apart, one of them lit.
    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "settings-home-content.png"), fullPage: false });

    await workspace.click();
    await expect(workspace).toBeChecked();
    await applySettings(page);
    await expect.poll(async () => (await writtenSettings()).homeLayout).toBe("workspace");

    await page.getByRole("button", { name: "Close settings" }).click();

    // The page it is about: the library's own numbers, and the hosts it holds.
    await expect(board.getByText("Sessions saved"), "this page counts what the library holds").toBeVisible();
    await expect(board.getByRole("heading", { name: "Getting started" })).toHaveCount(0);

    // Put back, so that the rest of the run meets the page it expects — which is the one a fresh
    // install has, since that is what every other case was written against.
    await chooseHomePage(page, "Introduction");
    await expect(board.getByRole("heading", { name: "Getting started" })).toBeVisible();
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
    const wrote = await page.evaluate(async () => {
      const internals = (
        window as unknown as { __TAURI_INTERNALS__?: { invoke: (command: string) => Promise<unknown> } }
      ).__TAURI_INTERNALS__;
      if (!internals) {
        return false;
      }
      await navigator.clipboard.writeText("seven-shell-e2e");
      return true;
    });

    expect(wrote, "the webview can still write a clipboard, which the copy entries rely on").toBe(true);

    // Read back through the backend, waiting for the operating system to have it.
    //
    // The clipboard is not the page's: the write is handed to the browser process, and on Windows a
    // reader that arrives while it — or anything else on the machine — holds the clipboard open gets
    // no text rather than the text. Reading exactly once, immediately, is reading a shared resource
    // before it settles, and the case then fails for a reason that has nothing to do with this
    // build. Polled rather than slept on, so a quiet machine is quick and a busy one still arrives.
    await expect
      .poll(
        () =>
          page.evaluate(() =>
            (
              window as unknown as { __TAURI_INTERNALS__?: { invoke: (command: string) => Promise<unknown> } }
            ).__TAURI_INTERNALS__?.invoke("clipboard_text"),
          ),
        { message: "the text the webview copied is the text the backend reads" },
      )
      .toBe("seven-shell-e2e");
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

/**
 * Whether an element paints over what is behind it.
 *
 * Asked of the rendering and then painted onto a canvas and read back, because these colours are
 * written in oklch: the computed value is the spelling rather than the pixels it stands for. A
 * background class that names a token the sheet does not define paints nothing at all, and a list
 * drawn over a terminal then reads as text on text.
 */
function paintsOver(element: Locator): Promise<boolean> {
  return element.evaluate((node) => {
    const canvas = document.createElement("canvas");
    canvas.width = 1;
    canvas.height = 1;
    const context = canvas.getContext("2d");
    if (context === null) {
      return false;
    }
    context.fillStyle = getComputedStyle(node).backgroundColor;
    context.fillRect(0, 0, 1, 1);
    return context.getImageData(0, 0, 1, 1).data[3] === 255;
  });
}

/**
 * The size an element's own text is drawn at, as the window computed it.
 *
 * Asked of the rendering rather than of the markup: a class only says what was intended, and any
 * rule anywhere in the sheet can have the last word about what a person sees.
 */
function fontSizeOf(element: Locator): Promise<string> {
  return element.evaluate((node) => getComputedStyle(node).fontSize);
}

/** A save dialog answered with a session that is only ever connected to by hand. */
async function saveSession(page: Page, name: string, host = "127.0.0.1", port = "22"): Promise<void> {
  await sidebar(page).getByRole("button", { name: "New session" }).click();
  const editor = page.getByRole("dialog");
  await editor.getByPlaceholder("Production web server").fill(name);
  await editor.getByPlaceholder("example.com").fill(host);
  await editor.getByPlaceholder("root").fill("labuser");
  await editor.locator("#session-port").fill(port);
  await editor.getByRole("button", { name: "Save" }).click();
  await expect(editor).toHaveCount(0);
}

/**
 * A group made where it is drawn: one click, and the name typed over the one it was made with.
 *
 * What the click produces is a row, not a form: the group exists from the moment it is asked for,
 * which is why the field that appears is the row's own name and the name in it is the default.
 */
async function saveGroup(page: Page, name: string): Promise<void> {
  await page.getByRole("button", { name: "New group" }).click();
  const field = page.getByRole("textbox", { name: "Group name" });
  await expect(field).toHaveValue("New group");
  await field.fill(name);
  await field.press("Enter");
  await expect(page.getByRole("heading", { name, exact: true })).toBeVisible();
}

/**
 * Giving a session a group that does not exist yet.
 *
 * This runs before any group is made, which is the case it is about: with no group to move into,
 * the session's own menu offers it as a row of its own — "Move to a new group" — and the group is
 * made with the session already inside it by the time its name is typed over.
 */
test("a session can be given a group of its own", async () => {
  const { page, close } = await openApp();
  try {
    await saveSession(page, "groupless-host");
    const row = sessionRow(page, "groupless-host");

    await row.click({ button: "right" });
    await page.getByRole("menuitem", { name: "Move to a new group" }).click();

    const field = page.getByRole("textbox", { name: "Group name" });
    await expect(field).toHaveValue("New group");
    await field.fill("Mine");
    await field.press("Enter");
    await expect(page.getByRole("heading", { name: "Mine", exact: true })).toBeVisible();

    // Inside it, which is the whole point: the row it sits under is the nearest group row above it.
    const keys = await rowKeys(page);
    const sessionKey = await rowKeyOf(row);
    const groupKey = await rowKeyOf(page.getByRole("heading", { name: "Mine", exact: true }));
    expect(groupOver(keys, keys.indexOf(sessionKey))).toBe(groupKey);

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "group-new.png"), fullPage: false });
  } finally {
    await close();
  }
});

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
    const row = sessionRow(page, "grouped-host");
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
    const heading = page.getByRole("heading", { name: "Lab (EU)", exact: true });
    await expect(heading).toBeVisible();

    // A group's name is set at the size a session's name is set at: the reference draws both with
    // one class, and a heading set larger or smaller makes the list read as two things stacked
    // rather than one tree. Measured rather than read off the classes, because that is what the
    // window actually renders.
    expect(await fontSizeOf(heading)).toBe(await fontSizeOf(row.locator("span.text-sm").first()));

    // Deleting asks first, in a dialog over everything, and says what happens to what is inside.
    await page.getByRole("heading", { name: "Lab (EU)", exact: true }).click({ button: "right" });
    await page.getByRole("menuitem", { name: "Delete group" }).click();
    const confirm = page.getByRole("dialog");
    await expect(confirm.getByText(/Delete the group “Lab \(EU\)”\?/)).toBeVisible();
    await confirm.getByRole("button", { name: "Delete group", exact: true }).click();

    // The group is gone and the session is not. With no group left there is no heading over it at
    // all — a flat list has nothing to set it apart from.
    await expect(confirm).toHaveCount(0);
    await expect(page.getByRole("heading", { name: "Lab (EU)", exact: true })).toHaveCount(0);
    await expect(page.getByRole("heading", { name: "Ungrouped", exact: true })).toHaveCount(0);
    await expect(row).toBeVisible();
  } finally {
    await close();
  }
});

/**
 * Pinning a group above the ones it was made beside.
 *
 * The half worth driving through a real window is that the heading really moves and really comes
 * back: the pin is a field written into the vault *beside* the order, so this is also the check that
 * pinning did not turn into a reorder — Lab goes back below Work when the pin comes off, which it
 * could not do if pinning had rewritten the arrangement.
 */
test("a group can be pinned above the ones it was made beside, and let go again", async () => {
  const { page, close } = await openApp();
  try {
    await saveGroup(page, "Work");
    await saveGroup(page, "Lab");

    const lab = page.getByRole("heading", { name: "Lab", exact: true });
    const row = lab.locator("xpath=ancestor::*[@data-row][1]");
    const labKey = await rowKeyOf(lab);
    const workKey = await rowKeyOf(page.getByRole("heading", { name: "Work", exact: true }));

    /** How far below Work the Lab heading is drawn: negative means it is above it. */
    const below = async (): Promise<number> => {
      const keys = await rowKeys(page);
      return keys.indexOf(labKey) - keys.indexOf(workKey);
    };

    expect(await below(), "Lab was made second, so it starts below Work").toBeGreaterThan(0);

    await lab.click({ button: "right" });
    await page.getByRole("menuitem", { name: "Pin group" }).click();

    // Above the group it was made beside, with the pin in the row saying why it is there.
    await expect.poll(below).toBeLessThan(0);
    await expect(row.locator("svg.lucide-pin")).toBeVisible();

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "group-pinned.png"), fullPage: false });

    await lab.click({ button: "right" });
    await page.getByRole("menuitem", { name: "Unpin group" }).click();

    // Back in its own place: the pin was laid over the arrangement, and the arrangement was kept.
    await expect.poll(below).toBeGreaterThan(0);
    await expect(row.locator("svg.lucide-pin")).toHaveCount(0);
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

    const row = sessionRow(page, "dragged-host");
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

/**
 * One command, run on a real host, with the verdict shown before it runs.
 *
 * This is the whole feature through the window and nothing else: a session saved the way a person
 * saves one, the batch panel opened from the title bar, the command typed into the box, the
 * classifier's own answer read off the banner, the button pressed, the host key question answered,
 * and one row that says what the host did.
 *
 * It needs a server, so it skips when the lab is not configured — the same four variables the Rust
 * live tests use. What it proves that they cannot is the wiring above them: that the panel's button
 * reaches the command, that a run stopped by a decision resumes by itself once the key is trusted,
 * and that a row drawn from an answer that arrived as an event says the same as one drawn from the
 * run's own result.
 */
test("a batch runs on a real host and the row says what happened", async () => {
  const host = process.env.SEVENSHELL_TEST_HOST;
  const port = process.env.SEVENSHELL_TEST_PORT ?? "22";
  const user = process.env.SEVENSHELL_TEST_USER;
  const password = process.env.SEVENSHELL_TEST_PASSWORD;
  test.skip(!host || !user || !password, "the SSH lab is not configured; set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD");

  const { page, close } = await openApp();
  try {
    // A session of its own, so nothing another case did to the list can change what is selected.
    const name = `lab-batch-${Date.now() % 100000}`;
    await sidebar(page).getByRole("button", { name: "New session" }).click();
    const editor = page.getByRole("dialog");
    await editor.getByPlaceholder("Production web server").fill(name);
    await editor.getByPlaceholder("example.com").fill(host as string);
    await editor.locator("#session-port").fill(port);
    await editor.getByPlaceholder("root").fill(user as string);
    await editor.getByPlaceholder("Password", { exact: true }).fill(password as string);
    await editor.getByRole("button", { name: "Save" }).click();
    await expect(editor).toHaveCount(0);

    await page.getByRole("button", { name: "Batch execution" }).click();

    // The host picker lists what is saved; this run is about exactly one of them.
    await page.getByRole("checkbox", { name: new RegExp(name) }).check();
    await page.getByPlaceholder("uptime").fill("uptime");

    // The verdict is the backend's, and it is on screen before the button is pressed. `uptime`
    // reads nothing and changes nothing, so the answer is the quiet one.
    await expect(page.getByText("Safe", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Run", exact: true }).click();

    // The first connection to this host is a question, and answering it re-runs the request: a
    // batch that stopped for a decision has not run anything.
    const question = page.getByText("A host key that has not been seen before");
    await expect(question).toBeVisible({ timeout: 30_000 });
    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "batch-host-key.png"), fullPage: false });
    await page.getByRole("button", { name: "Trust this key and connect" }).click();

    // One row, and it says the host exited zero.
    await expect(page.getByText("exit 0", { exact: true })).toBeVisible({ timeout: 60_000 });
    await expect(page.getByText("1 of 1 answered", { exact: false })).toBeVisible();

    // Unfolding the row shows what the command printed, which is what the host actually said.
    // Scoped to the workspace: the sidebar draws a row for the same session, and its button
    // carries the same address.
    await page
      .getByRole("main")
      .getByRole("button", { name: new RegExp(`${host}:${port}`) })
      .click();
    await expect(page.getByText("load average", { exact: false })).toBeVisible({ timeout: 15_000 });

    await page.screenshot({ path: path.join(ARTIFACTS, "batch-result.png"), fullPage: false });
  } finally {
    await close();
  }
});

/**
 * The panel's two small tables, with a real host's readings in them.
 *
 * What a window with nothing connected cannot show is what those tables are for: a row of readings,
 * and what happens to it under the pointer. The hover is the part that only exists in a rendering —
 * a class in the markup says nothing about what a browser paints — so this is the case that would
 * notice the panel going back to plain lines of text.
 *
 * It needs a server, so it skips when the lab is not configured: the same four variables the Rust
 * live tests and the other server cases here use. It sits immediately after the batch case for two
 * reasons, and both are about the order these cases run in rather than about the subject: that one
 * is the case that meets the host key question and says so in its own comment, so nothing may reach
 * the host before it — and this one wants the window while it is still healthy, which it is here.
 */
test("the panel's tables draw a real host, and mark the row under the pointer", async () => {
  const host = process.env.SEVENSHELL_TEST_HOST;
  const port = process.env.SEVENSHELL_TEST_PORT ?? "22";
  const user = process.env.SEVENSHELL_TEST_USER;
  const password = process.env.SEVENSHELL_TEST_PASSWORD;
  test.skip(!host || !user || !password, "the SSH lab is not configured; set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD");

  const { page, close } = await openApp();
  try {
    const stamp = `ss-e2e-${Date.now() % 100000}`;
    const name = `monitor-${stamp}`;

    await sidebar(page).getByRole("button", { name: "New session" }).click();
    const editor = page.getByRole("dialog");
    await editor.getByPlaceholder("Production web server").fill(name);
    await editor.getByPlaceholder("example.com").fill(host as string);
    await editor.locator("#session-port").fill(port);
    await editor.getByPlaceholder("root").fill(user as string);
    await editor.getByPlaceholder("Password", { exact: true }).fill(password as string);
    await editor.getByRole("button", { name: "Save" }).click();
    await expect(editor).toHaveCount(0);

    await sessionRow(page, name).click();

    const panel = page.getByRole("complementary", { name: "Host monitor" });
    const readings = panel.getByRole("table", { name: "Processes" }).locator("tbody tr").first();
    const question = page.getByRole("button", { name: "Trust this key and connect" });

    // Two things can happen after the click, and which one depends on whether the host is new to
    // this vault — the key is normally trusted by the case above, but this one is runnable on its
    // own as well. Waited for together so neither has to be guessed at, and the question is only
    // answered if it was actually asked.
    await expect(question.or(readings).first()).toBeVisible({ timeout: 30_000 });
    if (await question.isVisible()) {
      await question.click();
    }

    // Both tables, in the order the panel draws them: a heading, a rule, and readings. The rows are
    // waited for rather than assumed — the sample rides the connection the click just opened.
    for (const table of ["Processes", "File systems"]) {
      const sheet = panel.getByRole("table", { name: table });
      const row = sheet.locator("tbody tr").first();
      await expect(row, `${table}: the host's readings are drawn`).toBeVisible({ timeout: 30_000 });

      // The row under the pointer is painted, which is the whole of what "a table" means here: a row
      // is a right-click target rather than a click, so the band is the only thing saying so.
      const shade = (): Promise<string> => row.evaluate((element) => getComputedStyle(element).backgroundColor);
      const before = await shade();
      await row.hover();
      await expect.poll(shade, { message: `${table}: the row under the pointer is marked` }).not.toBe(before);
    }

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "monitor-tables.png"), fullPage: false });

    // The line under the terminal — the session's address and the way into the file panel — is
    // chrome too, and this is the only case that has a terminal open to draw it. A drag along it is
    // somebody reaching for the panel's toggle, not somebody reading the address, which is on the
    // panel as well.
    await expect(page.locator("[data-terminal-strip]")).toHaveCSS("user-select", "none");

    // And the card is the way in to the page: what it opens is the page itself, which no longer
    // says so in a strip of its own — the tab carries the page's name and the host. Scoped to the
    // workspace, because the panel draws a file-system table of its own under the same name.
    await panel.getByRole("button", { name: "System info" }).click();
    await expect(page.getByRole("main").getByRole("table", { name: "File systems" })).toBeVisible({
      timeout: 30_000,
    });

    // Closed on the way out, and this case is the reason the others say why: it leaves nothing of its
    // session open. Both of the things it opened count — a terminal *and* a page about the host are
    // "sessions that are open" as far as the sidebar is concerned, so a page left behind is an active
    // session for every case after it: the sidebar's own switch behaves differently, and a search that
    // should have found a row finds nothing.
    await page.getByRole("button", { name: "Close this page" }).click();
    await expect(page.getByRole("button", { name: "Close this page" })).toHaveCount(0);
    await page.getByRole("button", { name: "Close terminal" }).click();
    await expect(page.getByRole("button", { name: "Close terminal" })).toHaveCount(0);
  } finally {
    await close();
  }
});

/**
 * A saved command, typed into the shell of a session that is open.
 *
 * The whole feature through the window: a session saved the way a person saves one, a terminal
 * opened on it, the host key question answered, a command kept in the library with a `#1` slot,
 * the slot filled, and the send button — then the proof that the shell *ran* it, read through a
 * different feature entirely: the file it was told to create, seen in the file browser.
 *
 * That last step is why this case exists at all. A send is only bytes written to a pty: the send
 * "succeeding" while the far side did nothing is exactly the failure a test of the panel alone
 * would not notice. The terminal's own output cannot be read here — xterm draws to a canvas, not
 * into the document — so the observable is the file the command was told to leave behind.
 */
test("a saved command reaches the shell and the file it makes appears", async () => {
  const host = process.env.SEVENSHELL_TEST_HOST;
  const port = process.env.SEVENSHELL_TEST_PORT ?? "22";
  const user = process.env.SEVENSHELL_TEST_USER;
  const password = process.env.SEVENSHELL_TEST_PASSWORD;
  test.skip(!host || !user || !password, "the SSH lab is not configured; set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD");

  const { page, close } = await openApp();
  try {
    const stamp = `ss-e2e-${Date.now() % 100000}`;
    const name = `cmd-${stamp}`;

    await sidebar(page).getByRole("button", { name: "New session" }).click();
    const editor = page.getByRole("dialog");
    await editor.getByPlaceholder("Production web server").fill(name);
    await editor.getByPlaceholder("example.com").fill(host as string);
    await editor.locator("#session-port").fill(port);
    await editor.getByPlaceholder("root").fill(user as string);
    await editor.getByPlaceholder("Password", { exact: true }).fill(password as string);
    await editor.getByRole("button", { name: "Save" }).click();
    await expect(editor).toHaveCount(0);

    // Opening the terminal: the first connection to this host asks about its key.
    // Opening the terminal: the first connection to this host asks about its key, and which of the two
    // arrives first is a race with the network — so the terminal is waited for together with the
    // question rather than after it. (A case that looked once, before the answer came back, waited for
    // a terminal that was waiting for a dialog nobody had answered.)
    const question = page.getByRole("button", { name: "Trust this key and connect" });
    const commands = page.getByRole("button", { name: "Commands" });
    await sessionRow(page, name).click();
    await expect(question.or(commands).first()).toBeVisible({ timeout: 30_000 });
    if (await question.isVisible()) {
      await question.click();
    }

    // The commands panel lives under the terminal, beside the files. Waited for explicitly: the
    // footer only appears once there is a terminal tab, so a failure here means the terminal never
    // opened rather than that the button is missing.
    await expect(commands).toBeVisible({ timeout: 30_000 });
    await commands.click();
    await page.getByRole("button", { name: "New command" }).click();

    await page.getByPlaceholder("What it is called").fill("marker");
    await page.getByLabel(/The line, with/).fill(`touch /tmp/${stamp}-#1`);
    // The slot box, filled the way somebody fills it before sending.
    // The slot boxes are the inputs that carry a suggestion list; the first one is `#1`.
    await page.locator('input[list^="slot-history-"]').first().fill("done");
    await page.getByRole("button", { name: "Save", exact: true }).click();

    // What is sent is the line with the value in it, which is what the history then shows.
    await page.getByRole("button", { name: "Send", exact: true }).click();
    await expect(page.getByText(`touch /tmp/${stamp}-done`).first()).toBeVisible({ timeout: 30_000 });

    // And the shell ran it: switch to the file half of the same panel and look at /tmp.
    //
    // The button is a toggle and it is `exact` on purpose: the sidebar draws a row per session
    // whose button also says "files", so the name has to be the whole name. Clicking until the
    // path bar is there rather than clicking once keeps this independent of which half happened to
    // be showing.
    const fileToggle = page.getByRole("main").getByRole("button", { name: "Files", exact: true });
    await fileToggle.click();
    const pathBar = page.getByPlaceholder("Path");
    if (!(await pathBar.isVisible().catch(() => false))) {
      await fileToggle.click();
    }
    await expect(pathBar).toBeVisible({ timeout: 30_000 });
    await pathBar.fill("/tmp");
    await pathBar.press("Enter");
    await expect(page.getByText(`${stamp}-done`).first()).toBeVisible({ timeout: 30_000 });

    // And switching halves, which is where the two buttons have to disagree: the browser stays open
    // behind the commands — only the half being drawn changes — so exactly one of them may read as
    // the one on screen. Both lit at once is what this run was written after seeing: the files
    // button asked whether a panel existed rather than which half was showing.
    const commandsToggle = page.getByRole("main").getByRole("button", { name: "Commands", exact: true });
    await commandsToggle.click();
    await expect(commandsToggle).toHaveAttribute("aria-pressed", "true");
    await expect(fileToggle).toHaveAttribute("aria-pressed", "false");

    await fileToggle.click();
    await expect(fileToggle).toHaveAttribute("aria-pressed", "true");
    await expect(commandsToggle).toHaveAttribute("aria-pressed", "false");
    // The same browser, back in front: nothing was opened a second time on the same session.
    await expect(pathBar).toBeVisible();

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "commands-sent.png"), fullPage: false });

    // What the pinned headings are for, on a directory long enough to scroll: a row passing under
    // the strip has to be hidden by it, not read through it. /etc rather than /tmp because a
    // directory that fits on the panel cannot say anything about a pinned row.
    await pathBar.fill("/etc");
    await pathBar.press("Enter");
    const listing = page.getByRole("listbox", { name: "Files", exact: true });
    await expect(listing.getByRole("option").first()).toBeVisible({ timeout: 30_000 });
    const reached = await listing.evaluate((element) => {
      element.scrollTop = 200;
      return element.scrollTop;
    });
    expect(reached, "a list that cannot scroll says nothing about a pinned heading").toBeGreaterThan(0);
    await page.screenshot({ path: path.join(ARTIFACTS, "listing-scrolled.png"), fullPage: false });
    await listing.evaluate((element) => {
      element.scrollTop = 0;
    });

    // And the headings order the rows: pressed once, the column reads from its own end — the newest
    // time first — which is what the caret beside it then says.
    await page.getByRole("button", { name: "Sort by Modified" }).click();
    await expect(listing.getByRole("option").first()).toBeVisible();
    await page.screenshot({ path: path.join(ARTIFACTS, "listing-sorted.png"), fullPage: false });

    // The kind column, in a directory that holds nothing but folders: every row has the same value
    // for it, and the press has to answer anyway — this is the case that came back as "sorting by
    // type does nothing", because the rows fell back to the order they were already in.
    const firstRow = (): Promise<string | null> => listing.locator("li[data-path]").first().getAttribute("data-path");
    const before = await firstRow();
    await page.getByRole("button", { name: "Sort by Type" }).click();
    await expect.poll(firstRow, { message: "the folders answer the type column" }).not.toBe(before);

    // The bar under the terminal: the command somebody did not save. Enter sends it, and the line
    // it ran is kept in the same history the panel draws. Two files, because one of them is what the
    // type column is asked about below.
    const bar = page.getByLabel("Command input");
    await bar.fill(`touch /tmp/${stamp}-typed /tmp/${stamp}.tar.gz`);
    await bar.press("Enter");
    await page.getByRole("button", { name: "History" }).click();
    await expect(page.getByText(`touch /tmp/${stamp}-typed`).first()).toBeVisible({ timeout: 30_000 });

    // And the shell ran that one too: back to the listing, refreshed by navigating away and in.
    await page.getByRole("button", { name: "History" }).click();
    await pathBar.fill("/");
    await pathBar.press("Enter");
    await pathBar.fill("/tmp");
    await pathBar.press("Enter");
    await expect(page.getByText(`${stamp}-typed`).first()).toBeVisible({ timeout: 30_000 });

    // What the type column says about it: the type is read out of the name rather than the word
    // "File" being written on every file, which is what made sorting by it look like sorting by name.
    await expect(page.getByRole("option", { name: `${stamp}.tar.gz` })).toContainText("Archive");
    await expect(page.getByRole("option", { name: `${stamp}-typed` })).toContainText("File");
    await page.screenshot({ path: path.join(ARTIFACTS, "listing-types.png"), fullPage: false });

    // Find in terminal: Ctrl+F belongs to the terminal, not to the shell, and the bar says how many
    // matches the scrollback holds. The needle is in the typed line and in what it printed, so the
    // count is more than one — asserted as a count rather than as a number, because which match is
    // current depends on where the scrollback happened to be.
    await page.locator(".xterm").click();
    await page.keyboard.press("Control+f");
    const findBar = page.getByLabel("Find in terminal");
    await expect(findBar).toBeVisible();
    await findBar.fill("ss-e2e-");
    await expect(page.getByText(/\d+\/\d+/).first()).toBeVisible({ timeout: 15_000 });

    await page.keyboard.press("Escape");
    await expect(findBar).toHaveCount(0);

    // Closed on the way out, and not for tidiness: "which sessions are active" is read from the tabs
    // that are open, so a case that leaves a connected terminal behind changes what the cases after
    // it are describing. The sidebar's own case asserts what the active-only switch does when none
    // is open, and it was passing only because this one used to be skipped when the lab ran.
    await page.getByRole("button", { name: "Close terminal" }).click();
    await expect(page.getByRole("button", { name: "Close terminal" })).toHaveCount(0);
  } finally {
    await close();
  }
});

/**
 * The command bar offers the commands it knows, and Tab takes the row the keyboard is on.
 *
 * A case of its own rather than steps inside another one: it is about the bar, and a bar that was
 * only exercised while somebody else's file panel happened to be showing would stop being tested the
 * day that case changed.
 */
test("the command bar completes what somebody is typing", async () => {
  const host = process.env.SEVENSHELL_TEST_HOST;
  const port = process.env.SEVENSHELL_TEST_PORT ?? "22";
  const user = process.env.SEVENSHELL_TEST_USER;
  const password = process.env.SEVENSHELL_TEST_PASSWORD;
  test.skip(!host || !user || !password, "the SSH lab is not configured; set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD");

  const { page, close } = await openApp();
  try {
    const stamp = `ss-e2e-${Date.now() % 100000}`;
    const name = `complete-${stamp}`;

    // Whatever terminal an earlier case left behind is closed first. The panes stay mounted while
    // another tab is on screen, so a second terminal means a second command bar in the page and no
    // way to say which one this case is about. A case closes its own terminal on the way out; this
    // one does not depend on that having happened.
    const closeTerminal = page.getByRole("button", { name: "Close terminal" });
    for (let attempt = 0; attempt < 8 && (await closeTerminal.count()) > 0; attempt += 1) {
      await closeTerminal
        .first()
        .click({ timeout: 5_000 })
        .catch(() => undefined);
      await page.waitForTimeout(100);
    }

    await sidebar(page).getByRole("button", { name: "New session" }).click();
    const editor = page.getByRole("dialog");
    await editor.getByPlaceholder("Production web server").fill(name);
    await editor.getByPlaceholder("example.com").fill(host as string);
    await editor.locator("#session-port").fill(port);
    await editor.getByPlaceholder("root").fill(user as string);
    await editor.getByPlaceholder("Password", { exact: true }).fill(password as string);
    await editor.getByRole("button", { name: "Save" }).click();
    await expect(editor).toHaveCount(0);

    // The first connection to a host asks about its key, and which of the two arrives first is a race
    // with the network — so the terminal and the question are waited for together. A look taken once,
    // before the answer came back, left the case waiting for a terminal that was waiting for a dialog
    // nobody had answered.
    // The bar on screen: the panes stay mounted while another tab is in front, so a second terminal
    // means a second bar in the page, and this case is about the one somebody is typing into.
    const bar = page.locator('input[aria-label="Command input"]').filter({ visible: true });
    const historyButton = page.getByRole("button", { name: "History" }).filter({ visible: true }).first();
    const question = page.getByRole("button", { name: "Trust this key and connect" });
    await sessionRow(page, name).click();
    await expect(question.or(bar).first()).toBeVisible({ timeout: 30_000 });
    if (await question.isVisible()) {
      await question.click();
    }
    await expect(bar).toBeVisible({ timeout: 30_000 });

    // Typing draws the list of what this build knows, with the row the keyboard is on named for a
    // screen reader — the caret never leaves the line while somebody is choosing.
    await bar.fill("upt");
    const suggestions = page.getByRole("listbox", { name: "Completions" });
    await expect(suggestions).toBeVisible();
    await expect(suggestions.getByRole("option").first()).toContainText("uptime");
    // Nothing is chosen yet: the first row is what Tab would take, and the highlight — which is what
    // Enter acts on — appears only on a row somebody walked to.
    expect(await bar.getAttribute("aria-activedescendant"), "the list is up with nothing chosen").toBeNull();
    // Drawn over a terminal, so it has to be a surface and not a window: a background that names a
    // token the sheet does not define draws nothing at all, and then the terminal reads through it.
    expect(await paintsOver(suggestions), "the list paints over the terminal").toBe(true);
    // Kept as an artifact: what the list looks like is the thing a person asks about, and a run that
    // changed how it is drawn should be looked at rather than only asserted on.
    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "completion-command.png"), fullPage: false });

    // Enter runs the line somebody typed rather than the row the list is on: what the history keeps
    // afterwards is `upt`, which nobody completed, and not `uptime`. Read on screen — the same line
    // can be sitting in a hidden tab's list, and a case that read one of those would be reading
    // somebody else's row.
    await bar.press("Enter");
    await expect(suggestions, "running the line takes the list away").toHaveCount(0);
    await historyButton.click();
    await expect(page.getByText("upt", { exact: true }).filter({ visible: true }).first()).toBeVisible({
      timeout: 30_000,
    });
    await historyButton.click();

    // Pointing at a row with the arrow keys and pressing Enter puts *that* row into the line, and does
    // not run it: `chmod` chosen out of `ch` is half a command, and the line on screen is the answer to
    // "what will run". The step above is the other half of the rule — no arrows, and Enter runs the
    // line as it stands.
    await bar.fill("hos");
    await expect(suggestions).toBeVisible();
    await bar.press("ArrowDown");
    expect(await bar.getAttribute("aria-activedescendant"), "the row walked to is the one named").not.toBeNull();
    await bar.press("Enter");
    await expect(bar, "the row goes into the line").toHaveValue("hostname ");
    await expect(suggestions, "and the list is out of the way").toHaveCount(0);

    // The next Enter has nothing pointed at any more, so it runs what the line says.
    await bar.press("Enter");
    await expect(bar).toHaveValue("");
    await historyButton.click();
    await expect(page.getByText("hostname", { exact: true }).filter({ visible: true }).first()).toBeVisible({
      timeout: 30_000,
    });
    await historyButton.click();

    // Tab takes the row, writes it into the line and closes the list. A command is left with a space
    // after it, which is where the next word goes.
    await bar.fill("upt");
    await bar.press("Tab");
    await expect(bar, "Tab takes the row").toHaveValue("uptime ");
    await expect(suggestions, "and closes the list").toHaveCount(0);

    // And what Tab wrote is what the shell gets.
    await bar.press("Enter");
    await historyButton.click();
    await expect(page.getByText("uptime", { exact: true }).filter({ visible: true }).first()).toBeVisible({
      timeout: 30_000,
    });
    await historyButton.click();

    // Taking a command opens what comes after it: `systemctl` is left with a space, and the words it
    // is usually followed by are on offer without having to ask for them.
    await bar.fill("systemctl");
    await bar.press("Tab");
    await expect(bar, "a command is left with a space after it").toHaveValue("systemctl ");
    await expect(suggestions).toBeVisible();
    await expect(suggestions.getByRole("option").first()).toContainText("status");
    await page.screenshot({ path: path.join(ARTIFACTS, "completion-subcommands.png"), fullPage: false });
    await bar.press("Escape");
    await expect(suggestions, "and Escape puts the list away").toHaveCount(0);

    // Paths are the host's own answer: `/tm` lists what is in `/`, read over the same channel the file
    // panel under this terminal is holding. The row is the entry's name — the directory it sits in is
    // already in the line — and the read is a round trip, so the first look is given longer to arrive.
    await bar.fill("cd /tm");
    await expect(suggestions, "the host's directory arrives after the round trip").toBeVisible({ timeout: 15_000 });
    await expect(suggestions.getByRole("option").first()).toContainText("tmp/");
    await bar.press("Tab");
    await expect(bar, "and the token becomes the path the host actually has").toHaveValue("cd /tmp/");

    // Taking a directory opens what is inside it, and that happens on the tick after the accept: a
    // script that pressed Escape in the same tick would be racing the application's own next step, so
    // the list is left to the terminal being closed below rather than dismissed here.
    //
    // Closed on the way out: "which sessions are active" is read from the tabs that are open.
    await page.getByRole("button", { name: "Close terminal" }).click();
    await expect(page.getByRole("button", { name: "Close terminal" })).toHaveCount(0);
    await expect(suggestions, "the list went with the bar it belonged to").toHaveCount(0);
  } finally {
    await close();
  }
});

/**
 * The keys this device trusts, read and revoked from the settings.
 *
 * The other half of "a host key is verified before anything is sent": until now a key that was
 * trusted — during a hurried first connection, or for a machine since rebuilt — could not be looked
 * at or taken back from the interface at all. The key this case revokes is the lab's own, trusted
 * by the cases above, so the list is not empty when it opens.
 */
test("a trusted host key can be read and taken back", async () => {
  test.skip(
    !process.env.SEVENSHELL_TEST_HOST || !process.env.SEVENSHELL_TEST_USER,
    "the SSH lab is not configured; set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD",
  );

  const { page, close } = await openApp();
  try {
    const host = process.env.SEVENSHELL_TEST_HOST as string;
    const port = process.env.SEVENSHELL_TEST_PORT ?? "22";

    // The title bar's gear, not the tab that draws the same word: the banner is the one place the
    // settings are opened from, and the tab strip says "Settings" whenever they are open.
    await page.getByRole("banner").getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Security" }).click();

    // Scoped to the settings page: the sidebar draws the same address on every session row, and an
    // assertion that would pass on those would prove nothing about this list.
    const settings = page.getByRole("region", { name: "Settings" });

    // The other half of the page: the keys this device holds. Importing one opens a native file
    // dialog, which this run cannot drive, so what is asserted is that the section is there and
    // says what it has.
    await expect(settings.getByText("No key has been imported yet.")).toBeVisible();

    // The fingerprint, not just the address: comparing it is the whole reason to look at this list.
    await expect(settings.getByText(`${host}:${port}`, { exact: false }).first()).toBeVisible({ timeout: 15_000 });
    await expect(settings.getByText(/^SHA256:/).first()).toBeVisible();

    await page.getByLabel("Stop trusting this key").first().click();
    await expect(page.getByText("Stop trusting this key?")).toBeVisible();
    await page.getByRole("button", { name: "Forget it" }).click();

    // Gone from the list, and the next connection to it would ask again.
    await expect(settings.getByText(`${host}:${port}`, { exact: false })).toHaveCount(0, { timeout: 15_000 });

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "trusted-keys.png"), fullPage: false });
  } finally {
    await close();
  }
});

/**
 * Finding one session among many, and handing the library to a file.
 *
 * Three things only a real window can settle. The search box is a field a person types into, and
 * what it does to the rows — including the folders it has to open to show what it found — is the
 * answer to whether it is a search or a filter. The narrowing by authentication is behind a panel
 * that has to stay open while it is used and close when the pointer goes elsewhere. And the export
 * question is the last thing between this interface and a native dialog, which is where the run
 * stops: the file dialog is the platform's and cannot be driven from here, so what is asserted is
 * the question — including that the answer which writes passwords into a file starts off unset.
 */
test("the list is searched, narrowed and exported from one menu", async () => {
  const { page, close } = await openApp();
  try {
    await saveSession(page, "searchable-web");
    await saveSession(page, "searchable-db");

    const web = sessionRow(page, "searchable-web");
    const db = sessionRow(page, "searchable-db");
    await expect(web).toBeVisible();
    await expect(db).toBeVisible();

    // The search box, over the things that identify a machine.
    const search = page.getByLabel("Search sessions");
    await search.fill("searchable-web");
    await expect(web).toBeVisible();
    await expect(db).toHaveCount(0);

    // A query that matches nothing says so and offers the way back, rather than an empty list with
    // a search box somebody has to work out for themselves.
    await search.fill("nothing-like-this-at-all");
    // Nothing at all: no sentence, no button — what was typed is in the box with a cross in it.
    await expect(page.getByRole("heading", { name: "searchable-web", exact: true })).toHaveCount(0);
    await expect(sessionRow(page, "searchable-web")).toHaveCount(0);
    await page.screenshot({ path: path.join(ARTIFACTS, "sidebar-search.png"), fullPage: false });
    await search.fill("");
    await expect(web).toBeVisible();
    await expect(db).toBeVisible();

    // The filter panel, which is a panel rather than a menu: choosing one thing must not close it.
    await page.getByRole("button", { name: "Filter sessions" }).click();
    const publicKey = page.getByRole("menuitemcheckbox", { name: "Public key", exact: true });
    await publicKey.click();
    await expect(publicKey).toHaveAttribute("aria-checked", "true");
    // Both of these authenticate with a password, so neither is what was asked for.
    await expect(web).toHaveCount(0);
    await expect(db).toHaveCount(0);

    await page.screenshot({ path: path.join(ARTIFACTS, "sidebar-filter.png"), fullPage: false });

    // The panel's own way out, which clears the filter and leaves the search somebody typed alone.
    await page.getByRole("group", { name: "Filter sessions" }).getByRole("button", { name: "Clear filter" }).click();
    await expect(web).toBeVisible();

    // Exporting, from the library's own menu. The file dialog that follows is the platform's, so
    // the question before it is what this run can hold the interface to.
    await page.getByRole("button", { name: "Import / Export" }).click();
    await page.getByRole("menuitem", { name: /^Export sessions/ }).click();

    const asked = page.getByRole("dialog");
    await expect(asked.getByText(/Anyone who can read that file/)).toBeVisible();
    await expect(asked.getByRole("checkbox")).not.toBeChecked();
    await page.screenshot({ path: path.join(ARTIFACTS, "sidebar-export.png"), fullPage: false });

    await asked.getByRole("button", { name: "Cancel" }).click();
    await expect(asked).toHaveCount(0);

    // And the other half of the menu is there: importing is the one that opens a file dialog first,
    // so it is not run here.
    await page.getByRole("button", { name: "Import / Export" }).click();
    await expect(page.getByRole("menuitem", { name: /^Import sessions/ })).toBeVisible();
    await page.keyboard.press("Escape");
  } finally {
    await close();
  }
});

/**
 * The panel's own controls, driven the way a hand drives them.
 *
 * What only a real window can settle: the divider really moves and the list really follows it, the
 * sidebar really leaves the frame and the title bar really brings it back, and the two switches in
 * the search row do what their own state says they do. These run last, because two of them write
 * settings — the width and whether the panel is open — and a run that leaves the sidebar at 400
 * pixels wide would be a run that changed what the next one sees.
 */
test("the sidebar can be resized, put away, and narrowed by its own switches", async () => {
  const { page, close } = await openApp();
  try {
    await saveSession(page, "panel-host");

    const sidebar = page.locator("[data-session-list]").locator("xpath=ancestor::aside[1]");
    const width = () => sidebar.evaluate((element) => element.getBoundingClientRect().width);
    const before = await width();

    // The divider is a strip at the panel's edge with three dots on it. Dragging it wider has to
    // make the panel wider — the dots are the affordance, so this is what they promise.
    const divider = page.locator("[data-sidebar-divider]");
    await expect(divider).toBeVisible();
    const grip = await divider.boundingBox();
    if (!grip) {
      throw new Error("the divider has no place on screen");
    }
    await page.mouse.move(grip.x + grip.width / 2, grip.y + 300);
    await page.mouse.down();
    await page.mouse.move(grip.x + grip.width / 2 + 120, grip.y + 300, { steps: 8 });
    await page.mouse.up();

    await expect.poll(width).toBeGreaterThan(before + 60);

    // What it was left at is written down, not only remembered: the drag ends with a settings write,
    // which is an answer from the backend and does not land the moment the pointer comes up.
    await expect
      .poll(async () => {
        const written = JSON.parse(await readFile(SETTINGS_FILE, "utf8")) as { sidebarWidth?: number };
        return written.sidebarWidth ?? 0;
      }, "the width is written down when the drag ends")
      .toBeGreaterThan(300);

    // The switches beside the search: a pattern is read as one, and a broken one is marked.
    const regex = page.locator("[data-session-regex]");
    await expect(regex).toHaveAttribute("aria-pressed", "false");
    await page.getByLabel("Search sessions").fill("^panel-host$");
    await expect(sessionRow(page, "panel-host")).toHaveCount(0);

    await regex.click();
    await expect(regex).toHaveAttribute("aria-pressed", "true");
    await expect(sessionRow(page, "panel-host")).toBeVisible();

    await page.getByLabel("Search sessions").fill("([");
    await expect(page.getByLabel("Search sessions")).toHaveAttribute("aria-invalid", "true");
    await expect(sessionRow(page, "panel-host")).toHaveCount(0);

    await regex.click();
    await page.getByLabel("Search sessions").fill("");

    // Only what is open: nothing is connected in this run, so the switch refuses to stay on — an
    // empty list whose only clue is a pressed button is not an answer to anything. And a locate has
    // nothing to find, which is what its disabled state says.
    const activeOnly = page.locator("[data-session-active-only]");
    await activeOnly.click();
    await expect(activeOnly).toHaveAttribute("aria-pressed", "false");
    await expect(sessionRow(page, "panel-host")).toBeVisible();
    await expect(page.getByRole("button", { name: "Locate in sidebar" })).toBeDisabled();

    // Collapse all: a group is made first, so that closing every group is something that can be seen.
    await page.getByRole("button", { name: "New group" }).click();
    const field = page.getByRole("textbox", { name: "Group name" });
    await field.fill("Panel");
    await field.press("Enter");
    await expect(page.getByRole("heading", { name: "Panel", exact: true })).toBeVisible();

    // The row goes into the group by dragging it. The group has to *hold* something for "collapse
    // everything" to be something that can be seen — and the row's own menu, the other way to move it,
    // is the one thing in this sidebar that cannot be driven reliably: its "Move to group" list opens
    // beside the menu, so an item in it has to be landed on while the menu is still arriving.
    const row = sessionRow(page, "panel-host");
    const target = page.getByRole("heading", { name: "Panel", exact: true });
    const from = await row.boundingBox();
    const to = await target.boundingBox();
    if (!from || !to) {
      throw new Error("the rows have no place on screen");
    }
    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
    await page.mouse.down();
    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2 + 12, { steps: 2 });
    await page.mouse.move(to.x + to.width / 2, to.y + to.height / 2, { steps: 8 });
    await page.mouse.up();
    await expect(page.locator("[data-drag-ghost]")).toHaveCount(0);

    const sessionKey = await rowKeyOf(row);
    const panelKey = await rowKeyOf(target);
    await expect
      .poll(async () => {
        const keys = await rowKeys(page);
        const at = keys.indexOf(sessionKey);
        // The row has to be *in* the list: an index of -1 would make the comparison below true by
        // accident, because the group it is compared against is the last one drawn.
        return at < 0 ? "the row is not drawn" : groupOver(keys, at);
      })
      .toBe(panelKey);

    // Everything closes: what the button promises is that no group is left open, which is what the
    // rows drawn afterwards say. Counted rather than asked about one row, because a case that runs
    // after twenty others meets a library that is not the one it made — what "collapse all" does is
    // the same either way, and the row above is under the group either way.
    const drawn = await page.locator("[data-row]").count();
    await page.getByRole("button", { name: "Collapse all" }).click();
    await expect.poll(() => page.locator("[data-row]").count(), { message: "the groups closed" }).toBeLessThan(drawn);
    await expect(page.getByRole("heading", { name: "Panel", exact: true })).toBeVisible();
    await page.screenshot({ path: path.join(ARTIFACTS, "sidebar-chrome.png"), fullPage: false });

    // Away, and back: the way back is in the title bar, where the window's own controls are.
    await page.getByRole("button", { name: "Collapse sidebar" }).click();
    await expect(page.getByRole("button", { name: "Import / Export" })).toHaveCount(0);

    await page.getByRole("banner").getByRole("button", { name: "Expand sidebar" }).click();
    await expect(page.getByRole("button", { name: "Import / Export" })).toBeVisible();

    // A refresh is a read, not a change: what was saved is still there afterwards.
    await page.getByRole("button", { name: "Refresh" }).click();
    await expect(page.getByRole("heading", { name: "Panel", exact: true })).toBeVisible();
  } finally {
    await close();
  }
});

/**
 * A session that will not connect, and one that is picked out with others.
 *
 * The failure half is the part this interface changed its mind about: nothing came up, so there is
 * nothing for the workspace to show. What is asserted is where the answer went instead — a mark on
 * the row, the reason behind it, and a line at the foot of the window — and that no tab was opened
 * to hold a message about a connection that never happened. The address is a closed port on this
 * machine, which fails the same way everywhere and does not need the lab.
 *
 * The second half is the bulk selection: ticking two rows turns the panel's header into the
 * actions for what is ticked, and both of them land in the group that was chosen.
 */
test("a session that fails to connect says so on its row, and rows can be picked out together", async () => {
  const { page, close } = await openApp();
  try {
    // The editor's default port is 22; nothing is listening on 1.
    await saveSession(page, "closed-port-host", "127.0.0.1", "1");
    await saveSession(page, "picked-host");

    const doomed = sessionRow(page, "closed-port-host");
    await doomed.click();

    // No tab, a mark on the row, and the reason at the foot of the window.
    await expect(doomed).toBeVisible();
    await expect(page.locator("[data-row]").filter({ has: anySessionRow(page, "closed-port-host") })).not.toHaveCount(
      0,
    );
    const mark = page.locator("[data-session-failure]").first();
    await expect(mark).toBeVisible({ timeout: 20_000 });
    await expect(page.locator("body")).toContainText("Could not connect to closed-port-host", { timeout: 20_000 });

    const markBox = await mark.boundingBox();
    await mark.click();
    await expect(page.locator("[data-session-failure-detail]")).toBeVisible();

    // Above the row, which is where the reference puts it — and the reason our own bubble had to
    // lose a line of height: a row near the top has the toolbar above it and nothing else.
    const side = await page.locator("[data-session-failure-detail]").evaluate((element, markTop) => {
      const content = element.closest("[data-side]");
      const box = content.getBoundingClientRect();
      return { place: content.getAttribute("data-side"), bottom: Math.round(box.bottom), markTop: Math.round(markTop) };
    }, markBox.y);
    expect(side.place, `the reason opens above the row: ${JSON.stringify(side)}`).toBe("top");
    expect(side.bottom).toBeLessThanOrEqual(side.markTop);
    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "sidebar-failure.png"), fullPage: false });

    // Clearing it puts the row back to what it was.
    await page.getByRole("button", { name: "Clear error" }).click();
    await expect(page.locator("[data-session-failure]")).toHaveCount(0);

    // Picking two rows out: the header becomes the actions for what is picked, and the count says
    // how many there are.
    const first = page
      .locator("[data-row]")
      .filter({ has: anySessionRow(page, "closed-port-host") })
      .first();
    const second = page
      .locator("[data-row]")
      .filter({ has: anySessionRow(page, "picked-host") })
      .first();
    await first.hover();
    await first.locator("[data-session-select]").click();
    await second.hover();
    await second.locator("[data-session-select]").click();

    await expect(page.locator("[data-session-selected-count]")).toHaveText("2 selected");
    await page.screenshot({ path: path.join(ARTIFACTS, "sidebar-selection.png"), fullPage: false });

    // Both into one group, made from the selection: the name is typed over the one it is made with.
    await page.getByRole("button", { name: "New group" }).click();
    const field = page.getByRole("textbox", { name: "Group name" });
    await expect(field).toHaveValue("New group");
    await field.fill("Both");
    await field.press("Enter");

    await expect(page.locator("[data-session-selected-count]")).toHaveCount(0);
    const keys = await rowKeys(page);
    const groupKey = await rowKeyOf(page.getByRole("heading", { name: "Both", exact: true }));
    expect(groupOver(keys, keys.indexOf(await rowKeyOf(doomed)))).toBe(groupKey);
    expect(groupOver(keys, keys.indexOf(await rowKeyOf(sessionRow(page, "picked-host"))))).toBe(groupKey);
  } finally {
    await close();
  }
});

/**
 * What hovering a row offers.
 *
 * Two things that only a real window can settle, and both were wrong once: the tick that picks a
 * row out has to *appear* under the pointer — an unresolved component renders as an empty element
 * that is present, clickable and invisible, which no assertion about its label would catch — and
 * the card beside it has to say what the session is without opening anything.
 */
test("hovering a session row offers the tick and the session's details", async () => {
  const { page, close } = await openApp();
  try {
    await saveSession(page, "hovered-host", "203.0.113.7", "2200");

    // Narrowed to the one row, and not for tidiness: the bubble is taller than the room between a
    // row at the foot of the window and the window's own edge, so on the last row of a long list it
    // is pushed back up to stay on screen — which is the right thing to do and puts the pointer off
    // the row it is about. What is asserted below is where the pointer lands, and the row's middle
    // is where it lands when there is room for the bubble to be centred.
    await page.getByLabel("Search sessions").fill("hovered-host");
    const row = page.locator("[data-row]").filter({ has: anySessionRow(page, "hovered-host") });
    await expect(row).toHaveCount(1);

    await row.hover();

    // Visible, not merely present: opacity is what the hover reveals, so opacity is what is checked.
    const tick = row.locator("[data-session-select]");
    await expect(tick).toHaveCSS("opacity", "1");
    await expect(tick.locator("svg"), "and it draws something").toHaveCount(1);

    const card = page.locator("[data-session-details]");
    await expect(card).toBeVisible();
    await expect(card).toContainText("hovered-host");
    await expect(card).toContainText("labuser@203.0.113.7:2200");
    await expect(card).toContainText("SSH");

    // And the pointer meets the row: the bubble is centred on it rather than hung from its top, so
    // the tip — not the middle of a list of fields — is what says which row this is about.
    const rowBox = await row.boundingBox();
    const centring = await card.evaluate(
      (element, rowMiddle) => {
        const arrow = element.querySelector("span[aria-hidden='true']");
        const box = element.getBoundingClientRect();
        const tip = arrow.getBoundingClientRect();
        return {
          cardMiddle: Math.round(box.top + box.height / 2),
          arrowMiddle: Math.round(tip.top + tip.height / 2),
          rowMiddle: Math.round(rowMiddle),
        };
      },
      rowBox.y + rowBox.height / 2,
    );
    expect(
      Math.abs(centring.arrowMiddle - centring.rowMiddle),
      `the pointer is on the row: ${JSON.stringify(centring)}`,
    ).toBeLessThanOrEqual(2);

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "sidebar-hover.png"), fullPage: false });
  } finally {
    await close();
  }
});

/**
 * The SSH terminal's own page: the preview, the face, the colours and the size.
 *
 * What a real window adds to the unit cases is the preview being *drawn*: it is a block of text in the
 * terminal's own face and at the terminal's own size, so the size slider can be checked by reading the
 * pixels back rather than by reading the settings file twice. The face and the colours are rows on this
 * page rather than on the appearance one — they are what the preview above them is made of.
 */
test("the terminal's page previews what its settings do, and writes them down", async () => {
  const { page, close } = await openApp();
  try {
    await settingsButton(page).click();
    const settings = page.getByRole("region", { name: "Settings" });
    await settings.getByRole("button", { name: "SSH terminal", exact: true }).click();

    // The page opens on its own preview, drawn at the size a fresh install has.
    const preview = page.locator("[data-terminal-preview]");
    const text = preview.locator("pre");
    await expect(preview).toBeVisible();
    await expect(text, "the preview is drawn at the size the settings hold").toHaveCSS("font-size", "13px");

    // The face and the colours are this page's rows, rather than the appearance page's.
    await expect(settings.getByRole("button", { name: "Terminal font", exact: true })).toBeVisible();
    await expect(settings.getByRole("radiogroup", { name: "Terminal colours" })).toBeVisible();

    // The size, moved the way somebody moves it: the preview follows at once, and the file is told on
    // Apply — the same two halves every choice on this page has.
    await settings.getByRole("slider", { name: "Text size" }).fill("18");
    await expect(text, "the picture follows the slider").toHaveCSS("font-size", "18px");
    expect((await writtenSettings()).terminalFontSize ?? 0, "and the file has not been told yet").not.toBe(18);

    await applySettings(page);
    await expect.poll(async () => (await writtenSettings()).terminalFontSize).toBe(18);

    await mkdir(ARTIFACTS, { recursive: true });
    await page.screenshot({ path: path.join(ARTIFACTS, "settings-terminal.png"), fullPage: false });

    // Put back, so that the rest of the run meets the terminal it expects.
    await settings.getByRole("slider", { name: "Text size" }).fill("13");
    await applySettings(page);
    await expect.poll(async () => (await writtenSettings()).terminalFontSize).toBe(13);
    await page.getByRole("button", { name: "Close settings" }).click();
  } finally {
    await close();
  }
});
