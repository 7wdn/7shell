//! Workstation's own inventory, and the two fields of a `.vmx` worth knowing.
//!
//! Workstation keeps every machine it has opened in `inventory.vmls`: an INI-shaped document with
//! one block per VM naming the `.vmx` document and the display name. It is the only list that
//! includes machines that are not running, which is exactly the half `vmrun list` cannot answer.
//!
//! The file is written in the system's ANSI code page on Chinese Windows and says so in its header
//! (`.encoding = "GBK"`), so it is decoded by its own declaration rather than assumed to be UTF-8.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use encoding_rs::{GBK, UTF_8};

/// One machine the inventory knows about, before the `.vmx` itself is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryEntry {
    pub path: String,
    pub display_name: String,
}

/// Where Workstation keeps its inventory on this platform.
///
/// The file is per user, not per installation: two accounts on one machine have two lists, and so
/// does this build when it runs as one of them.
pub fn inventory_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let appdata = std::env::var_os("APPDATA")?;
        let path = PathBuf::from(appdata).join("VMware").join("inventory.vmls");
        path.is_file().then_some(path)
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var_os("HOME")?;
        let path = PathBuf::from(home).join(".vmware").join("inventory.vmls");
        path.is_file().then_some(path)
    }
}

/// Reads and parses the inventory, answering nothing when the file does not exist.
///
/// An absent inventory is not an error: a freshly installed Workstation has one, but a machine
/// that only ever used the command line may not, and the running list is still a useful answer.
pub fn load() -> Vec<InventoryEntry> {
    let Some(path) = inventory_path() else {
        return Vec::new();
    };
    match std::fs::read(&path) {
        Ok(bytes) => parse(&bytes),
        Err(_) => Vec::new(),
    }
}

/// Parses an inventory document. Public so the decoder can be tested with the bytes a Chinese
/// Windows writes rather than with the lossy reading of them.
pub fn parse(bytes: &[u8]) -> Vec<InventoryEntry> {
    let text = decode(bytes);
    let mut blocks: BTreeMap<String, (Option<String>, Option<String>)> = BTreeMap::new();

    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = unquote(value.trim());
        let Some((block, field)) = key.rsplit_once('.') else {
            continue;
        };
        if !block.starts_with("vmlist") {
            continue;
        }
        let entry = blocks.entry(block.to_owned()).or_default();
        match field {
            "config" => entry.0 = Some(value),
            "DisplayName" => entry.1 = Some(value),
            _ => {}
        }
    }

    let mut entries: Vec<InventoryEntry> = blocks
        .into_values()
        .filter_map(|(path, name)| {
            let path = path?;
            let path = path.trim().to_owned();
            if path.is_empty() {
                return None;
            }
            let display_name = name
                .map(|name| name.trim().to_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| file_stem(&path));
            Some(InventoryEntry { path, display_name })
        })
        .collect();

    entries.sort_by_key(|entry| entry.display_name.to_lowercase());
    entries.dedup_by(|left, right| same_path(&left.path, &right.path));
    entries
}

/// Decodes the document by the encoding its own header names.
fn decode(bytes: &[u8]) -> String {
    let declared = declared_encoding(bytes);
    match declared.as_deref() {
        Some("gbk") | Some("cp936") | Some("gb2312") | Some("gb18030") => GBK.decode(bytes).0.into_owned(),
        _ => UTF_8.decode(bytes).0.into_owned(),
    }
}

/// The `.encoding` header, lowercased. Only the first few kilobytes are scanned: the header is the
/// first line of a file this build wrote nothing to.
fn declared_encoding(bytes: &[u8]) -> Option<String> {
    let head = &bytes[..bytes.len().min(4096)];
    let head = String::from_utf8_lossy(head);
    for line in head.lines().take(8) {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case(".encoding") {
            return Some(unquote(value.trim()).to_lowercase());
        }
    }
    None
}

fn unquote(value: &str) -> String {
    let stripped = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value);
    stripped.replace("\\\"", "\"")
}

fn file_stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

/// Whether two paths name the same document. Windows compares case-insensitively; elsewhere a
/// case difference is a different file.
pub fn same_path(left: &str, right: &str) -> bool {
    #[cfg(windows)]
    {
        left.eq_ignore_ascii_case(right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

/// What a `.vmx` says about itself: the name Workstation draws, the guest identifier, and the
/// machine's own identity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VmxInfo {
    pub display_name: Option<String>,
    pub guest_os: Option<String>,
    /// `uuid.bios`, normalized to a compact lower-case string. This is the token the interface
    /// holds, because it never receives the `.vmx` path.
    pub uuid: Option<String>,
}

/// Reads the fields this build uses. Everything else in the document belongs to Workstation.
pub fn parse_vmx(text: &str) -> VmxInfo {
    let mut info = VmxInfo::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        if !matches!(key.as_str(), "displayname" | "guestos" | "uuid.bios") {
            continue;
        }
        let value = unquote(value.trim());
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key.as_str() {
            "displayname" => info.display_name = Some(value.to_owned()),
            "guestos" => info.guest_os = Some(value.to_owned()),
            "uuid.bios" => info.uuid = normalize_uuid(value),
            _ => {}
        }
    }
    info
}

/// Workstation writes the UUID with spaces and one hyphen; the token is the same value without
/// them, so two spellings of one machine compare equal. Anything that is not a UUID-shaped string
/// is refused rather than passed around: the interface is given an identifier, not a path.
pub fn normalize_uuid(value: &str) -> Option<String> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != '-')
        .map(|character| character.to_ascii_lowercase())
        .collect();
    (compact.len() == 32 && compact.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(compact)
}

/// Reads one `.vmx` document, answering nothing when it cannot be read.
pub async fn read_vmx(path: &str) -> Option<VmxInfo> {
    let bytes = tokio::fs::read(path).await.ok()?;
    let text = String::from_utf8_lossy(&bytes);
    Some(parse_vmx(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chinese_windows_inventory_is_decoded_by_its_own_header() {
        let source = ".encoding = \"GBK\"\n\
                      vmlist1.config = \"D:\\VMs\\测试机\\测试机.vmx\"\n\
                      vmlist1.DisplayName = \"测试机\"\n";
        let (bytes, _, _) = GBK.encode(source);
        let entries = parse(&bytes);
        assert_eq!(entries.len(), 1, "the GBK document must not be lost to UTF-8 decoding");
        assert_eq!(entries[0].display_name, "测试机");
        assert_eq!(entries[0].path, "D:\\VMs\\测试机\\测试机.vmx");
    }

    #[test]
    fn empty_slots_and_nameless_blocks_do_not_become_machines() {
        let source = ".encoding = \"UTF-8\"\n\
                      vmlist2.config = \"\"\n\
                      vmlist3.config = \"E:\\VMs\\b\\b.vmx\"\n";
        let entries = parse(source.as_bytes());
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].display_name, "b",
            "a block with no name falls back to the file name"
        );
    }

    #[test]
    fn the_inventory_is_sorted_by_the_name_workstation_draws() {
        let source = "vmlist1.config = \"a.vmx\"\n\
                      vmlist1.DisplayName = \"zulu\"\n\
                      vmlist2.config = \"b.vmx\"\n\
                      vmlist2.DisplayName = \"alpha\"\n";
        let entries = parse(source.as_bytes());
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.display_name.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "zulu"]
        );
    }

    #[test]
    fn a_vmx_answers_with_the_fields_this_build_reads() {
        let info = parse_vmx(
            "displayName = \"uos_test\"\nguestOS = \"debian12-64\"\n\
             uuid.bios = \"56 4d b5 f2 35 15 de 03-2c d9 33 8b a8 d8 31 b7\"\nmemsize = \"8192\"\n",
        );
        assert_eq!(info.display_name.as_deref(), Some("uos_test"));
        assert_eq!(info.guest_os.as_deref(), Some("debian12-64"));
        assert_eq!(info.uuid.as_deref(), Some("564db5f23515de032cd9338ba8d831b7"));
    }

    #[test]
    fn a_vmx_without_the_fields_answers_nothing_rather_than_guessing() {
        let info = parse_vmx("memsize = \"4096\"\n");
        assert_eq!(info, VmxInfo::default());
    }

    #[test]
    fn a_uuid_is_the_same_value_whichever_way_workstation_spaces_it() {
        assert_eq!(
            normalize_uuid("56 4d b5 f2 35 15 de 03-2c d9 33 8b a8 d8 31 b7").as_deref(),
            Some("564db5f23515de032cd9338ba8d831b7")
        );
        assert_eq!(normalize_uuid("not a uuid"), None);
        assert_eq!(normalize_uuid(""), None);
    }

    #[test]
    fn windows_paths_compare_without_case_and_other_platforms_do_not() {
        assert!(same_path("E:\\VMs\\a.vmx", "E:\\VMs\\a.vmx"));
        #[cfg(windows)]
        assert!(same_path("E:\\VMs\\A.VMX", "e:\\vms\\a.vmx"));
        #[cfg(not(windows))]
        assert!(!same_path("E:\\VMs\\A.VMX", "e:\\vms\\a.vmx"));
    }
}
