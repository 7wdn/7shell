/**
 * Whether the pointer is in a text field.
 *
 * A text field's own menu is the one place the interface cannot do better than the platform: cut,
 * copy, paste, the IME and the spelling tools are all there already, in the person's language. A
 * house menu would only be a worse copy of it. Everywhere else the browser's menu is a browser's
 * menu inside an application that is not a browser, which is why the shell suppresses it.
 *
 * The terminal's hidden textarea is excluded on purpose. It is off screen and never a real target,
 * but it is a textarea, and a terminal that suddenly offered to check its spelling would be absurd.
 */
const TEXT_ENTRY = "input, textarea, [contenteditable]:not([contenteditable='false'])";

export function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) {
    return false;
  }
  if (target.closest(".xterm")) {
    return false;
  }
  return target.closest(TEXT_ENTRY) !== null;
}
