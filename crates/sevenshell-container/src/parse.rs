//! Reading what the runtime printed into the shapes the interface draws.
//!
//! Four runtimes answer the same question in four dialects: Docker prints a pipe-separated line per
//! container, Podman and nerdctl print JSON, and WSLC prints a JSON array whose state and protocol
//! are numbers. The parsing here is deliberately forgiving in one direction only: a line that does
//! not parse is skipped and the rest of the answer is still used. A single malformed row must not
//! turn a list of forty containers into an error dialog.
//!
//! The other direction is bounded. A runtime will happily print tens of megabytes, and the answer
//! travels to the interface, so input, row count, and every list inside a detail are capped here.

use serde_json::{Map, Value};
use sevenshell_error::ShellError;
use sevenshell_types::{
    ContainerDetail, ContainerImage, ContainerMount, ContainerPort, ContainerRuntime, ContainerRuntimeInfo,
    ContainerStateCount, ContainerStats, ContainerSummary, ImageLayer,
};

/// The largest answer that is still parsed. Beyond this the interface would be showing a table
/// nobody can read, and the tab is better off saying the output was too large.
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
/// A single JSON row longer than this is treated as a bad row rather than parsed.
const MAX_LINE_BYTES: usize = 1024 * 1024;
/// How many rows of any list are kept.
const MAX_ITEMS: usize = 10_000;
const MAX_ENV_ENTRIES: usize = 256;
const MAX_ENV_CHARS: usize = 4096;
const MAX_MOUNTS: usize = 256;
const MAX_PORTS: usize = 256;
const MAX_TEXT_CHARS: usize = 4096;
/// The inspection tab is a viewer, not an export.
const MAX_RAW_BYTES: usize = 512 * 1024;
const MAX_NAMESPACES: usize = 256;

/// What a runtime that never started anything prints for a time.
const ZERO_TIME: &str = "0001-01-01T00:00:00Z";

pub(crate) fn containers(runtime: ContainerRuntime, output: &str) -> Vec<ContainerSummary> {
    if runtime == ContainerRuntime::Docker {
        return docker_containers(output);
    }
    documents(output)
        .iter()
        .take(MAX_ITEMS)
        .map(container_summary)
        .collect()
}

fn docker_containers(output: &str) -> Vec<ContainerSummary> {
    let mut list = Vec::new();
    for line in capped(output).lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // `splitn` rather than `split`: the last field is a timestamp, and a value containing the
        // separator would otherwise shift every field after it.
        let fields: Vec<&str> = line.splitn(7, '|').collect();
        if fields.len() != 7 {
            log::debug!("[container] skipped a malformed ps line: {line}");
            continue;
        }
        list.push(ContainerSummary {
            id: fields[0].trim().to_owned(),
            image: fields[1].trim().to_owned(),
            name: fields[2].trim().to_owned(),
            state: normalize_state(fields[3], fields[4]),
            status: fields[4].trim().to_owned(),
            ports: fields[5].trim().to_owned(),
            created_at: normalize_timestamp(fields[6]),
        });
        if list.len() >= MAX_ITEMS {
            break;
        }
    }
    list
}

fn container_summary(map: &Map<String, Value>) -> ContainerSummary {
    let status = text(map, &["Status", "status"]);
    let mut ports = text(map, &["Ports", "ports"]);
    if ports.is_empty() {
        ports = port_list(map.get("Ports").or_else(|| map.get("ports")));
    }
    let name = list_text(map, &["Names", "names", "Name", "name"])
        .into_iter()
        .next()
        .unwrap_or_default();
    ContainerSummary {
        id: text(map, &["ID", "Id", "id", "ContainerID", "containerId"]),
        name,
        image: text(map, &["Image", "image"]),
        state: normalize_state(&text(map, &["State", "state"]), &status),
        status,
        ports,
        created_at: created_at(map),
    }
}

/// Podman prints ports as objects and WSLC prints them with numeric protocols; both are folded
/// into the same `0.0.0.0:8080->80/tcp` sentence Docker prints.
fn port_list(value: Option<&Value>) -> String {
    match value {
        Some(Value::Array(entries)) => {
            let mut parts = Vec::new();
            for entry in entries {
                match entry {
                    Value::Object(port) => parts.push(format_port_object(port)),
                    Value::String(text) => parts.push(text.clone()),
                    _ => {}
                }
            }
            parts.retain(|part| !part.is_empty());
            parts.join(", ")
        }
        Some(Value::String(text)) => text.clone(),
        _ => String::new(),
    }
}

fn format_port_object(port: &Map<String, Value>) -> String {
    let container_port = text(port, &["container_port", "ContainerPort"]);
    if container_port.is_empty() {
        return String::new();
    }
    let protocol = match text(port, &["protocol", "Protocol"]).as_str() {
        // WSLC emits the IANA numbers rather than the names.
        "6" => "tcp".to_owned(),
        "17" => "udp".to_owned(),
        "" => "tcp".to_owned(),
        other => other.to_owned(),
    };
    let host_port = text(port, &["host_port", "HostPort"]);
    if host_port.is_empty() {
        return format!("{container_port}/{protocol}");
    }
    let host_ip = match text(port, &["host_ip", "HostIp", "BindingAddress"]) {
        ip if ip.is_empty() => "0.0.0.0".to_owned(),
        ip => ip,
    };
    format!("{host_ip}:{host_port}->{container_port}/{protocol}")
}

pub(crate) fn images(output: &str) -> Vec<ContainerImage> {
    documents(output)
        .iter()
        .take(MAX_ITEMS)
        .map(|map| {
            let mut repository = text(map, &["Repository", "repository"]);
            if repository.is_empty() {
                repository = list_text(map, &["Names", "names"])
                    .into_iter()
                    .next()
                    .unwrap_or_default();
            }
            let mut tag = text(map, &["Tag", "tag"]);
            if repository.is_empty() {
                repository = "<none>".to_owned();
            }
            if tag.is_empty() {
                tag = "<none>".to_owned();
            }
            ContainerImage {
                id: text(map, &["ID", "Id", "id"]),
                repository,
                tag,
                size: size(map),
                created_at: created_at(map),
            }
        })
        .collect()
}

pub(crate) fn image_history(output: &str) -> Vec<ImageLayer> {
    documents(output)
        .iter()
        .take(MAX_ITEMS)
        .map(|map| ImageLayer {
            id: text(map, &["ID", "Id", "id"]),
            created_by: bounded(&text(map, &["CreatedBy", "createdBy"]), MAX_TEXT_CHARS),
            size: size(map),
            // "3 weeks ago" is what a person wants beside a layer; the absolute time is the fallback.
            created_at: match text(map, &["CreatedSince", "createdSince"]) {
                since if !since.is_empty() => since,
                _ => created_at(map),
            },
        })
        .collect()
}

/// WSLC has no `history`; the layer digests are all its image inspect carries. They are listed
/// bottom-up there and top-down here, matching what `history` prints.
pub(crate) fn image_layers_from_inspect(output: &str) -> Result<Vec<ImageLayer>, ShellError> {
    let value: Value =
        serde_json::from_str(capped(output)).map_err(|error| malformed(&format!("image inspect: {error}")))?;
    let Value::Array(items) = value else {
        return Err(malformed("image inspect did not return an array"));
    };
    let Some(Value::Object(first)) = items.into_iter().next() else {
        return Err(malformed("image inspect returned nothing"));
    };
    let root = first
        .get("RootFS")
        .or_else(|| first.get("rootfs"))
        .and_then(Value::as_object);
    let layers = root
        .and_then(|root| root.get("Layers").or_else(|| root.get("layers")))
        .and_then(Value::as_array);
    let mut list = Vec::new();
    if let Some(layers) = layers {
        for layer in layers.iter().rev().take(MAX_ITEMS) {
            if let Some(id) = layer.as_str()
                && !id.is_empty()
            {
                list.push(ImageLayer {
                    id: id.to_owned(),
                    created_by: String::new(),
                    size: String::new(),
                    created_at: String::new(),
                });
            }
        }
    }
    Ok(list)
}

pub(crate) fn stats(output: &str) -> Vec<ContainerStats> {
    documents(output)
        .iter()
        .take(MAX_ITEMS)
        .map(|map| {
            let name = match text(map, &["Name", "name"]) {
                name if !name.is_empty() => name,
                _ => list_text(map, &["Names", "names"])
                    .into_iter()
                    .next()
                    .unwrap_or_default(),
            };
            let (memory_usage, memory_limit) = split_pair(&text(map, &["MemUsage", "mem_usage", "MemUsageBytes"]));
            let (network_input, network_output) = io_pair(
                map,
                &["NetIO", "net_io"],
                &["NetInput", "net_input"],
                &["NetOutput", "net_output"],
            );
            let (block_input, block_output) = io_pair(
                map,
                &["BlockIO", "block_io"],
                &["BlockInput", "block_input"],
                &["BlockOutput", "block_output"],
            );
            ContainerStats {
                id: text(map, &["ID", "Container", "ContainerID", "id"]),
                name,
                cpu_percent: percent(&text(map, &["CPUPerc", "CPUPercent", "cpu_percent", "cpu"])),
                memory_usage,
                memory_limit,
                memory_percent: percent(&text(map, &["MemPerc", "MemPercent", "mem_percent"])),
                network_input,
                network_output,
                block_input,
                block_output,
                pids: size_of(map, &["PIDs", "pids"]),
            }
        })
        .collect()
}

/// nerdctl prints a table; the first column is the name and the header is optional.
pub(crate) fn namespaces(output: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in capped(output).lines() {
        let Some(first) = line.split_whitespace().next() else {
            continue;
        };
        if first.eq_ignore_ascii_case("NAME") || first.eq_ignore_ascii_case("NAMESPACE") {
            continue;
        }
        if !names.iter().any(|existing: &String| existing == first) {
            names.push(first.to_owned());
        }
        if names.len() >= MAX_NAMESPACES {
            break;
        }
    }
    names
}

pub(crate) fn runtime_info(
    runtime: ContainerRuntime,
    info: Option<&str>,
    version: Option<&str>,
) -> ContainerRuntimeInfo {
    let mut result = ContainerRuntimeInfo {
        runtime: runtime.as_str().to_owned(),
        client_version: String::new(),
        client_component: String::new(),
        server_version: String::new(),
        server_component: String::new(),
        storage_driver: String::new(),
        cgroup_driver: String::new(),
        cgroup_version: String::new(),
        os: String::new(),
        arch: String::new(),
        kernel: String::new(),
        cpus: 0,
        memory: 0,
        containers_total: 0,
        containers_by_state: Vec::new(),
        images_total: 0,
        images_tagged: 0,
        images_dangling: 0,
    };

    if let Some(document) = documents(info.unwrap_or_default()).into_iter().next() {
        apply_info(&document, &mut result);
    }
    if let Some(document) = documents(version.unwrap_or_default()).into_iter().next() {
        apply_version(&document, &mut result);
    }

    if result.client_version.is_empty() {
        result.client_version = result.server_version.clone();
    }
    if result.client_component.is_empty() {
        result.client_component = runtime.as_str().to_owned();
    }
    if result.server_component.is_empty() {
        result.server_component = default_server_component(runtime).to_owned();
    }
    result
}

fn default_server_component(runtime: ContainerRuntime) -> &'static str {
    match runtime {
        ContainerRuntime::Nerdctl => "containerd",
        other => other.as_str(),
    }
}

fn apply_info(map: &Map<String, Value>, result: &mut ContainerRuntimeInfo) {
    // Podman nests almost everything; Docker and nerdctl answer flat. Both are read in the same
    // pass, with the nested value winning where it exists.
    let host = child(map, &["host"]);
    let store = child(map, &["store"]);
    let version = child(map, &["version"]);

    if let Some(host) = host {
        result.os = match child(host, &["Distribution", "distribution"]) {
            Some(distribution) => {
                let name = text(distribution, &["Distribution", "distribution", "Name", "name"]);
                let version = text(distribution, &["Version", "version"]);
                format!("{name} {version}").trim().to_owned()
            }
            None => text(host, &["OS", "Os", "os"]),
        };
        result.arch = text(host, &["Arch", "arch", "Architecture", "architecture"]);
        result.kernel = text(host, &["Kernel", "kernel", "KernelVersion", "kernelVersion"]);
        result.cgroup_driver = text(
            host,
            &["CgroupsDriver", "cgroupsDriver", "CgroupDriver", "cgroupDriver"],
        );
        result.cgroup_version = text(host, &["CgroupsVersion", "cgroupsVersion", "CgroupVersion"]);
        result.cpus = count(host, &["CPUs", "cpus", "NCPU", "ncpu"]);
        result.memory = size_of(host, &["Memory", "memory"]);
    }
    if let Some(store) = store {
        result.storage_driver = text(store, &["GraphDriverName", "graphDriverName", "Driver", "driver"]);
        if let Some(containers) = child(store, &["containerStore", "ContainerStore"]) {
            result.containers_total = count(containers, &["number", "Number"]);
            push_state(result, "running", count(containers, &["running", "Running"]));
            push_state(result, "paused", count(containers, &["paused", "Paused"]));
            push_state(result, "stopped", count(containers, &["stopped", "Stopped"]));
        }
        if let Some(images) = child(store, &["imageStore", "ImageStore"]) {
            result.images_total = count(images, &["number", "Number"]);
        }
    }
    if let Some(version) = version {
        result.server_version = text(version, &["Version", "version"]);
    }

    seed(
        &mut result.server_version,
        text(map, &["ServerVersion", "serverVersion"]),
    );
    seed(&mut result.os, text(map, &["OperatingSystem", "operatingSystem"]));
    seed(&mut result.arch, text(map, &["Architecture", "architecture"]));
    seed(&mut result.kernel, text(map, &["KernelVersion", "kernelVersion"]));
    seed(&mut result.storage_driver, text(map, &["Driver", "driver"]));
    seed(&mut result.cgroup_driver, text(map, &["CgroupDriver", "cgroupDriver"]));
    seed(
        &mut result.cgroup_version,
        text(map, &["CgroupVersion", "cgroupVersion"]),
    );
    if result.cpus == 0 {
        result.cpus = count(map, &["NCPU", "ncpu"]);
    }
    if result.memory == 0 {
        result.memory = size_of(map, &["MemTotal", "memTotal"]);
    }

    if result.containers_total == 0 {
        result.containers_total = count(map, &["Containers", "containers"]);
    }
    if result.containers_by_state.is_empty() {
        push_state(
            result,
            "running",
            count(map, &["ContainersRunning", "containersRunning"]),
        );
        push_state(result, "paused", count(map, &["ContainersPaused", "containersPaused"]));
        push_state(
            result,
            "stopped",
            count(map, &["ContainersStopped", "containersStopped"]),
        );
        push_state(
            result,
            "created",
            count(map, &["ContainersCreated", "containersCreated"]),
        );
    }
    if result.images_total == 0 {
        result.images_total = count(map, &["Images", "images"]);
    }
    result.images_tagged = result.images_tagged.max(count(map, &["ImagesTagged", "imagesTagged"]));
    result.images_dangling = result
        .images_dangling
        .max(count(map, &["ImagesDangling", "imagesDangling"]));
}

fn push_state(result: &mut ContainerRuntimeInfo, state: &str, count: u32) {
    if count == 0 {
        return;
    }
    result.containers_by_state.push(ContainerStateCount {
        state: state.to_owned(),
        count,
    });
}

fn apply_version(map: &Map<String, Value>, result: &mut ContainerRuntimeInfo) {
    if let Some(client) = child(map, &["Client", "client"]) {
        result.client_version = text(client, &["Version", "version"]);
        result.client_component = component_name(client).unwrap_or_default();
    }
    if result.client_version.is_empty() {
        result.client_version = text(map, &["Version", "version"]);
    }

    // nerdctl reports one server object per backend, so `Server` is either an object or an array.
    match map.get("Server").or_else(|| map.get("server")) {
        Some(Value::Object(server)) => read_server(server, result),
        Some(Value::Array(servers)) => {
            for server in servers {
                if let Value::Object(server) = server {
                    read_server(server, result);
                }
            }
        }
        _ => {}
    }
}

fn read_server(server: &Map<String, Value>, result: &mut ContainerRuntimeInfo) {
    if result.server_version.is_empty() {
        result.server_version = text(server, &["Version", "version"]);
    }
    if result.server_component.is_empty()
        && let Some(name) = component_name(server)
    {
        result.server_component = name;
    }
    // The engine version is what a person means by "server version" for Docker; for nerdctl the
    // component list carries containerd instead.
    if let Some(components) = server.get("Components").or_else(|| server.get("components"))
        && let Some(list) = components.as_array()
    {
        let containerd = list
            .iter()
            .filter_map(Value::as_object)
            .find(|component| text(component, &["Name", "name"]) == "containerd");
        if let Some(containerd) = containerd
            && result.server_version.is_empty()
        {
            result.server_version = text(containerd, &["Version", "version"]);
        }
    }
}

fn component_name(map: &Map<String, Value>) -> Option<String> {
    let components = map.get("Components").or_else(|| map.get("components"))?.as_array()?;
    let first = components.first()?.as_object()?;
    let name = text(first, &["Name", "name"]);
    (!name.is_empty()).then_some(name)
}

pub(crate) fn detail(output: &str) -> Result<ContainerDetail, ShellError> {
    let value: Value = serde_json::from_str(capped(output)).map_err(|error| malformed(&format!("inspect: {error}")))?;
    let Value::Array(items) = value else {
        return Err(malformed("inspect did not return an array"));
    };
    let Some(Value::Object(map)) = items.into_iter().next() else {
        return Err(malformed("inspect returned nothing"));
    };

    let config = child(&map, &["Config", "config"]);
    let state = child(&map, &["State", "state"]);
    let host = child(&map, &["HostConfig", "hostConfig", "Hostconfig"]);
    let network = child(&map, &["NetworkSettings", "networkSettings"]);

    let section = |section: Option<&Map<String, Value>>, keys: &[&str]| {
        section.map(|section| text(section, keys)).unwrap_or_default()
    };

    let status = section(state, &["Status", "status"]);
    let (ip, gateway) = network_addresses(network, &map);

    let mut detail = ContainerDetail {
        id: text(&map, &["Id", "ID", "id"]),
        name: text(&map, &["Name", "name"]).trim_start_matches('/').to_owned(),
        image: section(config, &["Image", "image"]),
        state: status.clone(),
        status,
        started_at: normalize_timestamp(&section(state, &["StartedAt", "startedAt"])),
        finished_at: normalize_timestamp(&section(state, &["FinishedAt", "finishedAt"])),
        exit_code: state
            .and_then(|state| pick(state, &["ExitCode", "exitCode"]))
            .and_then(value_i64),
        oom_killed: state
            .and_then(|state| pick(state, &["OOMKilled", "oomKilled"]))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        pid: state
            .and_then(|state| pick(state, &["Pid", "pid"]))
            .and_then(value_u64)
            .unwrap_or(0),
        restart_policy: restart_policy(host, &map),
        entrypoint: join_list(config.and_then(|config| pick(config, &["Entrypoint", "entrypoint"]))),
        command: join_list(config.and_then(|config| pick(config, &["Cmd", "cmd", "Command", "command"]))),
        work_dir: section(config, &["WorkingDir", "workingDir", "workDir"]),
        user: section(config, &["User", "user"]),
        network_mode: host
            .map(|host| text(host, &["NetworkMode", "networkMode"]))
            .unwrap_or_default(),
        ip,
        gateway,
        ports: collect_ports(network, &map),
        mounts: collect_mounts(&map),
        env: collect_env(config),
        raw: bounded(capped(output), MAX_RAW_BYTES),
    };
    detail.entrypoint = bounded(&detail.entrypoint, MAX_TEXT_CHARS);
    detail.command = bounded(&detail.command, MAX_TEXT_CHARS);
    Ok(detail)
}

fn restart_policy(host: Option<&Map<String, Value>>, map: &Map<String, Value>) -> String {
    if let Some(host) = host {
        match host.get("RestartPolicy").or_else(|| host.get("restartPolicy")) {
            Some(Value::Object(policy)) => return text(policy, &["Name", "name"]),
            Some(Value::String(policy)) => return policy.clone(),
            _ => {}
        }
        let named = text(host, &["RestartPolicyName", "restartPolicyName"]);
        if !named.is_empty() {
            return named;
        }
    }
    text(map, &["RestartPolicy", "restartPolicy", "restartPolicyName"])
}

fn network_addresses(network: Option<&Map<String, Value>>, map: &Map<String, Value>) -> (String, String) {
    let direct = |section: Option<&Map<String, Value>>| {
        section
            .map(|section| {
                let ip = text(section, &["IPAddress", "ipAddress", "ip"]);
                let gateway = text(section, &["Gateway", "gateway"]);
                (ip, gateway)
            })
            .unwrap_or_default()
    };
    let (mut ip, mut gateway) = direct(network);
    if !ip.is_empty() {
        return (ip, gateway);
    }
    // The OCI schema nests one settings object per network and has no top-level address.
    let networks = network
        .and_then(|network| network.get("networks").or_else(|| network.get("Networks")))
        .or_else(|| map.get("networks").or_else(|| map.get("Networks")))
        .and_then(Value::as_object);
    if let Some(networks) = networks {
        for settings in networks.values() {
            if let Some(settings) = settings.as_object() {
                if ip.is_empty() {
                    ip = text(settings, &["IPAddress", "ipAddress", "ip"]);
                }
                if gateway.is_empty() {
                    gateway = text(settings, &["Gateway", "gateway"]);
                }
            }
        }
    }
    (ip, gateway)
}

fn collect_ports(network: Option<&Map<String, Value>>, map: &Map<String, Value>) -> Vec<ContainerPort> {
    let mut ports = Vec::new();
    // Docker: a map of `80/tcp` to bindings; a null binding means exposed but not published.
    if let Some(bindings) = network
        .and_then(|network| network.get("Ports").or_else(|| network.get("ports")))
        .and_then(Value::as_object)
    {
        for (key, value) in bindings {
            let (container_port, protocol) = match key.split_once('/') {
                Some((port, protocol)) => (port.to_owned(), protocol.to_owned()),
                None => (key.clone(), "tcp".to_owned()),
            };
            let Some(entries) = value.as_array() else {
                continue;
            };
            for entry in entries {
                let Some(entry) = entry.as_object() else {
                    continue;
                };
                ports.push(ContainerPort {
                    host_ip: text(entry, &["HostIp", "hostIp", "HostIP", "host_ip"]),
                    host_port: text(entry, &["HostPort", "hostPort"]),
                    container_port: container_port.clone(),
                    protocol: protocol.clone(),
                });
            }
        }
    }
    // The OCI schema: a flat array at the top level.
    if ports.is_empty()
        && let Some(entries) = map.get("ports").or_else(|| map.get("Ports")).and_then(Value::as_array)
    {
        for entry in entries {
            let Some(entry) = entry.as_object() else {
                continue;
            };
            ports.push(ContainerPort {
                host_ip: text(entry, &["hostIP", "hostIp", "host_ip"]),
                host_port: text(entry, &["hostPort", "host_port"]),
                container_port: text(entry, &["containerPort", "container_port"]),
                protocol: match text(entry, &["protocol"]).as_str() {
                    "" => "tcp".to_owned(),
                    other => other.to_owned(),
                },
            });
        }
    }
    ports.sort_by(|left, right| {
        let left_port = left.container_port.parse::<u64>().unwrap_or(u64::MAX);
        let right_port = right.container_port.parse::<u64>().unwrap_or(u64::MAX);
        left_port
            .cmp(&right_port)
            .then_with(|| left.protocol.cmp(&right.protocol))
            .then_with(|| left.host_port.cmp(&right.host_port))
    });
    ports.truncate(MAX_PORTS);
    ports
}

fn collect_mounts(map: &Map<String, Value>) -> Vec<ContainerMount> {
    let mut mounts = Vec::new();
    let Some(entries) = map
        .get("Mounts")
        .or_else(|| map.get("mounts"))
        .and_then(Value::as_array)
    else {
        return mounts;
    };
    for entry in entries.iter().take(MAX_MOUNTS) {
        let Some(entry) = entry.as_object() else {
            continue;
        };
        let read_only = match entry.get("RW").or_else(|| entry.get("rw")).and_then(Value::as_bool) {
            Some(read_write) => !read_write,
            None => entry
                .get("ReadOnly")
                .or_else(|| entry.get("readOnly"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };
        mounts.push(ContainerMount {
            source: text(entry, &["Source", "source"]),
            destination: text(entry, &["Destination", "destination"]),
            read_only,
        });
    }
    mounts
}

fn collect_env(config: Option<&Map<String, Value>>) -> Vec<String> {
    let Some(entries) = config
        .and_then(|config| config.get("Env").or_else(|| config.get("env")))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    entries
        .iter()
        .take(MAX_ENV_ENTRIES)
        .filter_map(Value::as_str)
        .map(|entry| bounded(entry, MAX_ENV_CHARS))
        .collect()
}

fn documents(output: &str) -> Vec<Map<String, Value>> {
    let output = capped(output).trim();
    if output.is_empty() {
        return Vec::new();
    }
    // WSLC prints one array; Podman prints one array; Docker and nerdctl print one object per
    // line. Podman's `info` is a single pretty-printed object, so a whole-document object is tried
    // before falling back to line-by-line. Trying the array first and then falling back covers
    // both without asking which runtime produced the text.
    if output.starts_with('[')
        && let Ok(Value::Array(items)) = serde_json::from_str::<Value>(output)
    {
        return items.into_iter().take(MAX_ITEMS).filter_map(object).collect();
    }
    if output.starts_with('{')
        && let Ok(Value::Object(map)) = serde_json::from_str::<Value>(output)
    {
        return vec![map];
    }
    let mut list = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.len() > MAX_LINE_BYTES {
            log::debug!("[container] skipped an oversized output line");
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(value) => match object(value) {
                Some(map) => list.push(map),
                None => log::debug!("[container] skipped a JSON line that is not an object"),
            },
            Err(error) => log::debug!("[container] skipped a bad JSON line: {error}"),
        }
        if list.len() >= MAX_ITEMS {
            break;
        }
    }
    list
}

fn object(value: Value) -> Option<Map<String, Value>> {
    match value {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

fn capped(output: &str) -> &str {
    if output.len() <= MAX_OUTPUT_BYTES {
        return output;
    }
    let mut end = MAX_OUTPUT_BYTES;
    while end > 0 && !output.is_char_boundary(end) {
        end -= 1;
    }
    // Cutting at the last whole line keeps a half-written row out of the parser.
    match output[..end].rfind('\n') {
        Some(index) => &output[..index],
        None => &output[..end],
    }
}

fn bounded(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut bounded: String = text.chars().take(limit).collect();
    bounded.push_str("...");
    bounded
}

fn child<'a>(map: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Map<String, Value>> {
    keys.iter().find_map(|key| map.get(*key).and_then(Value::as_object))
}

/// Fills a field only when the nested shape did not already answer it.
fn seed(field: &mut String, value: String) {
    if field.is_empty() {
        *field = value;
    }
}

fn pick<'a>(map: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|key| map.get(*key))
        .filter(|value| !value.is_null())
}

fn text(map: &Map<String, Value>, keys: &[&str]) -> String {
    match pick(map, keys) {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn list_text(map: &Map<String, Value>, keys: &[&str]) -> Vec<String> {
    match pick(map, keys) {
        Some(Value::String(value)) if !value.is_empty() => vec![value.clone()],
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        _ => Vec::new(),
    }
}

fn join_list(value: Option<&Value>) -> String {
    match value {
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).collect::<Vec<&str>>().join(" "),
        Some(Value::String(value)) => value.clone(),
        _ => String::new(),
    }
}

fn value_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn value_u64(value: &Value) -> Option<u64> {
    value_i64(value).and_then(|value| u64::try_from(value).ok())
}

fn count(map: &Map<String, Value>, keys: &[&str]) -> u32 {
    pick(map, keys)
        .and_then(value_i64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0)
}

fn size_of(map: &Map<String, Value>, keys: &[&str]) -> u64 {
    match pick(map, keys) {
        Some(Value::Number(number)) => number.as_u64().unwrap_or_else(|| number.as_f64().unwrap_or(0.0) as u64),
        Some(Value::String(value)) => parse_size(value),
        _ => 0,
    }
}

fn size(map: &Map<String, Value>) -> String {
    match pick(map, &["Size", "size"]) {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(number)) => human_size(number.as_i64().unwrap_or(0)),
        _ => String::new(),
    }
}

fn human_size(bytes: i64) -> String {
    const UNITS: [char; 6] = ['K', 'M', 'G', 'T', 'P', 'E'];
    if bytes < 1024 {
        return format!("{}B", bytes.max(0));
    }
    let mut divisor = 1024_i64;
    let mut exponent = 0_usize;
    while exponent + 1 < UNITS.len() && bytes / divisor >= 1024 {
        divisor *= 1024;
        exponent += 1;
    }
    format!("{:.1}{}B", bytes as f64 / divisor as f64, UNITS[exponent])
}

/// Reads "1.395MiB / 15.04GiB" — the shape Docker and Podman print for memory.
fn split_pair(text: &str) -> (u64, u64) {
    let mut parts = text.split('/');
    let first = parse_size(parts.next().unwrap_or_default());
    let second = parts.next().map(parse_size).unwrap_or(0);
    (first, second)
}

fn io_pair(map: &Map<String, Value>, combined: &[&str], input: &[&str], output: &[&str]) -> (u64, u64) {
    let combined = text(map, combined);
    if !combined.trim().is_empty() {
        return split_pair(&combined);
    }
    (size_of(map, input), size_of(map, output))
}

/// Accepts the human strings the CLIs print and bare byte counts (WSLC prints numbers).
fn parse_size(text: &str) -> u64 {
    let text = text.trim();
    if text.is_empty() || text == "-" || text.eq_ignore_ascii_case("n/a") {
        return 0;
    }
    let number: String = text
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect();
    let unit: String = text[number.len()..].trim().to_ascii_lowercase();
    let Ok(value) = number.parse::<f64>() else {
        return 0;
    };
    let multiplier = match unit.as_str() {
        "" | "b" => 1.0,
        "k" | "kb" => 1_000.0,
        "kib" => 1024.0,
        "m" | "mb" => 1_000_000.0,
        "mib" => 1024.0 * 1024.0,
        "g" | "gb" => 1_000_000_000.0,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tb" => 1_000_000_000_000.0,
        "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        "p" | "pb" => 1_000_000_000_000_000.0,
        "pib" => 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return 0,
    };
    (value * multiplier) as u64
}

fn percent(text: &str) -> f64 {
    text.trim().trim_end_matches('%').parse::<f64>().unwrap_or(0.0)
}

fn created_at(map: &Map<String, Value>) -> String {
    match pick(
        map,
        &[
            "CreatedAt",
            "Created",
            "CreatedSince",
            "created",
            "createdAt",
            "createdSince",
        ],
    ) {
        Some(Value::String(value)) if !value.trim().is_empty() => normalize_timestamp(value),
        Some(Value::Number(number)) => {
            let seconds = number.as_i64().unwrap_or(0);
            unix_timestamp(seconds)
        }
        _ => String::new(),
    }
}

/// Turns the three shapes the runtimes print into one `YYYY-MM-DD HH:MM:SS`.
///
/// Docker prints `2026-08-24 09:15:00 +0800 CST`, nerdctl prints RFC 3339, and Podman and WSLC
/// print a Unix timestamp. Only the wall clock is kept: converting an offset needs a time zone
/// database this crate does not carry, and the runtime already printed the time in the zone it
/// considered local.
fn normalize_timestamp(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() || raw == ZERO_TIME {
        return String::new();
    }
    let bytes = raw.as_bytes();
    if bytes.len() >= 19
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && (bytes[10] == b'T' || bytes[10] == b' ')
        && bytes[11..19].iter().enumerate().all(|(index, byte)| {
            if index == 2 || index == 5 {
                *byte == b':'
            } else {
                byte.is_ascii_digit()
            }
        })
    {
        return format!("{} {}", &raw[..10], &raw[11..19]);
    }
    raw.to_owned()
}

fn unix_timestamp(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let remainder = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_date(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        remainder / 3_600,
        (remainder % 3_600) / 60,
        remainder % 60
    )
}

/// Days since the Unix epoch to a calendar date, by the algorithm the `chrono` crate documents
/// (Howard Hinnant's `civil_from_days`). A dependency for one conversion is not worth it.
fn civil_date(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 { shifted } else { shifted - 146_096 } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    (year + i64::from(month <= 2), month as u32, day as u32)
}

/// Folds the four vocabularies into the words the interface switches on.
fn normalize_state(state: &str, status: &str) -> String {
    let state = state.trim();
    match state {
        // WSLC prints the OCI status as a number and has no human sentence beside it.
        "0" | "1" => return "created".to_owned(),
        "2" => return "running".to_owned(),
        "3" | "5" => return "exited".to_owned(),
        "4" => return "paused".to_owned(),
        _ => {}
    }
    let lower = state.to_ascii_lowercase();
    match lower.as_str() {
        "running" => return "running".to_owned(),
        "stopped" | "terminated" | "exited" => return "exited".to_owned(),
        "paused" => return "paused".to_owned(),
        "created" | "restarting" => return state.to_owned(),
        _ => {}
    }
    if lower.starts_with("exited") {
        return "exited".to_owned();
    }
    if lower.starts_with("paused") {
        return "paused".to_owned();
    }
    let status = status.trim();
    if status.starts_with("Up") || status.eq_ignore_ascii_case("running") {
        return "running".to_owned();
    }
    if status.starts_with("Exited")
        || status.eq_ignore_ascii_case("exited")
        || status.eq_ignore_ascii_case("stopped")
        || status.eq_ignore_ascii_case("terminated")
    {
        return "exited".to_owned();
    }
    if status.starts_with("Paused") || status.eq_ignore_ascii_case("paused") {
        return "paused".to_owned();
    }
    if !status.is_empty() {
        return "unknown".to_owned();
    }
    state.to_owned()
}

fn malformed(detail: &str) -> ShellError {
    ShellError::container_command_failed("the inspect request", detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCKER_PS: &str = include_str!("../testdata/docker_ps.txt");
    const PODMAN_PS: &str = include_str!("../testdata/podman_ps.json");
    const NERDCTL_PS: &str = include_str!("../testdata/nerdctl_ps.jsonl");
    const WSLC_PS: &str = include_str!("../testdata/wslc_ps.json");
    const DOCKER_IMAGES: &str = include_str!("../testdata/docker_images.jsonl");
    const PODMAN_IMAGES: &str = include_str!("../testdata/podman_images.json");
    const WSLC_IMAGES: &str = include_str!("../testdata/wslc_images.json");
    const DOCKER_STATS: &str = include_str!("../testdata/docker_stats.jsonl");
    const PODMAN_STATS: &str = include_str!("../testdata/podman_stats.json");
    const DOCKER_INSPECT: &str = include_str!("../testdata/docker_inspect.json");
    const WSLC_INSPECT: &str = include_str!("../testdata/wslc_inspect.json");
    const DOCKER_HISTORY: &str = include_str!("../testdata/docker_history.jsonl");
    const WSLC_IMAGE_INSPECT: &str = include_str!("../testdata/wslc_image_inspect.json");
    const NERDCTL_NAMESPACES: &str = include_str!("../testdata/nerdctl_namespaces.txt");
    const DOCKER_INFO: &str = include_str!("../testdata/docker_info.json");
    const DOCKER_VERSION: &str = include_str!("../testdata/docker_version.json");
    const PODMAN_INFO: &str = include_str!("../testdata/podman_info.json");
    const PODMAN_VERSION: &str = include_str!("../testdata/podman_version.json");
    const NERDCTL_VERSION: &str = include_str!("../testdata/nerdctl_version.json");

    #[test]
    fn docker_ps_lines_become_rows_and_a_bad_line_is_skipped() {
        let rows = containers(ContainerRuntime::Docker, DOCKER_PS);
        assert_eq!(rows.len(), 3, "a malformed line should be skipped, not fatal: {rows:?}");
        assert_eq!(rows[0].id, "7c1a2b3c4d5e");
        assert_eq!(rows[0].name, "web");
        assert_eq!(rows[0].image, "nginx:1.27");
        assert_eq!(rows[0].state, "running");
        assert_eq!(rows[0].ports, "0.0.0.0:8080->80/tcp");
        assert_eq!(rows[0].created_at, "2026-08-24 09:15:00");
        assert_eq!(rows[1].state, "exited");
    }

    #[test]
    fn podman_ps_object_ports_and_names_are_flattened() {
        let rows = containers(ContainerRuntime::Podman, PODMAN_PS);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "edge-proxy");
        assert_eq!(rows[0].state, "running");
        assert_eq!(rows[0].ports, "127.0.0.1:8443->443/tcp");
        assert_eq!(rows[0].created_at, "2026-08-21 08:39:51");
        assert_eq!(rows[1].state, "exited");
    }

    #[test]
    fn nerdctl_ps_is_read_as_one_object_per_line() {
        let rows = containers(ContainerRuntime::Nerdctl, NERDCTL_PS);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].id, "a3f9c2d1e8b7");
        assert_eq!(rows[1].state, "paused");
        assert_eq!(rows[1].status, "Up 5 minutes (Paused)");
    }

    #[test]
    fn wslc_ps_numbers_become_states_and_ports() {
        let rows = containers(ContainerRuntime::Wslc, WSLC_PS);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].state, "running");
        assert_eq!(rows[0].ports, "127.0.0.1:18888->8888/tcp");
        assert_eq!(rows[0].created_at, "2026-08-24 11:40:32");
        assert_eq!(rows[1].state, "exited");
        assert_eq!(rows[2].ports, "");
    }

    #[test]
    fn image_sizes_arrive_as_text_or_as_bytes_and_leave_as_text() {
        let rows = images(DOCKER_IMAGES);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].repository, "nginx");
        assert_eq!(rows[0].tag, "1.27");
        assert_eq!(rows[0].size, "192MB");
        assert_eq!(rows[0].created_at, "2026-08-24 09:00:00");

        let rows = images(PODMAN_IMAGES);
        assert_eq!(rows[0].size, "178.3MB", "a byte count should be human readable");
        let rows = images(WSLC_IMAGES);
        assert_eq!(rows[1].repository, "redis");
        assert_eq!(rows[1].tag, "7");
        assert_eq!(rows[1].size, "49.6MB");
    }

    #[test]
    fn images_without_a_tag_are_shown_where_a_person_would_look_for_one() {
        let rows = images("{\"ID\":\"abc\",\"Repository\":\"\",\"Tag\":\"\"}\n");
        assert_eq!(rows[0].repository, "<none>");
        assert_eq!(rows[0].tag, "<none>");
    }

    #[test]
    fn stats_sentences_become_numbers() {
        let rows = stats(DOCKER_STATS);
        assert_eq!(rows.len(), 2);
        let first = &rows[0];
        assert_eq!(first.id, "7c1a2b3c4d5e");
        assert_eq!(first.cpu_percent, 0.42);
        assert_eq!(first.memory_usage, 1_462_763);
        assert!((first.memory_limit as f64 - 16.0 * 1024.0 * 1024.0 * 1024.0).abs() < 1024.0);
        assert_eq!(first.memory_percent, 0.01);
        assert_eq!(first.network_input, 74_600);
        assert_eq!(first.network_output, 0);
        assert_eq!(first.pids, 1);
    }

    #[test]
    fn podman_stats_are_read_from_the_separate_input_and_output_fields() {
        let rows = stats(PODMAN_STATS);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "edge-proxy");
        assert_eq!(rows[0].block_input, 4096);
        assert_eq!(rows[0].block_output, 8192);
        assert_eq!(rows[0].network_input, 1024);
        assert_eq!(rows[0].network_output, 2048);
    }

    #[test]
    fn docker_inspect_is_read_from_its_pascal_case_fields() {
        let detail = detail(DOCKER_INSPECT).unwrap();
        assert_eq!(detail.id, "7c1a2b3c4d5e");
        assert_eq!(detail.name, "web");
        assert_eq!(detail.image, "nginx:1.27");
        assert_eq!(detail.state, "running");
        assert_eq!(detail.exit_code, Some(0));
        assert!(!detail.oom_killed);
        assert_eq!(detail.pid, 4211);
        assert_eq!(detail.restart_policy, "unless-stopped");
        assert_eq!(detail.entrypoint, "/docker-entrypoint.sh");
        assert_eq!(detail.command, "nginx -g daemon off;");
        assert_eq!(detail.work_dir, "/usr/share/nginx/html");
        assert_eq!(detail.user, "nginx");
        assert_eq!(detail.network_mode, "bridge");
        assert_eq!(detail.ip, "172.18.0.4");
        assert_eq!(detail.gateway, "172.18.0.1");
        assert_eq!(detail.started_at, "2026-08-24 01:15:03");
        // A published port and an exposed-but-unpublished one: only the first is a port mapping.
        assert_eq!(detail.ports.len(), 1);
        assert_eq!(detail.ports[0].container_port, "80");
        assert_eq!(detail.ports[0].host_port, "8080");
        assert_eq!(detail.mounts.len(), 1);
        assert!(detail.mounts[0].read_only);
        assert_eq!(detail.env, vec!["MODE=fast", "TZ=UTC"]);
        assert!(detail.raw.contains("\"Id\""));
    }

    #[test]
    fn oci_inspect_is_read_from_its_lowercase_fields() {
        let detail = detail(WSLC_INSPECT).unwrap();
        assert_eq!(detail.id, "0c137eb750f2");
        assert_eq!(detail.name, "edge-proxy");
        assert_eq!(detail.state, "running");
        assert_eq!(detail.pid, 991);
        assert!(!detail.oom_killed);
        assert_eq!(detail.ip, "172.20.0.7");
        assert_eq!(detail.gateway, "172.20.0.1");
        assert_eq!(detail.restart_policy, "always");
        assert_eq!(detail.ports.len(), 1);
        assert_eq!(detail.ports[0].container_port, "443");
        assert_eq!(detail.ports[0].host_port, "8443");
        assert_eq!(detail.mounts.len(), 1);
        assert!(!detail.mounts[0].read_only);
        assert_eq!(detail.env, vec!["EDGE=1"]);
    }

    #[test]
    fn inspect_that_is_not_an_array_is_an_error_and_not_an_empty_detail() {
        assert!(detail("not json").is_err());
        assert!(detail("{}").is_err());
        assert!(detail("[]").is_err());
    }

    #[test]
    fn history_prefers_the_relative_time_people_read() {
        let rows = image_history(DOCKER_HISTORY);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "sha256:1111");
        assert_eq!(rows[0].created_by, "/bin/sh -c #(nop) ADD file:abc in /");
        assert_eq!(rows[0].size, "78.4MB");
        assert_eq!(rows[0].created_at, "2 weeks ago");
    }

    #[test]
    fn wslc_layers_come_out_of_the_inspect_in_newest_first_order() {
        let rows = image_layers_from_inspect(WSLC_IMAGE_INSPECT).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id, "sha256:cccc");
        assert_eq!(rows[2].id, "sha256:aaaa");
        assert!(image_layers_from_inspect("[]").is_err());
        assert!(image_layers_from_inspect("nonsense").is_err());
    }

    #[test]
    fn a_namespace_table_loses_its_header_and_keeps_the_order() {
        let names = namespaces(NERDCTL_NAMESPACES);
        assert_eq!(names, vec!["default", "k8s.io", "build"]);
    }

    #[test]
    fn docker_info_and_version_fill_the_overview() {
        let info = runtime_info(ContainerRuntime::Docker, Some(DOCKER_INFO), Some(DOCKER_VERSION));
        assert_eq!(info.runtime, "docker");
        assert_eq!(info.client_version, "28.4.2");
        assert_eq!(info.server_version, "28.4.1");
        assert_eq!(info.server_component, "Engine");
        assert_eq!(info.storage_driver, "overlayfs");
        assert_eq!(info.cgroup_driver, "cgroupfs");
        assert_eq!(info.cgroup_version, "2");
        assert_eq!(info.os, "Docker Desktop");
        assert_eq!(info.arch, "x86_64");
        assert_eq!(info.kernel, "6.6.87.2-microsoft-standard-WSL2");
        assert_eq!(info.cpus, 16);
        assert_eq!(info.memory, 16 * 1024 * 1024 * 1024);
        assert_eq!(info.containers_total, 3);
        assert_eq!(info.images_total, 7);
        assert_eq!(
            info.containers_by_state,
            vec![
                ContainerStateCount {
                    state: "running".to_owned(),
                    count: 1
                },
                ContainerStateCount {
                    state: "paused".to_owned(),
                    count: 1
                },
                ContainerStateCount {
                    state: "stopped".to_owned(),
                    count: 1
                },
            ]
        );
    }

    #[test]
    fn podman_info_is_read_through_its_nested_objects() {
        let info = runtime_info(ContainerRuntime::Podman, Some(PODMAN_INFO), Some(PODMAN_VERSION));
        assert_eq!(info.client_version, "5.2.3");
        assert_eq!(info.server_version, "5.2.3");
        assert_eq!(info.server_component, "Podman");
        assert_eq!(info.os, "ubuntu 24.04");
        assert_eq!(info.cgroup_version, "v2");
        assert_eq!(info.storage_driver, "overlay");
        assert_eq!(info.memory, 8 * 1024 * 1024 * 1024);
        assert_eq!(info.containers_total, 2);
        assert_eq!(info.images_total, 4);
    }

    #[test]
    fn nerdctl_reports_containerd_as_the_server_component() {
        let info = runtime_info(ContainerRuntime::Nerdctl, None, Some(NERDCTL_VERSION));
        assert_eq!(info.server_component, "containerd");
        assert_eq!(info.server_version, "2.0.2");
        assert_eq!(info.client_component, "nerdctl");
    }

    #[test]
    fn a_runtime_that_answers_nothing_still_has_a_name() {
        let info = runtime_info(ContainerRuntime::Wslc, None, None);
        assert_eq!(info.runtime, "wslc");
        assert_eq!(info.client_component, "wslc");
        assert_eq!(info.server_component, "wslc");
        assert!(info.client_version.is_empty());
    }

    #[test]
    fn a_bad_json_line_never_takes_the_rest_of_the_list_with_it() {
        let output = "not json\n{\"Names\":\"good\",\"State\":\"running\"}\n{\"Names\":\n";
        let rows = containers(ContainerRuntime::Podman, output);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "good");
    }

    #[test]
    fn an_array_of_json_survives_a_trailing_newline_and_pretty_printing() {
        let output = "[\n  {\"Name\": \"one\", \"State\": 2},\n  {\"Name\": \"two\", \"State\": 3}\n]\n";
        let rows = containers(ContainerRuntime::Wslc, output);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].state, "running");
        assert_eq!(rows[1].state, "exited");
    }

    #[test]
    fn timestamps_from_each_shape_arrive_in_one_shape() {
        assert_eq!(
            normalize_timestamp("2026-08-24T01:15:03.123456789Z"),
            "2026-08-24 01:15:03"
        );
        assert_eq!(
            normalize_timestamp("2026-08-24 09:15:00 +0800 CST"),
            "2026-08-24 09:15:00"
        );
        assert_eq!(normalize_timestamp("0001-01-01T00:00:00Z"), "");
        assert_eq!(normalize_timestamp(""), "");
        assert_eq!(unix_timestamp(0), "1970-01-01 00:00:00");
        assert_eq!(unix_timestamp(1_787_571_632), "2026-08-24 11:40:32");
    }

    #[test]
    fn sizes_print_with_the_unit_a_person_expects() {
        assert_eq!(human_size(0), "0B");
        assert_eq!(human_size(512), "512B");
        assert_eq!(human_size(1024), "1.0KB");
        assert_eq!(human_size(187_000_000), "178.3MB");
        assert_eq!(parse_size("1.5GiB"), 1_610_612_736);
        assert_eq!(parse_size("2kB"), 2000);
        assert_eq!(parse_size("1234567"), 1_234_567);
        assert_eq!(parse_size("-"), 0);
    }

    #[test]
    fn only_the_visible_part_of_a_very_long_answer_is_parsed() {
        let mut output = String::from("[");
        for index in 0..20_000 {
            output.push_str(&format!("{{\"Name\":\"n{index}\",\"State\":2}},"));
        }
        output.push_str("{\"Name\":\"last\",\"State\":2}]");
        let rows = containers(ContainerRuntime::Wslc, &output);
        assert!(rows.len() <= MAX_ITEMS);
        assert!(!rows.is_empty());
    }

    #[test]
    fn an_oversized_image_environment_is_cut_rather_than_carried() {
        let mut env = String::new();
        for index in 0..400 {
            env.push_str(&format!("\"K{index}=V\","));
        }
        let raw = format!(
            "[{{\"Id\":\"abc\",\"Config\":{{\"Env\":[{}]}},\"State\":{{\"Status\":\"running\"}}}}]",
            env.trim_end_matches(',')
        );
        let detail = detail(&raw).unwrap();
        assert_eq!(detail.env.len(), MAX_ENV_ENTRIES);
    }
}
