/**
 * The shape of a command specification, as far as this interface reads it.
 *
 * The data is a snapshot of the public Fig / Amazon Q corpus (see `NOTICE.txt` beside the generated
 * JSON): names, descriptions, nested subcommands, options, positional arguments, static suggestions
 * and declarative generator argv arrays. Functions and shell strings were removed when the snapshot
 * was generated; anything here can be read without executing third-party code.
 */

export interface SpecSuggestion {
  /** The first name is what is shown; aliases are offered under the prefix they answer. */
  readonly name: readonly string[];
  readonly description?: string;
  /** Not offered in the menu and not taken by the completion key. */
  readonly hidden?: boolean;
  readonly type?: string;
}

export interface SpecGenerator {
  /** Program and argv. The first element is the program; none of them are a shell line. */
  readonly script?: readonly string[];
  /** How the program's stdout is cut into candidates; newline when absent. */
  readonly splitOn?: string;
  /** A file or folder list. This interface answers those from the file panel, not by running one. */
  readonly template?: string | readonly string[];
}

export interface SpecArg {
  readonly name?: readonly string[];
  readonly description?: string;
  readonly isOptional?: boolean;
  readonly isVariadic?: boolean;
  /** The next word is a command name (`sudo`, `xargs`, `time`). */
  readonly isCommand?: boolean;
  /** The next word is a module name for this program (`python -m`). */
  readonly isModule?: string;
  readonly template?: string | readonly string[];
  readonly filterStrategy?: string;
  readonly suggestions?: readonly SpecSuggestion[];
  readonly generators?: readonly SpecGenerator[];
  /** A specification to continue with, by registry name. */
  readonly loadSpec?: string | readonly { readonly name: string }[];
}

export interface SpecOption {
  /** Short and long spellings of one option. */
  readonly names: readonly string[];
  readonly description?: string;
  readonly isRepeatable?: boolean | number;
  readonly args?: readonly SpecArg[];
}

export interface SpecCommand {
  readonly names: readonly string[];
  readonly description?: string;
  readonly args?: readonly SpecArg[];
  readonly options?: readonly SpecOption[];
  readonly subcommands?: readonly SpecCommand[];
  readonly loadSpec?: string;
  readonly additionalSuggestions?: readonly SpecSuggestion[];
}

export type SpecRegistry = Readonly<Record<string, SpecCommand>>;

/** The registry plus the names that are top-level commands rather than nested specification files. */
export interface SpecCatalog {
  readonly registry: SpecRegistry;
  /** Every spelling a top-level command answers to, in a stable order. */
  readonly commandNames: readonly string[];
  /** The canonical spelling by any name, lower-cased. */
  readonly byName: ReadonlyMap<string, SpecCommand>;
}
