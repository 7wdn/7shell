/**
 * The browser transport: the same backend contract as the desktop shell, carried over the local
 * service's HTTP endpoint and event socket.
 *
 * The token comes from the URL the launcher printed. It is sent with every call and on the event
 * socket; the service refuses anything else, including the other applications on this machine.
 */

import { noTransportError } from "./errors";

type Unlisten = () => void;
/** The shape the desktop event API hands a listener; wrappers read `event.payload`. */
type WebEvent = { event: string; id: number; payload: unknown };
type Listener = (event: WebEvent) => void;

const token = typeof window === "undefined" ? "" : (new URLSearchParams(window.location.search).get("token") ?? "");

/** The token the launcher put in the page URL, for the file endpoints the browser opens directly. */
export function webTokenValue(): string {
  return token;
}

export async function webInvoke<T>(cmd: string, args: unknown = {}): Promise<T> {
  let response: Response;
  try {
    response = await fetch("/invoke", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({ cmd, args }),
    });
  } catch {
    // A browser with no local service is the same situation the facade already names: there is no
    // transport, not a failed operation.
    throw noTransportError(cmd);
  }
  const body = await response.json().catch(() => null);
  if (!response.ok) {
    throw new Error(`the local service answered ${response.status}`);
  }
  if (body && typeof body === "object" && "code" in body) {
    throw body;
  }
  return body as T;
}

const listeners = new Map<string, Set<Listener>>();
let socket: WebSocket | null = null;
let reconnect: ReturnType<typeof setTimeout> | null = null;

function connect(): void {
  if (typeof window === "undefined" || socket) {
    return;
  }
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  socket = new WebSocket(`${protocol}//${window.location.host}/events?token=${encodeURIComponent(token)}`);
  socket.addEventListener("message", (message) => {
    let event: { event?: string; payload?: unknown };
    try {
      event = JSON.parse(String(message.data));
    } catch {
      return;
    }
    if (!event.event) {
      return;
    }
    for (const listener of listeners.get(event.event) ?? []) {
      listener({ event: event.event, id: 0, payload: event.payload });
    }
  });
  socket.addEventListener("close", () => {
    socket = null;
    if (listeners.size > 0 && reconnect === null) {
      reconnect = setTimeout(() => {
        reconnect = null;
        connect();
      }, 1000);
    }
  });
  socket.addEventListener("error", () => socket?.close());
}

export function webListen<T>(
  event: string,
  listener: (event: { event: string; id: number; payload: T }) => void,
): Promise<Unlisten> {
  let group = listeners.get(event);
  if (!group) {
    group = new Set();
    listeners.set(event, group);
  }
  group.add(listener as Listener);
  connect();
  return Promise.resolve(() => {
    group?.delete(listener as Listener);
    if (group?.size === 0) {
      listeners.delete(event);
    }
  });
}
