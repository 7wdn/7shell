/**
 * Reading a directory for the command line: asked for once per directory, held for a few seconds, and
 * quiet when the host will not answer.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({ filesList: vi.fn(), filesOpen: vi.fn() }));

import { useRemotePaths, type RemoteChannel } from "@/components/commands/useRemotePaths";
import { filesList } from "@/lib/backend/tauri";
import type { SftpEntry, SftpListing } from "@/types/session";

const filesListMock = vi.mocked(filesList);

function entry(name: string): SftpEntry {
  return { name, path: `/var/${name}`, kind: "file", size: 1 };
}

function listing(path: string, names: string[]): SftpListing {
  return { path, entries: names.map(entry) };
}

const CHANNEL: RemoteChannel = { handle: "sftp-1", cwd: "/var/log" };

beforeEach(() => {
  filesListMock.mockReset();
});

describe("reading a directory for the line", () => {
  it("asks the host once and answers from what came back", async () => {
    filesListMock.mockResolvedValue(listing("/var", ["log", "spool"]));
    const paths = useRemotePaths(() => CHANNEL);

    const settled = vi.fn();
    paths.warm("/var", settled);
    expect(filesListMock).toHaveBeenCalledWith("sftp-1", "/var");
    expect(paths.entries("/var"), "nothing yet: the answer is on its way").toBeNull();

    await vi.waitFor(() => expect(settled).toHaveBeenCalled());
    expect(paths.entries("/var")?.map((entry) => entry.name)).toEqual(["log", "spool"]);
  });

  it("does not ask again for a directory it has just read", () => {
    filesListMock.mockResolvedValue(listing("/var", ["log"]));
    const paths = useRemotePaths(() => CHANNEL);

    paths.warm("/var", () => {});
    paths.warm("/var", () => {});
    expect(filesListMock).toHaveBeenCalledTimes(1);
  });

  it("remembers a directory the host would not read as empty, without saying so", async () => {
    filesListMock.mockRejectedValue(new Error("no such file or directory"));
    const paths = useRemotePaths(() => CHANNEL);

    const settled = vi.fn();
    paths.warm("/nowhere", settled);
    await vi.waitFor(() => expect(settled).toHaveBeenCalled());

    expect(paths.entries("/nowhere")).toEqual([]);
  });

  it("has nothing to read and nothing to resolve against without a channel", () => {
    const paths = useRemotePaths(() => null);
    const settled = vi.fn();

    paths.warm("/var", settled);

    expect(filesListMock).not.toHaveBeenCalled();
    expect(paths.entries("/var")).toBeNull();
    expect(paths.base).toBeNull();
    expect(settled, "nobody is told to redraw for a question that was never asked").not.toHaveBeenCalled();
  });

  it("gives the directory a relative path starts from", () => {
    expect(useRemotePaths(() => CHANNEL).base).toBe("/var/log");
    expect(useRemotePaths(() => ({ handle: "sftp-1", cwd: null })).base).toBeNull();
  });

  it("keeps two channels apart even when they point at the same path", async () => {
    filesListMock.mockResolvedValue(listing("/var", ["log"]));
    let handle = "sftp-1";
    const paths = useRemotePaths(() => ({ handle, cwd: null }));

    paths.warm("/var", () => {});
    handle = "sftp-2";
    expect(paths.entries("/var"), "the other session has read nothing").toBeNull();

    paths.warm("/var", () => {});
    expect(filesListMock).toHaveBeenCalledTimes(2);
  });

  it("tells everyone waiting for a directory when the answer lands", async () => {
    filesListMock.mockResolvedValue(listing("/var", ["log"]));
    const paths = useRemotePaths(() => CHANNEL);

    const first = vi.fn();
    const second = vi.fn();
    paths.warm("/var", first);
    paths.warm("/var", second);

    expect(filesListMock, "asked once, for the two of them").toHaveBeenCalledTimes(1);
    await vi.waitFor(() => expect(second).toHaveBeenCalled());
    expect(first).toHaveBeenCalled();
  });
});
