/**
 * Container wire types, mirroring `crates/sevenshell-types/src/container.rs`.
 *
 * Nothing here is a credential except the password on a registry login, and that one only travels
 * inward. Nothing here is a local path: archives are chosen in a native dialog and never named to
 * the interface.
 */

import type { SecretInput } from "@/types/session";

export type ContainerRuntime = "docker" | "podman" | "nerdctl" | "wslc";
export type ContainerTransport = "local" | "ssh";

export interface ContainerSettings {
  readonly runtime: ContainerRuntime;
  readonly transport: ContainerTransport;
  readonly sshSession?: string;
  readonly namespace?: string;
}

export interface ContainerSummary {
  readonly id: string;
  readonly name: string;
  readonly image: string;
  readonly state: string;
  readonly status: string;
  readonly ports: string;
  readonly createdAt: string;
}

export interface ContainerPort {
  readonly hostIp?: string;
  readonly hostPort: string;
  readonly containerPort: string;
  readonly protocol: string;
}

export interface ContainerMount {
  readonly source: string;
  readonly destination: string;
  readonly readOnly: boolean;
}

export interface ContainerDetail {
  readonly id: string;
  readonly name: string;
  readonly image: string;
  readonly state: string;
  readonly status: string;
  readonly startedAt?: string;
  readonly finishedAt?: string;
  readonly exitCode?: number;
  readonly oomKilled: boolean;
  readonly pid: number;
  readonly restartPolicy?: string;
  readonly entrypoint?: string;
  readonly command?: string;
  readonly workDir?: string;
  readonly user?: string;
  readonly networkMode?: string;
  readonly ip?: string;
  readonly gateway?: string;
  readonly ports: readonly ContainerPort[];
  readonly mounts: readonly ContainerMount[];
  readonly env: readonly string[];
  readonly raw: string;
}

export interface ContainerImage {
  readonly id: string;
  readonly repository: string;
  readonly tag: string;
  readonly size: string;
  readonly createdAt: string;
}

export interface ImageLayer {
  readonly id: string;
  readonly createdBy: string;
  readonly size: string;
  readonly createdAt: string;
}

export interface ContainerStats {
  readonly id: string;
  readonly name: string;
  readonly cpuPercent: number;
  readonly memoryUsage: number;
  readonly memoryLimit: number;
  readonly memoryPercent: number;
  readonly networkInput: number;
  readonly networkOutput: number;
  readonly blockInput: number;
  readonly blockOutput: number;
  readonly pids: number;
}

export interface ContainerStateCount {
  readonly state: string;
  readonly count: number;
}

export interface ContainerRuntimeInfo {
  readonly runtime: string;
  readonly clientVersion?: string;
  readonly clientComponent?: string;
  readonly serverVersion?: string;
  readonly serverComponent?: string;
  readonly storageDriver?: string;
  readonly cgroupDriver?: string;
  readonly cgroupVersion?: string;
  readonly os?: string;
  readonly arch?: string;
  readonly kernel?: string;
  readonly cpus: number;
  readonly memory: number;
  readonly containersTotal: number;
  readonly containersByState: readonly ContainerStateCount[];
  readonly imagesTotal: number;
  readonly imagesTagged: number;
  readonly imagesDangling: number;
}

export interface ContainerStreamLine {
  readonly streamId: string;
  readonly line: string;
}

export interface ContainerStreamEnd {
  readonly streamId: string;
  readonly error?: string;
}

export interface ContainerPortMapping {
  readonly hostIp?: string;
  readonly hostPort: number;
  readonly containerPort: number;
  readonly protocol?: string;
}

export interface ContainerVolume {
  readonly source: string;
  readonly destination: string;
}

export interface ContainerCreateRequest {
  readonly image: string;
  readonly name?: string;
  readonly ports?: readonly ContainerPortMapping[];
  readonly volumes?: readonly ContainerVolume[];
  readonly env?: readonly string[];
  readonly restartPolicy?: string;
  readonly command?: string;
  readonly interactive?: boolean;
  readonly tty?: boolean;
}

export interface ContainerImageTransfer {
  readonly image: string;
  readonly platform?: string;
  readonly allTags?: boolean;
  readonly insecure?: boolean;
}

export interface ContainerRegistryLogin {
  readonly registry?: string;
  readonly username: string;
  readonly secret: SecretInput;
  readonly insecure?: boolean;
}

export interface ContainerCommitRequest {
  readonly containerId: string;
  readonly image: string;
  readonly author?: string;
  readonly message?: string;
}
