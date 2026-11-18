//! Live checks against the Workstation REST service.
//!
//! Gated by `SEVENSHELL_TEST_VMREST_USER` / `SEVENSHELL_TEST_VMREST_PASSWORD`. The reads leave no
//! trace; the one thing this file deliberately does not do is create a virtual network, because
//! the service offers no matching delete and a test must not leave a network behind.

use sevenshell_error::ErrorCode;
use sevenshell_vmware::Vmrest;

fn credentials() -> Option<(String, String)> {
    let user = std::env::var("SEVENSHELL_TEST_VMREST_USER").ok()?;
    let password = std::env::var("SEVENSHELL_TEST_VMREST_PASSWORD").ok()?;
    Some((user, password))
}

#[tokio::test]
#[ignore]
async fn the_service_answers_and_its_machine_list_carries_no_paths() {
    let Some((user, password)) = credentials() else {
        return;
    };
    let client = Vmrest::new(user.clone(), password.clone()).expect("a client");

    client.probe().await.expect("the service answers");
    let machines = client.list_vms().await.expect("the machine list");
    let machines = machines.as_array().expect("a list of machines");
    assert!(
        machines.iter().all(|machine| machine.get("path").is_none()),
        "a host path left the service: {machines:?}"
    );
    assert!(
        machines.iter().all(|machine| machine.get("id").is_some()),
        "an entry without an id is not a machine: {machines:?}"
    );

    let networks = client.list_networks().await.expect("the network list");
    let networks = networks["vmnets"].as_array().expect("a list of networks");
    assert!(!networks.is_empty(), "a Workstation install has at least one network");

    if let Some(id) = machines.first().and_then(|machine| machine["id"].as_str()) {
        let one = client.get_vm(id).await.expect("one machine");
        assert_eq!(one["id"], id);
        assert!(
            one.get("path").is_none(),
            "the machine's settings carry a path: {one:?}"
        );
    }
    if let Some(network) = networks.first().and_then(|network| network["name"].as_str()) {
        client.mac_to_ip(network).await.expect("the MAC-to-IP map");
    }

    let wrong = Vmrest::new(user, "definitely-not-the-password1!".to_owned()).expect("a client");
    assert_eq!(
        wrong.probe().await.expect_err("wrong credentials").code(),
        ErrorCode::VmrestUnauthorized
    );
}
