import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  sessionsList: vi.fn(),
  sessionSave: vi.fn(),
  sessionDelete: vi.fn(),
  groupsList: vi.fn(),
  k8sContexts: vi.fn(),
}));

import { groupsList, k8sContexts, sessionSave, sessionsList } from "@/lib/backend/tauri";
import { useSessionEditorStore } from "@/stores/sessionEditor";
import { useSessionsStore } from "@/stores/sessions";
import type { Session } from "@/types/session";

const sessionsListMock = vi.mocked(sessionsList);
const sessionSaveMock = vi.mocked(sessionSave);
const groupsListMock = vi.mocked(groupsList);
const k8sContextsMock = vi.mocked(k8sContexts);

function sample(id: string, name: string): Session {
  return {
    id,
    name,
    protocol: "ssh",
    host: `${name}.test`,
    port: 22,
    user: "root",
    auth: { method: "password", hasSecret: false },
    terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30, forwardAgent: false, record: false },
    updatedAt: 1_700_000_000,
  };
}

describe("session editor store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    sessionsListMock.mockReset().mockResolvedValue([sample("bastion", "bastion"), sample("db", "db")]);
    groupsListMock.mockReset().mockResolvedValue([]);
    sessionSaveMock.mockReset().mockImplementation(async (session: Session) => session);
    k8sContextsMock.mockReset().mockResolvedValue([]);
  });

  it("keeps hops in the order they were added, moved and removed", async () => {
    await useSessionsStore().load();
    const editor = useSessionEditorStore();
    editor.fill(null);

    editor.addHop("bastion");
    editor.addHop("db");
    editor.addHop("bastion");
    expect(editor.draft().jump).toEqual(["bastion", "db", "bastion"]);

    // A move off either end is not a move.
    editor.moveHop(0, -1);
    expect(editor.draft().jump).toEqual(["bastion", "db", "bastion"]);

    editor.moveHop(2, -1);
    expect(editor.draft().jump).toEqual(["bastion", "bastion", "db"]);

    editor.removeHopAt(0);
    expect(editor.draft().jump).toEqual(["bastion", "db"]);
  });

  it("never offers the session being edited as its own hop", async () => {
    await useSessionsStore().load();
    const editor = useSessionEditorStore();
    editor.fill(sample("db", "db"));

    expect(editor.hopChoices.map((session) => session.id)).toEqual(["bastion"]);
    editor.addHop("db");

    expect(editor.jumps).toEqual([]);
  });

  it("keeps a hop whose session was deleted, shows it as missing, and does not save until it is removed", async () => {
    await useSessionsStore().load();
    const editor = useSessionEditorStore();
    editor.fill({ ...sample("app", "app"), jump: ["session-that-is-gone", "bastion"] });

    expect(editor.hops).toHaveLength(2);
    expect(editor.hops[0]).toEqual({ id: "session-that-is-gone", session: undefined });
    expect(editor.hops[1]?.session?.name).toBe("bastion");
    // Still saved: dropping it silently would change the route without telling anyone.
    expect(editor.draft().jump).toEqual(["session-that-is-gone", "bastion"]);

    editor.removeHopAt(0);
    expect(editor.draft().jump).toEqual(["bastion"]);
  });

  it("starts a forward row at the local default and rejects one without a target", async () => {
    const editor = useSessionEditorStore();
    // A complete session, so the only thing under test is the row.
    editor.fill(sample("app", "app"));
    editor.addForward();

    const id = editor.forwards[0]?.id ?? "";
    expect(editor.forwards[0]).toMatchObject({ bindHost: "127.0.0.1", bindPort: "0", targetHost: "", targetPort: "" });
    expect(editor.forwardProblems[id]).toEqual(["targetHost", "targetPort"]);
    expect(editor.hasProblems).toBe(true);

    const row = editor.forwards[0];
    if (!row) {
      throw new Error("addForward should have added a row");
    }

    row.bindPort = "70000";
    expect(editor.forwardProblems[id]).toEqual(["bindPort", "targetHost", "targetPort"]);

    row.bindPort = "0";
    row.targetHost = "db.internal";
    row.targetPort = "5432";
    expect(editor.forwardProblems[id]).toEqual([]);
    expect(editor.hasProblems).toBe(false);
  });

  it("saves a valid forward row as part of the session", async () => {
    const library = useSessionsStore();
    const editor = useSessionEditorStore();
    editor.fill(sample("app", "app"));
    editor.addForward();

    const row = editor.forwards[0];
    if (!row) {
      throw new Error("addForward should have added a row");
    }
    row.targetHost = "db.internal";
    row.targetPort = "5432";

    await library.save(editor.draft());

    expect(sessionSaveMock).toHaveBeenCalledWith(
      expect.objectContaining({
        forwards: [
          {
            id: row.id,
            mode: "local",
            bindHost: "127.0.0.1",
            bindPort: 0,
            targetHost: "db.internal",
            targetPort: 5432,
          },
        ],
      }),
      undefined,
    );
    expect(library.sessions[0]?.forwards?.[0]?.targetPort).toBe(5432);
  });

  it("saves a dynamic forward with no destination of its own", async () => {
    const library = useSessionsStore();
    const editor = useSessionEditorStore();
    editor.fill(sample("app", "app"));
    editor.addForward();

    const row = editor.forwards[0];
    if (!row) {
      throw new Error("addForward should have added a row");
    }
    // A destination typed before the kind was switched is not carried: a field saved under a kind
    // that ignores it is a field the next reader has to know to ignore.
    row.targetHost = "db.internal";
    row.targetPort = "5432";
    row.mode = "dynamic";

    await library.save(editor.draft());

    expect(sessionSaveMock).toHaveBeenCalledWith(
      expect.objectContaining({
        forwards: [{ id: row.id, mode: "dynamic", bindHost: "127.0.0.1", bindPort: 0, targetHost: "", targetPort: 0 }],
      }),
      undefined,
    );
  });

  it("saves no route or forwards at all when there are none", async () => {
    const library = useSessionsStore();
    const editor = useSessionEditorStore();
    editor.fill(sample("app", "app"));

    const saved = editor.draft();
    expect(saved.jump).toBeUndefined();
    expect(saved.forwards).toBeUndefined();

    await library.save(saved);
    expect(sessionSaveMock).toHaveBeenCalledWith(saved, undefined);
  });

  /**
   * What the two entry points promise.
   *
   * The store carries *which* session the editor is opening on, and the dialog fills the draft from
   * it as it comes up — so a new session and an edit differ in this one value, and a stale `editing`
   * left behind by a previous edit is how a "new session" would quietly become an overwrite.
   */
  it("says what the editor is opening on, and closes when the dialog does", () => {
    const editor = useSessionEditorStore();
    expect(editor.open).toBe(false);

    editor.startEdit(sample("db", "db"));
    expect(editor.open).toBe(true);
    expect(editor.editing?.id).toBe("db");

    editor.setOpen(false);
    expect(editor.open).toBe(false);

    editor.startNew();
    expect(editor.open).toBe(true);
    expect(editor.editing).toBeNull();
  });

  it("keeps a WebDAV address in the host field and saves it under webdav", async () => {
    await useSessionsStore().load();
    const editor = useSessionEditorStore();
    editor.fill({
      ...sample("dav", "dav"),
      protocol: "webdav",
      host: "https://dav.example/files/",
      port: 443,
      user: "alice",
      webdav: { url: "https://dav.example/files/" },
    });

    expect(editor.isWebDav).toBe(true);
    expect(editor.host).toBe("https://dav.example/files/");
    expect(editor.sessionProblems.host).toBe(false);
    expect(editor.sessionProblems.port).toBe(false);

    const draft = editor.draft();
    expect(draft.protocol).toBe("webdav");
    expect(draft.host).toBe("https://dav.example/files/");
    expect(draft.webdav?.url).toBe("https://dav.example/files/");
  });

  it("refuses a WebDAV address that is not http, and allows an anonymous user", async () => {
    await useSessionsStore().load();
    const editor = useSessionEditorStore();
    editor.fill({
      ...sample("dav", "dav"),
      protocol: "webdav",
      host: "dav.example/files",
      port: 443,
      user: "",
      webdav: { url: "dav.example/files" },
    });

    expect(editor.sessionProblems.host).toBe(true);
    // A WebDAV folder may be readable without an account.
    expect(editor.sessionProblems.user).toBe(false);
  });

  it("saves the FTPS switch under ftp and leaves plain FTP without a block", async () => {
    await useSessionsStore().load();
    const editor = useSessionEditorStore();
    editor.fill({
      ...sample("ftp", "ftp"),
      protocol: "ftp",
      host: "ftp.example",
      port: 21,
      user: "alice",
    });

    expect(editor.isFtp).toBe(true);
    expect(editor.draft().ftp).toEqual({ secure: false });

    editor.ftpSecure = true;
    expect(editor.draft().ftp).toEqual({ secure: true });
  });

  it("starts a session that carries no container or cluster block at that kind's defaults", () => {
    const editor = useSessionEditorStore();
    editor.fill(sample("old", "old"));

    expect(editor.isContainer).toBe(false);
    expect(editor.containerRuntime).toBe("docker");
    expect(editor.containerTransport).toBe("local");
    expect(editor.containerSshSession).toBe("");
    expect(editor.isKubernetes).toBe(false);
    expect(editor.kubeSource).toBe("defaultPath");
    expect(editor.kubeContext).toBe("");
    // Neither block is written onto a session of another kind.
    expect(editor.draft().container).toBeUndefined();
    expect(editor.draft().kubernetes).toBeUndefined();
  });

  it("saves a container block only on container sessions and keeps typed values across a switch", () => {
    const editor = useSessionEditorStore();
    editor.fill({
      ...sample("dock", "dock"),
      protocol: "container",
      host: "",
      port: 0,
      user: "",
      container: { runtime: "podman", transport: "ssh", sshSession: "bastion" },
    });

    expect(editor.isContainer).toBe(true);
    expect(editor.draft().container).toEqual({ runtime: "podman", transport: "ssh", sshSession: "bastion" });
    // A container session has no endpoint of its own: the engine is found through the runtime.
    expect(editor.draft().host).toBe("");
    expect(editor.draft().user).toBe("");

    editor.setProtocol("ssh");
    expect(editor.draft().container).toBeUndefined();

    editor.setProtocol("container");
    expect(editor.containerRuntime).toBe("podman");
    expect(editor.containerTransport).toBe("ssh");
    expect(editor.containerSshSession).toBe("bastion");
    expect(editor.draft().container).toEqual({ runtime: "podman", transport: "ssh", sshSession: "bastion" });

    // The same rule the other protocols' fields follow: a switch is a change of question, never a
    // reason to throw away an answer — a pasted kubeconfig above all, which cannot be read back.
    editor.setProtocol("kubernetes");
    editor.kubeSource = "imported";
    editor.importedKubeconfig = "apiVersion: v1";
    editor.kubeContext = "prod";
    editor.setProtocol("ssh");
    expect(editor.draft().kubernetes).toBeUndefined();
    editor.setProtocol("kubernetes");
    expect(editor.kubeSource).toBe("imported");
    expect(editor.importedKubeconfig).toBe("apiVersion: v1");
    expect(editor.kubeContext).toBe("prod");
  });

  it("requires a saved SSH session only when the container engine is reached over SSH", () => {
    const editor = useSessionEditorStore();
    editor.fill({
      ...sample("dock", "dock"),
      protocol: "container",
      container: { runtime: "docker", transport: "ssh" },
    });

    expect(editor.sessionProblems.containerSshSession).toBe(true);
    expect(editor.hasProblems).toBe(true);

    editor.containerSshSession = "bastion";
    expect(editor.sessionProblems.containerSshSession).toBe(false);
    expect(editor.hasProblems).toBe(false);

    // A local engine has no route to name, so an empty session id is not a problem.
    editor.containerSshSession = "";
    editor.containerTransport = "local";
    expect(editor.sessionProblems.containerSshSession).toBe(false);
    expect(editor.draft().container).toEqual({
      runtime: "docker",
      transport: "local",
      sshSession: undefined,
      namespace: undefined,
    });
  });

  it("keeps a nerdctl namespace out of a session that runs another runtime", () => {
    const editor = useSessionEditorStore();
    editor.fill({
      ...sample("dock", "dock"),
      protocol: "container",
      container: { runtime: "nerdctl", transport: "local", namespace: "lab" },
    });

    expect(editor.draft().container).toMatchObject({ runtime: "nerdctl", namespace: "lab" });
    // A namespace is nerdctl's own axis; the other runtimes have nowhere to put one.
    editor.containerRuntime = "docker";
    expect(editor.draft().container?.namespace).toBeUndefined();
    expect(editor.containerNamespace).toBe("lab");
  });

  it("saves Kubernetes settings and sends a pasted kubeconfig as the session secret", async () => {
    const library = useSessionsStore();
    const editor = useSessionEditorStore();
    editor.fill({ ...sample("cluster", "cluster"), protocol: "kubernetes", host: "", port: 0, user: "" });

    expect(editor.isKubernetes).toBe(true);
    expect(editor.draft().kubernetes).toEqual({
      source: "defaultPath",
      context: undefined,
      namespace: undefined,
      viaSsh: undefined,
    });
    // Nothing pasted means "keep the stored kubeconfig", exactly like a password.
    expect(editor.secretInput()).toBeUndefined();

    editor.kubeSource = "imported";
    editor.importedKubeconfig = "apiVersion: v1\nkind: Config\n";
    editor.kubeContext = "prod";
    editor.kubeNamespace = "apps";
    editor.kubeViaSsh = "bastion";

    const kubernetes = { source: "imported", context: "prod", namespace: "apps", viaSsh: "bastion" };
    expect(editor.draft().kubernetes).toEqual(kubernetes);
    expect(editor.secretInput()).toEqual({ value: "apiVersion: v1\nkind: Config\n" });

    await library.save(editor.draft(), editor.secretInput());
    expect(sessionSaveMock).toHaveBeenCalledWith(expect.objectContaining({ protocol: "kubernetes", kubernetes }), {
      value: "apiVersion: v1\nkind: Config\n",
    });
  });

  it("requires a pasted kubeconfig on a new import and not when one is already stored", () => {
    const editor = useSessionEditorStore();
    editor.fill({ ...sample("new", "new"), protocol: "kubernetes" });
    editor.kubeSource = "imported";

    expect(editor.sessionProblems.kubeconfig).toBe(true);
    expect(editor.hasProblems).toBe(true);

    editor.importedKubeconfig = "apiVersion: v1";
    expect(editor.sessionProblems.kubeconfig).toBe(false);

    // An edited session that already holds a kubeconfig: the empty field keeps it rather than
    // replacing it, so it is not a problem.
    editor.fill({
      ...sample("stored", "stored"),
      protocol: "kubernetes",
      auth: { method: "password", hasSecret: true },
    });
    editor.kubeSource = "imported";
    expect(editor.kubeconfigStored).toBe(true);
    expect(editor.sessionProblems.kubeconfig).toBe(false);
    expect(editor.secretInput()).toBeUndefined();

    // An SSH session's stored password lives in the same secret slot but is not a kubeconfig, so
    // turning that session into a cluster session still has to ask for one.
    editor.fill({ ...sample("was-ssh", "was-ssh"), auth: { method: "password", hasSecret: true } });
    editor.setProtocol("kubernetes");
    editor.kubeSource = "imported";
    expect(editor.kubeconfigStored).toBe(false);
    expect(editor.sessionProblems.kubeconfig).toBe(true);
  });

  it("reads contexts from the pasted kubeconfig, and from the default path only when nothing is pasted", async () => {
    const editor = useSessionEditorStore();
    editor.fill({ ...sample("cluster", "cluster"), protocol: "kubernetes" });
    k8sContextsMock.mockResolvedValue([
      { name: "prod", cluster: "prod-cluster", user: "alice", namespace: "default", current: true },
    ]);

    await editor.readKubeContexts();
    expect(k8sContextsMock).toHaveBeenCalledWith(undefined);
    expect(editor.kubeContexts.map((context) => context.name)).toEqual(["prod"]);

    editor.kubeSource = "imported";
    // The stored document never comes back, so an empty textarea must not fall through to
    // whatever this machine's default kubeconfig says: that is a different cluster.
    await editor.readKubeContexts();
    expect(k8sContextsMock).toHaveBeenCalledTimes(1);

    editor.importedKubeconfig = "apiVersion: v1";
    await editor.readKubeContexts();
    expect(k8sContextsMock).toHaveBeenLastCalledWith({ value: "apiVersion: v1" });
  });

  it("keeps an unsupported read-contexts answer as an envelope instead of rejecting", async () => {
    const editor = useSessionEditorStore();
    editor.fill({ ...sample("cluster", "cluster"), protocol: "kubernetes" });
    k8sContextsMock.mockRejectedValueOnce(new Error("app_unsupported: the command is still a stub"));

    await expect(editor.readKubeContexts()).resolves.toBeUndefined();
    expect(editor.kubeContextsFailure?.code).toBeTruthy();
    expect(editor.kubeContextsBusy).toBe(false);
  });
});
