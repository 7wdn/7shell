//! Live checks of the management tools against a disposable Workstation VM.
//!
//! Gated by `SEVENSHELL_TEST_VMWARE_VMX`, the path of a scratch VM the caller created. The test
//! gives that VM an inventory of its own (through a temporary `APPDATA`) so the crate can resolve
//! it while it is powered off, without touching the real Workstation inventory:
//!
//! ```text
//! vmcli VM Create -n 7shell-live -d <scratch> -c other5xlinux-64
//! SEVENSHELL_TEST_VMWARE_VMX=<scratch>\7shell-live.vmx cargo test -p sevenshell-api \
//!   --test live_manage_tools -- --ignored --test-threads=1
//! ```
//!
//! Snapshot clone, template create and template deploy write under a scratch output folder; the
//! clone and the deployed machine are deleted before the test ends. The snapshot is taken and
//! deleted on the scratch VM. Reset and console keys run on the scratch VM only — no guest exists
//! there to lose, which is also why the keys exercise the console path rather than a guest account.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use sevenshell_api::{
    ApprovalBroker, ApprovalDecision, ApprovalRequest, ToolCall, ToolContext, ToolError, ToolOutcome, local,
};
use sevenshell_core::Core;
use sevenshell_types::{
    AuthMethod, ContainerSettings, DesktopSettings, FtpSettings, HostLocationKind, KubernetesSettings, LocalSettings,
    McpScope, Protocol, SerialSettings, Session, SessionAuth, TelnetSettings, TerminalSettings, VmwareSettings,
    WebDavSettings, WslSettings,
};
use sevenshell_vault::Vault;
use tempfile::tempdir;

struct ApproveAll;

#[async_trait]
impl ApprovalBroker for ApproveAll {
    async fn request(&self, _request: ApprovalRequest) -> Result<ApprovalDecision, ToolError> {
        Ok(ApprovalDecision::Approved)
    }
}

fn vm_session(vm_id: &str) -> Session {
    Session {
        id: "vm-manage".to_owned(),
        name: "vm manage".to_owned(),
        group_id: None,
        order: None,
        protocol: Protocol::Vmware,
        host: String::new(),
        port: 0,
        user: String::new(),
        auth: SessionAuth::with_password(AuthMethod::Password),
        terminal: TerminalSettings::default(),
        desktop: DesktopSettings::default(),
        telnet: TelnetSettings::default(),
        serial: SerialSettings::default(),
        local: LocalSettings::default(),
        wsl: WslSettings::default(),
        container: ContainerSettings::default(),
        kubernetes: KubernetesSettings::default(),
        webdav: WebDavSettings::default(),
        ftp: FtpSettings::default(),
        vmware: VmwareSettings {
            vm_id: vm_id.to_owned(),
            ssh_user: None,
            ssh_port: 22,
            rdp_user: None,
            rdp_port: 3389,
            ssh_session: None,
            rdp_session: None,
        },
        jump: Vec::new(),
        forwards: Vec::new(),
        updated_at: 1,
    }
}

fn uuid_of(vmx: &Path) -> String {
    let text = std::fs::read_to_string(vmx).expect("the scratch VMX reads");
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with("uuid.bios"))
        .expect("the scratch VMX has a uuid");
    let value = line.split_once('=').expect("a key and a value").1.trim();
    let value = value.trim_matches('"');
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != '-')
        .map(|character| character.to_ascii_lowercase())
        .collect();
    assert_eq!(compact.len(), 32, "the uuid is the identity the tools take");
    compact
}

fn vmrun(args: &[&str]) {
    let vmrun = sevenshell_vmware::Vmrun::locate().expect("vmrun");
    let output = Command::new(vmrun.program()).args(args).output().expect("vmrun runs");
    assert!(
        output.status.success(),
        "vmrun {args:?} failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn value_of(outcome: ToolOutcome, tool: &str) -> Value {
    match outcome {
        ToolOutcome::Succeeded { result } => result.value,
        ToolOutcome::Failed { error } => {
            panic!(
                "{tool} failed: {} {}",
                error.code(),
                error.message().unwrap_or_default()
            )
        }
    }
}

#[tokio::test]
#[ignore]
async fn snapshots_clone_templates_deploy_and_reset_through_the_tools() {
    let Ok(vmx) = std::env::var("SEVENSHELL_TEST_VMWARE_VMX") else {
        return;
    };
    let vmx_path = PathBuf::from(&vmx);
    assert!(vmx_path.is_file(), "the scratch VMX is there");
    let uuid = uuid_of(&vmx_path);

    // The crate resolves a powered-off VM through the inventory. Point this process at an inventory
    // that holds only the scratch VM, so the real Workstation list is never read or written.
    let appdata = tempdir().expect("a temporary appdata");
    let vmware = appdata.path().join("VMware");
    std::fs::create_dir_all(&vmware).expect("the inventory directory");
    std::fs::write(
        vmware.join("inventory.vmls"),
        format!(".encoding = \"UTF-8\"\nvmlist1.config = \"{vmx}\"\nvmlist1.DisplayName = \"7shell-live\"\n"),
    )
    .expect("the scratch inventory");
    // Single-threaded live tests: no other thread reads this process environment.
    unsafe {
        std::env::set_var("APPDATA", appdata.path());
    }

    let output_dir = tempdir().expect("a scratch output directory");
    let directory = tempdir().expect("a temporary vault");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault.create("master").expect("create");
    vault
        .save_session(vm_session(&uuid), None)
        .expect("the scratch VM becomes a session");
    let core = Core::new(Arc::clone(&vault)).0;
    let context = ToolContext::new(Arc::clone(&core), McpScope::HighRisk)
        .with_approvals(Arc::new(ApproveAll) as Arc<dyn ApprovalBroker>);
    let registry = local::default_registry();
    let call = |name: &'static str, arguments: Value| {
        let registry = &registry;
        let context = &context;
        async move {
            value_of(
                registry
                    .call(
                        context,
                        ToolCall {
                            name: name.to_owned(),
                            arguments,
                        },
                    )
                    .await,
                name,
            )
        }
    };

    // The session resolves to the scratch machine, even while it is powered off.
    let status = call("sshell_vm_status", json!({ "sessionId": "vm-manage" })).await;
    assert_eq!(status["status"]["id"], json!(uuid));
    assert_eq!(status["status"]["running"], json!(false));

    // A snapshot, taken and read back with its uid.
    call(
        "sshell_vm_snapshot_change",
        json!({ "sessionId": "vm-manage", "action": "take", "name": "7shell-live" }),
    )
    .await;
    let listing = call("sshell_vm_snapshots", json!({ "sessionId": "vm-manage" })).await;
    let listing = listing["output"].as_str().expect("the snapshot list is text");
    let uid = listing
        .split("uid:")
        .nth(1)
        .and_then(|rest| rest.lines().next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .expect("the snapshot list carries a uid")
        .to_owned();

    let folder_token = core
        .host_location_add(output_dir.path(), HostLocationKind::Folder)
        .expect("a folder token");

    // A linked clone of that snapshot.
    call(
        "sshell_vm_snapshot_change",
        json!({
            "sessionId": "vm-manage",
            "action": "clone",
            "name": "7shell-clone",
            "uid": uid,
            "token": folder_token.token,
            "linked": true
        }),
    )
    .await;
    let clone_vmx = output_dir.path().join("7shell-clone").join("7shell-clone.vmx");
    assert!(clone_vmx.is_file(), "the snapshot clone landed where it was told");

    // A template, then a machine deployed from it.
    call(
        "sshell_vm_image",
        json!({
            "sessionId": "vm-manage",
            "action": "templateCreate",
            "name": "7shell-tpl",
            "token": folder_token.token
        }),
    )
    .await;
    let template = output_dir.path().join("7shell-tpl.vmtx");
    assert!(template.is_file(), "the template landed where it was told");
    let file_token = core
        .host_location_add(&template, HostLocationKind::File)
        .expect("a file token");
    call(
        "sshell_vm_image",
        json!({
            "sessionId": "vm-manage",
            "action": "templateDeploy",
            "token": file_token.token
        }),
    )
    .await;
    let deployed: Vec<PathBuf> = std::fs::read_dir(output_dir.path())
        .expect("the output folder")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "vmx"))
        .collect();
    assert!(!deployed.is_empty(), "a template deploy creates a machine");

    // The snapshot is removed while the machine stays powered off.
    call(
        "sshell_vm_snapshot_change",
        json!({ "sessionId": "vm-manage", "action": "delete", "name": "7shell-live" }),
    )
    .await;

    // Start, send a console key (no guest account exists on this machine) and reset — both are
    // destructive actions the broker approved — then stop the scratch machine.
    call(
        "sshell_vm_power",
        json!({ "sessionId": "vm-manage", "action": "start" }),
    )
    .await;
    let keys = call(
        "sshell_vm_send_key",
        json!({ "sessionId": "vm-manage", "sequence": "esc" }),
    )
    .await;
    assert_eq!(keys["action"], json!("send-key"));
    call(
        "sshell_vm_power",
        json!({ "sessionId": "vm-manage", "action": "reset" }),
    )
    .await;
    vmrun(&["-T", "ws", "stop", &vmx, "hard"]);

    // Clean up every machine this test made, and the scratch originals.
    for machine in [&clone_vmx, &deployed[0], &vmx_path] {
        let _ = Command::new(sevenshell_vmware::Vmrun::locate().expect("vmrun").program())
            .args(["-T", "ws", "deleteVM", &machine.to_string_lossy()])
            .output();
    }
    let _ = core.host_location_forget(&folder_token.token);
    let _ = core.host_location_forget(&file_token.token);
}
