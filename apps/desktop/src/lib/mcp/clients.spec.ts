import { describe, expect, it } from "vitest";

import { clientConfigSnippet, parseBridgeConfig } from "@/lib/mcp/clients";

const RAW = JSON.stringify({
  mcpServers: {
    "7shell": {
      command: "C:/7Shell/sshell-mcp.exe",
      env: {
        SEVENSHELL_ENDPOINT_FILE: "C:/config/mcp-endpoint.json",
        SEVENSHELL_TOKEN: "must-not-travel",
      },
    },
  },
});

const bridge = parseBridgeConfig(RAW);

describe("a client's configuration snippet", () => {
  it("keeps the host's endpoint but never its token", () => {
    for (const client of ["qoder", "claudeCode", "cursor", "vscode", "codex", "opencode"] as const) {
      const snippet = clientConfigSnippet(client, bridge, RAW);
      expect(snippet, client).toContain("sshell-mcp.exe");
      expect(snippet, client).toContain("mcp-endpoint.json");
      expect(snippet, client).not.toContain("must-not-travel");
    }
  });

  it("writes Codex as TOML and OpenCode under its own mcp key", () => {
    const codex = clientConfigSnippet("codex", bridge, RAW);
    expect(codex).toContain("[mcp_servers.7shell]");
    expect(codex).toContain("[mcp_servers.7shell.env]");

    const opencode = JSON.parse(clientConfigSnippet("opencode", bridge, RAW));
    expect(opencode.mcp["7shell"].type).toBe("local");
    expect(opencode.mcp["7shell"].command).toEqual(["C:/7Shell/sshell-mcp.exe"]);
  });

  it("writes DeepSeek Harness as a YAML insert and Cherry Studio in its own shape", () => {
    expect(clientConfigSnippet("deepseekHarness", bridge, RAW)).toContain("- insert:");
    expect(clientConfigSnippet("deepseekHarness", bridge, RAW)).toContain("serverName: 7shell");

    const cherry = JSON.parse(clientConfigSnippet("cherryStudio", bridge, RAW));
    expect(cherry.mcpServers["7shell"].isActive).toBe(true);
  });
});
