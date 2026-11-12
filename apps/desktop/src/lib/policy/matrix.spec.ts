import { describe, expect, it } from "vitest";

import { DEFAULT_POLICY_MATRIX } from "@/types/app";
import { copyPolicyMatrix } from "./matrix";

describe("a policy matrix read from the shell", () => {
  it("fills the maps the file omits, rather than losing the whole settings read", () => {
    // The Rust setting skips empty maps on the wire. A page that called Object.entries on the
    // missing value threw before locale, theme and every other setting had been applied, which is
    // how a fresh install ended up back in the system language.
    const partial = {
      global: { safe: "allow", caution: "ask", destructive: "deny", catastrophic: "deny" },
    } as const;

    const copied = copyPolicyMatrix(partial as never);

    expect(copied.global).toEqual(partial.global);
    expect(copied.groups).toEqual({});
    expect(copied.hosts).toEqual({});
    expect(copied.directories).toEqual({});
  });

  it("keeps the built-in default when even the global rule is absent", () => {
    const copied = copyPolicyMatrix({} as never);
    expect(copied).toEqual(DEFAULT_POLICY_MATRIX);
  });
});
