// The port bookkeeping the e2e launcher depends on.
//
// Both rules here come from runs that failed somewhere else: a check that called a port free while
// a dev server was listening on it, and a dev server left behind by an interrupted run, which the
// next run then loaded the application from. The first case below fails against an IPv4-only probe,
// which is exactly what the check used to be.

import assert from "node:assert/strict";
import { createServer } from "node:net";
import test from "node:test";

import { listenersOn, portIsFree } from "../lib/dev-server.mjs";

/** Runs `body` with an ephemeral port held on one address, then releases it. */
async function holding(address, body) {
  const server = createServer();
  await new Promise((resolve) => server.listen(0, address, resolve));

  try {
    await body(server.address().port);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
}

/** An ephemeral port that was held for a moment and then released. */
async function releasedPort() {
  const server = createServer();
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return port;
}

test("a port held on one loopback address is not a free port", async () => {
  await holding("::1", async (port) => {
    assert.equal(await portIsFree(port), false, `port ${port} is listening on ::1`);
  });
});

test("a port nobody holds is free", async () => {
  const port = await releasedPort();
  assert.equal(await portIsFree(port), true, `nothing is listening on ${port}`);
});

test("the process holding a port can be named", { skip: process.platform !== "win32" }, async () => {
  await holding("::1", async (port) => {
    assert.ok(
      listenersOn(port).includes(String(process.pid)),
      `netstat should name this process as the holder of ${port}`,
    );
  });
});
