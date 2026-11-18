//! Live guest checks: the operations that sign in to a running machine.
//!
//! Gated by `SEVENSHELL_TEST_VMWARE=1` and the guest account in
//! `SEVENSHELL_TEST_GUEST_USER` / `SEVENSHELL_TEST_GUEST_PASSWORD`. They read, run a script that
//! prints one line, and copy one small temporary file into `/tmp` and back; the file is removed
//! before the test ends. Nothing else on the guest is touched.

fn guest_auth() -> Option<sevenshell_vmware::GuestAuth> {
    if std::env::var("SEVENSHELL_TEST_VMWARE").as_deref() != Ok("1") {
        return None;
    }
    let user = std::env::var("SEVENSHELL_TEST_GUEST_USER").ok()?;
    let password = std::env::var("SEVENSHELL_TEST_GUEST_PASSWORD").ok()?;
    Some(sevenshell_vmware::GuestAuth { user, password })
}

async fn running_vm() -> Option<String> {
    let machines = sevenshell_vmware::scan().await.expect("the machine lists its VMs");
    machines
        .iter()
        .find(|machine| machine.running)
        .map(|machine| machine.id.clone())
}

#[tokio::test]
#[ignore]
async fn the_guest_directory_and_processes_are_readable() {
    let Some(auth) = guest_auth() else {
        return;
    };
    let Some(vm_id) = running_vm().await else {
        return;
    };

    let listing = sevenshell_vmware::guest(
        &vm_id,
        &sevenshell_vmware::GuestOp::ListDirectory("/tmp".to_owned()),
        &auth,
    )
    .await
    .expect("the guest lists /tmp");
    assert!(!listing.trim().is_empty(), "an empty listing from /tmp proves nothing");

    let processes = sevenshell_vmware::guest(&vm_id, &sevenshell_vmware::GuestOp::Processes, &auth)
        .await
        .expect("the guest lists its processes");
    assert!(
        processes.contains("pid="),
        "the process list does not look like one: {processes}"
    );
}

#[tokio::test]
#[ignore]
async fn the_guest_environment_and_a_script_run() {
    let Some(auth) = guest_auth() else {
        return;
    };
    let Some(vm_id) = running_vm().await else {
        return;
    };

    let environment = sevenshell_vmware::guest_env(&vm_id, &auth)
        .await
        .expect("the guest prints its environment");
    assert!(
        environment.contains("PATH"),
        "the environment has no PATH in it: {environment}"
    );

    let script = sevenshell_vmware::guest(
        &vm_id,
        &sevenshell_vmware::GuestOp::RunScript {
            interpreter: "/bin/sh".to_owned(),
            script: "echo 7shell-live".to_owned(),
            no_wait: false,
            interactive: false,
        },
        &auth,
    )
    .await
    .expect("a script runs in the guest");
    assert!(
        script.contains("7shell-live") || script.is_empty(),
        "a script that failed would not have run: {script}"
    );
}

#[tokio::test]
#[ignore]
async fn a_small_file_copies_into_the_guest_and_back() {
    let Some(auth) = guest_auth() else {
        return;
    };
    let Some(vm_id) = running_vm().await else {
        return;
    };

    let directory = std::env::temp_dir().join(format!("7shell-vmware-guest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a temporary directory");
    let source = directory.join("copy-in.txt");
    let returned = directory.join("copy-out.txt");
    std::fs::write(&source, "7shell-live-copy").expect("a fixture");
    let guest_path = format!("/tmp/7shell-live-{}.txt", std::process::id());

    sevenshell_vmware::copy_to_guest(&vm_id, source.to_str().expect("a path"), &guest_path, &auth)
        .await
        .expect("the file copies in");
    sevenshell_vmware::copy_from_guest(&vm_id, &guest_path, returned.to_str().expect("a path"), &auth)
        .await
        .expect("the file copies back");
    assert_eq!(
        std::fs::read_to_string(&returned).expect("the returned file"),
        "7shell-live-copy",
        "the file changed on the round trip"
    );

    // Leave the guest as it was found: the one temporary file goes.
    let _ = sevenshell_vmware::guest(&vm_id, &sevenshell_vmware::GuestOp::RemoveFile(guest_path), &auth).await;
    let _ = std::fs::remove_dir_all(&directory);
}
