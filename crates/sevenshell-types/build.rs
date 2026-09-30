//! Generates the Rust half of the descriptor tables.
//!
//! `descriptors/**/*.yaml` is the source of truth; `scripts/sync-descriptors.mjs`
//! turns it into `descriptors/descriptors.manifest.json`, and this script reads that
//! JSON. The build graph deliberately stays free of a YAML parser: the manifest is
//! committed, so `cargo build` needs nothing else, and `pnpm check:descriptors` fails
//! when the YAML and the committed outputs drift apart.

use std::env;
use std::fmt::Write as _;
use std::path::PathBuf;

/// The vocabulary that is fixed by the generator rather than by the descriptors.
const SUPPORT_LEVELS: &[&str] = &["connect", "browse", "operate"];
const RUNTIMES: &[&str] = &["native", "sidecar", "external"];
const GROUPS: &[&str] = &["terminal", "transfer", "network", "session", "auth", "trust"];

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo"));
    let manifest_path = manifest_dir.join("../../descriptors/descriptors.manifest.json");
    println!("cargo:rerun-if-changed={}", manifest_path.display());

    let raw = std::fs::read_to_string(&manifest_path).unwrap_or_else(|error| {
        panic!(
            "cannot read {}: {error}\nRun `pnpm generate:descriptors` first.",
            manifest_path.display()
        )
    });
    let manifest: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} is not valid JSON: {error}", manifest_path.display()));

    let source = generate(&manifest).unwrap_or_else(|error| panic!("descriptor manifest is invalid: {error}"));

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
    std::fs::write(out_dir.join("descriptors.rs"), source).expect("failed to write the generated descriptor tables");
}

struct CapabilityEntry {
    key: String,
    variant: String,
    group: String,
    group_variant: String,
    label_key: String,
    order: u64,
}

struct ProtocolEntry {
    id: String,
    variant: String,
    label_key: String,
    default_port: Option<u64>,
    runtime_variant: String,
    transport: String,
    support_variant: String,
    capabilities: Vec<String>,
    capability_variants: Vec<String>,
}

struct AuthMethodEntry {
    id: String,
    variant: String,
    label_key: String,
    capabilities: Vec<String>,
    capability_variants: Vec<String>,
    platforms: Vec<String>,
}

struct Catalog {
    capabilities: Vec<CapabilityEntry>,
    protocols: Vec<ProtocolEntry>,
    auth_methods: Vec<AuthMethodEntry>,
}

fn generate(manifest: &serde_json::Value) -> Result<String, String> {
    let schema_version = manifest
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or("missing numeric field `schemaVersion`")?;
    if schema_version != 1 {
        return Err(format!("unsupported descriptor schema version {schema_version}"));
    }

    let catalog = Catalog {
        capabilities: read_capabilities(manifest)?,
        protocols: read_protocols(manifest)?,
        auth_methods: read_auth_methods(manifest)?,
    };
    validate(&catalog)?;
    Ok(emit(&catalog, schema_version))
}

fn read_capabilities(manifest: &serde_json::Value) -> Result<Vec<CapabilityEntry>, String> {
    let raw = array_at(manifest, "capabilities")?;
    let mut entries = Vec::with_capacity(raw.len());
    for item in raw {
        let key = string_at(item, "key")?;
        let group = string_at(item, "group")?;
        if !GROUPS.contains(&group.as_str()) {
            return Err(format!("capability `{key}` declares unknown group `{group}`"));
        }
        entries.push(CapabilityEntry {
            variant: pascal_case(&key)?,
            group_variant: pascal_case(&group)?,
            label_key: string_at(item, "labelKey")?,
            order: item
                .get("order")
                .and_then(serde_json::Value::as_u64)
                .ok_or("capability is missing `order`")?,
            key,
            group,
        });
    }
    Ok(entries)
}

fn read_protocols(manifest: &serde_json::Value) -> Result<Vec<ProtocolEntry>, String> {
    let raw = array_at(manifest, "protocols")?;
    let mut entries = Vec::with_capacity(raw.len());
    for item in raw {
        let id = string_at(item, "id")?;
        let runtime = string_at(item, "runtime")?;
        let support_level = string_at(item, "supportLevel")?;
        if !RUNTIMES.contains(&runtime.as_str()) {
            return Err(format!("protocol `{id}` declares unknown runtime `{runtime}`"));
        }
        if !SUPPORT_LEVELS.contains(&support_level.as_str()) {
            return Err(format!(
                "protocol `{id}` declares unknown support level `{support_level}`"
            ));
        }
        let capabilities = string_list(item, "capabilities")?;
        entries.push(ProtocolEntry {
            variant: identifier_at(item, "rustVariant")?,
            label_key: string_at(item, "labelKey")?,
            default_port: item.get("defaultPort").and_then(serde_json::Value::as_u64),
            runtime_variant: pascal_case(&runtime)?,
            support_variant: pascal_case(&support_level)?,
            transport: string_at(item, "transport")?,
            capability_variants: variants_of(&capabilities)?,
            id,
            capabilities,
        });
    }
    Ok(entries)
}

fn read_auth_methods(manifest: &serde_json::Value) -> Result<Vec<AuthMethodEntry>, String> {
    let raw = array_at(manifest, "authMethods")?;
    let mut entries = Vec::with_capacity(raw.len());
    for item in raw {
        let capabilities = string_list(item, "capabilities")?;
        entries.push(AuthMethodEntry {
            id: string_at(item, "id")?,
            variant: identifier_at(item, "rustVariant")?,
            label_key: string_at(item, "labelKey")?,
            capability_variants: variants_of(&capabilities)?,
            platforms: string_list(item, "platforms")?,
            capabilities,
        });
    }
    Ok(entries)
}

fn validate(catalog: &Catalog) -> Result<(), String> {
    let known: Vec<&str> = catalog.capabilities.iter().map(|entry| entry.key.as_str()).collect();
    unique(&known, "capability key")?;

    let mut protocol_ids: Vec<&str> = Vec::new();
    let mut protocol_variants: Vec<&str> = Vec::new();
    for protocol in &catalog.protocols {
        for capability in &protocol.capabilities {
            if !known.contains(&capability.as_str()) {
                return Err(format!(
                    "protocol `{}` claims unknown capability `{capability}`",
                    protocol.id
                ));
            }
        }
        protocol_ids.push(&protocol.id);
        protocol_variants.push(&protocol.variant);
    }
    unique(&protocol_ids, "protocol id")?;
    unique(&protocol_variants, "protocol rustVariant")?;

    let mut auth_ids: Vec<&str> = Vec::new();
    let mut auth_variants: Vec<&str> = Vec::new();
    for method in &catalog.auth_methods {
        for capability in &method.capabilities {
            if !known.contains(&capability.as_str()) {
                return Err(format!(
                    "auth method `{}` claims unknown capability `{capability}`",
                    method.id
                ));
            }
        }
        auth_ids.push(&method.id);
        auth_variants.push(&method.variant);
    }
    unique(&auth_ids, "auth method id")?;
    unique(&auth_variants, "auth method rustVariant")?;

    // The generated enums must cover every variant name used in tables.
    for entry in &catalog.capabilities {
        if !GROUPS.contains(&entry.group.as_str()) {
            return Err(format!("capability `{}` has an unhandled group", entry.key));
        }
    }
    Ok(())
}

fn unique(values: &[&str], what: &str) -> Result<(), String> {
    let mut seen: Vec<&str> = Vec::with_capacity(values.len());
    for value in values {
        if seen.contains(value) {
            return Err(format!("duplicate {what} `{value}`"));
        }
        seen.push(value);
    }
    Ok(())
}

fn emit(catalog: &Catalog, schema_version: u64) -> String {
    let mut out = String::with_capacity(8 * 1024);
    out.push_str("// @generated by crates/sevenshell-types/build.rs from descriptors/descriptors.manifest.json.\n");
    out.push_str("// Do not edit. Edit descriptors/**/*.yaml and run `pnpm generate:descriptors`.\n");
    out.push_str("use serde::{Deserialize, Serialize};\n");

    let _ = writeln!(out, "\n/// Descriptor schema version these tables were generated from.");
    let _ = writeln!(out, "pub const DESCRIPTOR_SCHEMA_VERSION: u32 = {schema_version};");

    let groups: Vec<(String, String)> = GROUPS
        .iter()
        .map(|group| (pascal_case(group).expect("known group name"), (*group).to_owned()))
        .collect();
    emit_enum(
        &mut out,
        "CapabilityGroup",
        "The section a capability belongs to, used to group it in the interface.",
        &groups,
    );

    let levels: Vec<(String, String)> = SUPPORT_LEVELS
        .iter()
        .map(|level| (pascal_case(level).unwrap(), (*level).to_owned()))
        .collect();
    emit_enum(
        &mut out,
        "SupportLevel",
        "How far support for a protocol goes today.",
        &levels,
    );

    let runtimes: Vec<(String, String)> = RUNTIMES
        .iter()
        .map(|runtime| (pascal_case(runtime).unwrap(), (*runtime).to_owned()))
        .collect();
    emit_enum(
        &mut out,
        "Runtime",
        "Where the protocol implementation runs.",
        &runtimes,
    );

    let capabilities: Vec<(String, String)> = catalog
        .capabilities
        .iter()
        .map(|entry| (entry.variant.clone(), entry.key.clone()))
        .collect();
    emit_enum(
        &mut out,
        "Capability",
        "A capability a protocol or an auth method can support.",
        &capabilities,
    );

    let _ = writeln!(
        out,
        "\n/// A capability as declared in `descriptors/capabilities.yaml`."
    );
    let _ = writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]");
    let _ = writeln!(out, "pub struct CapabilitySpec {{");
    let _ = writeln!(out, "    pub capability: Capability,");
    let _ = writeln!(out, "    pub group: CapabilityGroup,");
    let _ = writeln!(out, "    pub label_key: &'static str,");
    let _ = writeln!(out, "    pub order: u16,");
    let _ = writeln!(out, "}}");

    let _ = writeln!(
        out,
        "\n/// Every capability, in the declaration order of `descriptors/capabilities.yaml`."
    );
    let _ = writeln!(out, "pub const CAPABILITIES: &[CapabilitySpec] = &[");
    for entry in &catalog.capabilities {
        let _ = writeln!(
            out,
            "    CapabilitySpec {{ capability: Capability::{}, group: CapabilityGroup::{}, label_key: \"{}\", order: {} }},",
            entry.variant, entry.group_variant, entry.label_key, entry.order
        );
    }
    let _ = writeln!(out, "];");

    let protocols: Vec<(String, String)> = catalog
        .protocols
        .iter()
        .map(|entry| (entry.variant.clone(), entry.id.clone()))
        .collect();
    emit_enum(
        &mut out,
        "Protocol",
        "A transport 7Shell can speak, as declared in `descriptors/protocols/`.",
        &protocols,
    );

    let _ = writeln!(out, "\n/// A protocol and the capabilities committed for it.");
    let _ = writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]");
    let _ = writeln!(out, "pub struct ProtocolSpec {{");
    let _ = writeln!(out, "    pub protocol: Protocol,");
    let _ = writeln!(out, "    pub id: &'static str,");
    let _ = writeln!(out, "    pub label_key: &'static str,");
    let _ = writeln!(out, "    pub default_port: Option<u16>,");
    let _ = writeln!(out, "    pub runtime: Runtime,");
    let _ = writeln!(out, "    pub transport: &'static str,");
    let _ = writeln!(out, "    pub support_level: SupportLevel,");
    let _ = writeln!(out, "    pub capabilities: &'static [Capability],");
    let _ = writeln!(out, "}}");

    let _ = writeln!(out, "\n/// Every protocol, in the order declared by the descriptors.");
    let _ = writeln!(out, "pub const PROTOCOLS: &[ProtocolSpec] = &[");
    for entry in &catalog.protocols {
        let default_port = match entry.default_port {
            Some(port) => format!("Some({port})"),
            None => "None".to_owned(),
        };
        let capability_list: Vec<String> = entry
            .capability_variants
            .iter()
            .map(|variant| format!("Capability::{variant}"))
            .collect();
        let _ = writeln!(out, "    ProtocolSpec {{");
        let _ = writeln!(out, "        protocol: Protocol::{},", entry.variant);
        let _ = writeln!(out, "        id: \"{}\",", entry.id);
        let _ = writeln!(out, "        label_key: \"{}\",", entry.label_key);
        let _ = writeln!(out, "        default_port: {default_port},");
        let _ = writeln!(out, "        runtime: Runtime::{},", entry.runtime_variant);
        let _ = writeln!(out, "        transport: \"{}\",", entry.transport);
        let _ = writeln!(out, "        support_level: SupportLevel::{},", entry.support_variant);
        let _ = writeln!(out, "        capabilities: &[{}],", capability_list.join(", "));
        let _ = writeln!(out, "    }},");
    }
    let _ = writeln!(out, "];");

    let methods: Vec<(String, String)> = catalog
        .auth_methods
        .iter()
        .map(|entry| (entry.variant.clone(), entry.id.clone()))
        .collect();
    emit_enum(
        &mut out,
        "AuthMethod",
        "A way to authenticate, as declared in `descriptors/auth-methods/`.",
        &methods,
    );

    let _ = writeln!(out, "\n/// An authentication method and what it needs from the user.");
    let _ = writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]");
    let _ = writeln!(out, "pub struct AuthMethodSpec {{");
    let _ = writeln!(out, "    pub method: AuthMethod,");
    let _ = writeln!(out, "    pub id: &'static str,");
    let _ = writeln!(out, "    pub label_key: &'static str,");
    let _ = writeln!(out, "    pub capabilities: &'static [Capability],");
    let _ = writeln!(out, "    /// Empty means every platform.");
    let _ = writeln!(out, "    pub platforms: &'static [&'static str],");
    let _ = writeln!(out, "}}");

    let _ = writeln!(
        out,
        "\n/// Every auth method, in the order declared by the descriptors."
    );
    let _ = writeln!(out, "pub const AUTH_METHODS: &[AuthMethodSpec] = &[");
    for entry in &catalog.auth_methods {
        let capability_list: Vec<String> = entry
            .capability_variants
            .iter()
            .map(|variant| format!("Capability::{variant}"))
            .collect();
        let platform_list: Vec<String> = entry
            .platforms
            .iter()
            .map(|platform| format!("\"{platform}\""))
            .collect();
        let _ = writeln!(out, "    AuthMethodSpec {{");
        let _ = writeln!(out, "        method: AuthMethod::{},", entry.variant);
        let _ = writeln!(out, "        id: \"{}\",", entry.id);
        let _ = writeln!(out, "        label_key: \"{}\",", entry.label_key);
        let _ = writeln!(out, "        capabilities: &[{}],", capability_list.join(", "));
        let _ = writeln!(out, "        platforms: &[{}],", platform_list.join(", "));
        let _ = writeln!(out, "    }},");
    }
    let _ = writeln!(out, "];");

    out
}

fn emit_enum(out: &mut String, name: &str, doc: &str, entries: &[(String, String)]) {
    let _ = writeln!(out, "\n/// {doc}");
    let _ = writeln!(
        out,
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]"
    );
    let _ = writeln!(out, "pub enum {name} {{");
    for (variant, wire) in entries {
        let _ = writeln!(out, "    #[serde(rename = \"{wire}\")]");
        let _ = writeln!(out, "    {variant},");
    }
    let _ = writeln!(out, "}}");

    let variant_list: Vec<String> = entries
        .iter()
        .map(|(variant, _)| format!("{name}::{variant}"))
        .collect();
    let _ = writeln!(out, "\nimpl {name} {{");
    let _ = writeln!(out, "    /// Every variant, in declaration order.");
    let _ = writeln!(
        out,
        "    pub const ALL: &'static [{name}] = &[{}];",
        variant_list.join(", ")
    );
    let _ = writeln!(out, "\n    /// The stable wire identifier.");
    let _ = writeln!(out, "    pub const fn as_str(self) -> &'static str {{");
    let _ = writeln!(out, "        match self {{");
    for (variant, wire) in entries {
        let _ = writeln!(out, "            Self::{variant} => \"{wire}\",");
    }
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "\n    /// Looks a variant up by its wire identifier.");
    let _ = writeln!(
        out,
        "    pub fn from_id(id: &str) -> Option<Self> {{ Self::ALL.iter().copied().find(|candidate| candidate.as_str() == id) }}"
    );
    let _ = writeln!(out, "}}");

    let _ = writeln!(
        out,
        "
impl std::fmt::Display for {name} {{"
    );
    let _ = writeln!(
        out,
        "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ f.write_str(self.as_str()) }}"
    );
    let _ = writeln!(out, "}}");
}

fn array_at<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a Vec<serde_json::Value>, String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("missing array `{key}`"))
}

fn string_at(value: &serde_json::Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("missing string `{key}`"))
}

fn identifier_at(value: &serde_json::Value, key: &str) -> Result<String, String> {
    let raw = string_at(value, key)?;
    if !is_identifier(&raw) {
        return Err(format!("`{key}` = `{raw}` is not a valid Rust identifier"));
    }
    Ok(raw)
}

fn string_list(value: &serde_json::Value, key: &str) -> Result<Vec<String>, String> {
    match value.get(key) {
        None | Some(serde_json::Value::Null) => Ok(Vec::new()),
        Some(serde_json::Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let raw = item
                    .as_str()
                    .ok_or_else(|| format!("`{key}` must contain only strings"))?;
                out.push(raw.to_owned());
            }
            Ok(out)
        }
        Some(_) => Err(format!("`{key}` must be an array of strings")),
    }
}

fn variants_of(keys: &[String]) -> Result<Vec<String>, String> {
    keys.iter().map(|key| pascal_case(key)).collect()
}

fn pascal_case(value: &str) -> Result<String, String> {
    let mut out = String::with_capacity(value.len());
    for part in value.split(['-', '_']).filter(|part| !part.is_empty()) {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if !is_identifier(&out) {
        return Err(format!("`{value}` does not map to a valid Rust identifier"));
    }
    Ok(out)
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric())
}
