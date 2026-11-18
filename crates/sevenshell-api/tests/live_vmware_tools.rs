//! Live checks of the VM tools through the registry, against a real running machine.
//!
//! Gated by `SEVENSHELL_TEST_VMWARE=1` and the guest account in
//! `SEVENSHELL_TEST_GUEST_USER` / `SEVENSHELL_TEST_GUEST_PASSWORD`. The session is written to a
//! temporary vault with the guest password in its secret slot, which is exactly how the app stores
//! it; the tools then read, run one echo and copy one small file into `/tmp` and back. The guest
//! file is removed before the test ends.

use std::sync::Arc;

use serde_json::json;
use sevenshell_api::{ToolCall, ToolContext, ToolOutcome, local};
use sevenshell_core::Core;
use sevenshell_types::{
    AuthMethod, ContainerSettings, DesktopSettings, FtpSettings, HostLocationKind, KubernetesSettings, LocalSettings,
    McpScope, Protocol, SerialSettings, Session, SessionAuth, TelnetSettings, TerminalSettings, VmwareSettings,
    WebDavSettings, WslSettings,
};
use sevenshell_vault::{SessionSecret, Vault};
use tempfile::tempdir;

fn environment() -> Option<(String, String)> {
    if std::env::var("SEVENSHELL_TEST_VMWARE").as_deref() != Ok("1") {
        return None;
    }
    let user = std::env::var("SEVENSHELL_TEST_GUEST_USER").ok()?;
    let password = std::env::var("SEVENSHELL_TEST_GUEST_PASSWORD").ok()?;
    Some((user, password))
}

fn vm_session(vm_id: &str, user: &str) -> Session {
    Session {
        id: "vm-live".to_owned(),
        name: "vm live".to_owned(),
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
            ssh_user: Some(user.to_owned()),
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

#[tokio::test]
#[ignore]
async fn the_vm_tools_read_run_and_copy_through_a_saved_session() {
    let Some((user, password)) = environment() else {
        return;
    };

    let directory = tempdir().expect("a temporary directory");
    let vault = Arc::new(Vault::open_at(directory.path().join("vault.json")));
    vault.create("master").expect("create");
    let core = Core::new(Arc::clone(&vault)).0;

    let machines = core.vmware_scan().await.expect("the machine lists its VMs");
    let Some(running) = machines.iter().find(|machine| machine.running) else {
        return;
    };
    vault
        .save_session(vm_session(&running.id, &user), Some(SessionSecret::Password(password)))
        .expect("the session is saved with its guest password");

    let context = ToolContext::new(Arc::clone(&core), McpScope::HighRisk);
    let registry = local::default_registry();
    let call = |name: &'static str, arguments: serde_json::Value| {
        let registry = &registry;
        let context = &context;
        async move {
            match registry
                .call(
                    context,
                    ToolCall {
                        name: name.to_owned(),
                        arguments,
                    },
                )
                .await
            {
                ToolOutcome::Succeeded { result } => result.value,
                ToolOutcome::Failed { error } => {
                    panic!(
                        "{name} failed: {} {}",
                        error.code(),
                        error.message().unwrap_or_default()
                    )
                }
            }
        }
    };

    let processes = call(
        "sshell_vm_guest_read",
        json!({ "sessionId": "vm-live", "action": "processes" }),
    )
    .await;
    assert!(
        processes["output"].as_str().unwrap_or_default().contains("pid="),
        "the process list does not look like one: {processes}"
    );

    call(
        "sshell_vm_guest_write",
        json!({
            "sessionId": "vm-live",
            "action": "run",
            "program": "/bin/echo",
            "args": ["7shell-tool-live"]
        }),
    )
    .await;

    // One file in, one file back, through tokens the host mints.
    let source = directory.path().join("copy-in.txt");
    std::fs::write(&source, "7shell-tool-copy").expect("a fixture");
    let file_token = core
        .host_location_add(&source, HostLocationKind::File)
        .expect("a file token");
    let folder_token = core
        .host_location_add(directory.path(), HostLocationKind::Folder)
        .expect("a folder token");
    let guest_path = format!("/tmp/7shell-tool-{}.txt", std::process::id());

    call(
        "sshell_vm_guest_write",
        json!({
            "sessionId": "vm-live",
            "action": "copyTo",
            "token": file_token.token,
            "guestPath": guest_path
        }),
    )
    .await;
    call(
        "sshell_vm_guest_write",
        json!({
            "sessionId": "vm-live",
            "action": "copyFrom",
            "token": folder_token.token,
            "guestPath": guest_path
        }),
    )
    .await;
    let returned = directory.path().join(format!("7shell-tool-{}.txt", std::process::id()));
    assert_eq!(
        std::fs::read_to_string(&returned).expect("the returned file"),
        "7shell-tool-copy",
        "the file changed on the round trip"
    );

    // The one temporary file in the guest goes, and the tokens with it.
    let _ = core
        .vmware_guest("vm-live", &sevenshell_vmware::GuestOp::RemoveFile(guest_path))
        .await;
    let _ = core.host_location_forget(&file_token.token);
    let _ = core.host_location_forget(&folder_token.token);
}
