//! The half that talks to the desktop.
//!
//! The endpoint's port and token are read from the file the desktop writes when it starts listening
//! — read again for every call, because a restart rotates the token and a cached one would be a
//! token that used to work. Nothing here starts the desktop: a bridge that could launch the
//! application would be a bridge that decides when a vault is open.
//!
//! Three requests cross this boundary. A call carries a call id of its own, so the desktop can start
//! the work under a name that a cancellation can reach; a cancellation names that id; and the tool
//! table is fetched rather than compiled, because the desktop knows the external servers' tools and
//! this build cannot. The compiled list stays as the floor: when the table cannot be read, it is
//! what `tools/list` answers.
//!
//! The token appears in no log line and in no error message. The path of the endpoint file does
//! appear on stderr when the file cannot be read, because that is the one place a person debugging
//! their own machine looks, and it is not where an agent's context gets its text from.

use std::path::PathBuf;
use std::sync::RwLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use sevenshell_api::{
    CALL_ID_HEADER, ENDPOINT_FILE_NAME, EndpointFile, MAX_BODY_BYTES, TOOL_CALL_PATH, TOOL_CANCEL_PATH, TOOLS_PATH,
    ToolCall, ToolDescriptor, ToolOutcome, local,
};

use crate::jsonrpc::{RpcError, TOOL_FAILED};
use crate::server::ToolClient;

/// The deadline for the two requests that are not a tool call.
///
/// A call has no total timeout — an approval window is a person's to answer — but listing and
/// cancelling are this bridge's own bookkeeping. One that hung would stall `tools/list` forever or
/// hold a cancellation slot, and five seconds is already generous on loopback.
const BOOKKEEPING_TIMEOUT: Duration = Duration::from_secs(5);

/// Where the endpoint file is.
///
/// The explicit path is for a test or a machine with an unusual layout; otherwise it is the file in
/// this installation's configuration directory, resolved by the same function the desktop writes it
/// with.
pub fn endpoint_path(explicit: Option<PathBuf>) -> Result<PathBuf, RpcError> {
    explicit
        .or_else(|| sevenshell_platform::config_dir().map(|directory| directory.join(ENDPOINT_FILE_NAME)))
        .ok_or_else(|| {
            RpcError::desktop_unavailable("the configuration directory could not be located on this machine")
        })
}

/// The real client: one HTTP request per call to the desktop's loopback endpoint.
pub struct HttpToolClient {
    endpoint_file: PathBuf,
    http: reqwest::Client,
    /// The list this build knows without asking: the compiled registry. Kept separately so a
    /// refresh that cannot reach the desktop falls back to it rather than to nothing, and so a list
    /// fetched from a desktop that has since gone away does not outlive it.
    built_in: Vec<ToolDescriptor>,
    /// The table the desktop last listed, or `built_in` when it has never answered. A `RwLock`
    /// rather than an `async` lock: it is read and replaced in one statement, never held across an
    /// await.
    descriptors: RwLock<Vec<ToolDescriptor>>,
    /// The last limit the endpoint named. Re-read on every call because a restart may carry a new
    /// setting; the default is what a bridge uses until it has seen a file.
    max_result_bytes: AtomicUsize,
}

impl HttpToolClient {
    pub fn new(endpoint_file: PathBuf) -> Result<Self, RpcError> {
        let http = reqwest::Client::builder()
            // The endpoint is loopback. A proxy that a person configured for the internet has no
            // business in front of it, and on a machine with a system proxy the default client would
            // send the token there.
            .no_proxy()
            // The connection is to this machine; five seconds is already generous for a refusal.
            // No total timeout: a batch or an approval window may legitimately take minutes.
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| RpcError::internal(format!("the HTTP client could not be built: {error}")))?;

        let built_in = local::default_registry().descriptors();

        Ok(Self {
            endpoint_file,
            http,
            descriptors: RwLock::new(built_in.clone()),
            built_in,
            max_result_bytes: AtomicUsize::new(
                usize::try_from(sevenshell_api::DEFAULT_MAX_RESULT_BYTES).unwrap_or(usize::MAX),
            ),
        })
    }

    fn read_endpoint(&self) -> Result<EndpointFile, RpcError> {
        let raw = std::fs::read_to_string(&self.endpoint_file).map_err(|error| {
            eprintln!(
                "sshell-mcp: {} could not be read: {error}",
                self.endpoint_file.display()
            );
            RpcError::desktop_unavailable(
                "its control endpoint file is not there; start 7Shell and turn the agent interface on",
            )
        })?;

        let endpoint: EndpointFile = serde_json::from_str(&raw).map_err(|error| {
            eprintln!(
                "sshell-mcp: {} is not an endpoint file: {error}",
                self.endpoint_file.display()
            );
            RpcError::desktop_unavailable("its control endpoint file cannot be read")
        })?;

        if endpoint.port == 0 || endpoint.token.is_empty() {
            return Err(RpcError::desktop_unavailable(
                "its control endpoint file is incomplete; restart 7Shell",
            ));
        }

        Ok(endpoint)
    }

    /// Reads the desktop's whole tool table.
    ///
    /// A refusal, a status other than 200 and a body that is not a list are all [`RpcError`]s here;
    /// [`ToolClient::refresh_descriptors`] is what decides they are not fatal to the session.
    async fn fetch_tools(&self) -> Result<Vec<ToolDescriptor>, RpcError> {
        let endpoint = self.read_endpoint()?;

        let response = self
            .http
            .get(format!("{}{TOOLS_PATH}", endpoint.base_url()))
            .bearer_auth(&endpoint.token)
            .timeout(BOOKKEEPING_TIMEOUT)
            .send()
            .await
            .map_err(|error| {
                eprintln!("sshell-mcp: the control endpoint did not answer: {error}");
                RpcError::desktop_unavailable("it is not answering on its control endpoint")
            })?;

        let status = response.status();
        let bytes = response.bytes().await.map_err(|error| {
            eprintln!("sshell-mcp: the tool list could not be read: {error}");
            RpcError::desktop_unavailable("its answer could not be read")
        })?;

        match status.as_u16() {
            200 => parse_tools(&bytes).ok_or_else(|| {
                eprintln!("sshell-mcp: the control endpoint answered something that is not a tool list");
                RpcError::desktop_unavailable("it answered something that is not a tool list")
            }),
            401 => Err(RpcError::desktop_unavailable(
                "it refused this bridge's token; restart 7Shell",
            )),
            other => {
                eprintln!("sshell-mcp: the control endpoint answered status {other} for its tool list");
                Err(RpcError::desktop_unavailable(format!(
                    "its tool list answered status {other}"
                )))
            }
        }
    }

    fn set_descriptors(&self, descriptors: Vec<ToolDescriptor>) {
        *self
            .descriptors
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = descriptors;
    }

    fn cached_descriptors(&self) -> Vec<ToolDescriptor> {
        self.descriptors
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl ToolClient for HttpToolClient {
    fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.cached_descriptors()
    }

    /// Fetches the desktop's table, and answers `Ok` whether it arrived or not.
    ///
    /// The list is an answer either way — the compiled one is what this build offers — so a desktop
    /// that is not running is not a session-level failure. The reason is on stderr, and the next
    /// `tools/list` tries again, which is how a bridge that started before the desktop catches up.
    async fn refresh_descriptors(&self) -> Result<(), RpcError> {
        match self.fetch_tools().await {
            Ok(descriptors) => {
                self.set_descriptors(descriptors);
                Ok(())
            }
            Err(error) => {
                eprintln!(
                    "sshell-mcp: the desktop's tool list could not be read; using the built-in one: {}",
                    error.message
                );
                // Back to the floor, not to the last fetched list: a table from a desktop that has
                // since gone away would offer an agent tools that no running process can answer for.
                self.set_descriptors(self.built_in.clone());
                Ok(())
            }
        }
    }

    async fn call(&self, name: &str, arguments: Value, call_id: &str) -> Result<ToolOutcome, RpcError> {
        let endpoint = self.read_endpoint()?;
        // Zero is not a usable limit and a malformed file should not silently truncate every answer
        // to nothing; the default is what the desktop writes, and the clamp keeps a bad file sane.
        let limit = usize::try_from(endpoint.max_result_bytes).unwrap_or(usize::MAX).max(1);
        self.max_result_bytes.store(limit, Ordering::Relaxed);

        let call = ToolCall {
            name: name.to_owned(),
            arguments,
        };

        let body = serde_json::to_vec(&call)
            .map_err(|error| RpcError::internal(format!("the call could not be serialized: {error}")))?;

        // Refused before it is sent rather than after the desktop reads it: the endpoint's limit is
        // the contract, and a caller learns the number from the refusal.
        if body.len() > MAX_BODY_BYTES {
            return Err(RpcError::invalid_params(format!(
                "the arguments are {} bytes, over the control endpoint's {MAX_BODY_BYTES}-byte limit",
                body.len()
            )));
        }

        let response = self
            .http
            .post(format!("{}{TOOL_CALL_PATH}", endpoint.base_url()))
            .bearer_auth(&endpoint.token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            // The call's name for itself. A header rather than a body field because `ToolCall` is
            // frozen; a desktop that has never heard of it ignores it and the call still runs.
            .header(CALL_ID_HEADER, call_id)
            .body(body)
            .send()
            .await
            .map_err(|error| {
                eprintln!("sshell-mcp: the control endpoint did not answer: {error}");
                RpcError::desktop_unavailable("it is not answering on its control endpoint")
            })?;

        let status = response.status();
        let bytes = response.bytes().await.map_err(|error| {
            eprintln!("sshell-mcp: the control endpoint's answer could not be read: {error}");
            RpcError::desktop_unavailable("its answer could not be read")
        })?;

        match status.as_u16() {
            200 => serde_json::from_slice::<ToolOutcome>(&bytes).map_err(|error| {
                eprintln!("sshell-mcp: the control endpoint answered something else: {error}");
                RpcError::desktop_unavailable("it answered something that is not a tool outcome")
            }),
            401 => Err(RpcError::desktop_unavailable(
                "it refused this bridge's token; restart 7Shell",
            )),
            413 => Err(RpcError::invalid_params(
                "the arguments are larger than the control endpoint accepts",
            )),
            other => {
                eprintln!("sshell-mcp: the control endpoint answered status {other}");
                Err(RpcError::desktop_unavailable(format!(
                    "its control endpoint answered status {other}"
                )))
            }
        }
    }

    async fn cancel(&self, call_id: &str) -> Result<(), RpcError> {
        let endpoint = self.read_endpoint()?;

        let body = serde_json::to_vec(&CancelCall { call_id })
            .map_err(|error| RpcError::internal(format!("the cancellation could not be serialized: {error}")))?;

        let response = self
            .http
            .post(format!("{}{TOOL_CANCEL_PATH}", endpoint.base_url()))
            .bearer_auth(&endpoint.token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .timeout(BOOKKEEPING_TIMEOUT)
            .body(body)
            .send()
            .await
            .map_err(|error| {
                eprintln!("sshell-mcp: the control endpoint did not answer a cancellation: {error}");
                RpcError::desktop_unavailable("it is not answering on its control endpoint")
            })?;

        match response.status().as_u16() {
            // Any 200 is a cancellation; whether the call was still running is the desktop's answer
            // to give, and `false` is a truthful one for work that already ended.
            200 => Ok(()),
            401 => Err(RpcError::desktop_unavailable(
                "it refused this bridge's token; restart 7Shell",
            )),
            other => {
                // A desktop that answered and could not cancel is not an unreachable desktop: the
                // tool's own failure code is what says a call-level request went wrong.
                eprintln!("sshell-mcp: the control endpoint answered status {other} for a cancellation");
                Err(RpcError::new(
                    TOOL_FAILED,
                    format!("the control endpoint could not cancel the call: status {other}"),
                ))
            }
        }
    }

    fn max_result_bytes(&self) -> usize {
        self.max_result_bytes.load(Ordering::Relaxed)
    }
}

/// What `POST /tools/cancel` carries.
///
/// camelCase because it is wire: the desktop reads `callId`. A named field rather than a literal so
/// the body cannot drift from the document in `sevenshell-api` by a typo.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CancelCall<'a> {
    call_id: &'a str,
}

/// Reads the desktop's answer to `GET /tools`.
///
/// Two shapes are accepted, a bare array and the protocol's `{"tools": [...]}`. The endpoint's
/// contract names the route and the token but not the envelope, and a bridge that guessed one would
/// fall back to its compiled list against a desktop that thought it had just listed everything. An
/// empty list is treated as no answer: every desktop has its own tools, so `[]` is a broken reply,
/// and showing an agent zero tools is worse than showing the ten this build knows.
fn parse_tools(bytes: &[u8]) -> Option<Vec<ToolDescriptor>> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let list = match value {
        Value::Array(_) => value,
        Value::Object(object) => object.get("tools")?.clone(),
        _ => return None,
    };

    let descriptors: Vec<ToolDescriptor> = serde_json::from_value(list).ok()?;
    (!descriptors.is_empty()).then_some(descriptors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_endpoint_file_that_is_not_there_is_reported_without_the_path_in_the_answer() {
        let directory = tempfile::tempdir().expect("temp dir");
        let client = HttpToolClient::new(directory.path().join(ENDPOINT_FILE_NAME)).expect("a client");

        let error = client.read_endpoint().expect_err("nothing was written");

        assert_eq!(error.code, crate::jsonrpc::DESKTOP_UNAVAILABLE);
        assert!(
            !error.message.contains(directory.path().to_string_lossy().as_ref()),
            "the path belongs on stderr, not in an answer an agent reads: {}",
            error.message
        );
    }

    #[test]
    fn a_written_endpoint_file_is_read_back() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join(ENDPOINT_FILE_NAME);
        let written = EndpointFile::new(4711, "a-token", 4242, 1_700_000_000);
        std::fs::write(&path, serde_json::to_string(&written).expect("json")).expect("write");

        let client = HttpToolClient::new(path).expect("a client");
        let read = client.read_endpoint().expect("readable");

        assert_eq!(read, written);
    }

    #[test]
    fn an_incomplete_endpoint_file_is_refused_rather_than_sent_a_token() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join(ENDPOINT_FILE_NAME);
        std::fs::write(&path, r#"{"port":4711,"token":"","pid":1,"startedAt":0}"#).expect("write");

        let client = HttpToolClient::new(path).expect("a client");

        assert_eq!(
            client.read_endpoint().expect_err("an empty token").code,
            crate::jsonrpc::DESKTOP_UNAVAILABLE
        );
    }

    #[test]
    fn an_explicit_endpoint_file_wins_over_the_configuration_directory() {
        let path = endpoint_path(Some(PathBuf::from("somewhere/else.json"))).expect("a path");
        assert_eq!(path, PathBuf::from("somewhere/else.json"));
    }
}
