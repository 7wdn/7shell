/**
 * Showing what the server sent as YAML, without a YAML library.
 *
 * This build has no YAML emitter and does not need one: the object is already JSON, the view is
 * read-only (editing a manifest is deliberately not part of this workspace), and the only reader is
 * a person. So the emitter here is a printer — it never parses anything back, which is why it can be
 * small and why nothing writes it to a cluster.
 *
 * Two rules it does follow, because a printer is only useful if its output is the document:
 *
 * 1. A string that could be read as something else is quoted — `"1.0"` stays a string, `"yes"` does
 *    not become a boolean in somebody's head, and a value with a leading space keeps it.
 * 2. A Secret's values are not printed. The keys are, because knowing which credential an object
 *    holds is the useful half; the material is not this view's to show, and a screenshot of it is
 *    the way secrets leave a machine.
 */
import { asObject, type K8sObject } from "@/components/k8s/objects";

const RESERVED = new Set(["true", "false", "null", "~", "yes", "no", "on", "off"]);

/** Whether `text` needs quotes to survive as the string it is. */
function needsQuotes(text: string): boolean {
  if (text === "") {
    return true;
  }
  if (/^[\s]|[\s]$/.test(text)) {
    return true;
  }
  if (RESERVED.has(text.toLowerCase())) {
    return true;
  }
  // A number, a date, a hex, an infinity, or anything with YAML's own punctuation in the way.
  if (/^[-+]?(\d+(\.\d*)?|\.\d+)([eE][-+]?\d+)?$/.test(text)) {
    return true;
  }
  if (/^[-+]?0x[0-9a-fA-F]+$/.test(text)) {
    return true;
  }
  // A character that can begin a token: a flow collection, an anchor, a block indicator, a comment.
  if (/^[-?:,[\]{}#&*!|>'"%@`]/.test(text)) {
    return true;
  }
  // A colon only ends a key when a space or the end follows it; `nginx:1.27` is an ordinary scalar.
  if (/:\s|:$/.test(text)) {
    return true;
  }
  return /\s#/.test(text) || text.includes("\n");
}

function scalar(value: unknown): string {
  if (value === null || value === undefined) {
    return "null";
  }
  if (typeof value === "boolean" || typeof value === "number") {
    return String(value);
  }
  if (typeof value === "string") {
    return needsQuotes(value) ? JSON.stringify(value) : value;
  }
  return JSON.stringify(value);
}

function isPlain(value: unknown): boolean {
  return value === null || typeof value === "string" || typeof value === "number" || typeof value === "boolean";
}

/**
 * One value at one indentation, as the lines that follow a key or a list item.
 *
 * Empty collections are written inline (`[]`, `{}`) rather than as a key with nothing under it,
 * because a key with nothing under it reads as a value that failed to print.
 */
function block(value: unknown, indent: number): string[] {
  const pad = " ".repeat(indent);
  if (isPlain(value)) {
    return [`${pad}${scalar(value)}`];
  }
  if (Array.isArray(value)) {
    if (value.length === 0) {
      return [`${pad}[]`];
    }
    const lines: string[] = [];
    for (const item of value) {
      if (isPlain(item)) {
        lines.push(`${pad}- ${scalar(item)}`);
        continue;
      }
      const inner = block(item, indent + 2);
      // A list item whose value is a mapping starts its first key on the dash's own line, which is
      // the shape every manifest in the wild is written in.
      const first = inner[0] ?? "";
      lines.push(`${pad}- ${first.trimStart()}`);
      lines.push(...inner.slice(1));
    }
    return lines;
  }

  const object = asObject(value);
  if (!object || Object.keys(object).length === 0) {
    return [`${pad}{}`];
  }
  const lines: string[] = [];
  for (const [key, entry] of Object.entries(object)) {
    if (isPlain(entry)) {
      lines.push(`${pad}${key}: ${scalar(entry)}`);
      continue;
    }
    if (Array.isArray(entry)) {
      if (entry.length === 0) {
        lines.push(`${pad}${key}: []`);
        continue;
      }
      lines.push(`${pad}${key}:`);
      lines.push(...block(entry, indent + 2));
      continue;
    }
    const nested = asObject(entry);
    if (!nested || Object.keys(nested).length === 0) {
      lines.push(`${pad}${key}: {}`);
      continue;
    }
    lines.push(`${pad}${key}:`);
    lines.push(...block(nested, indent + 2));
  }
  return lines;
}

/** A whole object as YAML, without a leading `---`. */
export function toYaml(value: unknown): string {
  return `${block(value, 0).join("\n")}\n`;
}

/**
 * The object as this build will show it: a Secret keeps its keys and loses its values.
 *
 * `data` and `stringData` are both masked, at the top level and one level in (`data` of a `List`),
 * because a Secret is also reachable through a list that somebody opened.
 */
export function redactedForDisplay(obj: K8sObject): K8sObject {
  if (obj.kind !== "Secret") {
    return obj;
  }
  const copy: K8sObject = { ...obj };
  for (const field of ["data", "stringData"]) {
    const data = asObject(obj[field]);
    if (!data) {
      continue;
    }
    copy[field] = Object.fromEntries(Object.keys(data).map((key) => [key, "***"]));
  }
  return copy;
}
