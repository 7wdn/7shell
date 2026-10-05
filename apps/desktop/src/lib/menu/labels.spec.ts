/**
 * Every label a menu can show, in every language.
 *
 * `scripts/check-i18n.mjs` reads literal `t("...")` calls out of the sources, and a menu built from
 * data carries keys no call site ever writes down: a missing translation would reach the window as a
 * raw `menu.copyCommand`, which is exactly the kind of thing nobody notices until somebody does.
 *
 * So every menu this application can build is built here — from the smallest input it accepts — and
 * every key in it is asked for in both languages. A menu added without its words fails this file
 * rather than a user's window.
 */
import { describe, expect, it } from "vitest";

import { i18n } from "@/i18n";
import { MENU_TITLES } from "@/lib/menu/titles";
import { fileAreaMenu, fileEntryMenu } from "./files";
import { connectionMenu, environmentMenu, listenerMenu, processDetailMenu, processRowMenu } from "./inspect";
import { diskMenu, monitorProcessMenu } from "./monitor";
import { sessionMenu, groupMenu, tunnelMenu } from "./sessions";
import { terminalMenu } from "./terminal";
import { transferMenu } from "./transfers";
import type { MenuEntry } from "./types";
import { tabMenu } from "./workspace";
import type { DiskUsage, EnvironmentEntry, ProcessDetail, ProcessSample, SocketRow } from "@/types/monitor";
import type { OpenTunnel, Session, SessionGroup, SftpEntry, TransferInfo } from "@/types/session";

const LOCALES = ["en", "zh-CN"] as const;

const file: SftpEntry = { name: "notes.txt", path: "/home/user/notes.txt", kind: "file", size: 12 };
const directory: SftpEntry = { name: "logs", path: "/home/user/logs", kind: "directory", size: 0 };
const link: SftpEntry = { name: "current", path: "/home/user/current", kind: "symlink", size: 0, linkTarget: "/srv" };
const socket: SocketRow = {
  protocol: "tcp",
  state: "LISTEN",
  address: "0.0.0.0",
  port: 22,
  pid: 812,
  process: "sshd",
  peerAddress: "203.0.113.9",
  peerPort: 51515,
  sentBytes: null,
  receivedBytes: null,
};
const process: ProcessSample = {
  pid: 1,
  user: "root",
  name: "nginx",
  command: "nginx: worker process",
  executable: "/usr/sbin/nginx",
  cpuPercent: 0.4,
  memoryPercent: 1.2,
  residentBytes: 12_000_000,
  cpuTicks: 100,
};
const detail: ProcessDetail = {
  pid: 1,
  name: "nginx",
  commandLine: "nginx: worker process",
  executable: "/usr/sbin/nginx",
  workingDirectory: "/",
  environment: [],
};
const variable: EnvironmentEntry = { name: "PATH", value: "/usr/sbin:/usr/bin" };
const disk: DiskUsage = { mountPoint: "/", filesystem: "/dev/sda1", sizeBytes: 1, availableBytes: 1 };
const session: Session = {
  id: "session-1",
  name: "web",
  protocol: "ssh",
  host: "example.test",
  port: 22,
  user: "root",
  auth: { method: "password", hasSecret: true },
  terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30, forwardAgent: false, record: false },
  updatedAt: 1_700_000_000,
};
const group: SessionGroup = { id: "group-1", name: "Work", order: 0 };
const tunnel: OpenTunnel = {
  id: "tunnel-1",
  handle: "t1",
  sessionId: session.id,
  bindHost: "127.0.0.1",
  bindPort: 8080,
  mode: "local",
  targetHost: "db.internal",
  targetPort: 5432,
  connections: 0,
};
const transfer: TransferInfo = {
  id: "transfer-1",
  direction: "download",
  name: "nginx.conf",
  remotePath: "/etc/nginx/nginx.conf",
  size: 1024,
  transferred: 512,
  state: { kind: "running" },
  packed: false,
};

const nothing = () => {};

/** One menu per region, each built from the smallest input it accepts. */
function menus(): readonly { readonly title: string; readonly entries: readonly MenuEntry[] }[] {
  return [
    {
      title: MENU_TITLES.terminal,
      entries: terminalMenu(
        { selection: "x", interactive: true },
        { paste: nothing, selectAll: nothing, clear: nothing },
      ),
    },
    {
      title: MENU_TITLES.file,
      entries: fileEntryMenu(
        [file],
        { open: nothing, download: nothing, edit: nothing, rename: nothing, remove: nothing },
        { packed: false },
      ),
    },
    {
      title: MENU_TITLES.file,
      entries: fileEntryMenu(
        [directory],
        { open: nothing, download: nothing, edit: nothing, rename: nothing, remove: nothing },
        // Packed, so the one entry a directory gains is in the list below rather than only in the
        // locale files: a key nothing asks for is a key nothing checks.
        { packed: true },
      ),
    },
    {
      title: MENU_TITLES.file,
      entries: fileEntryMenu(
        [link],
        { open: nothing, download: nothing, edit: nothing, rename: nothing, remove: nothing },
        { packed: false },
      ),
    },
    {
      // Several rows at once, which is the only shape the counted labels exist in.
      title: MENU_TITLES.files,
      entries: fileEntryMenu(
        [file, directory, link],
        { open: nothing, download: nothing, edit: nothing, rename: nothing, remove: nothing },
        { packed: true },
      ),
    },
    {
      title: MENU_TITLES.directory,
      entries: fileAreaMenu(
        { path: "/", packed: false },
        {
          newFolder: nothing,
          newFile: nothing,
          upload: nothing,
          paste: nothing,
          refresh: nothing,
          togglePacked: nothing,
        },
      ),
    },
    {
      // Built again with packing on, because one of those rows says something else when it is.
      title: MENU_TITLES.directory,
      entries: fileAreaMenu(
        { path: "/", packed: true },
        {
          newFolder: nothing,
          newFile: nothing,
          upload: nothing,
          paste: nothing,
          refresh: nothing,
          togglePacked: nothing,
        },
      ),
    },
    { title: MENU_TITLES.transfer, entries: transferMenu(transfer, { cancel: nothing }) },
    {
      title: MENU_TITLES.tab,
      entries: tabMenu(
        { kind: "terminal", status: "open", hasHandle: true, index: 0, total: 2 },
        { close: nothing, closeOthers: nothing, closeToTheRight: nothing, reconnect: nothing },
      ),
    },
    {
      title: MENU_TITLES.session,
      entries: sessionMenu(session, [group], {
        openTerminal: nothing,
        openFiles: nothing,
        edit: nothing,
        remove: nothing,
        move: nothing,
      }),
    },
    {
      title: MENU_TITLES.group,
      entries: groupMenu(
        group,
        { rename: nothing, remove: nothing, move: nothing, addInside: nothing },
        { up: false, down: true },
      ),
    },
    { title: MENU_TITLES.tunnel, entries: tunnelMenu(tunnel, { stop: nothing }) },
    { title: MENU_TITLES.process, entries: monitorProcessMenu(process, { openPage: nothing }) },
    { title: MENU_TITLES.disk, entries: diskMenu(disk, { openPage: nothing }) },
    { title: MENU_TITLES.process, entries: processRowMenu(process, { open: nothing }) },
    { title: MENU_TITLES.detail, entries: processDetailMenu(detail, { refresh: nothing }) },
    { title: MENU_TITLES.environment, entries: environmentMenu(variable) },
    { title: MENU_TITLES.socket, entries: listenerMenu(socket, { showConnections: nothing }) },
    { title: MENU_TITLES.connection, entries: connectionMenu(socket) },
  ];
}

/** Every key any of those menus can put on screen, including the ones a submenu is holding. */
function keys(): string[] {
  const found = new Set<string>();
  const walk = (entries: readonly MenuEntry[]): void => {
    for (const entry of entries) {
      if (entry.kind === "action") {
        found.add(entry.labelKey);
      } else if (entry.kind === "submenu") {
        found.add(entry.labelKey);
        walk(entry.entries);
      }
    }
  };

  for (const menu of menus()) {
    found.add(menu.title);
    walk(menu.entries);
  }
  // Every title, whether or not a menu above happens to use it: a region that stops offering a menu
  // still has words that must be there the moment it offers one again.
  for (const title of Object.values(MENU_TITLES)) {
    found.add(title);
  }
  return [...found];
}

describe("the menus' words", () => {
  it("asks for a key in every language", () => {
    const all = keys();

    // A scan that finds nothing is a broken check rather than a translated interface.
    expect(all.length).toBeGreaterThanOrEqual(30);
    for (const menu of menus()) {
      expect(menu.entries.length, `${menu.title} built no entries`).toBeGreaterThanOrEqual(2);
    }

    for (const locale of LOCALES) {
      const missing = all.filter((key) => !i18n.global.te(key, locale));
      expect(missing, `${locale} has no translation for: ${missing.join(", ")}`).toEqual([]);
    }
  });
});
