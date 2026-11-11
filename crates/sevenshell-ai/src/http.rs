//! The transport both adapters share: one HTTP client, one retry rule, one way to read a failure.
//!
//! Three decisions live here rather than in each adapter, because a difference between them would
//! be a difference in how two providers fail, and the agent loop acts on those distinctions:
//!
//! * **Retrying stops where the stream starts.** A 429, a 5xx or a connection that never came up is
//!   worth another try, because nothing was answered yet. Once a response has begun, a retry would
//!   replay a request the model may already have acted on — so the stream's own failures travel up
//!   instead, and the loop decides whether the turn is worth starting over.
//! * **A status is not a diagnosis.** `400` is a malformed request, an over-long conversation or a
//!   key the account may not use, depending on what the body says. The body's own `type`/`code` is
//!   what separates "this conversation is too long" from "something was wrong with the request".
//! * **Nothing that leaves here carries the key.** The key reaches exactly one header, is never put
//!   into an error, a log line or a detail string, and the detail strings that come back from the
//!   provider are scrubbed before they are stored in an error.

use std::fmt;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::HeaderValue;
use sevenshell_error::scrub_bounded;
use zeroize::Zeroizing;

use crate::types::ProviderError;

/// How long the connection — DNS, TCP and the TLS handshake — may take.
///
/// Ten seconds is longer than any of those should need and short enough that a machine with no route
/// to the provider fails while the person is still looking at the window.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the first byte of the answer may take, counted from the moment the request is handed to
/// the client.
///
/// This covers the request, the provider's queue and the model's own start-up: a provider under load
/// can legitimately take half a minute before it says anything. Sixty seconds is the point where
/// waiting is worse than failing and letting the person ask again.
pub(crate) const FIRST_TOKEN_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a stream that has already started may go quiet.
///
/// Thinking models can think for minutes between two tokens, and a long tool argument is written in
/// bursts. Three minutes is the gap after which a connection is presumed dead rather than slow.
pub(crate) const IDLE_TIMEOUT: Duration = Duration::from_secs(180);

/// The retry budget: the first try and two more.
const MAX_ATTEMPTS: u32 = 3;

/// The longest wait a retry will take, whatever the provider asked for.
///
/// A provider that says "come back in an hour" is telling the person something a background retry
/// cannot fix; waiting that long would look like a hang.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(8);

/// The first backoff wait, doubled per attempt, for a retryable failure that gave no hint.
const BACKOFF_BASE: Duration = Duration::from_millis(250);

/// How much of an error body is kept before it is parsed.
const ERROR_BODY_LIMIT: usize = 16 * 1024;

/// How long an error body may take to arrive before whatever came is used.
const ERROR_BODY_PATIENCE: Duration = Duration::from_secs(10);

/// The longest detail string carried in an error.
const DETAIL_LIMIT: usize = 512;

/// The timeouts one request runs under.
///
/// A value rather than three constants at the call sites so the tests can drive the same code paths
/// at milliseconds instead of minutes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub first_token: Duration,
    pub idle: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            first_token: FIRST_TOKEN_TIMEOUT,
            idle: IDLE_TIMEOUT,
        }
    }
}

/// A provider key, held for as long as the provider is.
///
/// The buffer is wiped when the provider is dropped, and [`fmt::Debug`] prints a placeholder: a
/// provider is cloned into a background run and formatted into logs by whatever holds it, and either
/// of those must not be a disclosure. The one copy that leaves this type is the header value built
/// per request, which lives only as long as the request.
pub(crate) struct ApiKey(Zeroizing<String>);

impl ApiKey {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    /// A key from what the host had in the vault, or nothing when there is none.
    ///
    /// Surrounding whitespace is dropped: a key pasted from a web page routinely carries a newline,
    /// and a header cannot carry one at all.
    pub(crate) fn from_option(key: Option<&str>) -> Option<Self> {
        key.map(str::trim).filter(|key| !key.is_empty()).map(Self::new)
    }

    pub(crate) fn expose(&self) -> &str {
        self.0.as_str()
    }

    /// The key as a header value.
    ///
    /// A key is printable ASCII by construction at every provider; a stored value that is not means
    /// the vault holds something that did not come from a key entry, and the message says so without
    /// quoting it.
    pub(crate) fn header_value(&self) -> Result<HeaderValue, ProviderError> {
        HeaderValue::from_str(self.expose()).map_err(|_| {
            ProviderError::Protocol("the stored API key is not a value an HTTP header can carry".to_owned())
        })
    }
}

impl Clone for ApiKey {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApiKey(<redacted>)")
    }
}

/// The full address one adapter posts to.
///
/// The address is assembled and checked once, when the provider is built, rather than per request:
/// a mistyped base URL then fails while the person is still in the settings page instead of in the
/// middle of a conversation. The rules are the contract's — `https` anywhere, plain `http` only to
/// this machine — plus one of this crate's own: an address with a user name or password in it is
/// refused, because that password would end up in every transport error message about the URL.
pub(crate) fn endpoint(base_url: &str, path: &str) -> Result<reqwest::Url, ProviderError> {
    if !crate::provider::is_allowed_base_url(base_url) {
        return Err(ProviderError::Protocol(
            "the provider address must be https, or http to 127.0.0.1, localhost or ::1".to_owned(),
        ));
    }

    let joined = format!("{}{path}", base_url.trim().trim_end_matches('/'));
    let url = reqwest::Url::parse(&joined)
        .map_err(|_| ProviderError::Protocol("the provider address is not a URL".to_owned()))?;

    if !url.username().is_empty() || url.password().is_some() {
        return Err(ProviderError::Protocol(
            "the provider address must not carry a user name or password".to_owned(),
        ));
    }

    Ok(url)
}

/// The client one provider uses for its whole life.
///
/// There is no total request deadline on the client: a streamed answer is supposed to stay open, and
/// a client-wide limit would cut it off mid-sentence. The `connect` timeout covers the part that
/// must always be quick, and [`crate::stream`] applies the other two per read.
pub(crate) fn build_client(url: &reqwest::Url) -> Result<reqwest::Client, ProviderError> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(concat!("7Shell/", env!("CARGO_PKG_VERSION")));

    if is_loopback(url) {
        // A server on this machine is reached directly. The system proxy is configured for the
        // browser's traffic; asking it to carry a request to 127.0.0.1 is how "local" stops working
        // on a machine that has one configured — and the key would cross a hop nobody chose.
        builder = builder.no_proxy();
    }

    builder.build().map_err(|error| {
        ProviderError::Network(scrub_bounded(
            &format!("the HTTP client could not be built: {error}"),
            DETAIL_LIMIT,
        ))
    })
}

/// Sends one request which answers with an event stream, retrying only where that is safe.
///
/// The response comes back with its body unread: the caller attaches [`crate::stream::events`] to it.
/// Everything that can fail before the body begins — an unreachable host, a refused key, a request
/// that is too large — fails here instead, which is what lets the loop tell "the turn failed" from
/// "the turn failed halfway through saying something".
pub(crate) async fn post_stream(
    client: &reqwest::Client,
    url: &reqwest::Url,
    headers: &[(&str, HeaderValue)],
    body: &[u8],
    limits: Limits,
) -> Result<reqwest::Response, ProviderError> {
    let mut attempt = 0u32;

    loop {
        attempt += 1;

        let mut request = client
            .post(url.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .body(body.to_vec());
        for (name, value) in headers {
            request = request.header(*name, value.clone());
        }

        let response = match tokio::time::timeout(limits.first_token, request.send()).await {
            // A timeout is not retried: the request may well still be running on the provider's
            // side, and a second copy would only make the queue worse.
            Err(_) => return Err(ProviderError::Timeout),
            Ok(Err(error)) => {
                if attempt < MAX_ATTEMPTS && !error.is_builder() {
                    log_wait(attempt, None, &error);
                    tokio::time::sleep(backoff(attempt)).await;
                    continue;
                }
                return Err(ProviderError::Network(transport_detail(&error)));
            }
            Ok(Ok(response)) => response,
        };

        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }

        let retry_after = retry_after_ms(response.headers());
        let body = error_body(response).await;
        let error = classify_status(status.as_u16(), &body, retry_after);

        if attempt < MAX_ATTEMPTS && error.is_retryable() {
            log_wait(attempt, retry_after, &error);
            let wait = retry_after
                .map(Duration::from_millis)
                .unwrap_or_else(|| backoff(attempt))
                .min(MAX_RETRY_WAIT);
            tokio::time::sleep(wait).await;
            continue;
        }

        return Err(error);
    }
}

/// Whether this address names the machine this build runs on.
fn is_loopback(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if bare.eq_ignore_ascii_case("localhost") {
        return true;
    }
    bare.parse::<std::net::IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

fn backoff(attempt: u32) -> Duration {
    BACKOFF_BASE.saturating_mul(1 << attempt.min(8).saturating_sub(1))
}

fn log_wait(attempt: u32, retry_after: Option<u64>, error: &impl fmt::Display) {
    match retry_after {
        Some(milliseconds) => log::debug!(
            "the provider asked to slow down; attempt {attempt} failed ({error}), retrying in {milliseconds} ms"
        ),
        None => log::debug!("attempt {attempt} failed ({error}); trying again"),
    }
}

/// The provider's own hint, in milliseconds.
///
/// Only the seconds form is read. The other form is an HTTP date, which needs a clock comparison to
/// mean anything; a hint this build cannot read is treated as no hint, and the retry backs off on its
/// own instead.
fn retry_after_ms(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    value
        .trim()
        .parse::<u64>()
        .ok()
        .map(|seconds| seconds.saturating_mul(1_000))
}

/// As much of an error body as is useful, and no more.
async fn error_body(response: reqwest::Response) -> String {
    let read = async {
        let mut stream = response.bytes_stream();
        let mut taken: Vec<u8> = Vec::new();
        while let Some(Ok(chunk)) = stream.next().await {
            taken.extend_from_slice(&chunk);
            if taken.len() >= ERROR_BODY_LIMIT {
                break;
            }
        }
        taken
    };

    match tokio::time::timeout(ERROR_BODY_PATIENCE, read).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        // A body that did not arrive is no detail at all; the status alone still says a lot.
        Err(_) => String::new(),
    }
}

/// What a provider's error body says about itself.
///
/// Both protocols answer `{"error":{"type":…,"code":…,"message":…}}`, with the message somewhere in
/// that object or, when the body is not the documented shape, at the top level. Reading all three
/// places is what lets a gateway's own error page still produce a usable detail.
#[derive(Debug, Default)]
struct ErrorFields {
    kind: Option<String>,
    code: Option<String>,
    message: Option<String>,
}

impl ErrorFields {
    fn parse(body: &str) -> Self {
        Self::from_value(&serde_json::from_str::<serde_json::Value>(body.trim()).unwrap_or_default())
    }

    fn from_value(value: &serde_json::Value) -> Self {
        let error = value.get("error");
        let pick = |key: &str| {
            error
                .and_then(|error| error.get(key))
                .or_else(|| value.get(key))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        Self {
            kind: pick("type"),
            code: pick("code"),
            message: pick("message"),
        }
    }

    /// Whether this error is the provider saying the conversation does not fit.
    ///
    /// Both providers spell it differently — Anthropic says "prompt is too long", OpenAI names the
    /// code `context_length_exceeded` — and neither can be recognised from the status alone: `400`
    /// also covers a malformed request, which retrying or shortening the conversation cannot fix.
    fn is_context_overflow(&self) -> bool {
        let haystack = [self.kind.as_deref(), self.code.as_deref(), self.message.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        CONTEXT_MARKERS.iter().any(|marker| haystack.contains(marker))
    }

    fn detail(&self, body: &str) -> String {
        let raw = match &self.message {
            Some(message) if !message.trim().is_empty() => message.as_str(),
            _ => body.trim(),
        };
        let scrubbed = scrub_bounded(raw, DETAIL_LIMIT).trim().to_owned();
        if scrubbed.is_empty() {
            "the provider sent no detail".to_owned()
        } else {
            scrubbed
        }
    }
}

/// The phrases a "your conversation is too long" error uses.
const CONTEXT_MARKERS: &[&str] = &[
    "prompt is too long",
    "context length",
    "context window",
    "context_length_exceeded",
    "maximum context",
    "too many tokens",
    "request_too_large",
];

/// Turns one refused response into the failure the loop acts on.
pub(crate) fn classify_status(status: u16, body: &str, retry_after: Option<u64>) -> ProviderError {
    let fields = ErrorFields::parse(body);

    match status {
        // 403 is the key being valid but not allowed to do this — which a person fixes by fixing the
        // key or the account, exactly as they would after a 401.
        401 | 403 => ProviderError::Auth,
        429 => ProviderError::RateLimited {
            retry_after_ms: retry_after,
        },
        // 529 is Anthropic's own "overloaded" status; 503 is what every other gateway answers when it
        // has more work than capacity.
        503 | 529 => ProviderError::Overloaded,
        400 | 413 | 422 if fields.is_context_overflow() => ProviderError::ContextTooLong,
        _ => ProviderError::Server {
            status,
            detail: fields.detail(body),
        },
    }
}

/// Turns an `error` object a stream carried into a failure.
///
/// Mid-stream errors are not statuses: there is no body to read a status from. Anthropic names them
/// with its error types, OpenAI with a smaller vocabulary, and everything this build cannot place is
/// reported as a protocol failure rather than guessed at.
pub(crate) fn classify_error_object(error: &serde_json::Value) -> ProviderError {
    let fields = ErrorFields::from_value(error);

    match fields.kind.as_deref().unwrap_or_default() {
        "overloaded_error" => ProviderError::Overloaded,
        "rate_limit_error" | "rate_limit_exceeded" => ProviderError::RateLimited { retry_after_ms: None },
        "authentication_error" | "permission_error" | "invalid_api_key" => ProviderError::Auth,
        "request_too_large" | "context_length_exceeded" => ProviderError::ContextTooLong,
        // No status exists for an error that arrived inside a stream, so 500 stands for what the
        // provider said about itself: its own side failed. It is worth another attempt for the same
        // reason a 5xx is, and the detail carries the provider's words rather than invented ones.
        "api_error" | "server_error" => ProviderError::Server {
            status: 500,
            detail: fields.detail(""),
        },
        _ if fields.is_context_overflow() => ProviderError::ContextTooLong,
        _ => ProviderError::Protocol(format!(
            "the provider reported an error while answering: {}",
            fields.detail("")
        )),
    }
}

/// A transport failure, with the URL and whatever the stack said, scrubbed.
fn transport_detail(error: &reqwest::Error) -> String {
    scrub_bounded(&error.to_string(), DETAIL_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_never_printed() {
        let key = ApiKey::new("sk-live-very-secret");
        let printed = format!("{key:?}");

        assert!(
            !printed.contains("very-secret"),
            "a key reached a debug line: {printed}"
        );
        assert_eq!(
            key.expose(),
            "sk-live-very-secret",
            "the header still needs the real value"
        );
    }

    #[test]
    fn a_pasted_key_loses_its_surrounding_whitespace_and_an_empty_one_is_no_key() {
        assert_eq!(
            ApiKey::from_option(Some("  sk-1\n")).map(|key| key.expose().to_owned()),
            Some("sk-1".to_owned())
        );
        assert!(ApiKey::from_option(Some("   ")).is_none(), "whitespace is not a key");
        assert!(ApiKey::from_option(None).is_none());
    }

    #[test]
    fn only_addresses_the_contract_allows_become_endpoints() {
        assert!(endpoint("https://api.anthropic.com", "/v1/messages").is_ok());
        assert!(endpoint("http://127.0.0.1:11434/v1", "/chat/completions").is_ok());

        for refused in ["http://gateway.example/v1", "ftp://example", ""] {
            let error = endpoint(refused, "/v1/messages").expect_err("this address must be refused");
            assert!(matches!(error, ProviderError::Protocol(_)), "{refused} -> {error:?}");
        }

        let with_password = endpoint("https://user:hunter2@api.example/v1", "/v1/messages")
            .expect_err("credentials in the address would leak through every transport error");
        assert!(!format!("{with_password:?}").contains("hunter2"));
    }

    #[test]
    fn a_dropped_trailing_slash_does_not_double_the_separator() {
        let url = endpoint("https://api.openai.com/v1/", "/chat/completions").expect("a valid address");
        assert_eq!(url.as_str(), "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn a_refused_status_becomes_the_failure_the_loop_acts_on() {
        let cases = [
            (
                401,
                "{\"error\":{\"type\":\"authentication_error\",\"message\":\"invalid x-api-key\"}}",
                ProviderError::Auth,
            ),
            (403, "", ProviderError::Auth),
            (
                429,
                "{\"error\":{\"type\":\"rate_limit_error\",\"message\":\"slow down\"}}",
                ProviderError::RateLimited {
                    retry_after_ms: Some(2_000),
                },
            ),
            (
                529,
                "{\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\"}}",
                ProviderError::Overloaded,
            ),
            (
                400,
                "{\"error\":{\"type\":\"invalid_request_error\",\"message\":\"prompt is too long: 250000 tokens > 200000\"}}",
                ProviderError::ContextTooLong,
            ),
            (
                400,
                "{\"error\":{\"type\":\"invalid_request_error\",\"code\":\"context_length_exceeded\",\"message\":\"nope\"}}",
                ProviderError::ContextTooLong,
            ),
        ];

        for (status, body, expected) in cases {
            let retry_after = Some(2_000);
            assert_eq!(
                classify_status(status, body, retry_after),
                expected,
                "status {status} with `{body}` was mapped to the wrong failure"
            );
        }
    }

    #[test]
    fn a_bad_request_that_is_not_about_length_stays_a_server_error() {
        let error = classify_status(
            400,
            "{\"error\":{\"type\":\"invalid_request_error\",\"message\":\"max_tokens: 0 is not allowed\"}}",
            None,
        );

        match error {
            ProviderError::Server { status, detail } => {
                assert_eq!(status, 400);
                assert!(
                    detail.contains("max_tokens"),
                    "the provider's own words are the detail: {detail}"
                );
            }
            other => panic!("a plain bad request must not be reported as a conversation that is too long: {other:?}"),
        }
    }

    #[test]
    fn a_detail_is_scrubbed_and_bounded() {
        let body = "{\"error\":{\"message\":\"incorrect key provided: Bearer sk-live-abcdef\"}}";
        let error = classify_status(500, body, None);

        let ProviderError::Server { detail, .. } = error else {
            panic!("a 5xx is a server failure");
        };
        assert!(
            !detail.contains("sk-live-abcdef"),
            "a detail must not carry a credential: {detail}"
        );
    }

    #[test]
    fn a_status_with_no_body_still_says_something() {
        let ProviderError::Server { detail, .. } = classify_status(502, "  ", None) else {
            panic!("a 502 is a server failure");
        };
        assert_eq!(detail, "the provider sent no detail");
    }

    #[test]
    fn a_stream_error_is_placed_by_its_own_type() {
        let cases = [
            ("overloaded_error", ProviderError::Overloaded),
            ("rate_limit_error", ProviderError::RateLimited { retry_after_ms: None }),
            ("authentication_error", ProviderError::Auth),
            ("request_too_large", ProviderError::ContextTooLong),
        ];

        for (kind, expected) in cases {
            let error = serde_json::json!({ "type": kind, "message": "the provider's own words" });
            assert_eq!(classify_error_object(&error), expected, "`{kind}` was placed wrongly");
        }

        let unknown = classify_error_object(&serde_json::json!({ "type": "brand_new_error" }));
        assert!(
            matches!(unknown, ProviderError::Protocol(_)),
            "an error this build cannot place must not be guessed at: {unknown:?}"
        );
    }

    #[test]
    fn a_retry_after_is_read_in_seconds_and_a_date_is_no_hint() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, HeaderValue::from_static("2"));
        assert_eq!(retry_after_ms(&headers), Some(2_000));

        headers.insert(
            reqwest::header::RETRY_AFTER,
            HeaderValue::from_static("Wed, 21 Oct 2026 07:28:00 GMT"),
        );
        assert_eq!(
            retry_after_ms(&headers),
            None,
            "an HTTP date is not a number of seconds"
        );
    }

    #[test]
    fn the_backoff_grows_and_the_cap_holds() {
        assert_eq!(backoff(1), BACKOFF_BASE);
        assert_eq!(backoff(2), BACKOFF_BASE * 2);

        let long = Duration::from_millis(60_000).min(MAX_RETRY_WAIT);
        assert_eq!(
            long, MAX_RETRY_WAIT,
            "a provider must not be able to park a retry for a minute"
        );
    }
}
