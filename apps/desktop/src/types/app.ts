/**
 * Hand-written mirror of `sevenshell_types::AppInfo`.
 *
 * `packages/app-tests/contracts.test.ts` compares the field names here with the Rust
 * struct, and `crates/sevenshell-types/tests/descriptors.rs` pins the values.
 */
export interface AppInfo {
  readonly name: string;
  readonly version: string;
  readonly os: string;
  readonly arch: string;
  readonly debugBuild: boolean;
  readonly protocolCount: number;
  readonly descriptorSchemaVersion: number;
}
