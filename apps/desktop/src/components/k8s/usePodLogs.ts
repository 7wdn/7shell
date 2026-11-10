/**
 * The pod log stream, as a ref the drawer watches.
 *
 * A stream is opened by id and every line arrives on one application-wide channel, so this composes
 * the two: it keeps its own id, ignores anything addressed to another stream, and stops the stream
 * when it is asked to. The buffer is the bounded one from `logs.ts`.
 *
 * `paused` holds the *view* rather than the stream: lines keep arriving and the buffer keeps growing
 * while somebody reads back through it, which is what "pause" means for a log that is still being
 * written. Scrolling is the drawer's business — it owns the element.
 */
import { ref, type Ref } from "vue";

import { appendLogLines, type LogLine } from "@/components/k8s/logs";
import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";

export interface PodLogOptions {
  readonly container: string;
  readonly tail: number;
  readonly timestamps: boolean;
  readonly previous: boolean;
}

export interface PodLogs {
  readonly lines: Ref<LogLine[]>;
  /** Whether new lines are still arriving. */
  readonly streaming: Ref<boolean>;
  /** The stream is over, whichever way it ended. */
  readonly ended: Ref<boolean>;
  /** What the backend said when it ended, when it said anything. */
  readonly endedWith: Ref<string>;
  /** A stream that could not be opened at all. */
  readonly failure: Ref<ShellError | null>;
  /** Whether the view is held where it is rather than following the newest line. */
  readonly paused: Ref<boolean>;
  start: (connId: string, namespace: string, pod: string, options: PodLogOptions) => Promise<void>;
  stop: () => void;
  clear: () => void;
  /** Stops the stream and detaches the listeners, for a drawer that is going away. */
  close: () => void;
}

export function usePodLogs(): PodLogs {
  const lines = ref<LogLine[]>([]);
  const streaming = ref(false);
  const ended = ref(false);
  const endedWith = ref("");
  const failure = ref<ShellError | null>(null);
  const paused = ref(false);

  let streamId: string | null = null;
  let generation = 0;
  let listeners: Promise<void> | null = null;
  let stopLine: (() => void) | null = null;
  let stopEnd: (() => void) | null = null;

  /** Registers the two listeners once, whichever stream is current. */
  async function listen(): Promise<void> {
    if (listeners !== null) {
      await listeners;
      return;
    }
    listeners = (async () => {
      stopLine = await backend.k8s.onStreamLine((payload) => {
        if (payload.streamId !== streamId) {
          return;
        }
        lines.value = appendLogLines(lines.value, [payload.line]);
      });
      stopEnd = await backend.k8s.onStreamEnd((payload) => {
        if (payload.streamId !== streamId) {
          return;
        }
        streaming.value = false;
        ended.value = true;
        endedWith.value = payload.error ?? "";
        streamId = null;
      });
    })();
    await listeners;
  }

  async function start(connId: string, namespace: string, pod: string, options: PodLogOptions): Promise<void> {
    stop();
    const mine = ++generation;
    lines.value = [];
    failure.value = null;
    ended.value = false;
    endedWith.value = "";
    paused.value = false;
    streaming.value = true;

    await listen();
    if (mine !== generation) {
      return;
    }

    try {
      const id = await backend.k8s.logsStart(connId, {
        namespace,
        pod,
        container: options.container,
        tailLines: options.tail,
        timestamps: options.timestamps,
        previous: options.previous,
      });
      // A second start, or a close, may have happened while the backend was opening this one.
      if (mine !== generation) {
        await backend.k8s.streamStop(id).catch(() => undefined);
        return;
      }
      streamId = id;
    } catch (error) {
      if (mine !== generation) {
        return;
      }
      streaming.value = false;
      failure.value = asShellError(error, "k8s_logs_start");
    }
  }

  function stop(): void {
    generation += 1;
    const id = streamId;
    streamId = null;
    streaming.value = false;
    if (id !== null) {
      void backend.k8s.streamStop(id).catch(() => undefined);
    }
  }

  function clear(): void {
    lines.value = [];
  }

  /** Detaches the listeners. Called by the drawer when it goes away for good. */
  function close(): void {
    stop();
    stopLine?.();
    stopEnd?.();
    stopLine = null;
    stopEnd = null;
    listeners = null;
  }

  return { lines, streaming, ended, endedWith, failure, paused, start, stop, clear, close };
}
