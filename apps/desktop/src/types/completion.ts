/**
 * One completion generator, as the interface asks for it and as it answers.
 *
 * A generator is a program declared by a command specification; it runs on the machine the terminal
 * is connected to, not here. The request names an existing terminal and never a session of its own:
 * a generator is a question about a connection that is already open.
 */
export interface CompletionExecuteRequest {
  /** The terminal whose connection runs the generator. */
  readonly handle: string;
  /** A program name, not a shell line. Validated on the Rust side. */
  readonly command: string;
  readonly args: readonly string[];
  /** The directory to run in, when the shell's own directory is known. */
  readonly cwd?: string | null;
  readonly timeoutMs: number;
  readonly maxOutputBytes: number;
}

export interface CompletionExecuteResult {
  readonly exitCode: number | null;
  readonly stdout: string;
  readonly stderr: string;
  readonly truncated: boolean;
  readonly timedOut: boolean;
}
