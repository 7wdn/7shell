import type { ShellError } from "@/lib/backend/errors";

/** A translation function, so this module does not depend on how i18n is wired. */
export type Translate = (key: string, params?: Record<string, string>) => string;

export interface DescribedError {
  /** The localized sentence, ready to display. */
  readonly message: string;
  /** The stable code, always shown so a report can quote something searchable. */
  readonly code: string;
  /** What the backend added on top, when it did. */
  readonly detail?: string;
}

/** The parts of an envelope a message is rendered from, without the parts nobody reads. */
export interface ErrorMessage {
  readonly code: string;
  readonly messageKey: string;
  readonly messageParams?: Readonly<Record<string, string>>;
  readonly detail?: string;
}

/**
 * Renders an envelope in the active language.
 *
 * A message key with no translation is a bug that `pnpm check:i18n` is meant to catch,
 * so when it happens the code and the backend detail are shown instead of a raw key.
 */
export function describeShellError(t: Translate, error: ShellError): DescribedError {
  return describeErrorFields(t, error);
}

/**
 * The same rendering, from the fields alone.
 *
 * A batch result carries one host's failure as the parts a person reads rather than as an envelope
 * that went through `normalizeShellError`: there was no rejection to normalize, the failure is data
 * inside a successful answer. Both paths end here so that a code with no message key in this build
 * falls back the same way wherever it is shown.
 */
export function describeErrorFields(t: Translate, error: ErrorMessage): DescribedError {
  const translated = t(error.messageKey, { ...error.messageParams });
  if (translated === error.messageKey) {
    return { message: error.detail ?? error.messageKey, code: error.code };
  }
  return { message: translated, code: error.code, detail: error.detail };
}
