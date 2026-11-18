//! The Workstation REST service, `vmrest`.
//!
//! It is a local HTTP service on `127.0.0.1:8697` with basic authentication, and it is the one
//! interface that can create a virtual network. Everything else it offers has a command-line
//! equivalent this build already uses. Credentials live in memory for the session, or come from
//! `VMWARE_USERNAME`/`VMWARE_PASSWORD`; the service needs `vmrest -C` once before either works.

use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::{Client, Method, StatusCode};
use serde_json::Value;
use sevenshell_error::ShellError;

const DEFAULT_BASE: &str = "http://127.0.0.1:8697/api";
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Vmrest {
    http: Client,
    base: String,
    auth: Option<(String, String)>,
}

impl Vmrest {
    pub fn new(user: String, password: String) -> Result<Self, ShellError> {
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| ShellError::vmrest_unavailable(format!("the REST client could not be built: {error}")))?;
        let auth = (!user.trim().is_empty()).then(|| (user.trim().to_owned(), password));
        let base = std::env::var("VMREST_URL").unwrap_or_else(|_| DEFAULT_BASE.to_owned());
        Ok(Self { http, base, auth })
    }

    /// Whether the service answers at all, with the credentials it was given.
    pub async fn probe(&self) -> Result<(), ShellError> {
        self.get("vms", PROBE_TIMEOUT).await.map(|_| ())
    }

    /// The machines the service knows, as ids only.
    ///
    /// The service's own answer carries each machine's `.vmx` path. That path is exactly what this
    /// build never hands to an interface or an agent, so the answer is reduced to the id here.
    pub async fn list_vms(&self) -> Result<Value, ShellError> {
        let value = self.get("vms", REQUEST_TIMEOUT).await?;
        Ok(match value {
            Value::Array(machines) => Value::Array(
                machines
                    .into_iter()
                    .filter_map(|machine| {
                        machine
                            .get("id")
                            .and_then(Value::as_str)
                            .map(|id| serde_json::json!({ "id": id }))
                    })
                    .collect(),
            ),
            other => other,
        })
    }

    pub async fn get_vm(&self, id: &str) -> Result<Value, ShellError> {
        let id = component(id, "vm id")?;
        let value = self.get(&format!("vms/{id}"), REQUEST_TIMEOUT).await?;
        Ok(strip_paths(value))
    }

    pub async fn list_networks(&self) -> Result<Value, ShellError> {
        self.get("vmnet", REQUEST_TIMEOUT).await
    }

    pub async fn mac_to_ip(&self, vmnet: &str) -> Result<Value, ShellError> {
        let vmnet = component(vmnet, "vmnet")?;
        self.get(&format!("vmnet/{vmnet}/mactoip"), REQUEST_TIMEOUT).await
    }

    /// Creates one host virtual network, which no command line can do.
    pub async fn create_network(&self, name: &str, kind: &str) -> Result<Value, ShellError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ShellError::vmware_command_failed("vmrest", "the network name is empty"));
        }
        let kind = match kind.trim().to_ascii_lowercase().as_str() {
            "bridged" => "bridged",
            "nat" => "nat",
            "hostonly" => "hostonly",
            _ => {
                return Err(ShellError::vmware_command_failed(
                    "vmrest",
                    "the network type must be `bridged`, `nat` or `hostonly`",
                ));
            }
        };
        let body = serde_json::json!({ "name": name, "type": kind });
        self.send(Method::POST, "vmnets", Some(body), REQUEST_TIMEOUT).await
    }

    async fn get(&self, path: &str, timeout: Duration) -> Result<Value, ShellError> {
        self.send(Method::GET, path, None, timeout).await
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, ShellError> {
        let url = format!("{}/{}", self.base.trim_end_matches('/'), path);
        let mut request = self.http.request(method, url).timeout(timeout);
        if let Some((user, password)) = &self.auth {
            request = request.basic_auth(user, Some(password));
        }
        if let Some(body) = body {
            let text = serde_json::to_string(&body).map_err(|error| {
                ShellError::vmware_command_failed("vmrest", format!("the body is not JSON: {error}"))
            })?;
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(text);
        }

        let response = request
            .send()
            .await
            .map_err(|error| ShellError::vmrest_unavailable(format!("the REST request could not be sent: {error}")))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();

        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(ShellError::vmrest_unauthorized(format!(
                "the service answered {status}"
            )));
        }
        if !status.is_success() {
            return Err(ShellError::vmware_command_failed(
                "vmrest",
                format!("the service answered {status}: {text}"),
            ));
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text)
            .map_err(|error| ShellError::vmware_command_failed("vmrest", format!("the answer was not JSON: {error}")))
    }
}

/// Where `vmrest.exe` lives, for the host command that starts it.
pub fn locate() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("VMREST_PATH").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    if let Ok(vmrun) = crate::Vmrun::locate() {
        let name = if cfg!(windows) { "vmrest.exe" } else { "vmrest" };
        let sibling = vmrun.program().with_file_name(name);
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    let name = if cfg!(windows) { "vmrest.exe" } else { "vmrest" };
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(if cfg!(windows) { ';' } else { ':' })
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| Path::new(entry).join(name))
        .find(|path| path.is_file())
}

/// Drops every field called `path` at any depth, in case a service answer grows one.
fn strip_paths(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(key, _)| !key.eq_ignore_ascii_case("path"))
                .map(|(key, value)| (key, strip_paths(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(strip_paths).collect()),
        other => other,
    }
}

/// One path component: an id from the service, never a path an agent composed.
fn component(value: &str, what: &str) -> Result<String, ShellError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed
            .chars()
            .any(|character| character == '/' || character == '\\' || character.is_control())
    {
        return Err(ShellError::vmware_command_failed(
            "vmrest",
            format!("the {what} is not a single identifier"),
        ));
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identifier_may_not_be_a_path() {
        assert!(component("vm-1", "vm id").is_ok());
        assert!(component("../x", "vm id").is_err());
        assert!(component("a/b", "vm id").is_err());
        assert!(component("", "vm id").is_err());
    }
}
