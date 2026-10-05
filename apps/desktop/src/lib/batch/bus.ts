/**
 * The single subscription to batch progress, and the fan-out to whoever is drawing it.
 *
 * Batch progress is delivered to the whole window: `backend.events.onBatchEvent` is one channel,
 * and the events carry the session they are about. One subscription for the application is what
 * keeps a second panel — or a panel reopened after a run started — from missing the events that
 * arrived while it was not mounted. The store that owns the rows is the only listener today;
 * keeping the subscription here rather than in the panel is what makes a run survive switching
 * tabs, and what makes the wiring testable without a window.
 *
 * Nothing is buffered on purpose. Unlike a terminal's bytes, progress is only meaningful while the
 * run it belongs to is going, and the run's answer carries every row anyway — a listener that
 * arrives late is told the final state by the call it made, not by a replay.
 */

import { backend } from "@/lib/backend/api";
import type { Unlisten } from "@/lib/backend/api";
import type { BatchProgress } from "@/types/batch";

export type BatchProgressListener = (progress: BatchProgress) => void;

const listeners = new Set<BatchProgressListener>();
let stopListening: Unlisten | null = null;
let started = false;

/** Subscribes to the progress stream, once for the whole application. Safe to call again. */
export async function startBatchBus(): Promise<void> {
  if (started) {
    return;
  }
  started = true;
  stopListening = await backend.events.onBatchEvent(receive);
}

/** Unsubscribes and forgets every listener. Only for tearing the application down. */
export function stopBatchBus(): void {
  stopListening?.();
  stopListening = null;
  started = false;
  listeners.clear();
}

/** Registers a listener, and answers with the function that removes it again. */
export function onBatchProgress(listener: BatchProgressListener): Unlisten {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function receive(progress: BatchProgress): void {
  // Over a copy: a listener is allowed to unsubscribe itself while being told.
  for (const listener of Array.from(listeners)) {
    listener(progress);
  }
}
