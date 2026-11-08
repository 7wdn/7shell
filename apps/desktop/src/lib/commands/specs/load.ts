/**
 * The command specification corpus, read once and kept.
 *
 * The snapshot is a data file rather than a module: nothing from it executes. It is fetched as an
 * asset instead of imported as JSON so the type checker does not have to describe five megabytes of
 * third-party data, and so it arrives on its own rather than inside the first script the window
 * loads.
 */
import specsUrl from "./fig-specs.json?url";

import type { SpecCatalog, SpecCommand, SpecRegistry } from "./types";

let catalog: SpecCatalog | null = null;
let pending: Promise<SpecCatalog> | null = null;

/** The corpus when it has arrived, or `null` while it is still on its way. */
export function specsLoaded(): SpecCatalog | null {
  return catalog;
}

/** Reads the corpus once. A failure is an empty catalog, not an error: completion keeps working. */
export function loadSpecs(): Promise<SpecCatalog> {
  pending ??= fetch(specsUrl)
    .then((response) => {
      if (!response.ok) {
        return emptyCatalog();
      }
      return response.json().then((raw: unknown) => catalogFromRaw(raw));
    })
    .catch(() => emptyCatalog())
    .then((built) => {
      catalog = built;
      return built;
    });
  return pending;
}

/** Lets a test, or a new session, start from the beginning. */
export function resetSpecsForTesting(): void {
  catalog = null;
  pending = null;
}

function emptyCatalog(): SpecCatalog {
  return { registry: {}, commandNames: [], byName: new Map() };
}

/** The registry as it is stored in the snapshot, in the form the engine reads. Exported for tests. */
export function catalogFromRaw(raw: unknown): SpecCatalog {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    return emptyCatalog();
  }
  const registry = raw as SpecRegistry;
  const nested = new Set<string>();
  for (const spec of Object.values(registry)) {
    collectLoadSpecs(spec, nested);
  }

  const byName = new Map<string, SpecCommand>();
  const commandNames = new Set<string>();
  for (const [key, spec] of Object.entries(registry)) {
    const spellings = new Set<string>([key, ...spec.names]);
    for (const spelling of spellings) {
      const lower = spelling.toLowerCase();
      if (!byName.has(lower)) {
        byName.set(lower, spec);
      }
      if (!nested.has(key)) {
        commandNames.add(spelling);
      }
    }
  }

  return {
    registry,
    byName,
    commandNames: [...commandNames].sort((left, right) => left.localeCompare(right)),
  };
}

/** Every specification a `loadSpec` points at, so it is not offered as a command of its own. */
function collectLoadSpecs(spec: SpecCommand, out: Set<string>): void {
  if (typeof spec.loadSpec === "string") {
    out.add(spec.loadSpec);
  }
  for (const arg of spec.args ?? []) {
    collectArgLoadSpecs(arg, out);
  }
  for (const option of spec.options ?? []) {
    for (const arg of option.args ?? []) {
      collectArgLoadSpecs(arg, out);
    }
  }
  for (const subcommand of spec.subcommands ?? []) {
    collectLoadSpecs(subcommand, out);
  }
}

function collectArgLoadSpecs(
  arg: { readonly loadSpec?: string | readonly { readonly name: string }[] },
  out: Set<string>,
): void {
  if (typeof arg.loadSpec === "string") {
    out.add(arg.loadSpec);
  } else if (Array.isArray(arg.loadSpec)) {
    for (const location of arg.loadSpec) {
      if (location !== null && typeof location === "object" && typeof location.name === "string") {
        out.add(location.name);
      }
    }
  }
}
