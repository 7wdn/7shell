import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  droppedUpload: vi.fn(),
  clipboardFiles: vi.fn(),
  filesOpen: vi.fn(),
  filesList: vi.fn(),
  filesCreateFolder: vi.fn(),
  filesRename: vi.fn(),
  filesDelete: vi.fn(),
  filesDownload: vi.fn(),
  filesUpload: vi.fn(),
  filesClose: vi.fn(),
  transfersOpen: vi.fn(),
  transferCancel: vi.fn(),
}));

import {
  filesClose,
  filesCreateFolder,
  filesDelete,
  filesDownload,
  filesList,
  filesOpen,
  filesRename,
  filesUpload,
  transferCancel,
  transfersOpen,
  droppedUpload,
  clipboardFiles,
} from "@/lib/backend/tauri";
import { TRANSFER_POLL_MS, useFilesStore } from "@/stores/files";
import type { SftpEntry, SftpListing, SftpOutcome, TransferInfo } from "@/types/session";

const filesOpenMock = vi.mocked(filesOpen);
const filesListMock = vi.mocked(filesList);
const filesCreateFolderMock = vi.mocked(filesCreateFolder);
const filesRenameMock = vi.mocked(filesRename);
const filesDeleteMock = vi.mocked(filesDelete);
const filesDownloadMock = vi.mocked(filesDownload);
const filesUploadDroppedMock = vi.mocked(droppedUpload);
const clipboardFilesMock = vi.mocked(clipboardFiles);
const filesUploadMock = vi.mocked(filesUpload);
const filesCloseMock = vi.mocked(filesClose);
const transfersOpenMock = vi.mocked(transfersOpen);
const transferCancelMock = vi.mocked(transferCancel);

const sessionId = "session-1";
const handle = "f1";

function entry(name: string, kind: SftpEntry["kind"] = "file"): SftpEntry {
  return { name, path: `/home/user/${name}`, kind, size: 12, modifiedAt: 1_700_000_000 };
}

function listing(path: string, names: string[] = []): SftpListing {
  return { path, entries: names.map((name) => entry(name)) };
}

function opened(at: SftpListing): SftpOutcome {
  return { kind: "opened", handle, sessionId, listing: at };
}

const download: TransferInfo = {
  id: "x1",
  direction: "download",
  name: "notes.txt",
  remotePath: "/home/user/notes.txt",
  size: 100,
  transferred: 40,
  state: { kind: "running" },
  packed: false,
};

const upload: TransferInfo = {
  id: "x2",
  direction: "upload",
  name: "notes.txt",
  remotePath: "/home/user/notes.txt",
  size: 10,
  transferred: 0,
  state: { kind: "running" },
  packed: false,
};

/** An SFTP refusal, in the envelope shape the backend rejects with. */
function denied(): unknown {
  return {
    version: 1,
    code: "SFTP-PERMISSION-DENIED",
    messageKey: "errors.sftp.permissionDenied",
    messageParams: { path: "/home/user/notes.txt" },
    origin: { subsystem: "sftp" },
    outcome: "sent",
  };
}

describe("files store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    filesOpenMock.mockReset();
    filesListMock.mockReset().mockResolvedValue(listing("/home/user"));
    filesCreateFolderMock.mockReset().mockResolvedValue(undefined);
    filesRenameMock.mockReset().mockResolvedValue(undefined);
    filesDeleteMock.mockReset().mockResolvedValue(undefined);
    filesDownloadMock.mockReset().mockResolvedValue([]);
    filesUploadMock.mockReset().mockResolvedValue(undefined);
    filesCloseMock.mockReset().mockResolvedValue(undefined);
    transfersOpenMock.mockReset().mockResolvedValue([]);
    transferCancelMock.mockReset().mockResolvedValue(undefined);
  });

  afterEach(() => {
    // A poll left running would reach into the next test's mocks.
    useFilesStore().stopPolling();
    vi.useRealTimers();
  });

  it("records the listing an open answered with", async () => {
    filesOpenMock.mockResolvedValue(opened(listing("/home/user", ["notes.txt", "photos"])));

    const store = useFilesStore();
    const outcome = await store.open(sessionId);

    expect(outcome.kind).toBe("opened");
    expect(store.browser(handle)?.path).toBe("/home/user");
    expect(store.browser(handle)?.entries.map((item) => item.name)).toEqual(["notes.txt", "photos"]);
    // The first listing is part of the open answer; nothing had to be asked for separately.
    expect(filesListMock).not.toHaveBeenCalled();
  });

  it("replaces the listing when the path changes", async () => {
    filesOpenMock.mockResolvedValue(opened(listing("/home/user", ["notes.txt"])));
    filesListMock.mockResolvedValue(listing("/home/user/photos", ["one.jpg"]));

    const store = useFilesStore();
    await store.open(sessionId);
    await store.navigate(handle, "/home/user/photos");

    expect(filesListMock).toHaveBeenCalledWith(handle, "/home/user/photos");
    expect(store.browser(handle)?.path).toBe("/home/user/photos");
    expect(store.browser(handle)?.entries.map((item) => item.name)).toEqual(["one.jpg"]);
  });

  it("calls the transport for a change and re-reads the directory after it", async () => {
    filesOpenMock.mockResolvedValue(opened(listing("/home/user")));
    filesListMock.mockResolvedValue(listing("/home/user", ["made"]));

    const store = useFilesStore();
    await store.open(sessionId);

    await store.createFolder(handle, "/home/user/made");
    expect(filesCreateFolderMock).toHaveBeenCalledWith(handle, "/home/user/made");
    expect(filesListMock).toHaveBeenCalledTimes(1);

    await store.rename(handle, "/home/user/made", "/home/user/renamed");
    expect(filesRenameMock).toHaveBeenCalledWith(handle, "/home/user/made", "/home/user/renamed");
    expect(filesListMock).toHaveBeenCalledTimes(2);

    await store.removeMany(handle, [entry("renamed", "directory")]);
    expect(filesDeleteMock).toHaveBeenCalledWith(handle, "/home/user/renamed", "directory");
    expect(filesListMock).toHaveBeenCalledTimes(3);
  });

  it("keeps the failure and the last listing when a change is refused", async () => {
    filesOpenMock.mockResolvedValue(opened(listing("/home/user", ["notes.txt"])));
    filesDeleteMock.mockRejectedValue(denied());
    filesListMock.mockResolvedValue(listing("/home/user", ["notes.txt"]));

    const store = useFilesStore();
    await store.open(sessionId);
    const worked = await store.removeMany(handle, [entry("notes.txt")]);

    expect(worked).toBe(false);
    expect(store.browser(handle)?.failure?.code).toBe("SFTP-PERMISSION-DENIED");
    // The list stays usable: the re-read happened, and what it found is still on screen.
    expect(store.browser(handle)?.entries.map((item) => item.name)).toEqual(["notes.txt"]);
  });

  it("records nothing when the save dialog is dismissed", async () => {
    filesDownloadMock.mockResolvedValue([]);

    const store = useFilesStore();
    await store.download(handle, ["/home/user/notes.txt"], "Save", "notes.txt", false);

    expect(store.transfers).toEqual([]);
    expect(store.transferFailure).toBeNull();
    // No transfer started, so there was no list to read.
    expect(transfersOpenMock).not.toHaveBeenCalled();
  });

  it("reads a started download back into the transfer list", async () => {
    filesDownloadMock.mockResolvedValue([download.id]);
    transfersOpenMock.mockResolvedValue([download]);

    const store = useFilesStore();
    await store.download(handle, ["/home/user/notes.txt"], "Save", "notes.txt", false);

    expect(store.transfers).toEqual([download]);
  });

  it("hands the packing switch, and every chosen path, to the backend", async () => {
    filesDownloadMock.mockResolvedValue([download.id]);
    filesUploadMock.mockResolvedValue(upload.id);

    const store = useFilesStore();
    await store.download(handle, ["/home/user/logs"], "Save", "", true);
    await store.download(handle, ["/home/a.txt", "/home/b.txt"], "Save", "", false);
    await store.upload(handle, "/home/user", "Choose files to pack", true);

    expect(filesDownloadMock).toHaveBeenCalledWith(handle, ["/home/user/logs"], "Save", "", true);
    expect(filesDownloadMock).toHaveBeenCalledWith(handle, ["/home/a.txt", "/home/b.txt"], "Save", "", false);
    expect(filesUploadMock).toHaveBeenCalledWith(handle, "/home/user", "Choose files to pack", true);
  });

  /**
   * One `run` for a batch, so the directory is read once and the failure of the first refusal is what
   * the panel shows — a loop of separate calls would read it N times and keep only the last failure.
   */
  it("deletes a selection with one read of the directory, and stops at the first refusal", async () => {
    filesOpenMock.mockResolvedValue(opened(listing("/home/user", ["a.txt", "b.txt", "c.txt"])));
    filesDeleteMock.mockResolvedValueOnce(undefined).mockRejectedValueOnce(denied()).mockResolvedValue(undefined);

    const store = useFilesStore();
    await store.open(sessionId);
    const reads = filesListMock.mock.calls.length;

    const worked = await store.removeMany(handle, [entry("a.txt"), entry("b.txt"), entry("c.txt")]);

    expect(worked).toBe(false);
    // Two attempts and no third: what stopped it is reported, and what was deleted stays deleted.
    expect(filesDeleteMock).toHaveBeenCalledTimes(2);
    expect(filesListMock.mock.calls.length).toBe(reads + 1);
    expect(store.browser(handle)?.failure?.code).toBe("SFTP-PERMISSION-DENIED");
  });

  it("keeps a failed transfer and its reason", async () => {
    const failed: TransferInfo = {
      ...download,
      state: { kind: "failed", detail: "the local file could not be written" },
    };
    transfersOpenMock.mockResolvedValue([failed]);

    const store = useFilesStore();
    await store.refreshTransfers();

    expect(store.transfers).toEqual([failed]);
    expect(store.transfers[0]?.state).toEqual({ kind: "failed", detail: "the local file could not be written" });
  });

  it("holds a failure nobody has attended to, and never records it twice", async () => {
    const failed: TransferInfo = {
      ...download,
      state: { kind: "failed", detail: "the connection dropped" },
    };
    transfersOpenMock.mockResolvedValue([failed]);

    const store = useFilesStore();
    await store.refreshTransfers();
    await store.refreshTransfers();

    expect(store.unattendedFailures).toEqual([{ id: download.id, direction: "download", name: "notes.txt" }]);

    store.attendFailures([download.id]);
    expect(store.unattendedFailures).toEqual([]);
  });

  it("tells a failure that has just happened from one already known", async () => {
    const failed: TransferInfo = { ...download, state: { kind: "failed", detail: "gone" } };
    transfersOpenMock.mockResolvedValueOnce([download]).mockResolvedValueOnce([failed]);

    const store = useFilesStore();
    await store.refreshTransfers();
    // The first read saw it moving; only the change to `failed` is news.
    expect(store.unattendedFailures).toEqual([]);

    await store.refreshTransfers();
    expect(store.unattendedFailures).toHaveLength(1);
  });

  it("stops a transfer through the backend and drops the row", async () => {
    transfersOpenMock.mockResolvedValueOnce([download]).mockResolvedValueOnce([]);

    const store = useFilesStore();
    await store.refreshTransfers();
    await store.cancelTransfer(download.id);

    expect(transferCancelMock).toHaveBeenCalledWith(download.id);
    expect(store.transfers).toEqual([]);
  });

  it("clears a browser's state when it is closed", async () => {
    filesOpenMock.mockResolvedValue(opened(listing("/home/user", ["notes.txt"])));

    const store = useFilesStore();
    await store.open(sessionId);
    const failure = await store.close(handle);

    expect(failure).toBeNull();
    expect(filesCloseMock).toHaveBeenCalledWith(handle);
    expect(store.browser(handle)).toBeNull();
  });

  it("re-reads the directory an upload lands in when the transfer finishes", async () => {
    vi.useFakeTimers();
    transfersOpenMock
      .mockResolvedValueOnce([upload])
      .mockResolvedValueOnce([{ ...upload, transferred: upload.size, state: { kind: "done" } }]);
    filesOpenMock.mockResolvedValue(opened(listing("/home/user")));
    filesListMock.mockResolvedValue(listing("/home/user", ["notes.txt"]));

    const store = useFilesStore();
    await store.open(sessionId);
    await store.refreshTransfers();
    await store.refreshTransfers();

    expect(filesListMock).toHaveBeenCalledWith(handle, "/home/user");
    expect(store.browser(handle)?.entries.map((item) => item.name)).toEqual(["notes.txt"]);
  });

  it("starts reading an upload into the list as soon as it is started", async () => {
    filesUploadMock.mockResolvedValue(upload.id);
    transfersOpenMock.mockResolvedValue([upload]);

    const store = useFilesStore();
    await store.upload(handle, "/home/user", "Choose a file to upload", false);

    expect(filesUploadMock).toHaveBeenCalledWith(handle, "/home/user", "Choose a file to upload", false);
    expect(store.transfers).toEqual([upload]);
  });

  /**
   * A packed upload puts its files *in* the directory it was aimed at rather than a file inside it, so
   * the directory to read again is the one on the row rather than its parent. Reading the parent would
   * leave the panel showing a directory the transfer never touched — which is exactly when somebody
   * would decide the feature did nothing.
   */
  it("re-reads the directory itself when a packed upload lands", async () => {
    const packed: TransferInfo = {
      ...upload,
      name: "upload_pack_2_files.tar.gz",
      remotePath: "/home/user",
      packed: true,
    };
    vi.useFakeTimers();
    transfersOpenMock
      .mockResolvedValueOnce([packed])
      .mockResolvedValueOnce([{ ...packed, transferred: packed.size, state: { kind: "done" } }]);
    filesOpenMock.mockResolvedValue(opened(listing("/home/user")));
    filesListMock.mockResolvedValue(listing("/home/user", ["notes.txt", "report.pdf"]));

    const store = useFilesStore();
    await store.open(sessionId);
    await store.refreshTransfers();
    await store.refreshTransfers();

    expect(filesListMock).toHaveBeenCalledWith(handle, "/home/user");
    expect(store.browser(handle)?.entries.map((item) => item.name)).toEqual(["notes.txt", "report.pdf"]);
  });

  it("polls while a transfer is running and stops when none is", async () => {
    vi.useFakeTimers();
    const done: TransferInfo = { ...download, transferred: download.size, state: { kind: "done" } };
    transfersOpenMock.mockResolvedValueOnce([download]).mockResolvedValueOnce([done]);

    const store = useFilesStore();
    await store.refreshTransfers();
    expect(transfersOpenMock).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(TRANSFER_POLL_MS);
    expect(transfersOpenMock).toHaveBeenCalledTimes(2);

    // The second read answered "done", so the interval is gone: a later tick reads nothing.
    await vi.advanceTimersByTimeAsync(TRANSFER_POLL_MS * 2);
    expect(transfersOpenMock).toHaveBeenCalledTimes(2);
  });
});

describe("the directory tree's own listing", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    filesListMock.mockReset();
  });

  it("keeps only directories, because nothing else can be walked into", async () => {
    filesListMock.mockResolvedValue({
      path: "/home",
      entries: [
        { name: "var", path: "/home/var", kind: "directory", size: 0 },
        { name: "notes.txt", path: "/home/notes.txt", kind: "file", size: 12 },
        { name: "link", path: "/home/link", kind: "symlink", size: 0 },
      ],
    });

    const store = useFilesStore();
    const directories = await store.directories("f1", "/home");

    expect(directories.map((entry) => entry.name)).toEqual(["var"]);
  });

  it("answers with nothing when a branch cannot be read, leaving the listing to report it", async () => {
    filesListMock.mockRejectedValue({
      version: 1,
      code: "SFTP-PERMISSION-DENIED",
      messageKey: "errors.sftp.permissionDenied",
      messageParams: { path: "/root" },
      origin: { subsystem: "sftp" },
      outcome: "notSent",
    });

    const store = useFilesStore();

    await expect(store.directories("f1", "/root")).resolves.toEqual([]);
  });
});

describe("the revision the tree watches", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    filesOpenMock.mockReset();
    filesListMock.mockReset();
    filesCreateFolderMock.mockReset();
  });

  it("moves when a change happened", async () => {
    const listing = { path: "/home", entries: [] };
    filesOpenMock.mockResolvedValue({ kind: "opened", handle: "f1", sessionId: "s1", listing });
    filesListMock.mockResolvedValue(listing);
    filesCreateFolderMock.mockResolvedValue(undefined);

    const store = useFilesStore();
    await store.open("s1");
    const before = store.revision;

    await store.createFolder("f1", "/home/new");

    expect(store.revision).toBeGreaterThan(before);
  });

  it("stays where it is when the change was refused", async () => {
    const listing = { path: "/home", entries: [] };
    filesOpenMock.mockResolvedValue({ kind: "opened", handle: "f1", sessionId: "s1", listing });
    filesListMock.mockResolvedValue(listing);
    filesCreateFolderMock.mockRejectedValue({
      version: 1,
      code: "SFTP-PERMISSION-DENIED",
      messageKey: "errors.sftp.permissionDenied",
      messageParams: { path: "/home/new" },
      origin: { subsystem: "sftp" },
      outcome: "notSent",
    });

    const store = useFilesStore();
    await store.open("s1");
    const before = store.revision;

    await store.createFolder("f1", "/home/new");

    expect(store.revision).toBe(before);
  });
});

describe("files the operating system handed us", () => {
  const listing = { path: "/home", entries: [] };

  beforeEach(() => {
    setActivePinia(createPinia());
    filesOpenMock.mockReset().mockResolvedValue({ kind: "opened", handle: "f1", sessionId: "s1", listing });
    filesListMock.mockReset().mockResolvedValue(listing);
    filesUploadDroppedMock.mockReset();
    clipboardFilesMock.mockReset();
    transfersOpenMock.mockReset().mockResolvedValue([]);
  });

  it("uploads by token, one transfer per file, without ever seeing a path", async () => {
    filesUploadDroppedMock.mockResolvedValue(["x1", "x2"]);

    const store = useFilesStore();
    await store.open("s1");
    await store.uploadDropped("f1", "/home", "d1", false);

    expect(filesUploadDroppedMock).toHaveBeenCalledWith("f1", "/home", "d1", false);
    expect(transfersOpenMock).toHaveBeenCalled();
    expect(store.browser("f1")?.failure).toBeNull();
  });

  /**
   * A batch dropped from the desktop is a batch like any other, so the switch means the same thing at
   * that door too: one archive, one transfer, rather than one per file.
   */
  it("packs a drop when the switch is on, and says one transfer came back", async () => {
    filesUploadDroppedMock.mockResolvedValue(["x1"]);
    transfersOpenMock.mockResolvedValue([download]);

    const store = useFilesStore();
    await store.open("s1");
    await store.uploadDropped("f1", "/home", "d1", true);

    expect(filesUploadDroppedMock).toHaveBeenCalledWith("f1", "/home", "d1", true);
    expect(store.transfers).toEqual([download]);
  });

  it("keeps the reason when the drop could not be used", async () => {
    filesUploadDroppedMock.mockRejectedValue({
      version: 1,
      code: "APP-UNSUPPORTED",
      messageKey: "errors.app.unsupported",
      messageParams: { what: "dropping something that is not a file" },
      origin: { subsystem: "app" },
      outcome: "notSent",
    });

    const store = useFilesStore();
    await store.open("s1");
    await store.uploadDropped("f1", "/home", "d1", false);

    expect(store.browser("f1")?.failure?.code).toBe("APP-UNSUPPORTED");
  });

  it("pastes through the same door, and reports a platform that has no file list", async () => {
    clipboardFilesMock.mockResolvedValue({ token: "d9", names: ["notes.txt"] });
    filesUploadDroppedMock.mockResolvedValue(["x1"]);

    const store = useFilesStore();
    await store.open("s1");
    await store.uploadFromClipboard("f1", "/home", false);
    expect(filesUploadDroppedMock).toHaveBeenCalledWith("f1", "/home", "d9", false);

    clipboardFilesMock.mockRejectedValue({
      version: 1,
      code: "APP-UNSUPPORTED",
      messageKey: "errors.app.unsupported",
      messageParams: { what: "pasting files from the clipboard on this platform" },
      origin: { subsystem: "app" },
      outcome: "notSent",
    });
    await store.uploadFromClipboard("f1", "/home", false);

    expect(store.browser("f1")?.failure?.code).toBe("APP-UNSUPPORTED");
  });
});

describe("following the shell", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    filesOpenMock.mockReset();
    filesListMock.mockReset().mockResolvedValue(listing("/home/user"));
    transfersOpenMock.mockReset().mockResolvedValue([]);
  });

  /** The backend answering every listing with the path it was asked for. */
  function listingEveryDirectory(): void {
    filesListMock.mockImplementation(async (_handle: string, path: string) => listing(path));
  }

  it("goes where the shell went, once the directory answers for itself", async () => {
    filesOpenMock.mockResolvedValue(opened(listing("/home/user")));
    filesListMock.mockResolvedValue(listing("/data"));

    const store = useFilesStore();
    await store.open(sessionId);
    await store.followDirectory(handle, "/data");

    expect(filesListMock).toHaveBeenLastCalledWith(handle, "/data");
    expect(store.browser(handle)?.path).toBe("/data");
  });

  it("stays where it is, without a word, when the directory cannot be read", async () => {
    filesOpenMock.mockResolvedValue(opened(listing("/home/user")));
    filesListMock.mockRejectedValue(denied());

    const store = useFilesStore();
    await store.open(sessionId);
    await store.followDirectory(handle, "/nope");

    // The shell has already refused the `cd`, in the terminal, in its own words.
    expect(store.browser(handle)?.path).toBe("/home/user");
    expect(store.browser(handle)?.failure).toBeNull();
  });

  it("resolves a relative path against where the shell is, not against what is on screen", async () => {
    filesOpenMock.mockResolvedValue(opened(listing("/home/user")));
    listingEveryDirectory();

    const store = useFilesStore();
    await store.open(sessionId);
    // The panel is browsed somewhere else while the shell stays where it was.
    await store.navigate(handle, "/etc");
    await store.followDirectory(handle, "logs");

    expect(filesListMock).toHaveBeenLastCalledWith(handle, "/home/user/logs");
    expect(store.browser(handle)?.path).toBe("/home/user/logs");
  });

  it("keeps resolving from where the shell was last seen, not from where it started", async () => {
    filesOpenMock.mockResolvedValue(opened(listing("/home/user")));
    listingEveryDirectory();

    const store = useFilesStore();
    await store.open(sessionId);
    await store.followDirectory(handle, "/data");
    await store.navigate(handle, "/etc");
    await store.followDirectory(handle, "src");

    expect(filesListMock).toHaveBeenLastCalledWith(handle, "/data/src");
  });

  it("takes a bare cd to where the browser opened, which is where a login shell started", async () => {
    filesOpenMock.mockResolvedValue(opened(listing("/home/user")));
    listingEveryDirectory();

    const store = useFilesStore();
    await store.open(sessionId);
    await store.navigate(handle, "/etc");
    await store.followDirectory(handle, "~");

    expect(filesListMock).toHaveBeenLastCalledWith(handle, "/home/user");
  });

  it("reads nothing again when the shell is already where the browser is showing", async () => {
    filesOpenMock.mockResolvedValue(opened(listing("/home/user")));

    const store = useFilesStore();
    await store.open(sessionId);
    await store.followDirectory(handle, "/home/user");

    expect(filesListMock).not.toHaveBeenCalled();
  });

  it("does nothing when there is no browser left to move", async () => {
    const store = useFilesStore();
    await store.followDirectory("gone", "/data");

    expect(filesListMock).not.toHaveBeenCalled();
  });
});
