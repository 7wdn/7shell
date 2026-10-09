/**
 * The passphrase an export was sealed under, remembered for this run of the interface.
 *
 * In memory and nowhere else. Not the store, which a panel can show; not `localStorage` or
 * `sessionStorage`, which outlive the window and sit in a profile folder; and never on its way to
 * the shell, which is only ever handed the passphrase at the moment a file is written.
 *
 * It exists because the second export of a session is usually the same file for the same person,
 * and having to type the passphrase twice for that is an argument for choosing a shorter one — the
 * kind of pressure a length rule should never create. What is remembered is the passphrase of a
 * write that *succeeded*: a file dialog somebody closed is not a passphrase that worked.
 */

let remembered = "";

/** Remembers the passphrase a file was actually sealed under. */
export function rememberExportPassphrase(passphrase: string): void {
  remembered = passphrase;
}

/** This session's last successful export passphrase, or an empty string when there is none. */
export function rememberedExportPassphrase(): string {
  return remembered;
}

/**
 * Forgets it.
 *
 * Nothing in the application calls this yet: the passphrase is gone when the interface restarts,
 * and there is no shorter life it needs. It is here for tests, whose module state outlives a case.
 */
export function forgetExportPassphrase(): void {
  remembered = "";
}
