//! Hand-written behaviour for the generated descriptor tables.
//!
//! The tables are generated; the lookups and the questions the rest of the application
//! asks about them ("does this protocol support SFTP?", "is this auth method available
//! on this platform?") live here, where they can be read and tested.

use crate::generated::{
    AUTH_METHODS, AuthMethod, AuthMethodSpec, CAPABILITIES, Capability, CapabilityGroup, CapabilitySpec, PROTOCOLS,
    Protocol, ProtocolSpec, Runtime, SupportLevel,
};

impl Capability {
    /// The declaration of this capability in `descriptors/capabilities.yaml`.
    pub fn spec(self) -> &'static CapabilitySpec {
        CAPABILITIES
            .iter()
            .find(|spec| spec.capability == self)
            .expect("every capability has a spec")
    }

    /// The message key for the human-readable name.
    pub fn label_key(self) -> &'static str {
        self.spec().label_key
    }

    pub fn group(self) -> CapabilityGroup {
        self.spec().group
    }

    pub fn order(self) -> u16 {
        self.spec().order
    }
}

impl Protocol {
    /// The declaration of this protocol in `descriptors/protocols/`.
    pub fn spec(self) -> &'static ProtocolSpec {
        PROTOCOLS
            .iter()
            .find(|spec| spec.protocol == self)
            .expect("every protocol has a spec")
    }

    pub fn label_key(self) -> &'static str {
        self.spec().label_key
    }

    /// The port offered when a new host is created, when the protocol has a usual one.
    pub fn default_port(self) -> Option<u16> {
        self.spec().default_port
    }

    pub fn runtime(self) -> Runtime {
        self.spec().runtime
    }

    pub fn transport(self) -> &'static str {
        self.spec().transport
    }

    pub fn support_level(self) -> SupportLevel {
        self.spec().support_level
    }

    /// The capabilities committed for this protocol, in descriptor order.
    pub fn capabilities(self) -> &'static [Capability] {
        self.spec().capabilities
    }

    /// Whether the product supports `capability` on this protocol. The interface uses
    /// this to decide which panes exist; the policy layer uses it to decide which
    /// actions may be attempted at all.
    pub fn supports(self, capability: Capability) -> bool {
        self.capabilities().contains(&capability)
    }
}

impl AuthMethod {
    /// The declaration of this method in `descriptors/auth-methods/`.
    pub fn spec(self) -> &'static AuthMethodSpec {
        AUTH_METHODS
            .iter()
            .find(|spec| spec.method == self)
            .expect("every auth method has a spec")
    }

    pub fn label_key(self) -> &'static str {
        self.spec().label_key
    }

    pub fn capabilities(self) -> &'static [Capability] {
        self.spec().capabilities
    }

    pub fn supports(self, capability: Capability) -> bool {
        self.capabilities().contains(&capability)
    }

    /// The platforms this method exists on. Empty means every platform.
    pub fn platforms(self) -> &'static [&'static str] {
        self.spec().platforms
    }

    pub fn available_on(self, platform: &str) -> bool {
        let platforms = self.platforms();
        platforms.is_empty() || platforms.contains(&platform)
    }

    /// Whether this method is usable on the platform we are running on.
    pub fn available_here(self) -> bool {
        self.available_on(std::env::consts::OS)
    }

    /// Whether the user has to supply a secret that has to be stored.
    pub fn requires_secret(self) -> bool {
        self.supports(Capability::Secret)
    }

    /// Whether the method can ask follow-up questions during the handshake.
    pub fn is_interactive(self) -> bool {
        self.supports(Capability::Interactive)
    }
}
