/**
 * The pixels of an open remote desktop, on their way to the canvas that draws them.
 *
 * The Rust side sends one message per processing step: a little-endian `u32` count, then for each
 * changed rectangle `x, y, width, height` as `u16` and its RGBA8 bytes. Frames that arrive before
 * their pane has mounted are kept in order — the first batch is often the whole desktop, and
 * drawing only the newest rectangle would leave everything else black.
 */

export interface DesktopFrame {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  /** RGBA8, row-major, exactly `width * height * 4` bytes, sharing the IPC buffer. */
  readonly pixels: Uint8ClampedArray<ArrayBuffer>;
}

type Sink = (frame: DesktopFrame) => void;

/** How many frames to keep for a pane that has not mounted yet. A full desktop is a few hundred. */
const PENDING_LIMIT = 2048;

const sinks = new Map<string, Sink>();
const waiting = new Map<string, DesktopFrame[]>();

function parse(buffer: ArrayBuffer): DesktopFrame[] {
  if (buffer.byteLength < 4) {
    return [];
  }

  const header = new DataView(buffer);
  const count = header.getUint32(0, true);
  const frames: DesktopFrame[] = [];
  let offset = 4;

  for (let index = 0; index < count; index += 1) {
    if (offset + 8 > buffer.byteLength) {
      break;
    }
    const x = header.getUint16(offset, true);
    const y = header.getUint16(offset + 2, true);
    const width = header.getUint16(offset + 4, true);
    const height = header.getUint16(offset + 6, true);
    const size = width * height * 4;
    if (width === 0 || height === 0 || offset + 8 + size > buffer.byteLength) {
      break;
    }
    frames.push({
      x,
      y,
      width,
      height,
      pixels: new Uint8ClampedArray(buffer, offset + 8, size),
    });
    offset += 8 + size;
  }

  return frames;
}

/** Where a pane's frames go. Returns the unsubscribe. */
export function registerDesktopSink(handle: string, sink: Sink): () => void {
  sinks.set(handle, sink);
  const pending = waiting.get(handle);
  if (pending) {
    waiting.delete(handle);
    for (const frame of pending) {
      sink(frame);
    }
  }

  return () => {
    if (sinks.get(handle) === sink) {
      sinks.delete(handle);
    }
  };
}

/** Delivers one IPC message. Frames with nobody listening are kept, in order. */
export function receiveDesktopFrame(handle: string, buffer: ArrayBuffer): void {
  const frames = parse(buffer);
  if (frames.length === 0) {
    return;
  }

  const sink = sinks.get(handle);
  if (sink) {
    for (const frame of frames) {
      sink(frame);
    }
    return;
  }

  const pending = waiting.get(handle) ?? [];
  pending.push(...frames);
  if (pending.length > PENDING_LIMIT) {
    pending.splice(0, pending.length - PENDING_LIMIT);
  }
  waiting.set(handle, pending);
}

/** Forgets the buffered frames of a desktop that has ended. */
export function forgetDesktopFrames(handle: string): void {
  waiting.delete(handle);
  sinks.delete(handle);
}
