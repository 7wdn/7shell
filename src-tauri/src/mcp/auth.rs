//! What a request has to prove before it reaches a tool.
//!
//! Three facts, checked in the order that keeps work off a request that could never be allowed:
//!
//! * the `Host` header names this machine's loopback address, so a name that resolves to 127.0.0.1
//!   from somewhere else (DNS rebinding) is refused before anything else is looked at;
//! * there is no `Origin` header at all — a browser sets one, and nothing in this application talks
//!   to the endpoint through a browser;
//! * the bearer token is the one this run wrote down.
//!
//! The token is compared without `==`. A comparison that stops at the first differing byte tells
//! whoever can time the answer how much of a guess was right, which is a slower way of learning a
//! token but not a harmless one. The fold below reads every byte of both strings whatever the first
//! difference is.

use sevenshell_error::ShellError;

/// How many bytes of randomness a run's token holds.
///
/// 32 bytes is the size of the hash this application already trusts elsewhere; the token only ever
/// travels over loopback inside one machine, but it is the one thing between another local process
/// and a person's servers, so it is not the place to save four bytes.
pub const TOKEN_BYTES: usize = 32;

/// The header value's scheme, spelled the way HTTP clients send it.
const BEARER: &str = "bearer ";

/// A fresh bearer token for one run, as hex.
///
/// Rotated on every start: an endpoint file left behind by a previous run names a token that no
/// longer opens anything, so a stale file is a refusal rather than a way in.
pub fn generate_token() -> Result<String, ShellError> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|error| ShellError::app_internal(format!("the operating system's random source refused: {error}")))?;
    Ok(hex(&bytes))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit(u32::from(byte >> 4), 16).expect("a nibble is a hex digit"));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).expect("a nibble is a hex digit"));
    }
    out
}

/// Whether two tokens are the same, in time that does not depend on where they differ.
///
/// The length comparison is a short circuit, and deliberately so: the length of a token produced by
/// [`generate_token`] is fixed and public, so a difference there says nothing a caller did not
/// already know.
pub fn tokens_match(expected: &str, presented: &str) -> bool {
    let expected = expected.as_bytes();
    let presented = presented.as_bytes();
    if expected.len() != presented.len() {
        return false;
    }

    let mut difference = 0u8;
    for (left, right) in expected.iter().zip(presented) {
        difference |= left ^ right;
    }
    difference == 0
}

/// The token out of an `Authorization` header, when the header is a bearer one.
///
/// The scheme is matched case-insensitively because HTTP says it is; the space after it is required,
/// so `Bearer` with no token is not a token of length zero.
pub fn bearer_value(header: Option<&str>) -> Option<&str> {
    let header = header?;
    // `split_at_checked` rather than slicing at an index: a header value is bytes, and a multi-byte
    // character straddling the scheme's length would make an index slice panic instead of refusing.
    let (scheme, token) = header.split_at_checked(BEARER.len())?;
    if !scheme.eq_ignore_ascii_case(BEARER) {
        return None;
    }
    if token.is_empty() { None } else { Some(token) }
}

/// Whether a `Host` header names this machine's loopback address.
///
/// Only the IPv4 literal is accepted. A host name would have to be resolved, and resolving is the
/// step a rebinding attack needs; the IPv6 loopback is refused rather than taught, because the
/// listener binds IPv4 and a header naming a different address than the socket is a header worth
/// refusing.
pub fn host_is_loopback(host: &str) -> bool {
    match host.rsplit_once(':') {
        Some((address, port)) => {
            address == "127.0.0.1"
                && !port.is_empty()
                && port.bytes().all(|byte| byte.is_ascii_digit())
                && port.parse::<u16>().is_ok()
        }
        None => host == "127.0.0.1",
    }
}

/// Whether a `Content-Type` is JSON.
///
/// Parameters (`; charset=utf-8`) are allowed: a client is entitled to state its charset, and the
/// body's shape is what actually matters. Anything else is refused before the body is read.
pub fn is_json_content_type(value: &str) -> bool {
    value
        .split(';')
        .next()
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("application/json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_thirty_two_random_bytes_spelled_in_hex() {
        let first = generate_token().expect("a token");
        let second = generate_token().expect("a token");

        assert_eq!(first.len(), TOKEN_BYTES * 2);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second, "every run rotates its token");
    }

    #[test]
    fn only_the_exact_token_matches() {
        assert!(tokens_match("deadbeef", "deadbeef"));
        assert!(!tokens_match("deadbeef", "deadbeee"));
        assert!(!tokens_match("deadbeef", "deadbee"));
        assert!(!tokens_match("deadbeef", "deadbeeff"));
        assert!(!tokens_match("deadbeef", ""));
    }

    #[test]
    fn a_bearer_header_is_read_with_any_scheme_case_and_nothing_else() {
        assert_eq!(bearer_value(Some("Bearer abc123")), Some("abc123"));
        assert_eq!(bearer_value(Some("bearer abc123")), Some("abc123"));
        assert_eq!(bearer_value(Some("BEARER abc123")), Some("abc123"));
        assert_eq!(bearer_value(Some("Basic abc123")), None);
        assert_eq!(bearer_value(Some("Bearer")), None);
        assert_eq!(bearer_value(Some("Bearer ")), None);
        assert_eq!(bearer_value(None), None);
    }

    #[test]
    fn only_the_loopback_literal_is_a_host_this_endpoint_answers_to() {
        assert!(host_is_loopback("127.0.0.1"));
        assert!(host_is_loopback("127.0.0.1:0"));
        assert!(host_is_loopback("127.0.0.1:4711"));
        assert!(host_is_loopback("127.0.0.1:65535"));

        assert!(!host_is_loopback("localhost:4711"));
        assert!(!host_is_loopback("127.0.0.1:"));
        assert!(!host_is_loopback("127.0.0.1:http"));
        assert!(!host_is_loopback("127.0.0.1:65536"));
        assert!(!host_is_loopback("127.0.0.2:4711"));
        assert!(!host_is_loopback("0.0.0.0:4711"));
        assert!(!host_is_loopback("[::1]:4711"));
        assert!(!host_is_loopback("evil.example:4711"));
        assert!(!host_is_loopback(""));
    }

    #[test]
    fn json_content_type_may_carry_parameters_but_not_another_media_type() {
        assert!(is_json_content_type("application/json"));
        assert!(is_json_content_type("Application/JSON; charset=utf-8"));
        assert!(!is_json_content_type("text/plain"));
        assert!(!is_json_content_type("application/jsonx"));
        assert!(!is_json_content_type(""));
    }
}
