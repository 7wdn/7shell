//! The Workstation REST service, held at arm's length.
//!
//! The credentials live in memory for this session, or come from the two environment variables the
//! service's own documentation uses. Nothing here starts the service: the host command that does
//! is a separate, person-directed action, and `vmrest -C` remains a one-time setup somebody runs.

use std::path::PathBuf;

use serde_json::Value;
use sevenshell_error::ShellError;
use sevenshell_vmware::Vmrest;

use crate::Core;

impl Core {
    /// Keeps the REST credentials in the vault and uses them for this session too. An empty user
    /// forgets the stored account.
    pub fn set_vmrest_auth(&self, user: String, password: String) -> Result<(), ShellError> {
        self.vault.set_vmrest_credentials(&user, &password)?;
        let mut auth = self.vmrest_auth.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *auth = if user.trim().is_empty() {
            None
        } else {
            Some((user.trim().to_owned(), password))
        };
        Ok(())
    }

    fn vmrest(&self) -> Result<Vmrest, ShellError> {
        let stored = self
            .vmrest_auth
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some((user, password)) = stored {
            return Vmrest::new(user, password);
        }
        // A locked vault has no stored credentials to read; the environment is the only other place
        // they can come from, and the request itself will say whether they work.
        if let Ok(Some((user, password))) = self.vault.vmrest_credentials()
            && !user.trim().is_empty()
        {
            return Vmrest::new(user, password);
        }
        Vmrest::new(
            std::env::var("VMWARE_USERNAME").unwrap_or_default(),
            std::env::var("VMWARE_PASSWORD").unwrap_or_default(),
        )
    }

    /// Whether the service answers, with the credentials this session has.
    pub async fn vmrest_probe(&self) -> Result<(), ShellError> {
        self.vmrest()?.probe().await
    }

    /// The `vmrest` program beside the Workstation this build found.
    pub fn vmrest_program(&self) -> Option<PathBuf> {
        sevenshell_vmware::locate_vmrest()
    }

    pub async fn vmrest_list_vms(&self) -> Result<Value, ShellError> {
        self.vmrest()?.list_vms().await
    }

    pub async fn vmrest_get_vm(&self, id: &str) -> Result<Value, ShellError> {
        self.vmrest()?.get_vm(id).await
    }

    pub async fn vmrest_list_networks(&self) -> Result<Value, ShellError> {
        self.vmrest()?.list_networks().await
    }

    pub async fn vmrest_mac_to_ip(&self, vmnet: &str) -> Result<Value, ShellError> {
        self.vmrest()?.mac_to_ip(vmnet).await
    }

    pub async fn vmrest_create_network(&self, name: &str, kind: &str) -> Result<Value, ShellError> {
        self.vmrest()?.create_network(name, kind).await
    }
}
