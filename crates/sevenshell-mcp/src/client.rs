//! The half that talks to the desktop.
//!
//! The endpoint's port and token are read from the file the desktop writes when it starts listening
//! — read again for every call, because a restart rotates the token and a cached one would be a
//! token that used to work. Nothing here starts the desktop: a bridge that could launch the
//! application would be a bridge that decides when a vault is open.
//!
//! The token appears in no log line and in no error message. The path of the endpoint file does
//! appear on stderr when the file cannot be read, because that is the one place a person debugging
//! their own machine looks, and it is not where an agent's context gets its text from.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use sevenshell_api::{
    ENDPOINT_FILE_NAME, EndpointFile, MAX_BODY_BYTES, TOOL_CALL_PATH, ToolCall, ToolDescriptor, ToolOutcome, local,
};

use crate::jsonrpc::RpcError;
use crate::server::ToolClient;

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
    /// Compiled in, not fetched: the descriptors are this build's own registry, so `tools/list` works
    /// before the desktop is up and cannot disagree with what a call is allowed to be.
    descriptors: Vec<ToolDescriptor>,
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

        Ok(Self {
            endpoint_file,
            http,
            descriptors: local::default_registry().descriptors(),
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
}

#[async_trait]
impl ToolClient for HttpToolClient {
    fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.descriptors.clone()
    }

    async fn call(&self, name: &str, arguments: Value) -> Result<ToolOutcome, RpcError> {
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

    fn max_result_bytes(&self) -> usize {
        self.max_result_bytes.load(Ordering::Relaxed)
    }
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
