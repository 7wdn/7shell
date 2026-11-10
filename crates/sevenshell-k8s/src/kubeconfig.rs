//! Reading a kubeconfig.
//!
//! This module is the only place a bearer token or a client key exists as text, and it is on this
//! side of the wire by construction: what leaves through [`Kubeconfig::contexts`] is a name, a
//! cluster, a user and a namespace. Every error names the object it is about and never quotes the
//! credential it found there.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Deserialize;
use sevenshell_error::ShellError;

/// The variable `kubectl` reads instead of the default location.
pub const KUBECONFIG_ENV: &str = "KUBECONFIG";

/// The file the default path ends in, under the person's home directory.
const DEFAULT_FILE: &str = ".kube/config";

/// One context as the interface sees it.
///
/// Secrets are not in this type: a context names a cluster and a user, and the credential behind
/// either stays on this side of the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KubeContext {
    pub name: String,
    pub cluster: String,
    pub user: String,
    pub namespace: String,
    pub current: bool,
}

/// A parsed kubeconfig.
///
/// Nothing in it is printed: the document holds credentials, and a `Debug` that quoted one would be
/// a credential in a log line three weeks later. The manual implementation below says how many
/// contexts there are and nothing else.
pub struct Kubeconfig {
    contexts: Vec<KubeContext>,
    clusters: HashMap<String, Cluster>,
    users: HashMap<String, User>,
    current: String,
}

/// What one cluster entry says about reaching it.
pub(crate) struct Cluster {
    pub(crate) server: String,
    /// The authority the cluster's certificate has to chain to, when the document carries one.
    pub(crate) ca: Option<Vec<u8>>,
    pub(crate) insecure: bool,
}

/// What one user entry says about how to authenticate.
#[derive(Default)]
pub(crate) struct User {
    pub(crate) token: Option<String>,
    pub(crate) basic: Option<(String, String)>,
    pub(crate) cert: Option<Vec<u8>>,
    pub(crate) key: Option<Vec<u8>>,
}

/// The one context a connection is about.
pub(crate) struct Selected {
    pub(crate) cluster: Cluster,
    pub(crate) user: User,
}

impl User {
    /// The `Authorization` header this user's credential turns into.
    ///
    /// A token is sent as it is stored. A username and password are sent as the one header HTTP has
    /// for them — they are not a secret this build invents an encoding for.
    pub(crate) fn authorization(&self) -> Option<String> {
        if let Some(token) = &self.token {
            return Some(format!("Bearer {token}"));
        }
        self.basic.as_ref().map(|(user, password)| {
            let joined = format!("{user}:{password}");
            format!("Basic {}", BASE64.encode(joined.as_bytes()))
        })
    }
}

impl std::fmt::Debug for Kubeconfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kubeconfig")
            .field("contexts", &self.contexts.len())
            .field("current", &self.current)
            .finish()
    }
}

impl Kubeconfig {
    /// Parses the YAML document a person imported or that lives at the default location.
    pub fn parse(bytes: &[u8]) -> Result<Self, ShellError> {
        Self::from_document(bytes, None)
    }

    /// Reads and parses a kubeconfig from disk, resolving relative paths beside it.
    pub fn load(path: &Path) -> Result<Self, ShellError> {
        let bytes = std::fs::read(path).map_err(|error| {
            ShellError::k8s_kubeconfig_invalid(format!("the kubeconfig could not be read: {error}"))
        })?;
        Self::from_document(&bytes, path.parent())
    }

    /// Reads the kubeconfig `kubectl` would read on this machine.
    ///
    /// A missing file is refused rather than answered with an empty document: an empty list of
    /// contexts looks exactly like a cluster with nothing in it, and "there is no kubeconfig here"
    /// is a different thing to be told.
    pub fn load_default() -> Result<Self, ShellError> {
        let path = default_path()
            .ok_or_else(|| invalid("this machine has no home directory, so there is no kubeconfig to read"))?;
        Self::load(&path)
    }

    fn from_document(bytes: &[u8], base: Option<&Path>) -> Result<Self, ShellError> {
        let raw: RawConfig = serde_yaml_ng::from_slice(bytes).map_err(|error| {
            ShellError::k8s_kubeconfig_invalid(format!("the kubeconfig is not valid YAML: {error}"))
        })?;

        let mut clusters = HashMap::with_capacity(raw.clusters.len());
        for entry in raw.clusters {
            clusters.insert(entry.name.clone(), cluster(&entry.name, entry.cluster, base)?);
        }

        let mut users = HashMap::with_capacity(raw.users.len());
        for entry in raw.users {
            users.insert(entry.name.clone(), user(&entry.name, entry.user, base)?);
        }

        let current = raw.current_context;
        let contexts: Vec<KubeContext> = raw
            .contexts
            .into_iter()
            .map(|entry| KubeContext {
                current: entry.name == current,
                name: entry.name,
                cluster: entry.context.cluster,
                user: entry.context.user,
                namespace: entry.context.namespace,
            })
            .collect();

        Ok(Self {
            contexts,
            clusters,
            users,
            current,
        })
    }

    pub fn contexts(&self) -> &[KubeContext] {
        &self.contexts
    }

    /// The context a connection uses when nobody names one.
    pub fn current_context(&self) -> &str {
        &self.current
    }

    /// The cluster and credential one context resolves to, or why it cannot be used at all.
    ///
    /// This is where an unusable credential is refused, one context at a time rather than for the
    /// whole document: a kubeconfig written by a tool that also manages clouds this build cannot
    /// authenticate to is still a perfectly good kubeconfig for the cluster the person wants.
    pub(crate) fn select(&self, context: Option<&str>) -> Result<Selected, ShellError> {
        let name = match context.map(str::trim) {
            Some(name) if !name.is_empty() => name,
            _ => self.current.as_str(),
        };
        if name.is_empty() {
            return Err(invalid("the kubeconfig has no current context and none was chosen"));
        }
        let entry = self
            .contexts
            .iter()
            .find(|candidate| candidate.name == name)
            .ok_or_else(|| invalid(format!("the kubeconfig has no context named `{name}`")))?;

        if entry.cluster.is_empty() {
            return Err(invalid(format!("the context `{name}` names no cluster")));
        }
        let cluster = self
            .clusters
            .get(&entry.cluster)
            .ok_or_else(|| invalid(format!("the kubeconfig has no cluster named `{}`", entry.cluster)))?;

        if entry.user.is_empty() {
            return Err(invalid(format!(
                "the context `{name}` names no user, and this build does not talk to an anonymous API server"
            )));
        }
        let user = self
            .users
            .get(&entry.user)
            .ok_or_else(|| invalid(format!("the kubeconfig has no user named `{}`", entry.user)))?;

        Ok(Selected {
            cluster: Cluster {
                server: cluster.server.clone(),
                ca: cluster.ca.clone(),
                insecure: cluster.insecure,
            },
            user: User {
                token: user.token.clone(),
                basic: user.basic.clone(),
                cert: user.cert.clone(),
                key: user.key.clone(),
            },
        })
    }
}

/// The path `kubectl` would read on this machine.
///
/// `KUBECONFIG` wins when it is set, and only its first entry is used: the variable names a list to
/// merge, and merging kubeconfigs is a feature this build does not have — silently using the first
/// one is closer to what a person means than pretending to merge and dropping half of it.
pub fn default_path() -> Option<PathBuf> {
    path_from_environment(&|key| std::env::var(key).ok())
}

/// The same rule, reading the environment through a lookup so it can be tested off the machine it
/// describes.
pub fn path_from_environment(environment: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(value) = environment(KUBECONFIG_ENV) {
        let first = value.split([';', ':']).find(|entry| !entry.trim().is_empty());
        if let Some(first) = first {
            return Some(PathBuf::from(first.trim()));
        }
    }
    let home = environment("HOME").or_else(|| environment("USERPROFILE"))?;
    if home.trim().is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(DEFAULT_FILE))
}

fn cluster(name: &str, raw: RawCluster, base: Option<&Path>) -> Result<Cluster, ShellError> {
    let ca = match (
        raw.certificate_authority_data.is_empty(),
        raw.certificate_authority.is_empty(),
    ) {
        (false, _) => Some(decode_base64(
            &format!("the cluster `{name}` certificate authority"),
            &raw.certificate_authority_data,
        )?),
        (true, false) => {
            let path = resolve(
                base,
                &raw.certificate_authority,
                &format!("the cluster `{name}` certificate authority"),
            )?;
            Some(read(&path, &format!("the cluster `{name}` certificate authority"))?)
        }
        (true, true) => None,
    };

    Ok(Cluster {
        server: raw.server,
        ca,
        insecure: raw.insecure_skip_tls_verify,
    })
}

fn user(name: &str, raw: RawUser, base: Option<&Path>) -> Result<User, ShellError> {
    // An exec plugin runs a command this build would have to trust with the cluster's address, so
    // it is refused with a message that says so, rather than guessed at as "no credential".
    if raw.exec.is_some() {
        return Err(invalid(format!(
            "the user `{name}` authenticates with an exec plugin, which this build does not run"
        )));
    }
    if raw.auth_provider.is_some() {
        return Err(invalid(format!(
            "the user `{name}` authenticates through an auth-provider, which this build does not support"
        )));
    }

    let token = if !raw.token.is_empty() {
        Some(raw.token)
    } else if !raw.token_file.is_empty() {
        let path = resolve(base, &raw.token_file, &format!("the user `{name}` token file"))?;
        let text = read(&path, &format!("the user `{name}` token file"))?;
        let text = String::from_utf8(text).map_err(|_| invalid(format!("the user `{name}` token file is not text")))?;
        let text = text.trim().to_owned();
        (!text.is_empty()).then_some(text)
    } else {
        None
    };

    let basic = (!raw.username.is_empty() && !raw.password.is_empty()).then_some((raw.username, raw.password));

    let cert = match (
        raw.client_certificate_data.is_empty(),
        raw.client_certificate.is_empty(),
    ) {
        (false, _) => Some(decode_base64(
            &format!("the user `{name}` client certificate"),
            &raw.client_certificate_data,
        )?),
        (true, false) => {
            let path = resolve(
                base,
                &raw.client_certificate,
                &format!("the user `{name}` client certificate"),
            )?;
            Some(read(&path, &format!("the user `{name}` client certificate"))?)
        }
        (true, true) => None,
    };
    let key = match (raw.client_key_data.is_empty(), raw.client_key.is_empty()) {
        (false, _) => Some(decode_base64(
            &format!("the user `{name}` client key"),
            &raw.client_key_data,
        )?),
        (true, false) => {
            let path = resolve(base, &raw.client_key, &format!("the user `{name}` client key"))?;
            Some(read(&path, &format!("the user `{name}` client key"))?)
        }
        (true, true) => None,
    };

    // Half a client identity is not a smaller identity, it is a mistake: constructing one with only
    // the certificate produces a TLS error at the server instead of "the kubeconfig is incomplete"
    // here, where it can be said in a sentence about the file.
    match (&cert, &key) {
        (Some(_), None) => {
            return Err(invalid(format!(
                "the user `{name}` has a client certificate but no client key"
            )));
        }
        (None, Some(_)) => {
            return Err(invalid(format!(
                "the user `{name}` has a client key but no client certificate"
            )));
        }
        _ => {}
    }

    if token.is_none() && basic.is_none() && cert.is_none() {
        return Err(invalid(format!(
            "the user `{name}` has no token, password or client certificate"
        )));
    }

    Ok(User {
        token,
        basic,
        cert,
        key,
    })
}

fn decode_base64(what: &str, text: &str) -> Result<Vec<u8>, ShellError> {
    // YAML folds long values, and a folded base64 body carries the line breaks. They are not part of
    // the encoding, and refusing them would reject kubeconfigs written by hand.
    let compact: String = text.chars().filter(|character| !character.is_whitespace()).collect();
    BASE64
        .decode(compact.as_bytes())
        .map_err(|_| invalid(format!("{what} is not valid base64")))
}

/// Where a path in the document points.
///
/// A relative path is relative to the kubeconfig's own directory, which is what `kubectl` does and
/// therefore what every generated kubeconfig assumes. A document with no directory — the imported
/// shape — cannot resolve one, and says so instead of reading a file relative to wherever the
/// application happens to be running.
fn resolve(base: Option<&Path>, path: &str, what: &str) -> Result<PathBuf, ShellError> {
    let path = Path::new(path);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    match base {
        Some(base) => Ok(base.join(path)),
        None => Err(invalid(format!(
            "{what} points at the relative path `{}`, which cannot be resolved because the kubeconfig was imported rather than read from a file",
            path.display()
        ))),
    }
}

fn read(path: &Path, what: &str) -> Result<Vec<u8>, ShellError> {
    std::fs::read(path).map_err(|error| invalid(format!("{what} could not be read: {error}")))
}

fn invalid(detail: impl AsRef<str>) -> ShellError {
    ShellError::k8s_kubeconfig_invalid(detail)
}

// -- the document, as YAML ---------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawConfig {
    #[serde(default, rename = "current-context")]
    current_context: String,
    #[serde(default)]
    contexts: Vec<RawContextEntry>,
    #[serde(default)]
    clusters: Vec<RawClusterEntry>,
    #[serde(default)]
    users: Vec<RawUserEntry>,
}

#[derive(Debug, Deserialize)]
struct RawContextEntry {
    name: String,
    #[serde(default)]
    context: RawContext,
}

#[derive(Debug, Default, Deserialize)]
struct RawContext {
    #[serde(default)]
    cluster: String,
    #[serde(default)]
    user: String,
    #[serde(default)]
    namespace: String,
}

#[derive(Debug, Deserialize)]
struct RawClusterEntry {
    name: String,
    #[serde(default)]
    cluster: RawCluster,
}

#[derive(Debug, Default, Deserialize)]
struct RawCluster {
    #[serde(default)]
    server: String,
    #[serde(default, rename = "certificate-authority-data")]
    certificate_authority_data: String,
    #[serde(default, rename = "certificate-authority")]
    certificate_authority: String,
    #[serde(default, rename = "insecure-skip-tls-verify")]
    insecure_skip_tls_verify: bool,
}

#[derive(Debug, Deserialize)]
struct RawUserEntry {
    name: String,
    #[serde(default)]
    user: RawUser,
}

#[derive(Debug, Default, Deserialize)]
struct RawUser {
    #[serde(default)]
    token: String,
    #[serde(default, rename = "tokenFile")]
    token_file: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default, rename = "client-certificate-data")]
    client_certificate_data: String,
    #[serde(default, rename = "client-certificate")]
    client_certificate: String,
    #[serde(default, rename = "client-key-data")]
    client_key_data: String,
    #[serde(default, rename = "client-key")]
    client_key: String,
    #[serde(default)]
    exec: Option<serde_yaml_ng::Value>,
    #[serde(default, rename = "auth-provider")]
    auth_provider: Option<serde_yaml_ng::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &str = r#"
apiVersion: v1
kind: Config
current-context: lab
clusters:
  - name: lab-cluster
    cluster:
      server: https://127.0.0.1:6443
      insecure-skip-tls-verify: true
  - name: prod-cluster
    cluster:
      server: https://prod.example.test/k8s
      certificate-authority-data: "ZmFrZS1jYQ=="
contexts:
  - name: lab
    context:
      cluster: lab-cluster
      user: lab-user
      namespace: team-a
  - name: prod
    context:
      cluster: prod-cluster
      user: prod-user
users:
  - name: lab-user
    user:
      token: lab-token
  - name: prod-user
    user:
      client-certificate-data: "Y2VydA=="
      client-key-data: "a2V5"
"#;

    fn parsed() -> Kubeconfig {
        Kubeconfig::parse(DOCUMENT.as_bytes()).expect("the fixture should parse")
    }

    #[test]
    fn the_current_context_is_the_one_flagged_and_the_namespace_is_kept() {
        let config = parsed();
        assert_eq!(config.current_context(), "lab");
        let lab = config.contexts().iter().find(|entry| entry.name == "lab").expect("lab");
        assert!(lab.current);
        assert_eq!(lab.cluster, "lab-cluster");
        assert_eq!(lab.user, "lab-user");
        assert_eq!(lab.namespace, "team-a");
        let prod = config
            .contexts()
            .iter()
            .find(|entry| entry.name == "prod")
            .expect("prod");
        assert!(!prod.current);
    }

    #[test]
    fn base64_material_is_decoded_once_per_object() {
        let config = parsed();
        let prod = config.select(Some("prod")).expect("prod resolves");
        assert_eq!(prod.cluster.ca.as_deref(), Some(b"fake-ca".as_slice()));
        assert_eq!(prod.user.cert.as_deref(), Some(b"cert".as_slice()));
        assert_eq!(prod.user.key.as_deref(), Some(b"key".as_slice()));
        assert_eq!(prod.user.authorization(), None);
    }

    #[test]
    fn a_token_becomes_a_bearer_header_and_a_password_pair_a_basic_one() {
        let config = parsed();
        let lab = config.select(None).expect("the current context resolves");
        assert_eq!(lab.user.authorization().as_deref(), Some("Bearer lab-token"));

        let basic = Kubeconfig::parse(
            br#"
current-context: one
clusters: [{name: c, cluster: {server: "https://h"}}]
contexts: [{name: one, context: {cluster: c, user: u}}]
users: [{name: u, user: {username: alice, password: secret}}]
"#,
        )
        .expect("a basic-auth document");
        let selected = basic.select(None).expect("the context resolves");
        // `alice:secret` in base64; the value itself must not appear anywhere.
        assert_eq!(selected.user.authorization().as_deref(), Some("Basic YWxpY2U6c2VjcmV0"));
    }

    #[test]
    fn a_missing_user_or_cluster_is_named_in_the_refusal() {
        let config = Kubeconfig::parse(
            br#"
current-context: one
clusters: [{name: c, cluster: {server: "https://h"}}]
contexts: [{name: one, context: {cluster: c, user: ghost}}]
users: []
"#,
        )
        .expect("the document parses");
        let error = match config.select(None) {
            Ok(_) => panic!("a context that names a user nobody defined is unusable"),
            Err(error) => error,
        };
        assert_eq!(error.code(), sevenshell_error::ErrorCode::K8sKubeconfigInvalid);
        assert!(
            format!("{error}").contains("ghost"),
            "the refusal should name the missing user"
        );
    }

    #[test]
    fn a_context_with_no_user_at_all_is_refused_rather_than_treated_as_anonymous() {
        let config = Kubeconfig::parse(
            br#"
current-context: one
clusters: [{name: c, cluster: {server: "https://h"}}]
contexts: [{name: one, context: {cluster: c}}]
users: []
"#,
        )
        .expect("the document parses");
        assert!(config.select(None).is_err());
    }

    #[test]
    fn half_a_client_identity_is_refused() {
        let only_cert = Kubeconfig::parse(
            br#"
current-context: one
clusters: [{name: c, cluster: {server: "https://h"}}]
contexts: [{name: one, context: {cluster: c, user: u}}]
users: [{name: u, user: {client-certificate-data: "Y2VydA=="}}]
"#,
        );
        assert!(only_cert.is_err(), "a certificate without its key is not an identity");

        let only_key = Kubeconfig::parse(
            br#"
current-context: one
clusters: [{name: c, cluster: {server: "https://h"}}]
contexts: [{name: one, context: {cluster: c, user: u}}]
users: [{name: u, user: {client-key-data: "a2V5"}}]
"#,
        );
        assert!(only_key.is_err(), "a key without its certificate is not an identity");
    }

    #[test]
    fn an_exec_plugin_is_refused_with_a_reason_rather_than_as_a_missing_credential() {
        let config = Kubeconfig::parse(
            br#"
current-context: one
clusters: [{name: c, cluster: {server: "https://h"}}]
contexts: [{name: one, context: {cluster: c, user: u}}]
users: [{name: u, user: {exec: {command: aws, apiVersion: client.authentication.k8s.io/v1beta1}}}]
"#,
        )
        .expect_err("an exec credential cannot be run by this build");
        assert_eq!(config.code(), sevenshell_error::ErrorCode::K8sKubeconfigInvalid);
        assert!(
            format!("{config}").contains("exec"),
            "the refusal should say what it refused"
        );
    }

    #[test]
    fn a_document_that_is_not_yaml_is_refused_with_the_kubeconfig_code() {
        let error = Kubeconfig::parse(b"apiVersion: v1\n\tcontexts: [").expect_err("not YAML");
        assert_eq!(error.code(), sevenshell_error::ErrorCode::K8sKubeconfigInvalid);
    }

    /// A directory of our own under the system temporary directory.
    ///
    /// Hand-rolled because this crate has no `tempfile` in its dev-dependencies, and the name has to
    /// be unique enough that two test binaries running at once cannot collide.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("7shell-k8s-kubeconfig-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).expect("a scratch directory");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_relative_path_is_read_beside_the_document_and_refused_without_one() {
        let scratch = Scratch::new();
        let directory = &scratch.0;
        std::fs::write(directory.join("ca.pem"), b"-----BEGIN CERTIFICATE-----\n").expect("a file");
        let document = r#"
current-context: one
clusters: [{name: c, cluster: {server: "https://h", certificate-authority: ca.pem}}]
contexts: [{name: one, context: {cluster: c, user: u}}]
users: [{name: u, user: {token: t}}]
"#;
        let path = directory.join("config");
        std::fs::write(&path, document).expect("a kubeconfig");

        let from_file = Kubeconfig::load(&path).expect("a relative CA resolves beside the file");
        let selected = from_file.select(None).expect("the context resolves");
        assert_eq!(
            selected.cluster.ca.as_deref(),
            Some(b"-----BEGIN CERTIFICATE-----\n".as_slice())
        );

        // The same document as text has no directory, and guessing one would read whatever file
        // happens to be there.
        let imported = Kubeconfig::parse(document.as_bytes()).expect_err("no directory to resolve against");
        assert_eq!(imported.code(), sevenshell_error::ErrorCode::K8sKubeconfigInvalid);
        assert!(format!("{imported}").contains("relative"));
    }

    #[test]
    fn the_default_path_follows_kubectl() {
        let with_kubeconfig = path_from_environment(&|key| match key {
            "KUBECONFIG" => Some("/tmp/one:/tmp/two".to_owned()),
            "HOME" => Some("/home/me".to_owned()),
            _ => None,
        });
        assert_eq!(with_kubeconfig, Some(PathBuf::from("/tmp/one")));

        let with_home = path_from_environment(&|key| match key {
            "HOME" => Some("/home/me".to_owned()),
            _ => None,
        });
        assert_eq!(with_home, Some(PathBuf::from("/home/me").join(".kube/config")));

        assert_eq!(path_from_environment(&|_| None), None);
    }
}
