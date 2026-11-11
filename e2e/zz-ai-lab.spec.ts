/**
 * A destructive command, refused and then approved, with a real model in the loop.
 *
 * Environment-gated twice over: the gateway's address, key and model are a machine's own, and the
 * session the command acts on is the SSH lab's. A case that needed either would fail everywhere
 * else, so it skips when they are absent. It runs after the suite's vault case — the `zz-` name puts
 * it last — and with:
 *
 * ```text
 * SEVENSHELL_TEST_AI_BASE_URL=http://127.0.0.1:3425/v1 SEVENSHELL_TEST_AI_KEY=... \
 * SEVENSHELL_TEST_AI_MODEL=group/auto-deepseek-v4.1-flash \
 * SEVENSHELL_TEST_HOST=127.0.0.1 SEVENSHELL_TEST_PORT=2222 \
 * SEVENSHELL_TEST_USER=labuser SEVENSHELL_TEST_PASSWORD=... \
 * SEVENSHELL_E2E_PORT=9100 SEVENSHELL_E2E_GREP="a vault can be created and the shell opens|destructive command" \
 *   pnpm app:e2e
 * ```
 *
 * What this proves that the unit and contract tests cannot is the whole chain at once: a model
 * across a real socket picks the tools, this window's own approval dialog is what stands between it
 * and the host, and the answer a person gives decides whether the file is still there. Both halves
 * matter — a refusal that still ran the command, and an approval that did nothing, are the two ways
 * this could be wrong while looking entirely right.
 *
 * The key is read from the environment, typed into the password field, and never written to this
 * repository.
 */
import { execFileSync } from "node:child_process";

import { chromium, expect, test, type Locator, type Page } from "@playwright/test";

const CDP_URL = process.env.SEVENSHELL_E2E_CDP ?? "http://127.0.0.1:9222";

const AI_BASE_URL = process.env.SEVENSHELL_TEST_AI_BASE_URL;
const AI_KEY = process.env.SEVENSHELL_TEST_AI_KEY;
const AI_MODEL = process.env.SEVENSHELL_TEST_AI_MODEL;

const LAB_HOST = process.env.SEVENSHELL_TEST_HOST;
const LAB_PORT = process.env.SEVENSHELL_TEST_PORT ?? "22";
const LAB_USER = process.env.SEVENSHELL_TEST_USER;
const LAB_PASSWORD = process.env.SEVENSHELL_TEST_PASSWORD;

/**
 * The lab container, which is what holds the host's `/tmp`.
 *
 * Read through Docker rather than through the application on purpose: the claim being checked is
 * about the far side of the connection, and asking the same client that ran the command whether it
 * ran would accept its own answer as the evidence.
 */
const LAB_CONTAINER = "sevenshell-lab-sshd";

/** The one file the command under approval would delete. Named rather than stamped: the prompt says it. */
const SENTINEL = "/tmp/seven-shell-ai-e2e";
const SESSION_NAME = "e2e-ai-lab";

/**
 * Runs one shell line inside the lab container, and reports whether it exited zero.
 *
 * `as` is who runs it. Ownership is what makes the approved round mean anything: a file `docker
 * exec` makes belongs to root, and root's file inside a sticky `/tmp` is one the session's own
 * account may not delete — `rm` would answer "Operation not permitted" and the host would be right
 * to. The fixture is therefore made the way the host would see it, under the account that logs in.
 */
function inLab(script: string, as?: string): { ok: boolean; output: string } {
  try {
    const output = execFileSync("docker", ["exec", ...(as ? ["-u", as] : []), LAB_CONTAINER, "sh", "-lc", script], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
    return { ok: true, output: output.trim() };
  } catch (error) {
    const failure = error as { stdout?: string; stderr?: string };
    return { ok: false, output: `${failure.stdout ?? ""}${failure.stderr ?? ""}`.trim() };
  }
}

function sentinelExists(): boolean {
  // Read as root, so the answer is "the file is there" rather than "this account can see it".
  return inLab(`test -e ${SENTINEL}`).ok;
}

/** The assistant's open panel, which is what the question is typed into. */
async function openAssistant(page: Page): Promise<Locator> {
  const composer = page.getByPlaceholder("Ask about this host…");
  if (!(await composer.isVisible().catch(() => false))) {
    await page.locator("[data-assistant-toggle]").click();
  }
  await expect(composer).toBeVisible({ timeout: 15_000 });
  return composer;
}

/** The tool card of one call, wherever the transcript has got to. */
function toolCard(page: Page, name: string): Locator {
  return page.locator("[data-tool-card]").filter({ hasText: name }).last();
}

/**
 * Asks for the deletion, and waits for the question the policy raises.
 *
 * Two attempts rather than one because the second turn is the model's to spend: a listing first,
 * then the call that stops for approval. If the model answers in prose instead, one firmer question
 * is asked — and only one, because retrying a model into agreement is how a case starts passing
 * without the model having done anything.
 *
 * Returns false when the round ended without an approval, which the caller treats as a failed
 * attempt rather than as a refusal.
 */
async function askToDelete(page: Page, composer: Locator, approval: Locator): Promise<boolean> {
  const questions = [
    `Use the sshell_sessions_list tool with kind saved to find the session named "${SESSION_NAME}". ` +
      `Then call sshell_exec_run with that session's id and the command "rm -rf ${SENTINEL}". Do nothing else.`,
    `Call the sshell_exec_run tool now. Take the sessionId of the saved session named "${SESSION_NAME}" ` +
      `from sshell_sessions_list, and pass the command "rm -rf ${SENTINEL}". Make no other tool calls.`,
  ];

  for (const question of questions) {
    await expect(composer).toBeEnabled({ timeout: 30_000 });
    await composer.fill(question);
    await composer.press("Enter");

    // The run is under way when the line is busy or the question is up; the press having done
    // nothing at all is the third possibility, and it is a failure rather than a wait.
    await expect
      .poll(
        async () => {
          if (await approval.isVisible().catch(() => false)) {
            return "asked";
          }
          return (await composer.isEnabled()) ? "idle" : "busy";
        },
        { timeout: 15_000 },
      )
      .not.toBe("idle");

    const deadline = Date.now() + 150_000;
    while (Date.now() < deadline) {
      if (await approval.isVisible().catch(() => false)) {
        return true;
      }
      // The composer coming back is the end of the round: the model either called the tool or
      // talked about it, and only one of those is this case.
      if (await composer.isEnabled().catch(() => false)) {
        return false;
      }
      await page.waitForTimeout(500);
    }
    return false;
  }

  return false;
}

/**
 * Answers every question a round asks, and waits for the round to end.
 *
 * A round can carry more than one call — the listing, then the one being asked for — and a second
 * destructive call would be a second question. Answering each with the same decision keeps the
 * round's meaning; the loop's own deadline is what stops a model that keeps asking from running out
 * the test's clock with nothing said.
 */
async function settle(
  page: Page,
  composer: Locator,
  approval: Locator,
  decision: "Refuse" | "Approve",
): Promise<number> {
  const deadline = Date.now() + 180_000;
  let answered = 0;

  while (Date.now() < deadline) {
    if (await approval.isVisible().catch(() => false)) {
      await approval.getByRole("button", { name: decision, exact: true }).click();
      answered += 1;
      await expect(approval).toHaveCount(0, { timeout: 20_000 });
      continue;
    }
    if (await composer.isEnabled().catch(() => false)) {
      return answered;
    }
    await page.waitForTimeout(500);
  }

  throw new Error("the assistant's round did not end within three minutes");
}

test("the assistant refuses and then approves a destructive command", async () => {
  test.skip(!AI_BASE_URL || !AI_KEY || !AI_MODEL, "needs SEVENSHELL_TEST_AI_BASE_URL / _KEY / _MODEL");
  test.skip(
    !LAB_HOST || !LAB_USER || !LAB_PASSWORD,
    "the SSH lab is not configured; set SEVENSHELL_TEST_HOST/PORT/USER/PASSWORD",
  );
  // Two rounds, each a listing plus a tool call, against a model that takes its own time.
  test.setTimeout(300_000);

  const browser = await chromium.connectOverCDP(CDP_URL);
  const context = browser.contexts()[0];
  if (!context) {
    throw new Error("the application has no webview context");
  }
  const page = context.pages()[0] ?? (await context.waitForEvent("page"));

  // The suite's vault case runs first; a standalone run is skipped rather than described as a
  // failure of the assistant.
  if (
    !(await page
      .getByLabel("Search sessions")
      .isVisible()
      .catch(() => false))
  ) {
    await browser.close();
    test.skip(true, "run this with the suite's vault case so the shell is open");
  }

  try {
    // -- The host this case acts on -----------------------------------------
    //
    // Saved the way a person saves one, password and all: the assistant's calls reuse the vault's
    // credential, so a session that only had a host and a port would stop at a password prompt
    // nobody is there to answer.
    const sidebar = page.locator("[data-session-list]").locator("xpath=ancestor::aside[1]");
    await sidebar.getByRole("button", { name: "New session" }).click();
    const editor = page.getByRole("dialog");
    await editor.getByPlaceholder("Production web server").fill(SESSION_NAME);
    await editor.getByPlaceholder("example.com").fill(LAB_HOST as string);
    await editor.locator("#session-port").fill(LAB_PORT);
    await editor.getByPlaceholder("root").fill(LAB_USER as string);
    await editor.getByPlaceholder("Password", { exact: true }).fill(LAB_PASSWORD as string);
    await editor.getByRole("button", { name: "Save" }).click();
    await expect(editor).toHaveCount(0);

    // Opening the terminal is how the host key becomes trusted. Without this the assistant's first
    // call would stop on a host-key question, which is a different dialog with a different reason,
    // and the case would be about the wrong decision.
    const question = page.getByRole("button", { name: "Trust this key and connect" });
    const commands = page.getByRole("button", { name: "Commands" });
    await sidebar.getByTitle(`Open a terminal for ${SESSION_NAME}`).click();
    await expect(question.or(commands).first()).toBeVisible({ timeout: 30_000 });
    if (await question.isVisible()) {
      await question.click();
    }
    await expect(commands).toBeVisible({ timeout: 30_000 });

    // -- The two settings the case cannot be about --------------------------
    //
    // A fresh install reads only, and a read-only tier refuses `sshell_exec_run` before it grades
    // anything. "Writes you allow" is the tier this case is about: a command may run, and a
    // destructive one still stops for a person.
    const settings = page.getByRole("region", { name: "Settings" });
    if (!(await settings.isVisible().catch(() => false))) {
      await page.getByRole("banner").getByRole("button", { name: "Settings" }).click();
    }
    await expect(settings).toBeVisible();

    await settings.getByRole("button", { name: "Agent access", exact: true }).click();
    await settings.locator("#mcp-scope").click();
    await page.getByRole("option", { name: "Writes you allow", exact: true }).click();
    await settings.getByRole("button", { name: "Save and apply", exact: true }).click();
    await expect(settings.getByText(/^Saved/)).toBeVisible({ timeout: 20_000 });

    // The provider: address, model and key from the environment, the key only ever in the password
    // field. The kind changes the address default, so it is chosen before the address is typed.
    await settings.getByRole("button", { name: "Assistant", exact: true }).click();
    await settings.locator("[data-ai-provider-add]").click();
    const form = settings.locator("[data-ai-provider-form]");
    await expect(form).toBeVisible();
    await form.locator("#ai-provider-name").fill("AI lab gateway");
    await form.locator("#ai-provider-kind").click();
    await page.getByRole("option", { name: "OpenAI", exact: true }).click();
    await form.locator("#ai-provider-base-url").fill(AI_BASE_URL as string);
    await form.locator("#ai-provider-model").fill(AI_MODEL as string);
    await form.locator("#ai-provider-key").fill(AI_KEY as string);
    await form.getByRole("button", { name: "Save provider", exact: true }).click();
    await expect(settings.getByText("AI lab gateway", { exact: false })).toBeVisible({ timeout: 20_000 });
    await page.getByRole("button", { name: "Close settings" }).click();

    const composer = await openAssistant(page);

    // -- A file for the command to find -------------------------------------
    const touched = inLab(`touch ${SENTINEL}`, LAB_USER as string);
    expect(touched.ok, `the sentinel should have been made in the lab: ${touched.output}`).toBe(true);
    expect(sentinelExists(), "the sentinel exists before anything is asked").toBe(true);

    // -- Refused ------------------------------------------------------------
    const approval = page.locator("[data-mcp-approval]");
    expect(await askToDelete(page, composer, approval), "the model should have asked to run the command").toBe(true);

    // The question's own facts: this window's assistant asked, the policy's verdict is why, and the
    // command it would run names the file.
    await expect(approval).toHaveAttribute("data-approval-origin", "ai");
    await expect(approval).toHaveAttribute("data-approval-reason", "risk");
    await expect(approval).toContainText("sshell_exec_run");
    await expect(approval).toContainText(SESSION_NAME);
    await expect(approval).toContainText(`rm -rf ${SENTINEL}`);

    const refused = await settle(page, composer, approval, "Refuse");
    expect(refused, "at least one question was answered").toBeGreaterThan(0);

    // The call reports the decision rather than a malfunction: failed, with the policy's own code.
    const refusedCard = toolCard(page, "sshell_exec_run");
    await expect(refusedCard).toContainText("Failed", { timeout: 30_000 });
    await expect(refusedCard).toContainText("POLICY-DENIED", { timeout: 30_000 });

    // And nothing reached the host, which is the half the card cannot promise.
    expect(sentinelExists(), "a refused command must not have deleted anything").toBe(true);

    // -- Approved -----------------------------------------------------------
    //
    // A fresh conversation, so the second question is the same request rather than a model being
    // told by its own history that the person already said no.
    await page.locator("[data-assistant-session-new]").click();
    await expect(composer).toBeEnabled({ timeout: 30_000 });

    expect(await askToDelete(page, composer, approval), "the model should have asked a second time").toBe(true);
    await expect(approval).toHaveAttribute("data-approval-origin", "ai");
    await expect(approval).toHaveAttribute("data-approval-reason", "risk");

    const approved = await settle(page, composer, approval, "Approve");
    expect(approved, "at least one question was answered").toBeGreaterThan(0);

    // What the host did, in the card's own words, and then on the host itself.
    const approvedCard = toolCard(page, "sshell_exec_run");
    await expect(approvedCard).toContainText("Done", { timeout: 30_000 });
    await expect(approvedCard).toContainText('"exitCode":0', { timeout: 30_000 });

    await expect
      .poll(sentinelExists, { message: "an approved command must have deleted the file", timeout: 30_000 })
      .toBe(false);
  } finally {
    // The sentinel is this case's own doing, so it does not leave one behind — including after a
    // failure, which is exactly when a leftover would confuse the next run.
    inLab(`rm -f ${SENTINEL}`);
    await browser.close();
  }
});
