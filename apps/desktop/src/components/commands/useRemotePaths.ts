/**
 * What is in a directory on the far side, for a line being typed.
 *
 * The reading goes over the SFTP channel the terminal's own file panel already has open: the panel is
 * the terminal's view of the host, and borrowing its connection is what keeps this from opening a
 * second one behind somebody's back. A terminal with no panel (the layout that shows the terminal
 * alone) has nothing readable, and then only absolute paths are answered — they need no starting
 * point, which is the same rule `pathQueryAt` states. Where that channel is is the caller's business:
 * this is handed a way to ask for it and nothing else.
 *
 * Answers are held for a few seconds: typing `/usr/lo` asks about `/usr` on every keystroke, and the
 * host should be asked once. A read that fails is remembered as "nothing here" for the same few
 * seconds rather than reported — a path somebody is halfway through typing is not a place to talk
 * about an unreadable directory, and the next keystroke would ask again anyway.
 */
import { backend } from "@/lib/backend/api";
import type { SftpEntry } from "@/types/session";

const TTL_MS = 5_000;

/** The channel to read over, and where the shell that owns it is. */
export interface RemoteChannel {
  readonly handle: string;
  /** What a relative path is resolved against, or `null` when this side does not know. */
  readonly cwd: string | null;
}

export interface RemotePaths {
  /** Where a relative path starts from, or `null` when nothing is known. */
  readonly base: string | null;
  /** What has been read of a directory, or `null` when nothing has been read yet. */
  entries(directory: string): readonly SftpEntry[] | null;
  /** Reads a directory that has not been read recently, and calls `settle` when the answer lands. */
  warm(directory: string, settle: () => void): void;
}

interface Read {
  readonly at: number;
  readonly entries: readonly SftpEntry[];
}

export function useRemotePaths(channel: () => RemoteChannel | null): RemotePaths {
  /** By channel and directory: two sessions on one host may sit in the same path. */
  const read = new Map<string, Read>();
  /** Who is waiting for each directory that is being read right now. */
  const waiting = new Map<string, (() => void)[]>();

  function entries(directory: string): readonly SftpEntry[] | null {
    const handle = channel()?.handle;
    return handle === undefined ? null : (read.get(`${handle}:${directory}`)?.entries ?? null);
  }

  function warm(directory: string, settle: () => void): void {
    const open = channel();
    if (open === null) {
      return;
    }

    const key = `${open.handle}:${directory}`;
    const known = read.get(key);
    if (known !== undefined && Date.now() - known.at < TTL_MS) {
      return;
    }

    const queued = waiting.get(key);
    if (queued !== undefined) {
      // Somebody is already reading it: this caller waits for that answer instead of asking again.
      queued.push(settle);
      return;
    }
    waiting.set(key, [settle]);

    void backend.files
      .list(open.handle, directory)
      .then((listing) => {
        read.set(key, { at: Date.now(), entries: listing.entries });
      })
      .catch(() => {
        read.set(key, { at: Date.now(), entries: [] });
      })
      .finally(() => {
        const settled = waiting.get(key) ?? [];
        waiting.delete(key);
        for (const done of settled) {
          done();
        }
      });
  }

  return {
    get base() {
      return channel()?.cwd ?? null;
    },
    entries,
    warm,
  };
}
