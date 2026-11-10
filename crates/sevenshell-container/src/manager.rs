//! The connections a container tab holds open.

/// The container manager.
///
/// One per application, like the session registry: a container tab asks it to connect, then
/// addresses the connection by an id it was given. The implementation lives behind this type so
/// the host never learns which runtime is on the other side.
pub struct ContainerManager;

impl ContainerManager {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ContainerManager {
    fn default() -> Self {
        Self::new()
    }
}
