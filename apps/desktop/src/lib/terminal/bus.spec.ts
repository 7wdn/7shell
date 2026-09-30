import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({ onTerminalData: vi.fn(), onTerminalClosed: vi.fn() }));

import { onTerminalClosed, onTerminalData } from "@/lib/backend/tauri";
import {
  decodeBase64,
  encodeBase64,
  endedTerminal,
  onTerminalEnded,
  registerTerminalSink,
  startTerminalBus,
  stopTerminalBus,
} from "@/lib/terminal/bus";
import type { TerminalClosed, TerminalData } from "@/types/session";

const onTerminalDataMock = vi.mocked(onTerminalData);
const onTerminalClosedMock = vi.mocked(onTerminalClosed);

const encoder = new TextEncoder();

function dataListener(): (payload: TerminalData) => void {
  const call = onTerminalDataMock.mock.calls[0];
  if (!call) {
    throw new Error("the bus did not subscribe to terminal output");
  }
  return call[0];
}

function closedListener(): (payload: TerminalClosed) => void {
  const call = onTerminalClosedMock.mock.calls[0];
  if (!call) {
    throw new Error("the bus did not subscribe to terminals ending");
  }
  return call[0];
}

function send(handle: string, text: string): void {
  dataListener()({ handle, base64: encodeBase64(encoder.encode(text)) });
}

describe("terminal bus", () => {
  beforeEach(() => {
    stopTerminalBus();
    onTerminalDataMock.mockReset().mockResolvedValue(() => {});
    onTerminalClosedMock.mockReset().mockResolvedValue(() => {});
  });

  it("subscribes once for the whole application, however often it is started", async () => {
    await startTerminalBus();
    await startTerminalBus();

    expect(onTerminalDataMock).toHaveBeenCalledTimes(1);
    expect(onTerminalClosedMock).toHaveBeenCalledTimes(1);
  });

  it("delivers a handle's data only to the pane registered for it", async () => {
    await startTerminalBus();
    const first: number[] = [];
    const second: number[] = [];
    registerTerminalSink("handle-1", (bytes) => first.push(...bytes));
    registerTerminalSink("handle-2", (bytes) => second.push(...bytes));

    send("handle-1", "ls");

    expect(first).toEqual([108, 115]);
    expect(second).toEqual([]);
  });

  it("hands a pane the output that arrived before it registered", async () => {
    await startTerminalBus();
    send("handle-1", "Welcome");

    const received: number[] = [];
    registerTerminalSink("handle-1", (bytes) => received.push(...bytes));

    expect(new TextDecoder().decode(Uint8Array.from(received))).toBe("Welcome");
  });

  it("keeps a character split across two events intact, which is why bytes travel", async () => {
    await startTerminalBus();
    const received: Uint8Array[] = [];
    registerTerminalSink("handle-1", (bytes) => received.push(bytes));

    const whole = encoder.encode("中");
    dataListener()({ handle: "handle-1", base64: encodeBase64(whole.subarray(0, 1)) });
    dataListener()({ handle: "handle-1", base64: encodeBase64(whole.subarray(1)) });

    const head = received[0];
    const tail = received[1];
    if (!head || !tail) {
      throw new Error("both halves of the character should have been delivered");
    }
    expect(received).toHaveLength(2);
    expect(new TextDecoder().decode(new Uint8Array([...head, ...tail]))).toBe("中");
  });

  it("tells every listener about an ending and remembers the reason", async () => {
    await startTerminalBus();
    const seen: [string, string | undefined][] = [];
    onTerminalEnded((handle, reason) => seen.push([handle, reason]));

    closedListener()({ handle: "handle-1", reason: "the server closed the connection" });

    expect(seen).toEqual([["handle-1", "the server closed the connection"]]);
    expect(endedTerminal("handle-1")).toEqual({ reason: "the server closed the connection" });
    expect(endedTerminal("handle-2")).toBeUndefined();
  });

  it("unsubscribes when it is stopped", async () => {
    const stopData = vi.fn();
    const stopClosed = vi.fn();
    onTerminalDataMock.mockResolvedValue(stopData);
    onTerminalClosedMock.mockResolvedValue(stopClosed);
    await startTerminalBus();

    stopTerminalBus();

    expect(stopData).toHaveBeenCalled();
    expect(stopClosed).toHaveBeenCalled();
  });

  it("round-trips bytes through the base64 the wire carries", () => {
    const bytes = new Uint8Array([0, 1, 127, 128, 200, 255]);

    expect(decodeBase64(encodeBase64(bytes))).toEqual(bytes);
  });
});
