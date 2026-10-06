/**
 * The session editor's draft, and the rules that decide whether it can be saved.
 *
 * It lives outside the dialog because a route and its forwards are not presentation: a hop list
 * that dropped a deleted session on save, or a forward with an empty target, would reach the
 * backend as a connection that cannot work. What the dialog owns is which message a problem gets
 * translated to — the store reports codes, because a store has no translation context.
 *
 * Ports are held as text while they are being typed: an input that has to hold a number cannot
 * represent "the person cleared it", and an empty target port is not the same as port zero.
 */

import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { useSessionsStore } from "@/stores/sessions";
import { PROTOCOLS, type AuthMethodId } from "@/types/generated/protocols";
import type { ForwardMode, KeyInfo, LocalForward, SecretInput, Session, TerminalSettings } from "@/types/session";

/** What `TerminalSettings::default()` uses in Rust: the wire type has no "not set" for these. */
const DEFAULT_TERMINAL: TerminalSettings = {
  terminalType: "xterm-256color",
  keepaliveSeconds: 30,
  forwardAgent: false,
  record: false,
};

/** Bounds of a TCP port, and the one case where zero is a real answer: asking for a free one. */
const PORT_MAX = 65535;

/** A forward whose ports are still what the person typed. */
export interface ForwardDraft {
  readonly id: string;
  /** Local or dynamic, which decides whether the destination fields are asked for at all. */
  mode: ForwardMode;
  bindHost: string;
  bindPort: string;
  targetHost: string;
  targetPort: string;
}

/** What is wrong with one forward row, named by the field it belongs to. */
export type ForwardProblem = "bindHost" | "bindPort" | "targetHost" | "targetPort";

/** What is wrong with the session itself, named by the field it belongs to. */
export type SessionFieldProblem = "name" | "host" | "user" | "port" | "key";

/** One hop as the list draws it. No session means the saved one it named has been deleted. */
export interface HopEntry {
  readonly id: string;
  readonly session: Session | undefined;
}

function portNumber(text: string): number {
  return Number(text.trim());
}

/** Whether the text is a whole number a port may hold: zero only where the system picks one. */
function validPort(text: string, minimum: number): boolean {
  if (text.trim() === "") {
    return false;
  }
  const value = portNumber(text);
  return Number.isInteger(value) && value >= minimum && value <= PORT_MAX;
}

/** A row that is only waiting for a target: the bind side starts at what almost every forward uses. */
function newForwardDraft(): ForwardDraft {
  return {
    id: crypto.randomUUID(),
    mode: "local",
    bindHost: "127.0.0.1",
    bindPort: "0",
    targetHost: "",
    targetPort: "",
  };
}

function draftFrom(forward: LocalForward): ForwardDraft {
  return {
    id: forward.id,
    mode: forward.mode,
    bindHost: forward.bindHost,
    bindPort: String(forward.bindPort),
    targetHost: forward.targetHost,
    targetPort: String(forward.targetPort),
  };
}

function forwardFrom(row: ForwardDraft): LocalForward {
  const dynamic = row.mode === "dynamic";
  return {
    id: row.id,
    mode: row.mode,
    bindHost: row.bindHost.trim(),
    bindPort: portNumber(row.bindPort),
    // Empty for a dynamic forward, which has no destination of its own. Whatever was typed before
    // switching the kind is not carried either: a destination saved under a kind that ignores it is
    // a field the next reader would have to know to ignore.
    targetHost: dynamic ? "" : row.targetHost.trim(),
    targetPort: dynamic ? 0 : portNumber(row.targetPort),
  };
}

export const useSessionEditorStore = defineStore("sessionEditor", () => {
  const library = useSessionsStore();

  /**
   * Whether the editor is on screen.
   *
   * It lives here, beside the draft, because the editor is opened from more than one place — the
   * sidebar's own button, the right-click menu on a row, the shortcuts on the home page — and one
   * of those has to work while the sidebar is put away. The dialog itself is drawn by the frame for
   * the same reason: one owned by the sidebar would be gone exactly when the sidebar is.
   */
  const open = ref(false);

  /** The session being edited, which is where the protocol, the terminal settings and the id
   *  come from; null means a new one, and only then is a fresh id made. */
  const editing = ref<Session | null>(null);

  const name = ref("");
  const host = ref("");
  const port = ref("");
  const user = ref("");
  const groupId = ref("");
  const method = ref<AuthMethodId>("password");
  const attachedKey = ref<KeyInfo | null>(null);
  const secret = ref("");
  const passphrase = ref("");
  const keyHasPassphrase = ref(false);
  const jumps = ref<string[]>([]);
  const forwards = ref<ForwardDraft[]>([]);

  /**
   * The session's terminal settings, one of which is a choice: whether the far side may use this
   * machine's key agent while the session lasts. The other two are the defaults Rust uses.
   */
  const terminal = ref<TerminalSettings>({ ...DEFAULT_TERMINAL });

  /** The route as the list draws it, in the order it is connected. A missing session stays a row. */
  const hops = computed<HopEntry[]>(() =>
    jumps.value.map((id) => ({ id, session: library.sessions.find((session) => session.id === id) })),
  );

  /** What may be added: never the session being edited, which the backend refuses as a loop. */
  const hopChoices = computed(() => library.sessions.filter((session) => session.id !== editing.value?.id));

  const sessionProblems = computed<Record<SessionFieldProblem, boolean>>(() => ({
    name: name.value.trim() === "",
    host: host.value.trim() === "",
    user: user.value.trim() === "",
    port: !validPort(port.value, 1),
    key: method.value === "publicKey" && !attachedKey.value?.keyRef,
  }));

  const forwardProblems = computed<Record<string, readonly ForwardProblem[]>>(() => {
    const found: Record<string, readonly ForwardProblem[]> = {};
    for (const row of forwards.value) {
      const problems: ForwardProblem[] = [];
      if (row.bindHost.trim() === "") {
        problems.push("bindHost");
      }
      if (!validPort(row.bindPort, 0)) {
        problems.push("bindPort");
      }
      if (row.targetHost.trim() === "") {
        problems.push("targetHost");
      }
      if (!validPort(row.targetPort, 1)) {
        problems.push("targetPort");
      }
      found[row.id] = problems;
    }
    return found;
  });

  const hasProblems = computed(
    () =>
      Object.values(sessionProblems.value).some(Boolean) ||
      forwards.value.some((row) => (forwardProblems.value[row.id]?.length ?? 0) > 0),
  );

  /**
   * Opens the editor on a new session.
   *
   * The draft is not filled here: the dialog fills it from `editing` as it comes up, so opening and
   * filling happen in one order no matter which of the callers asked for it.
   */
  function startNew(): void {
    editing.value = null;
    open.value = true;
  }

  /** Opens the editor on a saved session. */
  function startEdit(session: Session): void {
    editing.value = session;
    open.value = true;
  }

  /** What the dialog says when it goes away, whether it saved or was dismissed. */
  function setOpen(value: boolean): void {
    open.value = value;
  }

  /** Starts a draft from a saved session, or from nothing when a new one is being made. */
  function fill(session: Session | null): void {
    editing.value = session;
    name.value = session?.name ?? "";
    host.value = session?.host ?? "";
    port.value = String(session?.port ?? PROTOCOLS[0]?.defaultPort ?? "");
    user.value = session?.user ?? "";
    groupId.value = session?.groupId ?? "";
    method.value = session?.auth.method ?? "password";
    attachedKey.value = session?.auth.key ?? null;
    secret.value = "";
    passphrase.value = "";
    keyHasPassphrase.value = false;
    jumps.value = [...(session?.jump ?? [])];
    forwards.value = (session?.forwards ?? []).map(draftFrom);
    terminal.value = { ...(session?.terminal ?? DEFAULT_TERMINAL) };
  }

  /**
   * A secret travels only when one was typed: an empty field means "keep the stored one", which is
   * not the same as blank, and there is deliberately no way to say "remove it".
   */
  function secretInput(): SecretInput | undefined {
    return secret.value === "" ? undefined : { value: secret.value };
  }

  /** What would be saved. Routes and forwards are part of the session, like every other field. */
  function draft(): Session {
    const existing = editing.value;
    return {
      id: existing?.id ?? crypto.randomUUID(),
      name: name.value.trim(),
      groupId: groupId.value === "" ? undefined : groupId.value,
      protocol: existing?.protocol ?? PROTOCOLS[0]?.id ?? "ssh",
      host: host.value.trim(),
      port: portNumber(port.value),
      user: user.value.trim(),
      auth: {
        method: method.value,
        key: attachedKey.value ?? undefined,
        hasSecret: existing?.auth.hasSecret ?? false,
      },
      terminal: { ...terminal.value },
      jump: jumps.value.length > 0 ? [...jumps.value] : undefined,
      forwards: forwards.value.length > 0 ? forwards.value.map(forwardFrom) : undefined,
      updatedAt: Math.floor(Date.now() / 1000),
    };
  }

  function addHop(id: string): void {
    if (id === "" || id === editing.value?.id) {
      return;
    }
    jumps.value.push(id);
  }

  function removeHopAt(index: number): void {
    if (index < 0 || index >= jumps.value.length) {
      return;
    }
    jumps.value.splice(index, 1);
  }

  /** Moves one hop one place up or down. A move off either end is not a move. */
  function moveHop(index: number, delta: -1 | 1): void {
    const target = index + delta;
    if (index < 0 || index >= jumps.value.length || target < 0 || target >= jumps.value.length) {
      return;
    }
    const moved = jumps.value.splice(index, 1)[0];
    if (moved !== undefined) {
      jumps.value.splice(target, 0, moved);
    }
  }

  function addForward(): void {
    forwards.value.push(newForwardDraft());
  }

  function removeForward(id: string): void {
    forwards.value = forwards.value.filter((row) => row.id !== id);
  }

  return {
    open,
    editing,
    name,
    host,
    port,
    user,
    groupId,
    method,
    attachedKey,
    secret,
    passphrase,
    keyHasPassphrase,
    jumps,
    forwards,
    terminal,
    hops,
    hopChoices,
    sessionProblems,
    forwardProblems,
    hasProblems,
    startNew,
    startEdit,
    setOpen,
    fill,
    secretInput,
    draft,
    addHop,
    removeHopAt,
    moveHop,
    addForward,
    removeForward,
  };
});
