import { afterEach, describe, expect, it, vi } from "vitest";

class FakeWebSocket {
  static instances: FakeWebSocket[] = [];
  listeners = new Map<string, Set<(event: { data: string }) => void>>();

  constructor(public url: string) {
    FakeWebSocket.instances.push(this);
  }

  addEventListener(type: string, listener: (event: { data: string }) => void): void {
    const group = this.listeners.get(type) ?? new Set();
    group.add(listener);
    this.listeners.set(type, group);
  }

  send(message: string): void {
    for (const listener of this.listeners.get("message") ?? []) {
      listener({ data: message });
    }
  }

  close(): void {
    for (const listener of this.listeners.get("close") ?? []) {
      listener({ data: "" });
    }
  }
}

afterEach(() => {
  FakeWebSocket.instances = [];
  vi.unstubAllGlobals();
  vi.resetModules();
});

describe("the browser event socket", () => {
  it("hands a listener the same event shape the desktop transport does", async () => {
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const { webListen } = await import("./web");
    const seen: unknown[] = [];
    await webListen<{ data: string }>("terminal:data", (event) => seen.push(event));

    const socket = FakeWebSocket.instances[0];
    expect(socket?.url).toContain("/events?token=");
    socket?.send(JSON.stringify({ event: "terminal:data", payload: { data: "hello" } }));

    expect(seen).toEqual([{ event: "terminal:data", id: 0, payload: { data: "hello" } }]);
  });

  it("stops calling a listener once it unsubscribes", async () => {
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const { webListen } = await import("./web");
    const seen: unknown[] = [];
    const unlisten = await webListen("ai:delta", (event) => seen.push(event));
    unlisten();

    FakeWebSocket.instances[0]?.send(JSON.stringify({ event: "ai:delta", payload: { text: "x" } }));

    expect(seen).toEqual([]);
  });
});
