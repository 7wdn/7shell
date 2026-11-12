/**
 * The client configuration snippets, derived from the one the host already builds.
 *
 * `mcp_client_config` answers with a single `mcpServers` object naming the bridge beside this
 * application and the endpoint file the bridge reads. That is the whole truth about how a client
 * starts it; these are the same facts written the way the clients a person may use expect to read
 * them. Two rules shape the derivation:
 *
 * 1. **Only the endpoint variable travels.** A snippet is pasted into files other programs read, so
 *    the environment is rebuilt from the one key the bridge documents rather than passed through.
 *    A token that ever appeared in the host's object would not reach a clipboard from here.
 * 2. **Nothing is invented when the host cannot answer.** With no command in the host's text, the
 *    caller is handed that text unchanged (often empty), and the page says it cannot be read —
 *    naming `sshell-mcp` on a machine that does not have it would be a client that starts nothing.
 */

/** The clients the access tab offers a snippet for. */
export type McpClientId = "claudeCode" | "cursor" | "vscode" | "windsurf" | "qoder" | "codex";

/** The order the tab strip lists them in: the ones most people reach for first. */
export const MCP_CLIENTS: readonly McpClientId[] = ["claudeCode", "cursor", "vscode", "windsurf", "qoder", "codex"];

/**
 * Where each client reads its MCP servers from, as the line under the snippet says.
 *
 * A path, not a sentence: it is the same on every language and is pasted as it is.
 */
export const MCP_CLIENT_LOCATIONS: Record<McpClientId, string> = {
  claudeCode: ".mcp.json",
  cursor: ".cursor/mcp.json",
  vscode: ".vscode/mcp.json",
  windsurf: "~/.codeium/windsurf/mcp_config.json",
  qoder: "~/.qoder/mcp.json",
  codex: "~/.codex/config.toml",
};

/** The one environment variable a client needs: where the endpoint file is. */
export const MCP_ENDPOINT_ENV = "SEVENSHELL_ENDPOINT_FILE";

/** The two facts a client's configuration is made of, taken from the host's own snippet. */
export interface BridgeConfig {
  readonly command: string;
  readonly env: Record<string, string>;
}

/** What one JSON object is, when it is one. */
function record(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/**
 * The bridge command and endpoint variable out of the host's `mcpServers` snippet.
 *
 * The server is still called `7shell`: that name is what the host wrote, what the tool prefixes in
 * an agent's list are built from, and what a configuration somebody already pasted keeps working.
 * `null` means the text is not one of these snippets or names no program — the caller draws that as
 * "cannot be read", not as a guessed command.
 */
export function parseBridgeConfig(raw: string): BridgeConfig | null {
  if (raw.trim() === "") {
    return null;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }

  const servers = record(record(parsed)?.["mcpServers"]);
  const entry = record(servers?.["7shell"]);
  const command = entry?.["command"];
  if (typeof command !== "string" || command.trim() === "") {
    return null;
  }

  // Rebuilt, not copied: see rule 1 at the top of this file.
  const endpoint = record(entry?.["env"])?.[MCP_ENDPOINT_ENV];
  const env: Record<string, string> = {};
  if (typeof endpoint === "string" && endpoint !== "") {
    env[MCP_ENDPOINT_ENV] = endpoint;
  }

  return { command, env };
}

/**
 * One client's snippet: the host's facts in the shape that client reads.
 *
 * VS Code keeps its servers under `servers` and wants the transport named; every other client on
 * this list reads the `mcpServers` object the host already wrote. When the host named no command,
 * its own text is handed back untouched, and an empty one is the page's "cannot be read yet".
 */
export function clientConfigSnippet(client: McpClientId, bridge: BridgeConfig | null, raw: string): string {
  if (!bridge) {
    return raw;
  }
  if (client === "vscode") {
    return JSON.stringify(
      { servers: { "7shell": { type: "stdio", command: bridge.command, args: [], env: bridge.env } } },
      null,
      2,
    );
  }
  return JSON.stringify({ mcpServers: { "7shell": { command: bridge.command, args: [], env: bridge.env } } }, null, 2);
}
