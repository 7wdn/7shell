// The port the interface is served on, and who is holding it.
//
// `tauri dev` starts the dev server (`pnpm dev`) and the application, and the window reads the page
// from a fixed dev URL. Two things about that arrangement have failed runs in a way that named
// nothing:
//
// 1. The dev server binds whichever loopback address `localhost` resolves to first — on Windows
//    that is `::1` — so a check that only tries `127.0.0.1` reports the port free and lets a run
//    start on top of a server that is very much listening. The failure then surfaces at the dev
//    server ("Port 1420 is already in use"), several steps away from the check that was wrong.
// 2. The dev server is a grandchild of the command a run starts, so killing the command's process,
//    or the application by name, leaves it behind holding the port for the next run to trip over.
//
// Both questions are answered here, once, so the launcher and the guards cannot disagree about
// them.

import { spawnSync } from "node:child_process";
import { createServer } from "node:net";

/** The port `apps/desktop/vite.config.ts` serves the interface on, and the window reads it from. */
export const DEV_PORT = 1420;

/**
 * The addresses a dev server can be reached at.
 *
 * Both, because they are separate sockets: a server bound to one of them leaves the other free, so
 * asking about a single one is a coin toss over which address it chose.
 */
export const LOOPBACKS = ["127.0.0.1", "::1"];

/** Whether one address of a port can be bound right now. */
function canBind(port, address) {
  return new Promise((resolve) => {
    const probe = createServer();
    // A family this machine does not have is not a family anything can be listening on either.
    probe.once("error", (error) => resolve(error.code === "EADDRNOTAVAIL"));
    probe.once("listening", () => probe.close(() => resolve(true)));
    probe.listen(port, address);
  });
}

/** Whether a port is free on every address a dev server might have bound. */
export async function portIsFree(port) {
  for (const address of LOOPBACKS) {
    if (!(await canBind(port, address))) {
      return false;
    }
  }
  return true;
}

/**
 * The processes listening on a port, by PID, on every address family.
 *
 * `netstat` rather than another socket probe: a probe can only say "taken", and what a stale run
 * needs is the PID to stop. Windows-only, like the rest of the process handling around it.
 */
export function listenersOn(port) {
  const lineHasPort = new RegExp(`:${port}\\s`);

  return [
    ...new Set(
      (spawnSync("netstat", ["-ano"], { encoding: "utf8", shell: true }).stdout ?? "")
        .split("\n")
        .filter((line) => lineHasPort.test(line) && line.includes("LISTENING"))
        .map((line) => line.trim().split(/\s+/).pop())
        .filter((pid) => pid && /^[0-9]+$/.test(pid)),
    ),
  ];
}
