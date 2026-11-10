/**
 * Kubernetes wire types, mirroring `crates/sevenshell-types/src/k8s.rs`.
 *
 * A resource is JSON the API server sent; this file types the envelope, not the object. The
 * kubeconfig document itself never appears here — it lives in the vault and only its contexts are
 * shown.
 */

export type KubeconfigSource = "defaultPath" | "imported";

export interface KubernetesSettings {
  readonly source: KubeconfigSource;
  readonly context?: string;
  readonly namespace?: string;
  readonly viaSsh?: string;
}

export interface KubeContextInfo {
  readonly name: string;
  readonly cluster: string;
  readonly user: string;
  readonly namespace: string;
  readonly current: boolean;
}

export type K8sMethod = "get" | "post" | "put" | "patch" | "delete";

export interface K8sRequest {
  readonly method: K8sMethod;
  readonly path: string;
  readonly body?: string;
  readonly contentType?: string;
  readonly acknowledged?: boolean;
}

export interface K8sResponse {
  readonly status: number;
  readonly body: string;
}

export interface K8sStreamLine {
  readonly streamId: string;
  readonly line: string;
}

export interface K8sStreamEnd {
  readonly streamId: string;
  readonly error?: string;
}

export interface K8sWatchEvent {
  readonly streamId: string;
  readonly kind: string;
  readonly object: string;
}

export interface K8sExecRequest {
  readonly namespace: string;
  readonly pod: string;
  readonly container: string;
  readonly command?: string;
}

export interface K8sLogRequest {
  readonly namespace: string;
  readonly pod: string;
  readonly container: string;
  readonly tailLines?: number;
  readonly timestamps?: boolean;
  readonly previous?: boolean;
}
