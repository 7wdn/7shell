/**
 * Which right-clicks keep the platform's own menu.
 *
 * The answer decides whether somebody can cut and paste in a text field, so both halves of it are
 * worth pinning down: the fields themselves, and the terminal — which owns a textarea that is not a
 * field anybody can use.
 */
import { beforeEach, describe, expect, it } from "vitest";

import { isTextEntry } from "@/lib/menu/editable";

/** Draws the markup and hands back its first element, which is the one a click would land on. */
function element(html: string): Element {
  const host = document.createElement("div");
  host.innerHTML = html;
  document.body.append(host);
  const target = host.firstElementChild;
  if (!target) {
    throw new Error("the markup produced no element");
  }
  return target;
}

beforeEach(() => {
  document.body.innerHTML = "";
});

describe("deciding where the platform's menu stays", () => {
  it("keeps it in a text field", () => {
    expect(isTextEntry(element("<input />"))).toBe(true);
    expect(isTextEntry(element("<textarea></textarea>"))).toBe(true);
  });

  it("keeps it for anything drawn inside a rich text field", () => {
    const field = element('<div contenteditable="true"><span>text</span></div>');
    expect(isTextEntry(field)).toBe(true);
    expect(isTextEntry(field.querySelector("span"))).toBe(true);
  });

  it("takes it away everywhere that is not a field", () => {
    expect(isTextEntry(element("<div>plain</div>"))).toBe(false);
    expect(isTextEntry(document.body)).toBe(false);
    expect(isTextEntry(null)).toBe(false);
  });

  it("ignores a field that has been switched off", () => {
    expect(isTextEntry(element('<div contenteditable="false">text</div>'))).toBe(false);
  });

  it("does not treat the terminal's hidden textarea as a field", () => {
    // xterm keeps a textarea off screen to receive keystrokes. It is a textarea, and it is never a
    // real target — but if it ever became one, the terminal would offer to check its spelling.
    expect(isTextEntry(element('<div class="xterm"><textarea></textarea></div>'))).toBe(false);
  });
});
