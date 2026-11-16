/**
 * The browser's file doors.
 *
 * The desktop asks the operating system for a path and never tells the interface where it is. A
 * browser has no such dialog: the page reads the file the person picked and posts it to the local
 * service, which answers a token for the staged copy. Downloads go the other way — the service
 * stages the bytes and the page asks the browser to save a token-addressed URL.
 */

import { webTokenValue } from "./web";

export interface StagedUpload {
  readonly token: string;
  readonly name: string;
}

export interface StagedDownload {
  readonly token: string;
  readonly name: string;
}

/** One file picker. A dismissed dialog answers an empty list, not a rejected promise. */
export function pickBrowserFiles(multiple: boolean): Promise<File[]> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.multiple = multiple;
    input.style.display = "none";
    document.body.append(input);

    let settled = false;
    const finish = (files: File[]): void => {
      if (settled) {
        return;
      }
      settled = true;
      window.removeEventListener("focus", onFocus);
      input.remove();
      resolve(files);
    };
    // Chromium has no cancel event on an <input type=file>; the window regaining focus with no
    // selection is what a person closing the dialog looks like.
    const onFocus = (): void => {
      window.setTimeout(() => {
        if (!settled && (input.files?.length ?? 0) === 0) {
          finish([]);
        }
      }, 500);
    };
    window.addEventListener("focus", onFocus);
    input.addEventListener(
      "change",
      () => {
        finish(Array.from(input.files ?? []));
      },
      { once: true },
    );
    input.click();
  });
}

function base64Name(name: string): string {
  const bytes = new TextEncoder().encode(name);
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary);
}

/** Posts one file to the local service and answers the token that stands for it. */
export async function stageBrowserFile(file: File): Promise<StagedUpload> {
  const response = await fetch("/upload", {
    method: "POST",
    headers: {
      authorization: `Bearer ${webTokenValue()}`,
      "x-sshell-name": base64Name(file.name),
    },
    body: file,
  });
  if (!response.ok) {
    throw new Error(`the local service refused the upload (${response.status})`);
  }
  return (await response.json()) as StagedUpload;
}

/** Asks the browser to save a file the service has staged. */
export function saveBrowserFile(token: string, name: string): void {
  const link = document.createElement("a");
  link.href = `/download/${encodeURIComponent(token)}?token=${encodeURIComponent(webTokenValue())}`;
  link.download = name;
  link.style.display = "none";
  document.body.append(link);
  link.click();
  link.remove();
}

/** Saves text the page is already holding — a session export, a backup — under its own name. */
export function saveBrowserText(name: string, text: string): void {
  const blob = new Blob([text], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.style.display = "none";
  document.body.append(link);
  link.click();
  link.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}
