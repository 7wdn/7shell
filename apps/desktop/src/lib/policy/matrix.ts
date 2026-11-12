/**
 * The policy matrix as a draft: rows for the page, records for the file.
 *
 * The settings file keeps three maps keyed by a group id, a host or a path prefix. A map is the
 * right shape there — it is a lookup the resolver performs — and the wrong shape here, where a
 * person adds and removes rows and types the keys. The conversion between the two is the whole of
 * this file, written once so the page and its tests cannot disagree about what "an empty key" or
 * "the same matrix" means.
 */

import { DEFAULT_POLICY_MATRIX, type PolicyMatrix, type PolicyRule } from "@/types/app";

/** One exception row while it is being edited: a key and the four rules beside it. */
export interface PolicyRow {
  readonly key: string;
  readonly rule: PolicyRule;
}

/** A rule copied out of anything shared, so a draft can be changed without touching the store. */
export function copyPolicyRule(rule: PolicyRule): PolicyRule {
  return {
    safe: rule.safe,
    caution: rule.caution,
    destructive: rule.destructive,
    catastrophic: rule.catastrophic,
  };
}

/** The whole matrix copied, maps included: the store and the draft must not share an object. */
export function copyPolicyMatrix(matrix: PolicyMatrix): PolicyMatrix {
  return {
    global: copyPolicyRule(matrix.global ?? DEFAULT_POLICY_MATRIX.global),
    groups: copyRecord(matrix.groups ?? {}),
    hosts: copyRecord(matrix.hosts ?? {}),
    directories: copyRecord(matrix.directories ?? {}),
  };
}

function copyRecord(record: Readonly<Record<string, PolicyRule>>): Record<string, PolicyRule> {
  const out: Record<string, PolicyRule> = {};
  for (const [key, rule] of Object.entries(record)) {
    out[key] = copyPolicyRule(rule);
  }
  return out;
}

/** The rows a record draws, in key order so the page does not shuffle between reads. */
export function policyRows(record: Readonly<Record<string, PolicyRule>>): PolicyRow[] {
  return Object.keys(record)
    .sort((left, right) => left.localeCompare(right))
    .map((key) => {
      const rule = record[key];
      return { key, rule: copyPolicyRule(rule as PolicyRule) };
    });
}

/**
 * The record a set of rows writes.
 *
 * A row whose key is empty is a row somebody added and has not named yet, so it is not written: an
 * entry filed under the empty string would match nothing and clutter the file. A duplicated key
 * resolves to the last row, which is the one nearest where the person was typing.
 */
export function policyRecord(rows: readonly PolicyRow[]): Record<string, PolicyRule> {
  const out: Record<string, PolicyRule> = {};
  for (const row of rows) {
    const key = row.key.trim();
    if (key === "") {
      continue;
    }
    out[key] = copyPolicyRule(row.rule);
  }
  return out;
}

/**
 * Whether two matrices say the same thing.
 *
 * Compared canonical rather than with `===`: the two are built by different code paths, so the
 * object identities and the insertion order of the maps differ even when every rule agrees. The
 * Apply button reads this to stay quiet when a draft has not changed.
 */
export function samePolicyMatrix(left: PolicyMatrix, right: PolicyMatrix): boolean {
  return canonical(left) === canonical(right);
}

function canonical(matrix: PolicyMatrix): string {
  return JSON.stringify({
    global: ruleTuple(matrix.global),
    groups: recordTuples(matrix.groups),
    hosts: recordTuples(matrix.hosts),
    directories: recordTuples(matrix.directories),
  });
}

function recordTuples(record: Readonly<Record<string, PolicyRule>> | undefined): [string, string][] {
  return Object.keys(record ?? {})
    .sort((left, right) => left.localeCompare(right))
    .map((key) => [key, ruleTuple(record?.[key] ?? DEFAULT_POLICY_MATRIX.global)]);
}

function ruleTuple(rule: PolicyRule): string {
  return [rule.safe, rule.caution, rule.destructive, rule.catastrophic].join(":");
}
