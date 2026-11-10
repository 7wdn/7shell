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

import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { useSessionsStore } from "@/stores/sessions";
import type { ContainerRuntime, ContainerSettings, ContainerTransport } from "@/types/container";
import { PROTOCOLS, protocolById, type AuthMethodId, type Protocol } from "@/types/generated/protocols";
import type { KubeconfigSource, KubeContextInfo, KubernetesSettings } from "@/types/k8s";
import type {
  BackspaceMode,
  DesktopSettings,
  EnterMode,
  ForwardMode,
  FtpSettings,
  KeyInfo,
  LocalForward,
  LocalSettings,
  SecretInput,
  SerialFlowControl,
  SerialParity,
  SerialSettings,
  SerialStopBits,
  Session,
  TelnetSettings,
  TerminalSettings,
  WslSettings,
} from "@/types/session";

/** What `TerminalSettings::default()` uses in Rust: the wire type has no "not set" for these. */
const DEFAULT_TERMINAL: TerminalSettings = {
  terminalType: "xterm-256color",
  keepaliveSeconds: 30,
  forwardAgent: false,
  record: false,
};

/** What `DesktopSettings::default()` uses in Rust: the reference dialog's regular setup. */
const DEFAULT_DESKTOP: DesktopSettings = {
  width: 1280,
  height: 1024,
  fullscreen: false,
  redirectDrives: true,
};

/** What the Rust defaults use for the transports that are not SSH. */
const DEFAULT_TELNET: TelnetSettings = { enter: "crlf", backspace: "del" };
const DEFAULT_SERIAL: SerialSettings = {
  port: "",
  baudRate: 115200,
  dataBits: 8,
  parity: "none",
  stopBits: "1",
  flowControl: "none",
  backspace: "del",
};
const DEFAULT_LOCAL: LocalSettings = { shell: "", args: "" };
const DEFAULT_WSL: WslSettings = { distro: "", args: "" };
const DEFAULT_FTP: FtpSettings = { secure: false };
/** A new container session asks the local Docker engine, which is the one most machines have. */
const DEFAULT_CONTAINER: ContainerSettings = { runtime: "docker", transport: "local" };
/** A new cluster session reads the kubeconfig this machine already has and lets it pick a context. */
const DEFAULT_KUBERNETES: KubernetesSettings = { source: "defaultPath" };

/** The range the serial driver accepts, and the ceiling the backend checks. */
const BAUD_MIN = 50;
const BAUD_MAX = 4_000_000;

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
export type SessionFieldProblem =
  | "name"
  | "host"
  | "user"
  | "port"
  | "key"
  | "serialPort"
  | "serialBaud"
  | "containerSshSession"
  | "kubeconfig";

/** One hop as the list draws it. No session means the saved one it named has been deleted. */
export interface HopEntry {
  readonly id: string;
  readonly session: Session | undefined;
}

function portNumber(text: string): number {
  return Number(text.trim());
}

/** A dimension that stays inside what the system client will accept, with a sane fallback. */
function clampDimension(text: string, minimum: number, maximum: number, fallback: number): number {
  const value = Number(text.trim());
  if (!Number.isFinite(value)) {
    return fallback;
  }
  return Math.min(Math.max(Math.round(value), minimum), maximum);
}

/** Whether the text is a whole number a port may hold: zero only where the system picks one. */
function validPort(text: string, minimum: number): boolean {
  if (text.trim() === "") {
    return false;
  }
  const value = portNumber(text);
  return Number.isInteger(value) && value >= minimum && value <= PORT_MAX;
}

/** A WebDAV address is a folder on an HTTP server, not a bare host name. */
function validWebDavUrl(text: string): boolean {
  try {
    const url = new URL(text.trim());
    // Credentials in the address would be a password in a text field and in the saved session;
    // the form has two fields for them, and the backend refuses the URL as well.
    return (url.protocol === "http:" || url.protocol === "https:") && url.username === "" && url.password === "";
  } catch {
    return false;
  }
}

/** Whether the text is a rate the serial driver accepts; custom rates are real. */
function validBaud(text: string): boolean {
  if (text.trim() === "") {
    return false;
  }
  const value = Number(text.trim());
  return Number.isInteger(value) && value >= BAUD_MIN && value <= BAUD_MAX;
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
  /**
   * Which kind of connection this is.
   *
   * A saved session keeps the protocol it was made with; a new one starts at the first descriptor,
   * which is SSH. Switching it changes the port only when the port is still what the previous
   * protocol would have chosen — a port somebody typed is theirs, not a default.
   */
  const protocol = ref<Protocol>(PROTOCOLS[0]?.id ?? "ssh");
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

  /**
   * The remote desktop's display and mapping settings.
   *
   * Dimensions are held as text while they are being typed, for the same reason ports are: an
   * empty field is not zero, and an input that has to hold a number cannot represent "cleared".
   */
  const desktopWidth = ref(String(DEFAULT_DESKTOP.width));
  const desktopHeight = ref(String(DEFAULT_DESKTOP.height));
  const desktopFullscreen = ref(DEFAULT_DESKTOP.fullscreen);
  const desktopDrives = ref(DEFAULT_DESKTOP.redirectDrives);

  /** The Telnet keyboard translation, which is the whole of what a console can be configured with. */
  const telnetEnter = ref<EnterMode>(DEFAULT_TELNET.enter);
  const telnetBackspace = ref<BackspaceMode>(DEFAULT_TELNET.backspace);

  /**
   * The serial line. The baud rate is text while it is typed, like a port: an empty field is not
   * 115200, and the backend must refuse it rather than receive a number the person never chose.
   */
  const serialPort = ref(DEFAULT_SERIAL.port);
  const serialBaud = ref(String(DEFAULT_SERIAL.baudRate));
  const serialDataBits = ref(DEFAULT_SERIAL.dataBits);
  const serialParity = ref<SerialParity>(DEFAULT_SERIAL.parity);
  const serialStopBits = ref<SerialStopBits>(DEFAULT_SERIAL.stopBits);
  const serialFlowControl = ref<SerialFlowControl>(DEFAULT_SERIAL.flowControl);
  const serialBackspace = ref<BackspaceMode>(DEFAULT_SERIAL.backspace);

  /** Which shell on this machine, and which WSL distribution, when those are the transport. */
  const localShell = ref(DEFAULT_LOCAL.shell);
  const localArgs = ref(DEFAULT_LOCAL.args);
  const wslDistro = ref(DEFAULT_WSL.distro);
  const wslArgs = ref(DEFAULT_WSL.args);

  /** Whether an FTP session requires TLS before the login. */
  const ftpSecure = ref(DEFAULT_FTP.secure);

  /**
   * Which engine a container session drives, and where that engine is.
   *
   * `ssh` names a saved SSH session rather than repeating an address: the credentials, the route
   * and the host key are that session's, so the engine is reached without a second copy of any of
   * them. A namespace is a nerdctl concept only, so it is kept beside the runtime and not inside it.
   */
  const containerRuntime = ref<ContainerRuntime>(DEFAULT_CONTAINER.runtime);
  const containerTransport = ref<ContainerTransport>(DEFAULT_CONTAINER.transport);
  const containerSshSession = ref("");
  const containerNamespace = ref("");

  /**
   * Where a cluster session's kubeconfig comes from, and what it should be read as.
   *
   * The imported document is held as text and never read back: it travels inwards as the session's
   * secret, the same way a password does, and the wire type carries only where to look.
   */
  const kubeSource = ref<KubeconfigSource>(DEFAULT_KUBERNETES.source);
  const kubeContext = ref("");
  const kubeNamespace = ref("");
  const kubeViaSsh = ref("");
  const importedKubeconfig = ref("");

  /** What the last "read contexts" answer held, and how that question is going. */
  const kubeContexts = ref<readonly KubeContextInfo[]>([]);
  const kubeContextsBusy = ref(false);
  const kubeContextsFailure = ref<ShellError | null>(null);

  /** Whether this connection has a host and a port to type. A serial port and a shell do not. */
  const needsEndpoint = computed(
    () =>
      protocol.value === "ssh" ||
      protocol.value === "telnet" ||
      protocol.value === "rawTcp" ||
      protocol.value === "rdp" ||
      protocol.value === "webdav" ||
      protocol.value === "ftp",
  );

  /** Whether this connection has a person to name. Only SSH, RDP, WebDAV and FTP do. */
  const needsUser = computed(
    () =>
      protocol.value === "ssh" || protocol.value === "rdp" || protocol.value === "webdav" || protocol.value === "ftp",
  );

  /** Whether SSH's authentication, route and forwarding sections apply. */
  const isSsh = computed(() => protocol.value === "ssh");

  /** Whether the remote-desktop display section applies. */
  const isRdp = computed(() => protocol.value === "rdp");

  /** Whether the address is a WebDAV folder rather than a host and a port. */
  const isWebDav = computed(() => protocol.value === "webdav");

  /** Whether this is an FTP or FTPS session. */
  const isFtp = computed(() => protocol.value === "ftp");

  /** Whether this session drives a container engine on this machine or over SSH. */
  const isContainer = computed(() => protocol.value === "container");

  /** Whether this session talks to a Kubernetes API server. */
  const isKubernetes = computed(() => protocol.value === "kubernetes");

  /**
   * The saved sessions either new protocol may reach its target through.
   *
   * A reference and not a copy: both protocols run on the far side of an SSH session exactly as a
   * jump host does, and the connection it names brings its credentials and route with it.
   */
  const sshSessions = computed(() => library.sessions.filter((session) => session.protocol === "ssh"));

  /**
   * Whether the session being edited already holds a kubeconfig.
   *
   * The editor never receives the document back, so a stored one is a fact reported by the session
   * and the only way to know that an empty textarea is "keep what is there" rather than "none".
   * It has to be a session that *was* a cluster session: an SSH session's stored password is the
   * same secret slot but not a kubeconfig, and switching that session's kind must ask for one.
   */
  const kubeconfigStored = computed(
    () => editing.value?.protocol === "kubernetes" && (editing.value?.auth.hasSecret ?? false),
  );

  /** The route as the list draws it, in the order it is connected. A missing session stays a row. */
  const hops = computed<HopEntry[]>(() =>
    jumps.value.map((id) => ({ id, session: library.sessions.find((session) => session.id === id) })),
  );

  /** What may be added: never the session being edited, which the backend refuses as a loop. */
  const hopChoices = computed(() =>
    library.sessions.filter((session) => session.id !== editing.value?.id && session.protocol === "ssh"),
  );

  const sessionProblems = computed<Record<SessionFieldProblem, boolean>>(() => ({
    name: name.value.trim() === "",
    host: needsEndpoint.value && (host.value.trim() === "" || (isWebDav.value && !validWebDavUrl(host.value))),
    // A WebDAV folder may be readable without an account; the password field is the only one that
    // has to be answered, and even that can be left for an anonymous server.
    user: needsUser.value && !isWebDav.value && user.value.trim() === "",
    port: needsEndpoint.value && !isWebDav.value && !validPort(port.value, 1),
    key: isSsh.value && method.value === "publicKey" && !attachedKey.value?.keyRef,
    serialPort: protocol.value === "serial" && serialPort.value.trim() === "",
    serialBaud: protocol.value === "serial" && !validBaud(serialBaud.value),
    // An engine on another machine is reached through a saved session; an empty id has no route.
    containerSshSession: isContainer.value && containerTransport.value === "ssh" && containerSshSession.value === "",
    // A new imported session needs its document pasted in; an edited one may already hold it, and
    // an empty textarea there means "keep the stored kubeconfig" rather than "there is none".
    kubeconfig:
      isKubernetes.value &&
      kubeSource.value === "imported" &&
      importedKubeconfig.value.trim() === "" &&
      !kubeconfigStored.value,
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
    host.value = session?.webdav?.url || session?.host || "";
    protocol.value = session?.protocol ?? PROTOCOLS[0]?.id ?? "ssh";
    port.value = String(session?.port ?? protocolById(protocol.value)?.defaultPort ?? "");
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
    const desktop = session?.desktop ?? DEFAULT_DESKTOP;
    desktopWidth.value = String(desktop.width);
    desktopHeight.value = String(desktop.height);
    desktopFullscreen.value = desktop.fullscreen;
    desktopDrives.value = desktop.redirectDrives;
    const telnet = session?.telnet ?? DEFAULT_TELNET;
    telnetEnter.value = telnet.enter;
    telnetBackspace.value = telnet.backspace;
    const serial = session?.serial ?? DEFAULT_SERIAL;
    serialPort.value = serial.port;
    serialBaud.value = String(serial.baudRate);
    serialDataBits.value = serial.dataBits;
    serialParity.value = serial.parity;
    serialStopBits.value = serial.stopBits;
    serialFlowControl.value = serial.flowControl;
    serialBackspace.value = serial.backspace;
    const local = session?.local ?? DEFAULT_LOCAL;
    localShell.value = local.shell;
    localArgs.value = local.args;
    const wsl = session?.wsl ?? DEFAULT_WSL;
    wslDistro.value = wsl.distro;
    wslArgs.value = wsl.args;
    ftpSecure.value = session?.ftp?.secure ?? DEFAULT_FTP.secure;
    // A session saved before these protocols existed carries no block: it starts at the defaults
    // its own kind would use, which is also what a new session starts from.
    const container = session?.container ?? DEFAULT_CONTAINER;
    containerRuntime.value = container.runtime;
    containerTransport.value = container.transport;
    containerSshSession.value = container.sshSession ?? "";
    containerNamespace.value = container.namespace ?? "";
    const kubernetes = session?.kubernetes ?? DEFAULT_KUBERNETES;
    kubeSource.value = kubernetes.source;
    kubeContext.value = kubernetes.context ?? "";
    kubeNamespace.value = kubernetes.namespace ?? "";
    kubeViaSsh.value = kubernetes.viaSsh ?? "";
    importedKubeconfig.value = "";
    // What was listed for the previous kubeconfig is not an answer about this one.
    kubeContexts.value = [];
    kubeContextsBusy.value = false;
    kubeContextsFailure.value = null;
  }

  /**
   * A secret travels only when one was typed: an empty field means "keep the stored one", which is
   * not the same as blank, and there is deliberately no way to say "remove it".
   */
  function secretInput(): SecretInput | undefined {
    // A cluster session's secret is the kubeconfig document itself: the wire type says where to
    // look and never carries the file, so pasted text travels the way a password does and is
    // stored under the same "empty means keep it" rule.
    if (isKubernetes.value) {
      return importedKubeconfig.value.trim() === "" ? undefined : { value: importedKubeconfig.value };
    }
    return secret.value === "" ? undefined : { value: secret.value };
  }

  /**
   * Asks the shell which contexts a kubeconfig holds.
   *
   * For the default path there is nothing to send; for an imported one the pasted document is sent
   * as a secret so the contexts are read from it rather than from whatever this machine's default
   * happens to be. A stored kubeconfig cannot be read back through the editor, so an import with an
   * empty textarea asks nothing instead of reading the wrong file.
   */
  async function readKubeContexts(): Promise<void> {
    if (!isKubernetes.value || kubeContextsBusy.value) {
      return;
    }
    if (kubeSource.value === "imported" && importedKubeconfig.value.trim() === "") {
      return;
    }

    kubeContextsBusy.value = true;
    kubeContextsFailure.value = null;
    try {
      const input = kubeSource.value === "imported" ? { value: importedKubeconfig.value } : undefined;
      kubeContexts.value = await backend.k8s.contexts(input);
    } catch (error) {
      kubeContextsFailure.value = asShellError(error, "k8s_contexts");
    } finally {
      kubeContextsBusy.value = false;
    }
  }

  /** What would be saved. Routes and forwards are part of the session, like every other field. */
  function draft(): Session {
    const existing = editing.value;
    const endpoint = needsEndpoint.value;
    return {
      id: existing?.id ?? crypto.randomUUID(),
      name: name.value.trim(),
      groupId: groupId.value === "" ? undefined : groupId.value,
      protocol: protocol.value,
      host: endpoint ? host.value.trim() : "",
      port: endpoint ? portNumber(port.value) : 0,
      user: needsUser.value ? user.value.trim() : "",
      auth: {
        method: method.value,
        key: attachedKey.value ?? undefined,
        hasSecret: existing?.auth.hasSecret ?? false,
      },
      terminal: { ...terminal.value },
      desktop: {
        width: clampDimension(desktopWidth.value, 640, 3840, DEFAULT_DESKTOP.width),
        height: clampDimension(desktopHeight.value, 480, 2160, DEFAULT_DESKTOP.height),
        fullscreen: desktopFullscreen.value,
        redirectDrives: desktopDrives.value,
      },
      telnet: { enter: telnetEnter.value, backspace: telnetBackspace.value },
      serial: {
        port: serialPort.value.trim(),
        baudRate: Number.isInteger(Number(serialBaud.value)) ? Number(serialBaud.value) : DEFAULT_SERIAL.baudRate,
        dataBits: serialDataBits.value,
        parity: serialParity.value,
        stopBits: serialStopBits.value,
        flowControl: serialFlowControl.value,
        backspace: serialBackspace.value,
      },
      local: { shell: localShell.value, args: localArgs.value.trim() },
      wsl: { distro: wslDistro.value, args: wslArgs.value.trim() },
      webdav: { url: isWebDav.value ? host.value.trim() : "" },
      ftp: { secure: isFtp.value && ftpSecure.value },
      container: isContainer.value
        ? {
            runtime: containerRuntime.value,
            transport: containerTransport.value,
            // A local engine has no remote session, and a namespace is nerdctl's alone: either
            // saved under a transport that ignores it would be a field the next reader has to
            // know to ignore.
            sshSession:
              containerTransport.value === "ssh" && containerSshSession.value !== ""
                ? containerSshSession.value
                : undefined,
            namespace:
              containerRuntime.value === "nerdctl" && containerNamespace.value.trim() !== ""
                ? containerNamespace.value.trim()
                : undefined,
          }
        : undefined,
      kubernetes: isKubernetes.value
        ? {
            source: kubeSource.value,
            // Empty means "whatever the kubeconfig says": both fields are answered on the far side.
            context: kubeContext.value.trim() === "" ? undefined : kubeContext.value.trim(),
            namespace: kubeNamespace.value.trim() === "" ? undefined : kubeNamespace.value.trim(),
            viaSsh: kubeViaSsh.value === "" ? undefined : kubeViaSsh.value,
          }
        : undefined,
      jump: jumps.value.length > 0 ? [...jumps.value] : undefined,
      forwards: forwards.value.length > 0 ? forwards.value.map(forwardFrom) : undefined,
      updatedAt: Math.floor(Date.now() / 1000),
    };
  }

  /**
   * Changes the connection kind.
   *
   * A remote desktop authenticates with a password and has no SSH-only sections; the values those
   * sections hold are kept, not cleared, so switching back and forth does not lose what was typed.
   */
  function setProtocol(next: Protocol): void {
    if (next === protocol.value) {
      return;
    }

    const previousDefault = protocolById(protocol.value)?.defaultPort;
    protocol.value = next;
    if (port.value.trim() === "" || Number(port.value) === previousDefault) {
      port.value = String(protocolById(next)?.defaultPort ?? "");
    }
    if (next !== "ssh") {
      method.value = "password";
      attachedKey.value = null;
    }
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
    protocol,
    method,
    attachedKey,
    secret,
    passphrase,
    keyHasPassphrase,
    jumps,
    forwards,
    terminal,
    desktopWidth,
    desktopHeight,
    desktopFullscreen,
    desktopDrives,
    telnetEnter,
    telnetBackspace,
    serialPort,
    serialBaud,
    serialDataBits,
    serialParity,
    serialStopBits,
    serialFlowControl,
    serialBackspace,
    localShell,
    localArgs,
    wslDistro,
    wslArgs,
    needsEndpoint,
    needsUser,
    isSsh,
    isRdp,
    isWebDav,
    isFtp,
    isContainer,
    isKubernetes,
    ftpSecure,
    containerRuntime,
    containerTransport,
    containerSshSession,
    containerNamespace,
    kubeSource,
    kubeContext,
    kubeNamespace,
    kubeViaSsh,
    importedKubeconfig,
    kubeContexts,
    kubeContextsBusy,
    kubeContextsFailure,
    kubeconfigStored,
    sshSessions,
    readKubeContexts,
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
    setProtocol,
    addHop,
    removeHopAt,
    moveHop,
    addForward,
    removeForward,
  };
});
