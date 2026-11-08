import { describe, expect, it } from "vitest";

import { interactivePromptNearCursor, isInteractivePromptLine } from "./promptGuard";

describe("isInteractivePromptLine", () => {
  it("recognises the shapes that wait for credentials or a choice", () => {
    expect(isInteractivePromptLine("[sudo] password for alice:")).toBe(true);
    expect(isInteractivePromptLine("Password: ")).toBe(true);
    expect(isInteractivePromptLine("请输入密码：")).toBe(true);
    expect(isInteractivePromptLine("Verification code:")).toBe(true);
    expect(isInteractivePromptLine("Please enter MFA code")).toBe(false);
    expect(isInteractivePromptLine("Continue? [y/N]")).toBe(true);
    expect(isInteractivePromptLine("请选择操作：")).toBe(true);
    expect(isInteractivePromptLine("Select a server:")).toBe(true);
  });

  it("does not mistake a command line for a prompt", () => {
    expect(isInteractivePromptLine("grep password /etc/shadow")).toBe(false);
    expect(isInteractivePromptLine("user@host:~$ git status")).toBe(false);
    expect(isInteractivePromptLine("docker logs --tail 100 web")).toBe(false);
    expect(isInteractivePromptLine("")).toBe(false);
  });
});

describe("interactivePromptNearCursor", () => {
  it("looks at the cursor's line and the one above", () => {
    const rows = ["user@host:~$ ", "Password:", ""];
    expect(interactivePromptNearCursor(1, (row) => rows[row] ?? "")).toBe(true);
    expect(interactivePromptNearCursor(2, (row) => rows[row] ?? "")).toBe(true);
    expect(interactivePromptNearCursor(0, (row) => rows[row] ?? "")).toBe(false);
  });
});
