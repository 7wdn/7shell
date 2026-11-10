/**
 * Kubernetes quantities, in the two units a table can sort: millicores and bytes.
 *
 * The API server spells a CPU as `500m`, `250000u`, `2` or `1e3`, and a memory as `128Mi`, `1G` or a
 * plain byte count. Comparing those as text is how `10m` ends up below `2m`, so everything is parsed
 * once and every column that sorts by an amount sorts by the number rather than by the spelling.
 *
 * Nothing here formats a quantity back into what the server said: a table showing `1.5` cores and
 * `128Mi` is easier to read than `1500m` and `134217728`, and the exact value is one click away in
 * the YAML view.
 */

/** Millicores, the unit `formatCpu` takes and `parseCpu` answers. */
export function parseCpu(quantity: string): number {
  const text = quantity.trim();
  if (text === "") {
    return 0;
  }
  const match = /^([+-]?\d+(?:\.\d+)?)(?:[eE]([+-]?\d+))?([numkMGTPE]?)$/.exec(text);
  if (!match) {
    return 0;
  }
  const value = Number(match[1]) * 10 ** Number(match[2] ?? 0);
  if (!Number.isFinite(value)) {
    return 0;
  }
  switch (match[3]) {
    // A CPU's suffixed forms are all fractions of a core; the trailing `k` and friends are not
    // quantities the API uses for a CPU, but rounding them as cores is closer than dropping them.
    case "n":
      return Math.round(value / 1e6);
    case "u":
      return Math.round(value / 1e3);
    case "m":
      return Math.round(value);
    default:
      return Math.round(value * 1000);
  }
}

/** How many bytes one memory suffix stands for. Binary suffixes are the ones the API uses. */
const MEMORY_UNITS: Readonly<Record<string, number>> = {
  Ki: 1024,
  Mi: 1024 ** 2,
  Gi: 1024 ** 3,
  Ti: 1024 ** 4,
  Pi: 1024 ** 5,
  Ei: 1024 ** 6,
  k: 1e3,
  M: 1e6,
  G: 1e9,
  T: 1e12,
  P: 1e15,
  E: 1e18,
};

/** Bytes, from the spelling the API server sent. */
export function parseMemory(quantity: string): number {
  const text = quantity.trim();
  if (text === "") {
    return 0;
  }
  const match = /^([+-]?\d+(?:\.\d+)?)(?:[eE]([+-]?\d+))?([A-Za-z]*)$/.exec(text);
  if (!match) {
    return 0;
  }
  const value = Number(match[1]) * 10 ** Number(match[2] ?? 0);
  if (!Number.isFinite(value)) {
    return 0;
  }
  const suffix = match[3] ?? "";
  // A bare suffix of `m` on a memory is a milli-byte, which is what `1m` means to the API. Nobody
  // writes it, and treating it as zero would be a lie in the one case it appears.
  const scale = suffix === "" ? 1 : (MEMORY_UNITS[suffix] ?? (suffix === "m" ? 1e-3 : 1));
  return Math.round(value * scale);
}

/** A CPU reading, in the shape a table column shows: `250m`, `1.5`, `2`. */
export function formatCpu(millicores: number): string {
  if (!Number.isFinite(millicores) || millicores < 0) {
    return "—";
  }
  if (millicores < 1000) {
    return `${Math.round(millicores)}m`;
  }
  const cores = millicores / 1000;
  return Number.isInteger(cores) ? String(cores) : cores.toFixed(1);
}

/** A memory reading, in the shape a table column shows: `128Mi`, `1.5Gi`, `512`. */
export function formatMemory(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) {
    return "—";
  }
  const units: readonly (readonly [string, number])[] = [
    ["Pi", 1024 ** 5],
    ["Ti", 1024 ** 4],
    ["Gi", 1024 ** 3],
    ["Mi", 1024 ** 2],
    ["Ki", 1024],
  ];
  for (const [suffix, size] of units) {
    if (bytes >= size) {
      const value = bytes / size;
      return `${Number.isInteger(value) ? value : value.toFixed(1)}${suffix}`;
    }
  }
  return String(Math.round(bytes));
}

/**
 * A used-out-of-total reading.
 *
 * `—` rather than `0%` when there is nothing to divide by: a pod that requests no CPU has no
 * meaningful percentage of it, and `0%` reads as "plenty of headroom" instead of "not asked for".
 */
export function percent(used: number, total: number): string {
  if (!Number.isFinite(used) || !Number.isFinite(total) || total <= 0) {
    return "—";
  }
  return `${Math.round((used / total) * 100)}%`;
}

/**
 * A comparable number for text a column produced, or `null` when the text is not an amount.
 *
 * Columns that already know their amount pass `sortKey` and never reach this; the rest — ages,
 * percentages, ratios like `2/3` — are parsed here so that pressing a heading orders by meaning
 * rather than by character code.
 */
export function numericValueOf(text: string): number | null {
  const trimmed = text.trim();
  if (trimmed === "" || trimmed === "—") {
    return null;
  }
  const ratio = /^(\d+(?:\.\d+)?)\s*\/\s*\d+$/.exec(trimmed);
  if (ratio) {
    return Number(ratio[1]);
  }
  const withSuffix = /^([+-]?\d+(?:\.\d+)?)\s*(%|Ki|Mi|Gi|Ti|Pi|Ei|k|M|G|T|P|E|d|h|m|s)?i?$/.exec(trimmed);
  if (!withSuffix) {
    return null;
  }
  const value = Number(withSuffix[1]);
  if (!Number.isFinite(value)) {
    return null;
  }
  switch (withSuffix[2]) {
    case undefined:
      return value;
    case "%":
      return value;
    case "Ki":
      return value * 1024;
    case "Mi":
      return value * 1024 ** 2;
    case "Gi":
      return value * 1024 ** 3;
    case "Ti":
      return value * 1024 ** 4;
    case "Pi":
      return value * 1024 ** 5;
    case "Ei":
      return value * 1024 ** 6;
    case "k":
      return value * 1e3;
    case "M":
      return value * 1e6;
    case "G":
      return value * 1e9;
    case "T":
      return value * 1e12;
    case "P":
      return value * 1e15;
    case "E":
      return value * 1e18;
    case "d":
      return value * 86_400;
    case "h":
      return value * 3_600;
    case "m":
      return value * 60;
    case "s":
      return value;
    default:
      return value;
  }
}
