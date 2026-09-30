// Descriptor loading and validation.
//
// `descriptors/**/*.yaml` is the source of truth for everything Rust and TypeScript
// must agree on. This module reads it, validates it with messages that name the file
// and the field, and returns the manifest that both generators consume.

import { readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import YAML from "yaml";

export const GENERATOR_ID = "scripts/sync-descriptors.mjs";
export const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
export const DESCRIPTORS_DIR = path.join(REPO_ROOT, "descriptors");
export const MANIFEST_PATH = path.join(DESCRIPTORS_DIR, "descriptors.manifest.json");
export const GENERATED_TS_DIR = path.join(REPO_ROOT, "apps", "desktop", "src", "types", "generated");

export const SCHEMA_VERSION = 1;
export const GROUPS = ["terminal", "transfer", "network", "session", "auth", "trust"];
export const RUNTIMES = ["native", "sidecar", "external"];
export const SUPPORT_LEVELS = ["connect", "browse", "operate"];
export const PLATFORMS = ["windows", "macos", "linux"];

export class DescriptorError extends Error {}

const IDENTIFIER = /^[a-z][A-Za-z0-9]*$/;
const RUST_VARIANT = /^[A-Z][A-Za-z0-9]*$/;
const MESSAGE_KEY = /^[a-z][A-Za-z0-9]*(\.[A-Za-z0-9]+)+$/;

function relative(file) {
  return path.relative(REPO_ROOT, file).split(path.sep).join("/");
}

function fail(file, message) {
  throw new DescriptorError(`${relative(file)}: ${message}`);
}

function requireSchemaVersion(file, data) {
  if (data?.schemaVersion !== SCHEMA_VERSION) {
    fail(file, `\`schemaVersion\` must be ${SCHEMA_VERSION}`);
  }
}

function requireText(file, data, key) {
  const value = data?.[key];
  if (typeof value !== "string" || value.trim() === "") {
    fail(file, `\`${key}\` must be a non-empty string`);
  }
  return value;
}

function requireOrder(file, data) {
  const value = data?.order;
  if (!Number.isInteger(value) || value <= 0) {
    fail(file, "`order` must be a positive integer");
  }
  return value;
}

function requirePositiveInt(file, data, key, { optional = false } = {}) {
  const value = data?.[key];
  if (optional && (value === undefined || value === null)) {
    return null;
  }
  if (!Number.isInteger(value) || value <= 0) {
    fail(file, `\`${key}\` must be a positive integer`);
  }
  return value;
}

function requireOneOf(file, data, key, allowed) {
  const value = requireText(file, data, key);
  if (!allowed.includes(value)) {
    fail(file, `\`${key}\` must be one of ${allowed.join(", ")} (found \`${value}\`)`);
  }
  return value;
}

function requireId(file, data) {
  const value = requireText(file, data, "id");
  if (!IDENTIFIER.test(value)) {
    fail(file, `\`id\` must be lowerCamelCase and start with a letter (found \`${value}\`)`);
  }
  return value;
}

function requireRustVariant(file, data) {
  const value = requireText(file, data, "rustVariant");
  if (!RUST_VARIANT.test(value)) {
    fail(file, `\`rustVariant\` must be a PascalCase Rust identifier (found \`${value}\`)`);
  }
  return value;
}

function requireLabelKey(file, data, prefix) {
  const value = requireText(file, data, "labelKey");
  if (!MESSAGE_KEY.test(value) || !value.startsWith(prefix)) {
    fail(file, `\`labelKey\` must be a message key starting with \`${prefix}\` (found \`${value}\`)`);
  }
  if (!value.endsWith(".label")) {
    fail(file, `\`labelKey\` must end with \`.label\` (found \`${value}\`)`);
  }
  return value;
}

function requireStringList(file, data, key, { allowed = null } = {}) {
  const value = data?.[key];
  if (value === undefined || value === null) {
    return [];
  }
  if (!Array.isArray(value)) {
    fail(file, `\`${key}\` must be a list`);
  }
  const seen = new Set();
  for (const item of value) {
    if (typeof item !== "string" || item.trim() === "") {
      fail(file, `\`${key}\` must contain non-empty strings`);
    }
    if (allowed && !allowed.includes(item)) {
      fail(file, `\`${key}\` contains \`${item}\`, which is not one of ${allowed.join(", ")}`);
    }
    if (seen.has(item)) {
      fail(file, `\`${key}\` lists \`${item}\` twice`);
    }
    seen.add(item);
  }
  return [...value];
}

async function readYamlDirectory(directory) {
  let entries;
  try {
    entries = await readdir(directory, { withFileTypes: true });
  } catch {
    return [];
  }
  const files = entries
    .filter((entry) => entry.isFile() && /\.ya?ml$/.test(entry.name))
    .map((entry) => path.join(directory, entry.name))
    .sort();

  const documents = [];
  for (const file of files) {
    const text = await readFile(file, "utf8");
    try {
      documents.push({ file, data: YAML.parse(text) });
    } catch (error) {
      fail(file, `is not valid YAML: ${error.message}`);
    }
  }
  return documents;
}

function assertUnique(entries, key, describe) {
  const seen = new Map();
  for (const entry of entries) {
    const value = entry[key];
    const previous = seen.get(value);
    if (previous) {
      throw new DescriptorError(`duplicate ${describe} \`${value}\`: ${previous} and ${entry.$file}`);
    }
    seen.set(value, entry.$file);
  }
}

function assertUniqueOrder(entries, describe) {
  entries.forEach((entry, index) => {
    if (index > 0 && entry.order < entries[index - 1].order) {
      throw new DescriptorError(`entries are not sorted by \`order\` around ${entry.$file}`);
    }
    if (index > 0 && entry.order === entries[index - 1].order) {
      throw new DescriptorError(
        `two ${describe} share \`order\` ${entry.order}: ${entries[index - 1].$file} and ${entry.$file}`,
      );
    }
  });
}

async function loadCapabilities() {
  const documents = await readYamlDirectory(DESCRIPTORS_DIR);
  const catalogFile = documents.find((document) => Array.isArray(document.data?.capabilities));
  if (!catalogFile) {
    throw new DescriptorError("descriptors/capabilities.yaml is missing or declares no `capabilities` list");
  }

  const capabilities = catalogFile.data.capabilities.map((entry, index) => {
    const where = `${relative(catalogFile.file)}#capabilities[${index}]`;
    if (typeof entry !== "object" || entry === null) {
      throw new DescriptorError(`${where}: entries must be mappings`);
    }
    const key = entry.key;
    if (typeof key !== "string" || !IDENTIFIER.test(key)) {
      throw new DescriptorError(`${where}: \`key\` must be lowerCamelCase and start with a letter`);
    }
    if (!Number.isInteger(entry.order) || entry.order <= 0) {
      throw new DescriptorError(`${where}: \`order\` must be a positive integer`);
    }
    if (!GROUPS.includes(entry.group)) {
      throw new DescriptorError(`${where}: \`group\` must be one of ${GROUPS.join(", ")}`);
    }
    if (
      typeof entry.labelKey !== "string" ||
      !entry.labelKey.startsWith("capability.") ||
      !entry.labelKey.endsWith(".label")
    ) {
      throw new DescriptorError(`${where}: \`labelKey\` must start with \`capability.\` and end with \`.label\``);
    }
    if (entry.planned !== undefined && typeof entry.planned !== "boolean") {
      throw new DescriptorError(`${where}: \`planned\` must be a boolean when present`);
    }
    return {
      key,
      order: entry.order,
      group: entry.group,
      labelKey: entry.labelKey,
      planned: entry.planned === true,
      $file: where,
    };
  });

  assertUnique(capabilities, "key", "capability key");
  assertUniqueOrder(capabilities, "capabilities");

  return capabilities.map(({ key, order, group, labelKey, planned }) => ({ key, order, group, labelKey, planned }));
}

async function loadProtocols(capabilityKeys) {
  const documents = await readYamlDirectory(path.join(DESCRIPTORS_DIR, "protocols"));
  const protocols = documents.map(({ file, data }) => {
    requireSchemaVersion(file, data);
    return {
      id: requireId(file, data),
      rustVariant: requireRustVariant(file, data),
      order: requireOrder(file, data),
      labelKey: requireLabelKey(file, data, "protocol."),
      defaultPort: requirePositiveInt(file, data, "defaultPort", { optional: true }),
      runtime: requireOneOf(file, data, "runtime", RUNTIMES),
      transport: requireText(file, data, "transport"),
      supportLevel: requireOneOf(file, data, "supportLevel", SUPPORT_LEVELS),
      capabilities: requireStringList(file, data, "capabilities", { allowed: capabilityKeys }),
      $file: relative(file),
    };
  });

  assertUnique(protocols, "id", "protocol id");
  assertUnique(protocols, "rustVariant", "protocol rustVariant");
  protocols.sort((left, right) => left.order - right.order);
  assertUniqueOrder(protocols, "protocols");

  return protocols.map(({ $file, ...protocol }) => protocol);
}

async function loadAuthMethods(capabilityKeys) {
  const documents = await readYamlDirectory(path.join(DESCRIPTORS_DIR, "auth-methods"));
  const methods = documents.map(({ file, data }) => {
    requireSchemaVersion(file, data);
    return {
      id: requireId(file, data),
      rustVariant: requireRustVariant(file, data),
      order: requireOrder(file, data),
      labelKey: requireLabelKey(file, data, "auth."),
      capabilities: requireStringList(file, data, "capabilities", { allowed: capabilityKeys }),
      platforms: requireStringList(file, data, "platforms", { allowed: PLATFORMS }),
      $file: relative(file),
    };
  });

  assertUnique(methods, "id", "auth method id");
  assertUnique(methods, "rustVariant", "auth method rustVariant");
  methods.sort((left, right) => left.order - right.order);
  assertUniqueOrder(methods, "auth methods");

  return methods.map(({ $file, ...method }) => method);
}

/** Reads and validates every descriptor, returning the manifest and its parts. */
export async function loadDescriptors() {
  const capabilities = await loadCapabilities();
  const capabilityKeys = capabilities.map((capability) => capability.key);
  const protocols = await loadProtocols(capabilityKeys);
  const authMethods = await loadAuthMethods(capabilityKeys);

  if (protocols.length === 0) {
    throw new DescriptorError("descriptors/protocols/ declares no protocol");
  }
  if (authMethods.length === 0) {
    throw new DescriptorError("descriptors/auth-methods/ declares no auth method");
  }
  for (const capability of capabilities) {
    const claimed =
      protocols.some((protocol) => protocol.capabilities.includes(capability.key)) ||
      authMethods.some((method) => method.capabilities.includes(capability.key));
    if (capability.planned && claimed) {
      throw new DescriptorError(
        `capability \`${capability.key}\` is marked \`planned: true\` but a descriptor already claims it — drop the flag`,
      );
    }
    if (!capability.planned && !claimed) {
      throw new DescriptorError(
        `capability \`${capability.key}\` is declared but no descriptor claims it — claim it or mark it \`planned: true\``,
      );
    }
  }

  const manifest = { schemaVersion: SCHEMA_VERSION, generator: GENERATOR_ID, capabilities, protocols, authMethods };
  return { manifest, capabilities, protocols, authMethods };
}

/** The Rust identifier the generator expects for a wire identifier. */
export function pascalCase(value) {
  return value
    .split(/[-_]/)
    .filter((part) => part.length > 0)
    .map((part) => part[0].toUpperCase() + part.slice(1))
    .join("");
}

export function serializeManifest(manifest) {
  return `${JSON.stringify(manifest, null, 2)}\n`;
}

export function renderGeneratedTs({ capabilities, protocols, authMethods }) {
  const lines = [];
  const push = (line = "") => lines.push(line);

  push("// @generated by scripts/sync-descriptors.mjs from descriptors/**/*.yaml.");
  push("// Do not edit. Edit the YAML and run `pnpm generate:descriptors`.");
  push("");
  push("export type ProtocolId =");
  protocols.forEach((protocol, index) => {
    push(`  | "${protocol.id}"${index === protocols.length - 1 ? ";" : ""}`);
  });
  push("");
  // The descriptor tables call these `…Id`; the wire types that mirror the Rust structs call them
  // `Protocol` and `AuthMethod`, because that is what the Rust enums are named. Both names mean
  // the same union, so the mirror does not have to translate between them.
  push("export type Protocol = ProtocolId;");
  push("");
  push("export type AuthMethodId =");
  authMethods.forEach((method, index) => {
    push(`  | "${method.id}"${index === authMethods.length - 1 ? ";" : ""}`);
  });
  push("");
  push("export type AuthMethod = AuthMethodId;");
  push("");
  push("export type CapabilityKey =");
  capabilities.forEach((capability, index) => {
    push(`  | "${capability.key}"${index === capabilities.length - 1 ? ";" : ""}`);
  });
  push("");
  push(
    `export type CapabilityGroup = ${capabilities
      .map((capability) => capability.group)
      .filter((group, index, all) => all.indexOf(group) === index)
      .map((group) => `"${group}"`)
      .join(" | ")};`,
  );
  push("");
  push(
    `export type ProtocolRuntime = ${[...new Set(protocols.map((protocol) => protocol.runtime))]
      .map((runtime) => `"${runtime}"`)
      .join(" | ")};`,
  );
  push("");
  push(
    `export type SupportLevel = ${[...new Set(protocols.map((protocol) => protocol.supportLevel))]
      .map((level) => `"${level}"`)
      .join(" | ")};`,
  );
  push("");
  push("export interface ProtocolDescriptor {");
  push("  readonly id: ProtocolId;");
  push("  readonly labelKey: string;");
  push("  readonly defaultPort: number | null;");
  push("  readonly runtime: ProtocolRuntime;");
  push("  readonly transport: string;");
  push("  readonly supportLevel: SupportLevel;");
  push("  readonly capabilities: readonly CapabilityKey[];");
  push("}");
  push("");
  push("export interface AuthMethodDescriptor {");
  push("  readonly id: AuthMethodId;");
  push("  readonly labelKey: string;");
  push("  readonly capabilities: readonly CapabilityKey[];");
  push("  /** Empty means every platform. */");
  push("  readonly platforms: readonly string[];");
  push("}");
  push("");
  push("export interface CapabilityDescriptor {");
  push("  readonly key: CapabilityKey;");
  push("  readonly group: CapabilityGroup;");
  push("  readonly labelKey: string;");
  push("}");
  push("");
  push("export const PROTOCOLS: readonly ProtocolDescriptor[] = [");
  for (const protocol of protocols) {
    push("  {");
    push(`    id: "${protocol.id}",`);
    push(`    labelKey: "${protocol.labelKey}",`);
    push(`    defaultPort: ${protocol.defaultPort === null ? "null" : protocol.defaultPort},`);
    push(`    runtime: "${protocol.runtime}",`);
    push(`    transport: "${protocol.transport}",`);
    push(`    supportLevel: "${protocol.supportLevel}",`);
    push(`    capabilities: [${protocol.capabilities.map((capability) => `"${capability}"`).join(", ")}],`);
    push("  },");
  }
  push("];");
  push("");
  push("export const AUTH_METHODS: readonly AuthMethodDescriptor[] = [");
  for (const method of authMethods) {
    push("  {");
    push(`    id: "${method.id}",`);
    push(`    labelKey: "${method.labelKey}",`);
    push(`    capabilities: [${method.capabilities.map((capability) => `"${capability}"`).join(", ")}],`);
    push(`    platforms: [${method.platforms.map((platform) => `"${platform}"`).join(", ")}],`);
    push("  },");
  }
  push("];");
  push("");
  push("export const CAPABILITIES: readonly CapabilityDescriptor[] = [");
  for (const capability of capabilities) {
    push(`  { key: "${capability.key}", group: "${capability.group}", labelKey: "${capability.labelKey}" },`);
  }
  push("];");
  push("");
  push("export const CAPABILITY_GROUP_ORDER: readonly CapabilityGroup[] = [");
  push(
    `  ${[...new Set(capabilities.map((capability) => capability.group))].map((group) => `"${group}"`).join(", ")},`,
  );
  push("];");
  push("");
  push("export function protocolById(id: string): ProtocolDescriptor | undefined {");
  push("  return PROTOCOLS.find((protocol) => protocol.id === id);");
  push("}");
  push("");
  push("export function protocolSupports(id: ProtocolId, capability: CapabilityKey): boolean {");
  push("  return protocolById(id)?.capabilities.includes(capability) ?? false;");
  push("}");
  push("");
  push("export function authMethodById(id: string): AuthMethodDescriptor | undefined {");
  push("  return AUTH_METHODS.find((method) => method.id === id);");
  push("}");
  push("");

  return lines.join("\n");
}

export function generatedOutputs({ manifest, capabilities, protocols, authMethods }) {
  return new Map([
    [MANIFEST_PATH, serializeManifest(manifest)],
    [path.join(GENERATED_TS_DIR, "protocols.ts"), renderGeneratedTs({ capabilities, protocols, authMethods })],
  ]);
}
