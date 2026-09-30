/**
 * The single subscription to terminal events, and the fan-out to the panes that draw them.
 *
 * Terminal events are delivered to the whole window: `backend.events.onTerminalData` is one
 * channel for every terminal, and each event names the handle it belongs to. A subscription made
 * per pane would therefore hand the same bytes to every open terminal — every pane would draw
 * every other session's output. What is subscribed exactly once is the pair of event streams; what
 * is registered per handle is a single sink, and bytes for a handle go to that sink alone.
 *
 * Two things cannot be handed over at the instant they arrive, and both are kept here in bounded
 * form rather than being dropped:
 *
 * * output for a handle whose pane has not mounted yet — the server's banner can arrive between
 *   the connection opening and the tab being drawn;
 * * the ending of a terminal whose tab does not exist yet — a connection that dies immediately is
 *   still something the person who opened it has to be told about.
 *
 * The base64 codec lives here too: this is the module that owns the boundary where the wire's text
 * becomes the terminal's bytes, and the reverse.
 */

import { backend } from "@/lib/backend/api";
import type { TerminalClosed, TerminalData } from "@/types/session";

/** Receives the bytes of one handle. Registering a second sink for a handle replaces the first. */
export type TerminalSink = (bytes: Uint8Array) => void;

/** Told when a terminal ends, by handle, so tabs and panes can react. */
export type TerminalEndListener = (handle: string, reason?: string) => void;

/** Output held per handle while no pane claims it. Beyond this the older chunks are dropped. */
const PENDING_LIMIT_BYTES = 256 * 1024;

/** How many endings are remembered, so a terminal nobody has drawn yet is still reported. */
const ENDING_MEMORY = 32;

const sinks = new Map<string, TerminalSink>();
const endListeners = new Set<TerminalEndListener>();
const pending = new Map<string, Uint8Array[]>();
const pendingBytes = new Map<string, number>();
const endings = new Map<string, string | undefined>();

let stopListening: (() => void)[] = [];
let started = false;

/** Subscribes to both event streams, once for the whole application. Safe to call again. */
export async function startTerminalBus(): Promise<void> {
  if (started) {
    return;
  }
  started = true;
  stopListening = await Promise.all([backend.events.onTerminalData(receive), backend.events.onTerminalClosed(finish)]);
}

/** Unsubscribes and forgets everything held. Only for tearing the application down. */
export function stopTerminalBus(): void {
  for (const stop of stopListening) {
    stop();
  }
  stopListening = [];
  started = false;
  sinks.clear();
  endListeners.clear();
  pending.clear();
  pendingBytes.clear();
  endings.clear();
}

/** Claims a handle for one pane, and hands it whatever arrived before it got there. */
export function registerTerminalSink(handle: string, sink: TerminalSink): () => void {
  sinks.set(handle, sink);

  const held = pending.get(handle);
  if (held) {
    pending.delete(handle);
    pendingBytes.delete(handle);
    for (const chunk of held) {
      sink(chunk);
    }
  }

  return () => {
    if (sinks.get(handle) === sink) {
      sinks.delete(handle);
    }
  };
}

/** Registers a listener for terminals ending. The pane drawing the handle is not told twice. */
export function onTerminalEnded(listener: TerminalEndListener): () => void {
  endListeners.add(listener);
  return () => {
    endListeners.delete(listener);
  };
}

/** Whether this handle has already ended, and what the server said, if anyone has asked. */
export function endedTerminal(handle: string): { reason?: string } | undefined {
  return endings.has(handle) ? { reason: endings.get(handle) } : undefined;
}

function receive(payload: TerminalData): void {
  const bytes = decodeBase64(payload.base64);
  const sink = sinks.get(payload.handle);
  if (sink) {
    sink(bytes);
    return;
  }
  hold(payload.handle, bytes);
}

function hold(handle: string, bytes: Uint8Array): void {
  const held = pending.get(handle) ?? [];
  if ((pendingBytes.get(handle) ?? 0) + bytes.length > PENDING_LIMIT_BYTES) {
    // Nothing is drawing this handle, and holding more would only grow: the tail of the output
    // is the part a pane coming late is better off with.
    held.length = 0;
    pendingBytes.set(handle, 0);
  }
  held.push(bytes);
  pending.set(handle, held);
  pendingBytes.set(handle, (pendingBytes.get(handle) ?? 0) + bytes.length);
}

function finish(payload: TerminalClosed): void {
  endings.delete(payload.handle);
  endings.set(payload.handle, payload.reason);
  while (endings.size > ENDING_MEMORY) {
    const oldest = endings.keys().next();
    if (oldest.done) {
      break;
    }
    endings.delete(oldest.value);
  }

  // Over a copy: a listener is allowed to unsubscribe itself while being told.
  for (const listener of Array.from(endListeners)) {
    listener(payload.handle, payload.reason);
  }
}

/** The wire's base64 back to the bytes a terminal renders. Never decoded to text on the way. */
export function decodeBase64(text: string): Uint8Array {
  const binary = atob(text);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

/** The bytes of a keystroke or a paste, as the wire wants them. */
export function encodeBase64(bytes: Uint8Array): string {
  // Chunked: spreading a large array into `fromCharCode` overflows the argument list.
  const CHUNK = 0x8000;
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + CHUNK));
  }
  return btoa(binary);
}
