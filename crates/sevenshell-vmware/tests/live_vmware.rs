//! Live checks against the Workstation installed on this machine.
//!
//! These read only: they scan what Workstation's inventory holds and ask the running machine what
//! it is doing. Power actions change somebody's machine, so they are not part of this file — the
//! unit tests pin the argument vectors, and a person decides when a guest boots or stops.
//!
//! `SEVENSHELL_TEST_VMWARE=1` gates them, the same way the other live suites are gated.

#[tokio::test]
#[ignore]
async fn the_inventory_lists_machines_and_a_running_one_answers_its_status() {
    if std::env::var("SEVENSHELL_TEST_VMWARE").as_deref() != Ok("1") {
        return;
    }

    let machines = sevenshell_vmware::scan().await.expect("the machine lists its VMs");
    assert!(
        !machines.is_empty(),
        "a Workstation install with VMs must list at least one, even when none is running"
    );
    assert!(
        machines.iter().all(|machine| machine.id.len() == 32),
        "every entry carries Workstation's identity and no path"
    );

    let Some(running) = machines.iter().find(|machine| machine.running) else {
        return;
    };
    let status = sevenshell_vmware::status(&running.id)
        .await
        .expect("a running machine answers its status");
    assert!(status.running, "{} was listed as running", running.name);
}

#[tokio::test]
#[ignore]
async fn an_id_that_is_not_a_virtual_machine_is_refused_before_vmrun_runs() {
    if std::env::var("SEVENSHELL_TEST_VMWARE").as_deref() != Ok("1") {
        return;
    }

    let error = sevenshell_vmware::status("not-a-machine").await.unwrap_err();
    assert_eq!(error.code(), sevenshell_error::ErrorCode::VmwareCommandFailed);
}

#[tokio::test]
#[ignore]
async fn a_screen_captures_into_a_temporary_file() {
    if std::env::var("SEVENSHELL_TEST_VMWARE").as_deref() != Ok("1") {
        return;
    }

    let machines = sevenshell_vmware::scan().await.expect("the machine lists its VMs");
    let Some(running) = machines.iter().find(|machine| machine.running) else {
        return;
    };
    let directory = std::env::temp_dir().join(format!("7shell-vmware-shot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a temporary directory");
    let file = directory.join("screen.png");

    sevenshell_vmware::screenshot(&running.id, file.to_str().expect("a path"))
        .await
        .expect("the console captures");
    let bytes = std::fs::metadata(&file).expect("the file is there").len();
    assert!(bytes > 0, "a screenshot with no bytes is not a screenshot");
    let _ = std::fs::remove_dir_all(&directory);
}
