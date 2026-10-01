/**
 * The two clipboard calls the menus make.
 *
 * Copying is a receipt: the menu closes, and the line at the foot of the window is the only sign it
 * worked — so a copy that failed silently would look exactly like one that worked until somebody
 * pasted. Reading goes through the backend rather than through `navigator.clipboard`, because
 * WebView2 answers a script's clipboard read with a permission request nobody is asked to answer and
 * the promise never settles; see `src-tauri/src/clipboard.rs` for the whole of that story.
 *
 * The translation comes from the module rather than from the caller because these are called from
 * menu actions, which are not render functions and have no `t` of their own.
 */
import { useToast } from "@/composables/useToast";
import { i18n } from "@/i18n";
import { describeShellError } from "@/i18n/errors";
import { backend } from "@/lib/backend/api";
import { normalizeShellError } from "@/lib/backend/errors";

function translate(key: string, params?: Record<string, string>): string {
  return params ? i18n.global.t(key, params) : i18n.global.t(key);
}

/** Writes the value, and says so. Returns whether the clipboard took it. */
export async function copyText(value: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(value);
    useToast().toast(i18n.global.t("menu.copied"), "receipt");
    return true;
  } catch {
    useToast().toast(i18n.global.t("menu.copyFailed"), "failure");
    return false;
  }
}

/**
 * The clipboard's text, or null — and a word about it — when there is nothing to paste.
 *
 * Both refusals are said out loud rather than swallowed, and they are said differently because they
 * are different: a platform that cannot read a clipboard at all is a failure with a reason, while a
 * clipboard holding no text is an answer — the one a person gets after copying a picture.
 */
export async function readPastedText(): Promise<string | null> {
  let text: string;
  try {
    text = await backend.clipboard.text();
  } catch (error) {
    // Normalized rather than coerced: whatever reached here is the backend's own envelope — the
    // facade passes it through untouched — and its words are better than anything written here.
    const described = describeShellError(translate, normalizeShellError(error, "clipboard_text"));
    useToast().toast(described.message, "refusal");
    return null;
  }

  if (text === "") {
    useToast().toast(i18n.global.t("menu.pasteEmpty"), "refusal");
    return null;
  }
  return text;
}
