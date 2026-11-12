#!/usr/bin/env node
/**
 * A stdio MCP server that serves content and nothing else.
 *
 * The window's resources and prompts picker needs a server on the other side of a real pipe: one
 * that declares the capabilities, answers `resources/list`, `resources/read`, `prompts/list` and
 * `prompts/get`, and stays up while the picker is open. No fixture inside the Rust crate can prove
 * the last part — that the desktop's child process is still answering minutes after the handshake —
 * because that harness never leaves the test binary.
 *
 * Deliberately small: one text resource, one prompt with one required argument, and no tools at all.
 * An empty `tools/list` is the interesting half — a server that offers content but no tools is
 * exactly the case where a client that only cared about tools would drop the server entirely.
 *
 * Framing is one JSON-RPC message per line, which is what `sevenshell-mcp-client`'s stdio transport
 * reads and writes.
 */
import { createInterface } from "node:readline";

const PROTOCOL_VERSION = "2025-06-18";
const RESOURCE_URI = "file:///welcome.txt";
const RESOURCE_TEXT = "Hello from the fixture.";

function resource() {
  return { uri: RESOURCE_URI, name: "welcome", mimeType: "text/plain" };
}

function prompt() {
  return {
    name: "summarize",
    description: "Summarize a topic in one paragraph.",
    arguments: [{ name: "topic", description: "What to summarize.", required: true }],
  };
}

/** A missing argument is the caller's mistake, and JSON-RPC has a code for that. */
class InvalidParams extends Error {}

function method(name, params) {
  switch (name) {
    case "initialize":
      // The version is answered as asked for: this server speaks the one the client sends.
      return {
        protocolVersion: PROTOCOL_VERSION,
        capabilities: { tools: {}, resources: {}, prompts: {} },
        serverInfo: { name: "content-fixture", version: "1.0.0" },
      };
    case "ping":
      return {};
    case "tools/list":
      return { tools: [] };
    case "resources/list":
      return { resources: [resource()] };
    case "resources/read": {
      if (params?.uri !== RESOURCE_URI) {
        throw new InvalidParams(`no resource at ${String(params?.uri)}`);
      }
      return { contents: [{ uri: RESOURCE_URI, mimeType: "text/plain", text: RESOURCE_TEXT }] };
    }
    case "prompts/list":
      return { prompts: [prompt()] };
    case "prompts/get": {
      if (params?.name !== "summarize") {
        throw new InvalidParams(`no prompt named ${String(params?.name)}`);
      }
      const topic = params?.arguments?.topic;
      if (typeof topic !== "string" || topic.trim() === "") {
        throw new InvalidParams("the summarize prompt needs a topic");
      }
      return {
        description: "Summarize a topic in one paragraph.",
        messages: [{ role: "user", content: { type: "text", text: `Summarize ${topic}` } }],
      };
    }
    default:
      throw Object.assign(new Error(`unknown method ${name}`), { code: -32601 });
  }
}

function answer(line) {
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    // A line that is not JSON has no id to answer with; saying so on stderr keeps stdout clean.
    process.stderr.write(`content-fixture: ignoring a line that is not JSON\n`);
    return;
  }
  if (message === null || typeof message !== "object") {
    return;
  }
  // Notifications — anything without an id — are never answered, `notifications/initialized`
  // included. That silence is part of the protocol, not an omission.
  if (message.id === undefined || message.id === null) {
    return;
  }

  const reply = { jsonrpc: "2.0", id: message.id };
  try {
    reply.result = method(message.method, message.params);
  } catch (error) {
    reply.error = {
      code: typeof error?.code === "number" ? error.code : -32602,
      message: String(error?.message ?? error),
    };
  }
  process.stdout.write(`${JSON.stringify(reply)}\n`);
}

const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
lines.on("line", (line) => {
  if (line.trim() !== "") {
    answer(line);
  }
});
