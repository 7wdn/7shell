//! The descriptor tables, from the outside.
//!
//! These assertions are what keeps `descriptors/**/*.yaml` honest: a capability claimed
//! without a catalog entry, an unlabelled protocol or a renamed wire identifier fails
//! here rather than surprising the interface at runtime.

use sevenshell_types::{
    AUTH_METHODS, AppInfo, AuthMethod, Capability, CapabilityGroup, PROTOCOLS, Protocol, Runtime, SupportLevel,
};

#[test]
fn protocols_are_declared_with_a_port_runtime_and_support_level() {
    assert!(!PROTOCOLS.is_empty(), "at least one protocol must be declared");
    for spec in PROTOCOLS {
        assert!(!spec.id.is_empty());
        assert!(!spec.transport.is_empty());
        assert!(matches!(
            spec.runtime,
            Runtime::Native | Runtime::Sidecar | Runtime::External
        ));
        assert!(matches!(
            spec.support_level,
            SupportLevel::Connect | SupportLevel::Browse | SupportLevel::Operate
        ));
    }
}

#[test]
fn ssh_carries_the_capabilities_we_committed_to() {
    let ssh = Protocol::Ssh;

    assert_eq!(ssh.default_port(), Some(22));
    assert_eq!(ssh.support_level(), SupportLevel::Operate);
    assert_eq!(ssh.runtime(), Runtime::Native);

    // What this build does today: a shell, a command run without one, resized when the window
    // changes, kept alive while idle, reachable through another saved session, able to answer a
    // local port from the far side of the connection — or to be a SOCKS5 proxy for whatever a
    // client asks for — and able to say what the host is doing.
    assert!(ssh.supports(Capability::Shell));
    assert!(ssh.supports(Capability::Exec));
    assert!(ssh.supports(Capability::Resize));
    assert!(ssh.supports(Capability::Keepalive));
    assert!(ssh.supports(Capability::HostMetrics));
    assert!(ssh.supports(Capability::Sftp));
    assert!(ssh.supports(Capability::Forwarding));
    assert!(ssh.supports(Capability::DynamicForwarding));
    assert!(ssh.supports(Capability::JumpHost));

    // What it does not do yet. These stay in the vocabulary as `planned`, so they are neither
    // offered by the interface nor quietly forgotten.
    for unclaimed in [Capability::AgentForwarding, Capability::Recording] {
        assert!(
            !ssh.supports(unclaimed),
            "{unclaimed} is not implemented and must not be claimed"
        );
    }
}

#[test]
fn every_claimed_capability_exists_in_the_catalog() {
    for spec in PROTOCOLS {
        for capability in spec.capabilities {
            assert!(
                Capability::ALL.contains(capability),
                "{} claims an unknown capability",
                spec.id
            );
        }
    }
    for method in AUTH_METHODS {
        for capability in method.capabilities {
            assert!(
                Capability::ALL.contains(capability),
                "{} claims an unknown capability",
                method.id
            );
        }
    }
}

#[test]
fn wire_identifiers_round_trip() {
    for protocol in Protocol::ALL {
        assert_eq!(Protocol::from_id(protocol.as_str()), Some(*protocol));
    }
    for capability in Capability::ALL {
        assert_eq!(Capability::from_id(capability.as_str()), Some(*capability));
    }
    for method in AuthMethod::ALL {
        assert_eq!(AuthMethod::from_id(method.as_str()), Some(*method));
    }
}

#[test]
fn labels_are_message_keys_and_not_literal_text() {
    for spec in PROTOCOLS {
        assert!(
            spec.label_key.starts_with("protocol."),
            "{} is not namespaced",
            spec.label_key
        );
        assert!(
            spec.label_key.ends_with(".label"),
            "{} is not a label key",
            spec.label_key
        );
    }
    for method in AUTH_METHODS {
        assert!(
            method.label_key.starts_with("auth."),
            "{} is not namespaced",
            method.label_key
        );
        assert!(
            method.label_key.ends_with(".label"),
            "{} is not a label key",
            method.label_key
        );
    }
    for capability in Capability::ALL {
        let key = capability.label_key();
        assert!(key.starts_with("capability."), "{key} is not namespaced");
        assert!(key.ends_with(".label"), "{key} is not a label key");
    }
}

#[test]
fn capabilities_are_grouped_and_ordered() {
    for capability in Capability::ALL {
        assert!(capability.order() > 0, "{capability} has no display order");
        assert!(matches!(
            capability.group(),
            CapabilityGroup::Terminal
                | CapabilityGroup::Transfer
                | CapabilityGroup::Network
                | CapabilityGroup::Session
                | CapabilityGroup::Auth
                | CapabilityGroup::Trust
        ));
    }
}

#[test]
fn auth_methods_declare_where_they_exist() {
    assert!(AuthMethod::Password.available_on("linux"));
    assert!(AuthMethod::Password.available_on("windows"));
    assert!(AuthMethod::PublicKey.requires_secret(), "a key may carry a passphrase");
    assert!(AuthMethod::KeyboardInteractive.is_interactive());
    assert!(!AuthMethod::Password.is_interactive());

    let pageant = AuthMethod::Pageant;
    assert!(pageant.available_on("windows"));
    assert!(!pageant.available_on("linux"), "Pageant only exists on Windows");
}

#[test]
fn app_info_reports_the_product_and_the_descriptor_pipeline() {
    let info = AppInfo::current();

    assert_eq!(info.name, "7Shell");
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(info.protocol_count, Protocol::ALL.len());
    assert_eq!(info.descriptor_schema_version, 1);
}
