/**
 * The passphrase an export is remembered under: what it holds, and what it must never touch.
 *
 * The last case is the one worth having. Everything else here is three lines of assignment; the
 * claim that matters is that a passphrase which sealed somebody's logins does not end up in a
 * browser store, where it would outlive the window and sit in a profile folder.
 */
import { beforeEach, describe, expect, it } from "vitest";

import {
  forgetExportPassphrase,
  rememberExportPassphrase,
  rememberedExportPassphrase,
} from "@/lib/sessions/exportPassphrase";

describe("the passphrase an export is remembered under", () => {
  beforeEach(() => {
    forgetExportPassphrase();
    localStorage.clear();
    sessionStorage.clear();
  });

  it("starts with nothing remembered", () => {
    expect(rememberedExportPassphrase()).toBe("");
  });

  it("holds the last one, replacing whatever was there before", () => {
    rememberExportPassphrase("correct horse");

    expect(rememberedExportPassphrase()).toBe("correct horse");

    rememberExportPassphrase("battery staple");

    expect(rememberedExportPassphrase()).toBe("battery staple");
  });

  it("writes it down nowhere", () => {
    rememberExportPassphrase("correct horse");

    expect(localStorage.length).toBe(0);
    expect(sessionStorage.length).toBe(0);
  });

  it("forgets it on request, which is how a test starts from nothing", () => {
    rememberExportPassphrase("correct horse");

    forgetExportPassphrase();

    expect(rememberedExportPassphrase()).toBe("");
  });
});
