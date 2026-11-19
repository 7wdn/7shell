/**
 * Hand-written mirror of the Rust wire types for sessions, keys and host keys.
 *
 * `packages/app-tests/contracts.test.ts` compares these field names with their Rust counterparts
 * in both directions, so a rename on one side fails the test suite rather than a release. Note
 * what is *not* here: no password, no passphrase, no key material. The interface is never sent one.
 */

import type { ContainerSettings } from "@/types/container";
import type { AuthMethod, Protocol } from "@/types/generated/protocols";
import type { KubernetesSettings } from "@/types/k8s";

/**
 * One host this machine's OpenSSH configuration names.
 *
 * The alias is what a saved session stores; the resolved values are shown beside it. No identity
 * path is here: the connection reads those in the process that owns them.
 */
export interface SshConfigHost {
  readonly alias: string;
  readonly host: string;
  readonly user?: string;
  readonly port?: number;
  /** Directives this build cannot honour; selecting one would make a session that refuses. */
  readonly unsupported: readonly string[];
  /** The `ProxyJump` value as written, when this alias connects through a jump host. */
  readonly proxyJump?: string;
}

/** What the interface may know about an imported key. */
export interface KeyInfo {
  /** The handle the vault knows the material by. */
  readonly keyRef?: string;
  readonly displayName: string;
  readonly algorithm: string;
  readonly fingerprint: string;
  readonly hasPassphrase: boolean;
}

/**
 * The kinds of key this build can make.
 *
 * Short and lower case — `ed25519`, not `ssh-ed25519` — because these are what the dropdown stores
 * and what the request carries. The *key type* shown beside a key in the list is the OpenSSH name,
 * and it comes back from reading the key itself rather than from this list.
 */
export type KeyAlgorithm = "ed25519" | "ecdsa" | "rsa";

/** The order the dropdown offers them in: the one to pick when in doubt first. */
export const KEY_ALGORITHMS: readonly [KeyAlgorithm, ...KeyAlgorithm[]] = ["ed25519", "ecdsa", "rsa"];

/**
 * A key that was just made, and the half of it that goes on a server.
 *
 * The public line is the one key-shaped thing that crosses this boundary, and it is meant to: a
 * public key is not a secret, it is the thing somebody pastes into `authorized_keys`.
 */
export interface CreatedKey {
  readonly info: KeyInfo;
  readonly publicKey: string;
}

export interface SessionAuth {
  readonly method: AuthMethod;
  readonly key?: KeyInfo;
  readonly hasSecret: boolean;
  /** The host, account and identity come from this machine's ~/.ssh/config; `host` holds the alias. */
  readonly sshConfig?: boolean;
}

export interface TerminalSettings {
  readonly terminalType: string;
  readonly keepaliveSeconds: number;
  /** The encoding the far side prints and expects. */
  readonly encoding: TerminalEncoding;
  /** What the Backspace key sends to this host. */
  readonly backspace: BackspaceMode;
  /**
   * Whether the far side may use the key agent on this machine while the session lasts.
   *
   * Off by default, and off is the safe answer: while it is on, anything running on that server can
   * ask this machine's agent to sign. It is a deliberate lending of the keys.
   */
  readonly forwardAgent: boolean;
  /**
   * Whether this session is written down as it happens.
   *
   * A recording is a plain file on disk holding everything the terminal showed, so it is off unless
   * somebody says so.
   */
  readonly record: boolean;
}

/**
 * What kind of forward a saved forward is.
 *
 * `local` is the `ssh -L` shape: one port, one destination. `dynamic` is `ssh -D`: one port that
 * speaks SOCKS5, and whoever connects says where it wants to go.
 */
export type ForwardMode = "local" | "dynamic";

/** A local port answered from the far side of a connection: the `ssh -L` shape. */
export interface LocalForward {
  readonly id: string;
  /** The address to listen on, `127.0.0.1` unless somebody deliberately says otherwise. */
  readonly bindHost: string;
  /** Zero asks the system for a free port, and the open outcome says which one it got. */
  readonly bindPort: number;
  /** Local or dynamic. A document written before dynamic forwarding existed carries no mode. */
  readonly mode: ForwardMode;
  readonly targetHost: string;
  readonly targetPort: number;
}

/** A saved way to reach a machine. */
export interface Session {
  readonly id: string;
  readonly name: string;
  readonly groupId?: string;
  /**
   * Where the session sits among the sessions of its group, lowest first.
   *
   * Absent is "nobody has placed this one": it is drawn level with order zero, so it stays where
   * it was created relative to the sessions that were never dragged. A reorder writes a number
   * for every session it moves, which is how the arrangement survives a restart.
   */
  readonly order?: number;
  readonly protocol: Protocol;
  readonly host: string;
  readonly port: number;
  readonly user: string;
  readonly auth: SessionAuth;
  readonly terminal: TerminalSettings;
  /** What a remote desktop asks of the system client. Absent on SSH sessions and older exports. */
  readonly desktop?: DesktopSettings;
  /** What a Telnet console expects on the wire. Absent on every other protocol. */
  readonly telnet?: TelnetSettings;
  /** The serial port and its line parameters. Absent on every other protocol. */
  readonly serial?: SerialSettings;
  /** Which shell on this machine a local session starts. Absent on every other protocol. */
  readonly local?: LocalSettings;
  /** Which WSL distribution to start. Absent on every other protocol. */
  readonly wsl?: WslSettings;
  /** Which container runtime and transport this session uses. Absent on every other protocol. */
  readonly container?: ContainerSettings;
  /** Where the kubeconfig comes from and which context to use. Absent on every other protocol. */
  readonly kubernetes?: KubernetesSettings;
  /** The HTTP folder a WebDAV browser opens. Absent on every other protocol. */
  readonly webdav?: WebDavSettings;
  /** Which virtual machine this session stands for, and how to reach its guest. */
  readonly vmware?: VmwareSettings;
  /** Whether an FTP session asks for TLS before login. Absent on every other protocol. */
  readonly ftp?: FtpSettings;
  /** Saved sessions to reach this one through, in order: the first is connected to directly. */
  readonly jump?: readonly string[];
  /** Ports to forward while this session is open. */
  readonly forwards?: readonly LocalForward[];
  readonly updatedAt: number;
}

/** The fields the RDP editor groups under "display" and "mapping". */
export interface DesktopSettings {
  /** Window size in pixels; ignored when `fullscreen` is on. */
  width: number;
  height: number;
  fullscreen: boolean;
  /** Whether the local drives are mapped into the session. */
  redirectDrives: boolean;
}

/** What the Enter key puts on a Telnet wire: the teletype pair, a bare CR, or a bare LF. */
export type EnterMode = "crlf" | "cr" | "lf";

/** What the Backspace key puts on the wire: what an xterm sends, or the older Ctrl+H. */
export type BackspaceMode = "del" | "ctrlH";

/** The encoding a terminal session speaks. */
export type TerminalEncoding = "utf-8" | "gbk" | "gb18030";

/** How a serial frame checks itself. */
export type SerialParity = "none" | "even" | "odd";

/** How many stop bits follow a serial frame. */
export type SerialStopBits = "1" | "2";

/** How a serial link asks the far side to pause. */
export type SerialFlowControl = "none" | "rtsCts" | "xonXoff";

/** What a Telnet console asks of the keyboard. */
export interface TelnetSettings {
  readonly enter: EnterMode;
  readonly backspace: BackspaceMode;
  readonly encoding: TerminalEncoding;
}

/** A serial port and the line parameters it was chosen with. */
export interface SerialSettings {
  /** The device name, for example `COM3` or `/dev/ttyUSB0`. */
  readonly port: string;
  /** 50 through 4,000,000. The driver accepts custom rates inside that range. */
  readonly baudRate: number;
  /** Five through eight. */
  readonly dataBits: number;
  readonly parity: SerialParity;
  readonly stopBits: SerialStopBits;
  readonly flowControl: SerialFlowControl;
  readonly backspace: BackspaceMode;
}

/** Which shell on this machine a local session starts. */
export interface LocalSettings {
  /** An identifier the backend resolves — `pwsh`, `cmd`, `git-bash` — never a path. */
  readonly shell: string;
  /** Extra arguments appended to the shell's own launch arguments. */
  readonly args: string;
}

/** Which WSL distribution a WSL session starts. */
export interface WslSettings {
  /** Empty asks Windows for its default distribution. */
  readonly distro: string;
  readonly args: string;
}

/** The HTTP folder a WebDAV browser opens: the whole address, scheme and folder included. */
export interface WebDavSettings {
  readonly url: string;
}

/** What an FTP session asks of the protocol. */
export interface FtpSettings {
  /** Explicit FTPS: negotiate TLS before the login is sent. */
  readonly secure: boolean;
}

/**
 * What a virtual-machine session remembers.
 *
 * `vmId` is the token a scan answered with, not the `.vmx` path: the host resolves it against its
 * inventory, so the saved session keeps working after a machine is renamed or moved. The SSH and
 * RDP halves are the ports and accounts the workspace uses when it makes a session that reaches
 * the guest; the references name the sessions it made, once it made them.
 */
export interface VmwareSettings {
  readonly vmId: string;
  readonly sshUser?: string;
  readonly sshPort: number;
  readonly rdpUser?: string;
  readonly rdpPort: number;
  readonly sshSession?: string;
  readonly rdpSession?: string;
}

/** One shell this machine can start, as the picker sees it. */
export interface LocalShell {
  readonly id: string;
  readonly label: string;
}

/** One serial port this machine has, as the picker sees it. */
export interface SerialPortInfo {
  readonly path: string;
  readonly description?: string;
}

/** A forward that is up right now. */
export interface OpenTunnel {
  readonly id: string;
  /** The terminal it belongs to; closing the terminal closes the forward. */
  readonly handle: string;
  readonly sessionId: string;
  readonly bindHost: string;
  readonly bindPort: number;
  /** What the listener is for: a dynamic one draws "SOCKS5" where a local one draws an address. */
  readonly mode: ForwardMode;
  readonly targetHost: string;
  readonly targetPort: number;
  readonly connections: number;
}

/** A forward that was asked for and could not be opened. */
export interface TunnelFailure {
  readonly forwardId: string;
  readonly bindHost: string;
  readonly bindPort: number;
  readonly detail: string;
}

export interface SessionGroup {
  readonly id: string;
  readonly name: string;
  readonly parentId?: string;
  readonly order: number;
  /**
   * Whether the group is drawn above the groups it was made beside.
   *
   * Laid over `order` rather than written into it: `order` is the arrangement somebody dragged the
   * list into, and unpinning has to put the group back in it.
   */
  readonly pinned: boolean;
}

export interface KnownHostEntry {
  readonly host: string;
  readonly port: number;
  readonly keyType: string;
  readonly publicKey: string;
  readonly fingerprint: string;
  readonly addedAt: number;
}

/** A host key as presented during a handshake, before anyone has decided to trust it. */
export interface PresentedHostKey {
  readonly host: string;
  readonly port: number;
  readonly keyType: string;
  readonly publicKey: string;
  readonly fingerprint: string;
}

/** What opening a session answered. */
export type OpenOutcome =
  | { readonly kind: "hostKeyUnknown"; readonly key: PresentedHostKey }
  | { readonly kind: "hostKeyMismatch"; readonly key: PresentedHostKey; readonly trusted: KnownHostEntry }
  | {
      readonly kind: "opened";
      readonly handle: string;
      readonly sessionId: string;
      /** The forwards this session brought up. */
      readonly tunnels: readonly OpenTunnel[];
      /** The ones that did not come up, each with its reason. */
      readonly tunnelFailures: readonly TunnelFailure[];
    };

/**
 * What testing a connection answered.
 *
 * `connected` with `authenticated: false` is a transport that has no login — Telnet and a raw TCP
 * console — where reachability is the whole question. The two host key variants mean the same as
 * they do for an open: an unknown key is a question, a changed key is a stop.
 */
export type ConnectionTestOutcome =
  | {
      readonly kind: "connected";
      readonly latencyMs: number;
      readonly authenticated: boolean;
      readonly fingerprint?: string;
    }
  | { readonly kind: "hostKeyUnknown"; readonly key: PresentedHostKey }
  | { readonly kind: "hostKeyMismatch"; readonly key: PresentedHostKey; readonly trusted: KnownHostEntry };

export type VaultState = "missing" | "locked" | "unlocked";

/**
 * Whether this machine can keep the vault's unlock key in the system's own store.
 *
 * `enrolled` is about the open vault: while the application is locked the status is reported as
 * not enrolled, because drawing the unlock screen must not ask the keychain a question that can
 * turn into a system prompt. The settings page, which is where this is acted on, is only reachable
 * with the vault open.
 */
export interface VaultKeychainStatus {
  readonly available: boolean;
  readonly enrolled: boolean;
}

/**
 * One archived vault file that sits beside the live one.
 *
 * The name and never the directory: which archive this is, is the whole of what the interface is
 * told, and the shell joins the two on its own side.
 */
export interface VaultArchive {
  readonly name: string;
  /** When the file was last written, in seconds since the Unix epoch. */
  readonly modifiedAt: number;
  readonly sizeBytes: number;
}

/** A secret travelling inwards. There is no type for one travelling out. */
export interface SecretInput {
  readonly value: string;
}

/** A terminal that is open. */
export interface OpenTerminal {
  readonly handle: string;
  readonly sessionId: string;
}

/** Output from a terminal: bytes, base64-encoded, because a terminal stream is not always text. */
export interface TerminalData {
  readonly handle: string;
  readonly base64: string;
}

export interface TerminalClosed {
  readonly handle: string;
  readonly reason?: string;
}

/** A ZMODEM session on a terminal, as the shell reports it. */
export interface ZmodemEvent {
  readonly handle: string;
  readonly kind:
    | "started"
    | "choose"
    | "receiving"
    | "sending"
    | "sent"
    | "done"
    | "finished"
    | "skipped"
    | "failed"
    | "cancelled";
  readonly name?: string;
  readonly detail?: string;
  /** A staged download the page can save, on a received browser file. */
  readonly token?: string;
}

/** What an entry in a remote directory is. */
export type EntryKind = "directory" | "file" | "symlink" | "other";

export interface SftpEntry {
  readonly name: string;
  /** The full path, so the interface never joins strings and gets it wrong. */
  readonly path: string;
  readonly kind: EntryKind;
  readonly size: number;
  readonly modifiedAt?: number;
  readonly mode?: number;
  /** The owner's name, or the number the server sent when its accounts do not name it. */
  readonly owner?: string;
  readonly group?: string;
  /** The numbers behind those names, for the properties page that shows both. */
  readonly uid?: number;
  readonly gid?: number;
  readonly linkTarget?: string;
}

export interface SftpListing {
  /** The path the server resolved, which is what the breadcrumbs show. */
  readonly path: string;
  readonly entries: readonly SftpEntry[];
}

/** One path a paste copied or moved, and what became of it. */
export interface SftpPasteItem {
  readonly source: string;
  readonly target: string;
  readonly ok: boolean;
  readonly error?: string;
}

/** What pasting a clipboard's paths answered: existing targets to ask about, or the per-item outcome. */
export interface SftpPasteOutcome {
  readonly conflicts: readonly string[];
  readonly results: readonly SftpPasteItem[];
}

/** What opening a file browser answered. */
export type SftpOutcome =
  | { readonly kind: "hostKeyUnknown"; readonly key: PresentedHostKey }
  | { readonly kind: "hostKeyMismatch"; readonly key: PresentedHostKey; readonly trusted: KnownHostEntry }
  | {
      readonly kind: "opened";
      readonly handle: string;
      readonly sessionId: string;
      readonly listing: SftpListing;
      /** Whether this backend offers packed transfers. SFTP does; the others do not. */
      readonly packed: boolean;
    };

export type TransferDirection = "download" | "upload";

export type TransferState =
  | { readonly kind: "running" }
  /**
   * Waiting because somebody asked it to, with its connection and its part of the file intact.
   *
   * Not a kind of stopped: a paused transfer is still going to happen, and the button beside it says
   * "continue" rather than "try again".
   */
  | { readonly kind: "paused" }
  | { readonly kind: "done" }
  | { readonly kind: "failed"; readonly detail: string }
  | { readonly kind: "cancelled" };

export interface TransferInfo {
  readonly id: string;
  readonly direction: TransferDirection;
  /** The file's own name, not a path on either side. */
  readonly name: string;
  readonly remotePath: string;
  /**
   * Which session it is moving bytes for.
   *
   * A transfer outlives the file browser that started it, so a list that is no longer one browser's
   * own has to be able to say whose it is — and what this side knows is sessions, not the handles
   * the backend keeps to itself.
   */
  readonly sessionId: string;
  readonly size: number;
  readonly transferred: number;
  readonly state: TransferState;
  /**
   * Whether the backend holds a local path for this transfer — not what it is.
   *
   * The path was chosen in a native dialog and has never come back this way. What the two entries
   * that open the file or show where it is need is only whether asking is possible at all; the
   * asking itself names the row, and the path is looked up on the other side.
   */
  readonly hasLocalPath: boolean;
  /**
   * Whether the files travelled as one archive, packed on the side they came from.
   *
   * Told apart from the name because the two answer different questions — the name says what moved,
   * this says what the transfer did — and because it decides which directory is read again after an
   * upload: a packed one lands its files *in* the directory it was aimed at.
   */
  readonly packed: boolean;
  /** Whether this backend can hold the transfer where it is. SCP cannot. */
  readonly pausable: boolean;
}

/**
 * Where downloads land, as this side is told about it.
 *
 * Names, never paths: the folder was chosen in a native dialog, so what this side can do is ask for
 * the dialog back, name a row of the remembered list, or ask for the dialog instead. Where the bytes
 * go is looked up on the other side, which is the same rule the transfer rows follow.
 */
export interface DownloadFolderView {
  /** The folder downloads land in, by its own name — or nothing when every download asks. */
  readonly current: string | null;
  /** The folders chosen before, newest first, by their own names. The current one is the first. */
  readonly history: readonly string[];
}

/**
 * What an upload did: sent everything, asked about a name, or nothing at all.
 *
 * An upload is the one transfer that can replace something without being asked to, so a name that
 * is already there comes back as a question instead of as a transfer.
 */
export type UploadOutcome =
  | { readonly kind: "started"; readonly ids: readonly string[] }
  | {
      readonly kind: "conflict";
      /** What stands for this upload while the question is open. */
      readonly token: string;
      /** The names already on the far side, which is what the question is about. */
      readonly names: readonly string[];
      /** Whether "send only what is missing" is a real answer — it is not, for one archive. */
      readonly skippable: boolean;
    }
  | { readonly kind: "cancelled" };

/** The answer to that question. */
export type UploadAnswer = "overwrite" | "skip" | "cancel";

/**
 * What an export of the session library wrote.
 *
 * A count and a file name: the file stays where Rust put it, and the interface is never told where
 * that is.
 */
export interface SessionExportSummary {
  readonly sessions: number;
  readonly groups: number;
  /** How many stored credentials travelled. Zero unless a passphrase sealed them into the file. */
  readonly secrets: number;
  /** Whether a passphrase stands between the file and what is in it. */
  readonly encrypted: boolean;
  readonly fileName: string;
}

/**
 * What an import added.
 *
 * The last two are warnings rather than failures: what they count was imported anyway, and they are
 * the only sign that a session may not connect the way it did where the file came from.
 */
export interface SessionImportSummary {
  readonly sessions: number;
  readonly groups: number;
  readonly secrets: number;
  /** Imported sessions that name a key this device does not hold. */
  readonly missingKeys: number;
  /** Imported sessions that jumped through a session the file did not carry. */
  readonly droppedHops: number;
}

/**
 * What came of asking to import a file.
 *
 * Two answers, and neither is a failure: the library was written, or the file is sealed and the
 * passphrase is the next question. A dismissed file dialog is the absence of this value — nothing
 * happened, so there is nothing to say.
 */
export type SessionImportOutcome =
  | { readonly kind: "imported"; readonly summary: SessionImportSummary }
  | { readonly kind: "needsPassphrase"; readonly fileName: string };
