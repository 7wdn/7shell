//! The parts of an API request that are neither the method nor the body.
//!
//! Everything here is written by hand rather than delegated to a URL crate for one reason: the
//! server address comes from a kubeconfig, and the path comes from the interface. The join between
//! them is where a request ends up on a different endpoint than the one that was asked for, so it is
//! a function with a test rather than a `format!` in the middle of a request.

use sevenshell_error::ShellError;

/// The largest response body one request will read.
///
/// A Kubernetes response is an object or a list of them; the biggest realistic one is a few
/// hundred megabytes of CRDs if somebody asks for all of them at once, which is exactly the case a
/// person cannot see coming. The cap turns that into an error instead of an out-of-memory.
pub(crate) const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

/// Where one API server answers, as the kubeconfig spelled it.
pub(crate) struct Server {
    /// The host without brackets or port, which is what a certificate is checked against.
    pub(crate) host: String,
    /// `scheme://authority/path` with no trailing slash, which is the prefix every request hangs
    /// its own path off.
    pub(crate) base: String,
}

/// Reads an address out of a kubeconfig.
///
/// The path is kept, because an API server behind a proxy is at `https://host/k8s` and a client
/// that dropped the prefix would answer 404 for every object. A query or a fragment is refused:
/// neither belongs to a server address, and accepting one would make the join with a request path
/// ambiguous.
pub(crate) fn parse_server(server: &str) -> Result<Server, ShellError> {
    let server = server.trim();
    let (scheme, rest) = server
        .split_once("://")
        .ok_or_else(|| invalid("the cluster address has no scheme"))?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(invalid("the cluster address is neither http nor https"));
    }
    if rest.contains('?') || rest.contains('#') {
        return Err(invalid("the cluster address carries a query or a fragment"));
    }

    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return Err(invalid("the cluster address names no host"));
    }
    // The trailing slash is dropped rather than kept: requests are joined as `base` + `/path`, and
    // keeping it would ask for `//api/v1`, which some proxies answer differently from `/api/v1`.
    let base = format!("{scheme}://{authority}{}", path.trim_end_matches('/'));

    // The port is checked on the way out (a malformed one is refused) but not kept: the base URL
    // already carries it, and a second copy is a second thing to get out of step.
    let (host, _port) = split_authority(authority)?;
    Ok(Server { host, base })
}

/// Splits `host`, `host:port` and `[v6]:port`, the three shapes an address may take.
fn split_authority(authority: &str) -> Result<(String, Option<u16>), ShellError> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest
            .split_once(']')
            .ok_or_else(|| invalid("the cluster address has an unclosed IPv6 host"))?;
        let port = match after.strip_prefix(':') {
            Some(port) => Some(parse_port(port)?),
            None if after.is_empty() => None,
            None => return Err(invalid("the cluster address has something after the host")),
        };
        return Ok((host.to_owned(), port));
    }

    match authority.rsplit_once(':') {
        // Anything after a colon has to be a port: an IPv6 host without brackets is not a valid
        // address, and `host:notaport` is a typo somebody needs told about rather than a host name
        // that happens to contain a colon.
        Some((host, port)) if !port.is_empty() => {
            if !port.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(invalid("the cluster address names a port that is not a number"));
            }
            Ok((host.to_owned(), Some(parse_port(port)?)))
        }
        Some((host, _)) => Ok((host.to_owned(), None)),
        None => Ok((authority.to_owned(), None)),
    }
}

fn parse_port(port: &str) -> Result<u16, ShellError> {
    port.parse::<u16>()
        .map_err(|_| invalid("the cluster address names a port that is not a number"))
}

/// Refuses a path that would not stay on the server it is being sent to.
///
/// An absolute path is the API server's whole address space; a relative one would be appended to
/// the base URL's own path and land somewhere nobody asked for.
pub(crate) fn check_path(path: &str) -> Result<(), ShellError> {
    if !path.starts_with('/') {
        return Err(malformed("an API path has to start with a slash"));
    }
    if path.starts_with("//") {
        return Err(malformed("an API path cannot start with two slashes"));
    }
    Ok(())
}

/// The whole address of one request.
pub(crate) fn url(base: &str, path: &str) -> Result<String, ShellError> {
    check_path(path)?;
    Ok(format!("{base}{path}"))
}

/// The method, as `reqwest` knows it.
///
/// Parsed rather than passed through so that a caller cannot smuggle a header or a space into the
/// request line.
pub(crate) fn method(name: &str) -> Result<reqwest::Method, ShellError> {
    reqwest::Method::from_bytes(name.trim().to_ascii_uppercase().as_bytes())
        .map_err(|_| malformed("the method is not a valid HTTP method"))
}

/// Why a response body could not be read.
pub(crate) enum BodyError {
    /// The server sent more than the caller agreed to read.
    TooLarge,
    /// The connection broke, or the body was not readable at all.
    Transport(String),
}

/// Reads a response body, stopping at `limit`.
///
/// Streamed rather than read to the end, because the cap has to hold for a server that would
/// happily send a gigabyte: the point of the limit is what this side allocates, not what the other
/// side claims.
pub(crate) async fn read_capped(mut response: reqwest::Response, limit: usize) -> Result<String, BodyError> {
    let mut body = Vec::new();
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|error| BodyError::Transport(error.to_string()))?;
        let Some(chunk) = chunk else {
            break;
        };
        if body.len() + chunk.len() > limit {
            return Err(BodyError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    // Lossy rather than refused: a proxy that answers an error page in the wrong charset is still
    // an answer the person needs to see, and the caller shows the body as it came.
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Appends the two parameters a watch needs, keeping whatever the caller already asked for.
pub(crate) fn with_watch_params(path: &str) -> String {
    rebuild(
        path,
        &["watch", "allowWatchBookmarks"],
        &["watch=true", "allowWatchBookmarks=true"],
    )
}

/// Appends the two parameters a watch needs and the resource version to continue from.
///
/// Continuing from the last version seen is what keeps a reconnect from losing the events that
/// happened while the connection was down — and what makes the server answer `410 Gone` when it has
/// already forgotten that version, which is the one case where the caller has to read the list
/// again.
pub(crate) fn with_resource_version(path: &str, resource_version: &str) -> String {
    let wanted = format!("resourceVersion={}", encode_query_value(resource_version));
    rebuild(
        path,
        &["watch", "allowWatchBookmarks", "resourceVersion"],
        &["watch=true", "allowWatchBookmarks=true", &wanted],
    )
}

/// Drops the named parameters from the query and appends `add` in its place.
fn rebuild(path: &str, drop: &[&str], add: &[&str]) -> String {
    let (head, query) = match path.split_once('?') {
        Some((head, query)) => (head, query),
        None => (path, ""),
    };
    let mut pairs: Vec<String> = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter(|pair| {
            let name = pair.split_once('=').map_or(*pair, |(name, _)| name);
            !drop.contains(&name)
        })
        .map(str::to_owned)
        .collect();
    pairs.extend(add.iter().map(|pair| (*pair).to_owned()));
    format!("{head}?{}", pairs.join("&"))
}

/// Percent-encodes a query value.
///
/// The values this build writes itself are `true` and a resource version, and a resource version is
/// opaque to this side: the server may hand back anything it likes, and one `&` in it would end the
/// parameter and start another.
pub(crate) fn encode_query_value(value: &str) -> String {
    const UNRESERVED: &[u8] = b"-._~";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || UNRESERVED.contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// A problem with the cluster address, which is a problem with the kubeconfig.
fn invalid(detail: impl AsRef<str>) -> ShellError {
    ShellError::k8s_kubeconfig_invalid(detail)
}

/// A request this side built wrong: a path that is not absolute, a method that is not one.
///
/// The caller is this application's own interface, so a request line that cannot be sent is a bug
/// here rather than anything the server, the kubeconfig or the person did.
fn malformed(detail: impl AsRef<str>) -> ShellError {
    ShellError::app_internal(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_address_keeps_the_path_it_was_given() {
        // An API server behind a proxy is at `https://host/k8s`, and a client that dropped the
        // prefix would answer 404 for every object on it.
        let server = parse_server("https://api.example.test:6443/k8s").expect("a valid address");
        assert_eq!(server.base, "https://api.example.test:6443/k8s");
        assert_eq!(server.host, "api.example.test");
    }

    #[test]
    fn a_trailing_slash_is_not_part_of_the_base() {
        let server = parse_server("https://api.example.test/").expect("a valid address");
        assert_eq!(server.base, "https://api.example.test");
    }

    #[test]
    fn an_ipv6_authority_keeps_its_brackets_out_of_the_host() {
        let server = parse_server("https://[::1]:6443/prefix").expect("a valid address");
        assert_eq!(server.host, "::1");
        assert_eq!(server.base, "https://[::1]:6443/prefix");
    }

    #[test]
    fn an_address_without_a_scheme_or_with_a_query_is_refused() {
        for address in [
            "api.example.test:6443",
            "https://",
            "ftp://api.example.test",
            "https://host?x=1",
            "https://host:notaport",
        ] {
            assert!(
                parse_server(address).is_err(),
                "{address} should not parse as a cluster address"
            );
        }
    }

    #[test]
    fn a_relative_path_is_refused_rather_than_appended_to_the_base() {
        assert!(check_path("api/v1/pods").is_err());
        assert!(check_path("//api/v1/pods").is_err());
        assert!(check_path("/api/v1/pods").is_ok());
        assert_eq!(
            url("https://host/k8s", "/api/v1/pods").expect("a joined url"),
            "https://host/k8s/api/v1/pods"
        );
    }

    #[test]
    fn a_watch_keeps_the_selectors_it_was_given() {
        let target = with_watch_params("/api/v1/pods?labelSelector=app%3Dweb&limit=500");
        assert_eq!(
            target,
            "/api/v1/pods?labelSelector=app%3Dweb&limit=500&watch=true&allowWatchBookmarks=true"
        );
    }

    #[test]
    fn a_reconnect_replaces_the_resource_version_instead_of_adding_a_second_one() {
        let target = with_resource_version("/api/v1/pods?watch=true&resourceVersion=10", "11.5");
        assert_eq!(
            target,
            "/api/v1/pods?watch=true&allowWatchBookmarks=true&resourceVersion=11.5"
        );
    }

    #[test]
    fn a_resource_version_that_looks_like_a_query_is_escaped() {
        // The server owns the shape of a resource version. One `&` in it would otherwise end the
        // parameter and start another, which is how a watch silently asks for the wrong thing.
        assert_eq!(encode_query_value("12&watch=false"), "12%26watch%3Dfalse");
        assert_eq!(encode_query_value("1234.5678"), "1234.5678");
    }

    #[test]
    fn a_method_is_parsed_rather_than_passed_through() {
        assert_eq!(method("get").expect("a method").as_str(), "GET");
        assert!(method("GET /admin").is_err());
    }
}
