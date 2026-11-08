/**
 * A prompt that is waiting for an answer rather than for a command.
 *
 * Inline completion assumes the line on screen belongs to a shell. A password prompt, a one-time
 * code, a menu that wants a number and a `(y/n)` question all look like a shell prompt to a line
 * tracker — the cursor is at the end of a line and typing works — but Tab and completion there are
 * at best useless and at worst a rewrite of a credential. This reads the two lines around the cursor
 * and suppresses completion when one of them is asking.
 *
 * It is a heuristic, deliberately narrow: a prompt has to end in a colon (or be one of the literal
 * confirmation shapes) to count, so an ordinary command line that happens to mention "password" is
 * not mistaken for one.
 */

const PASSWORD_PROMPT = /(?:pass(?:word|wd|\s?phrase)|口令|密码)[^:：\n]*[:：]\s*$|^enter password\b/i;

const AUTH_CHALLENGE =
  /(?:otp|mfa|2fa|one[-\s]?time(?:\s+(?:password|passcode|code))?|two[-\s]?factor|verification(?:\s+code)?|authentication\s+code|验证码|动态[口令密码]|一次性密码|双因子|二次验证)[^:：\n]{0,32}[:：]\s*$/i;

const CONFIRMATION =
  /(?:\[[YyNn]\]|\[[Yy]\/[Nn]\]|\((?:[Yy]\/[Nn]|[Nn]\/[Yy]|[Yy]es\/[Nn]o|[Nn]o\/[Yy]es)\)|\(y\/n\))\s*(?:[:：?？]\s*)?$/i;

const SELECTION =
  /(?:(?:please\s+)?(?:select|choose|pick|enter)\b[^:：\n?？]{0,40}|请(?:选择|输入|确认)|输入(?:序号|编号|选项))[^:：\n?？]{0,40}[:：?？]\s*$/i;

/** Whether one terminal line, as it is drawn, is asking for an answer. */
export function isInteractivePromptLine(text: string): boolean {
  const trimmed = text.trimEnd();
  if (trimmed === "") {
    return false;
  }
  return (
    PASSWORD_PROMPT.test(trimmed) ||
    AUTH_CHALLENGE.test(trimmed) ||
    CONFIRMATION.test(trimmed) ||
    SELECTION.test(trimmed)
  );
}

/**
 * Samples the cursor's own line and the one above it.
 *
 * `rowText` is injected so the decision can be tested with plain strings; the caller passes the
 * terminal buffer's absolute rows.
 */
export function interactivePromptNearCursor(cursorRow: number, rowText: (row: number) => string): boolean {
  for (let row = cursorRow; row >= Math.max(0, cursorRow - 1); row -= 1) {
    if (isInteractivePromptLine(rowText(row))) {
      return true;
    }
  }
  return false;
}
