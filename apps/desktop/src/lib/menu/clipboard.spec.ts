/**
 * The two clipboard calls, and what they say when they cannot do their job.
 *
 * This is the only place the real module runs: every menu spec replaces it, because what a menu entry
 * does with the clipboard is not what a menu spec is about. What matters here is the part a person
 * meets — a copy that failed and a paste with nothing to paste are both silent without these lines,
 * and silence is indistinguishable from a broken menu.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useToast } from "@/composables/useToast";
import { copyText, readPastedText } from "@/lib/menu/clipboard";

vi.mock("@/lib/backend/api", () => ({
  backend: { clipboard: { text: vi.fn(async () => "from the clipboard") } },
}));

import { backend } from "@/lib/backend/api";

const text = vi.mocked(backend.clipboard.text);

/** The envelope a platform that cannot read a clipboard answers with. */
const unsupported = {
  version: 1,
  code: "APP-UNSUPPORTED",
  messageKey: "errors.app.unsupported",
  messageParams: { what: "pasting text from the clipboard on this platform" },
  origin: { subsystem: "app" },
  outcome: "rejected",
};

beforeEach(() => {
  text.mockReset().mockResolvedValue("from the clipboard");
  vi.stubGlobal("navigator", { clipboard: { writeText: vi.fn(async () => {}) } });
});

describe("copying", () => {
  it("writes the value and says so, because the menu closing is the only other sign", async () => {
    const copied = await copyText("/etc/nginx/nginx.conf");

    expect(navigator.clipboard.writeText).toHaveBeenCalledWith("/etc/nginx/nginx.conf");
    expect(copied).toBe(true);
    expect(useToast().message.value).toBe("Copied");
    expect(useToast().tier.value).toBe("receipt");
  });

  it("says so when the clipboard refuses the write", async () => {
    vi.stubGlobal("navigator", {
      clipboard: {
        writeText: vi.fn(async () => {
          throw new Error("no");
        }),
      },
    });

    const copied = await copyText("anything");

    expect(copied).toBe(false);
    expect(useToast().message.value).toBe("The clipboard could not be written to.");
    expect(useToast().tier.value).toBe("failure");
  });
});

describe("pasting", () => {
  it("answers with the clipboard's text", async () => {
    expect(await readPastedText()).toBe("from the clipboard");
  });

  it("says why when the platform cannot read a clipboard at all", async () => {
    text.mockRejectedValue(unsupported);

    expect(await readPastedText()).toBeNull();
    // The envelope's own words, in the active language, rather than a sentence written here.
    expect(useToast().message.value).toContain("pasting text from the clipboard on this platform");
    expect(useToast().tier.value).toBe("refusal");
  });

  it("says so when the clipboard holds no text", async () => {
    text.mockResolvedValue("");

    expect(await readPastedText()).toBeNull();
    expect(useToast().message.value).toBe("The clipboard holds no text.");
    expect(useToast().tier.value).toBe("refusal");
  });
});
