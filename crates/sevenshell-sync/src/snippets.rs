//! The same backup, parked in a gist or a snippet.
//!
//! The second place a backup can live, beside a WebDAV server and a file. What travels is *the same
//! snapshot* — the container is what differs, and it is what the feature is named after: a gist on
//! GitHub, a snippet on GitLab or Gitee, which are those services' own objects for a small file.
//!
//! Two rules make that safe, and both are about the fact that the container belongs to somebody
//! else:
//!
//! * **The file is always sealed.** There is no plaintext form here, unlike a file the person holds
//!   or a server they run: a gist marked private is *unlisted* — anyone with the address can read it
//!   — so nothing of this application's ever sits in one in the clear.
//! * **A file that is not ours is refused, never overwritten.** A snippet id is something somebody
//!   types, and a typo or a stale id can point at content that belongs to another purpose entirely.
//!   The reader recognises this application's envelope and nothing else, and an upload stops rather
//!   than writing over what it does not recognise.
//!
//! What this module holds is the part that has to be exactly right and can be checked without a
//! network: which address each service is talked to at, what each one is sent, and how each one
//! answers. The client that carries those requests is a thin skin over it.

use serde::{Deserialize, Serialize};
use sevenshell_error::ShellError;
use sevenshell_types::SnippetProvider;
use sevenshell_vault::{PayloadPurpose, SealedPayload, ServicePassword, open_payload, seal_payload};

use crate::snapshot::{SyncSnapshot, open};

/// The file inside the gist or snippet. One file, named as plainly as the service allows.
pub const SNIPPET_FILE_NAME: &str = "7shell-sync.json";

/// The `format` of the envelope a snippet carries.
///
/// Its own name rather than the snapshot's own, because what a reader meets first is this wrapper:
/// it says the file was written by a snippet upload, which is what decides whether the content is
/// this application's to overwrite.
pub const SNIPPET_ENVELOPE_FORMAT: &str = "7shell-encrypted-sync-snapshot";

/// The envelope version this build writes, refused if newer, like every other format here.
pub const SNIPPET_ENVELOPE_VERSION: u32 = 1;

/// The title a new gist or snippet is created with.
pub const SNIPPET_TITLE: &str = "7Shell encrypted configuration sync";

/// Where a self-hosted GitLab is not involved, which is where a GitLab account is talked to.
///
/// Written down here as well as in the form, because the two have to agree: the field offers this
/// address, and an account with no address gets this one — the same installation either way.
pub const GITLAB_DEFAULT_INSTANCE: &str = "https://gitlab.com";

/// What names one code-hosting account.
///
/// One token per account, and an account is a service and — for GitLab — an installation: the
/// hosted one and somebody's own are two services with two tokens, and treating them as one would
/// send the wrong token to one of them. GitHub and Gitee have a single address each, so their name
/// is their name; GitLab's carries the address it is addressed at.
///
/// Readable rather than hashed, unlike a fingerprint: this ends up as a key in the settings file and
/// in the vault's own table, where a person may well look at it, and the address it names is written
/// in a setting two lines away. A hash would hide a hostname that is already on screen.
pub fn snippet_account(provider: SnippetProvider, instance_url: Option<&str>) -> Result<String, ShellError> {
    Ok(match provider {
        SnippetProvider::GitLab => format!("gitlab:{}", gitlab_instance(instance_url)?),
        other => other.as_str().to_owned(),
    })
}

/// Where a snippet lives and what opens the door to it.
#[derive(Clone)]
pub struct SnippetSync {
    pub provider: SnippetProvider,
    /// GitLab only: which installation. A self-hosted GitLab is the normal case there, and the
    /// hosted one is only one of its addresses.
    pub instance_url: Option<String>,
    pub token: ServicePassword,
    /// The gist or snippet this build writes to, once one has been created. Empty means the next
    /// upload creates one.
    pub snippet_id: Option<String>,
}

impl std::fmt::Debug for SnippetSync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnippetSync")
            .field("provider", &self.provider)
            .field("instance_url", &self.instance_url)
            .field("snippet_id", &self.snippet_id)
            .field("has_token", &!self.token.is_empty())
            .finish()
    }
}

/// One request, ready to be sent: the shape that has to match three services, kept apart from the
/// client that sends it so it can be checked without a network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetRequest {
    pub method: &'static str,
    pub url: String,
    /// Headers, in the order they are set. The token is one of them — or, for one service, a query
    /// parameter, which is why the URL above is built by the same function that builds these.
    pub headers: Vec<(String, String)>,
}

/// The address a service's API lives at.
pub fn api_base(provider: SnippetProvider, instance_url: Option<&str>) -> Result<String, ShellError> {
    match provider {
        SnippetProvider::GitHub => Ok("https://api.github.com".to_owned()),
        SnippetProvider::Gitee => Ok("https://gitee.com/api/v5".to_owned()),
        SnippetProvider::GitLab => {
            let instance = gitlab_instance(instance_url)?;
            Ok(format!("{instance}/api/v4"))
        }
    }
}

/// A GitLab installation's address, as a person would type it.
///
/// An address with a query, a fragment or credentials in it is not one somebody meant to type, and
/// a scheme other than http or https is a service this cannot talk to. The trailing slash goes,
/// because the path is joined onto it. Nothing at all means the hosted GitLab, which is what an
/// empty field on the form means.
fn gitlab_instance(instance_url: Option<&str>) -> Result<String, ShellError> {
    let raw = instance_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(GITLAB_DEFAULT_INSTANCE);

    let parsed = reqwest::Url::parse(raw)
        .map_err(|error| ShellError::sync_not_configured().with_detail(format!("the address: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(
            ShellError::sync_not_configured().with_detail(format!("the address starts with '{}'", parsed.scheme()))
        );
    }
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(
            ShellError::sync_not_configured().with_detail("the address carries something other than a host and a path")
        );
    }

    Ok(raw.trim_end_matches('/').to_owned())
}

/// The address of a service's collection of snippets, or of one of them.
pub fn snippet_url(provider: SnippetProvider, api_base: &str, id: Option<&str>) -> String {
    let collection = match provider {
        SnippetProvider::GitLab => "snippets",
        _ => "gists",
    };

    match id.map(str::trim).filter(|value| !value.is_empty()) {
        Some(id) => format!("{api_base}/{collection}/{id}"),
        None => format!("{api_base}/{collection}"),
    }
}

/// The request that asks a service who this token belongs to, or whether the snippet is there.
///
/// The question a person asks before trusting a setup, and the one that costs nothing: a read, of
/// either the account or the file.
pub fn test_request(sync: &SnippetSync, api_base: &str) -> Result<SnippetRequest, ShellError> {
    let base = api_base.to_owned();
    let has_id = sync.snippet_id.as_deref().is_some_and(|id| !id.trim().is_empty());
    let url = if has_id {
        snippet_url(sync.provider, &base, sync.snippet_id.as_deref())
    } else {
        // Without one, the account is the thing that can be checked: the token is either good or it
        // is not, and that is what somebody is testing.
        format!("{base}/user")
    };

    request_for(sync, "GET", url, None)
}

/// The request that reads the file.
pub fn read_request(sync: &SnippetSync, api_base: &str) -> Result<SnippetRequest, ShellError> {
    let id = require_id(sync)?;
    let base = api_base.to_owned();

    // GitHub and Gitee answer with the file too, but a long one comes back with `truncated: true`
    // and no content at all. The raw address is asked for directly when that happens — and asking
    // for the snippet's own metadata first is what makes the difference visible.
    let url = snippet_url(sync.provider, &base, Some(&id));
    request_for(sync, "GET", url, None)
}

/// The request that writes the file, creating the gist or snippet when there is none yet.
///
/// The verb is the one difference between updating and creating that the three services agree on
/// only partly: GitLab updates with `PUT` and creates with `POST`, the other two update with
/// `PATCH`. A creation in a service that has no id yet is a `POST` everywhere.
pub fn write_request(sync: &SnippetSync, api_base: &str, content: &str) -> Result<SnippetRequest, ShellError> {
    let base = api_base.to_owned();
    let id = sync
        .snippet_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let method = match (sync.provider, id) {
        (SnippetProvider::GitLab, Some(_)) => "PUT",
        (_, Some(_)) => "PATCH",
        (_, None) => "POST",
    };
    let url = snippet_url(sync.provider, &base, id);

    request_for(
        sync,
        method,
        url,
        Some(create_body(sync.provider, content, id.is_some())),
    )
}

/// What a service is sent when a file is written into it.
///
/// Three shapes, and each one is a rule the service enforces rather than a preference: GitHub and
/// Gitee take `files` as an object keyed by file name, while GitLab takes a list of instructions and
/// wants `action: update` to change one that is there — and refuses an update that names a file it
/// does not have.
pub fn create_body(provider: SnippetProvider, content: &str, updating: bool) -> serde_json::Value {
    match provider {
        SnippetProvider::GitHub => serde_json::json!({
            "description": SNIPPET_TITLE,
            "public": false,
            "files": { SNIPPET_FILE_NAME: { "content": content } }
        }),
        // The same shape as GitHub's, and sent as JSON on purpose: encoded as a form, `files` would
        // arrive as a string and be refused.
        SnippetProvider::Gitee => serde_json::json!({
            "description": SNIPPET_TITLE,
            "public": false,
            "files": { SNIPPET_FILE_NAME: { "content": content } }
        }),
        SnippetProvider::GitLab => {
            if updating {
                serde_json::json!({
                    "files": [{ "action": "update", "file_path": SNIPPET_FILE_NAME, "content": content }]
                })
            } else {
                serde_json::json!({
                    "title": SNIPPET_TITLE,
                    "visibility": "private",
                    "files": [{ "file_path": SNIPPET_FILE_NAME, "content": content }]
                })
            }
        }
    }
}

/// The id a service's answer names, if it names one.
///
/// GitHub answers with an object, Gitee with a list of them. A service that answers without an id
/// leaves the caller to keep the one it already had, which is what an update does.
pub fn response_id(value: &serde_json::Value) -> Option<String> {
    let entry = match value {
        serde_json::Value::Array(items) => items.first()?,
        other => other,
    };

    match entry.get("id") {
        Some(serde_json::Value::String(id)) => Some(id.clone()),
        Some(serde_json::Value::Number(id)) => Some(id.to_string()),
        _ => None,
    }
}

/// The file inside a service's answer: its content, or the address to fetch it from.
///
/// A long file comes back truncated with no content, and the raw address is the only way to read
/// it. Gitee answers with `files` as a string holding JSON — which is the same nesting, written
/// twice — so it is unwrapped before anything is looked for.
pub fn file_in(value: &serde_json::Value) -> Result<(Option<String>, Option<String>), ShellError> {
    let files = value
        .get("files")
        .ok_or_else(|| ShellError::sync_snapshot_invalid("the answer named no files"))?;
    let files = match files {
        serde_json::Value::String(text) => serde_json::from_str::<serde_json::Value>(text)
            .map_err(|error| ShellError::sync_snapshot_invalid(format!("the file list: {error}")))?,
        other => other.clone(),
    };

    let file = files
        .get(SNIPPET_FILE_NAME)
        // A file the service renamed or re-created is still the only file there; the name is this
        // application's, and it is not what a person would change by hand.
        .or_else(|| files.as_object().and_then(|entries| entries.values().next()))
        .ok_or_else(|| ShellError::sync_snapshot_invalid("the answer held no file"))?;

    let truncated = file
        .get("truncated")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let content = if truncated {
        None
    } else {
        file.get("content")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let raw_url = file
        .get("raw_url")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);

    Ok((content, raw_url))
}

/// Wraps a snapshot into the envelope a snippet carries, sealed under a passphrase.
///
/// There is no unsealed form: what is uploaded leaves this machine and lands in somebody else's
/// service, and the passphrase is what makes that acceptable. The payload is the same snapshot the
/// other two homes carry — the same context string, so a file moved between a server and a snippet
/// still opens.
///
/// Taking the snapshot rather than a string is what lets the credentials be checked here: a caller
/// that assembled one and forgot to read the credentials out of the vault gets an error instead of a
/// snippet that says it holds somebody's passwords and does not.
pub fn seal(
    snapshot: &SyncSnapshot,
    passphrase: &str,
    kdf: &sevenshell_vault::KdfParams,
) -> Result<String, ShellError> {
    crate::snapshot::ensure_credentials_came(snapshot)?;

    let payload = zeroize::Zeroizing::new(
        serde_json::to_vec(snapshot)
            .map_err(|error| ShellError::sync_io(format!("the snapshot could not be written: {error}")))?,
    );
    let envelope = SnippetEnvelope {
        format: SNIPPET_ENVELOPE_FORMAT.to_owned(),
        version: SNIPPET_ENVELOPE_VERSION,
        sealed: seal_payload(passphrase, PayloadPurpose::SyncSnapshot, payload.as_slice(), kdf)?,
    };

    serde_json::to_string_pretty(&envelope).map_err(|error| ShellError::sync_io(error.to_string()))
}

/// What a snippet holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetEnvelope {
    pub format: String,
    pub version: u32,
    #[serde(flatten)]
    pub sealed: SealedPayload,
}

/// Reads what a snippet holds, refusing anything that is not this application's envelope.
///
/// The refusal is the point rather than a side effect: content that is not this envelope is
/// somebody else's, and an upload that overwrote it because the address happened to be typed right
/// would be the worst thing this feature could do.
pub fn open_snippet(text: &str, passphrase: &str) -> Result<SyncSnapshot, ShellError> {
    let envelope: SnippetEnvelope = serde_json::from_str(text)
        .map_err(|_| ShellError::sync_snapshot_invalid("the file is not one this application wrote"))?;
    if envelope.format != SNIPPET_ENVELOPE_FORMAT {
        return Err(ShellError::sync_snapshot_invalid(
            "the file is not one this application wrote",
        ));
    }
    if envelope.version > SNIPPET_ENVELOPE_VERSION {
        return Err(ShellError::store_schema_newer(
            envelope.version,
            SNIPPET_ENVELOPE_VERSION,
        ));
    }

    let payload = open_payload(passphrase, PayloadPurpose::SyncSnapshot, &envelope.sealed)?;
    let inner = std::str::from_utf8(payload.as_slice())
        .map_err(|error| ShellError::sync_snapshot_invalid(format!("the file could not be read: {error}")))?;

    // Read through the plain snapshot reader rather than parsed here: a snippet and a file must not
    // be able to disagree about what a snapshot is, and that reader is where "what this build
    // understands" is decided.
    match open(inner)? {
        crate::snapshot::OpenedSnapshot::Plain(snapshot) => Ok(*snapshot),
        crate::snapshot::OpenedSnapshot::Sealed(_) => Err(ShellError::sync_snapshot_invalid(
            "the file holds another sealed file rather than a snapshot",
        )),
    }
}

/// Whether a snippet holds a file this application wrote, without needing the passphrase.
///
/// What an upload asks before it writes over anything: opening the file needs the passphrase, and
/// recognising it does not.
pub fn is_ours(text: &str) -> bool {
    serde_json::from_str::<SnippetEnvelope>(text)
        .map(|envelope| envelope.format == SNIPPET_ENVELOPE_FORMAT)
        .unwrap_or(false)
}

fn require_id(sync: &SnippetSync) -> Result<String, ShellError> {
    sync.snippet_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            ShellError::sync_not_configured().with_detail("no gist or snippet has been made for this account yet")
        })
}

/// One request, with whatever this service wants for authentication.
fn request_for(
    sync: &SnippetSync,
    method: &'static str,
    mut url: String,
    body: Option<serde_json::Value>,
) -> Result<SnippetRequest, ShellError> {
    if sync.token.is_empty() {
        return Err(ShellError::sync_not_configured().with_detail("a token is needed for this account"));
    }

    let mut headers: Vec<(String, String)> = Vec::new();
    match sync.provider {
        SnippetProvider::GitHub => {
            headers.push(("accept".to_owned(), "application/vnd.github+json".to_owned()));
            headers.push((
                "user-agent".to_owned(),
                concat!("7Shell/", env!("CARGO_PKG_VERSION")).to_owned(),
            ));
            headers.push(("x-github-api-version".to_owned(), "2022-11-28".to_owned()));
            headers.push(("authorization".to_owned(), format!("Bearer {}", sync.token.expose())));
        }
        // Gitee's own documentation puts the token in the query string rather than a header.
        SnippetProvider::Gitee => {
            let separator = if url.contains('?') { '&' } else { '?' };
            url = format!("{url}{separator}access_token={}", sync.token.expose());
        }
        SnippetProvider::GitLab => {
            headers.push(("private-token".to_owned(), sync.token.expose().to_owned()));
        }
    }

    if body.is_some() {
        headers.push(("content-type".to_owned(), "application/json".to_owned()));
    }

    Ok(SnippetRequest { method, url, headers })
}

/// How long one request to a code-hosting service may take.
///
/// The same budget as a WebDAV request: enough for a small file on a slow line, short enough that a
/// person watching is told something is wrong while they are still looking at it.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A client for one account.
///
/// The requests themselves are the functions above; this is the skin that sends them and reads the
/// answers, so that the shapes can be checked without a network and this can be read as what it is —
/// a request, a status, and a body.
pub struct Snippets {
    http: reqwest::Client,
    sync: SnippetSync,
    base: String,
}

impl Snippets {
    pub fn new(sync: &SnippetSync) -> Result<Self, ShellError> {
        let base = api_base(sync.provider, sync.instance_url.as_deref())?;

        Self::with_base(sync, base)
    }

    /// The same client against an address of the caller's choosing.
    ///
    /// Private, and the reason it exists is worth stating: two of the three services have their API
    /// address compiled in, so a client that wants to be checked against a service standing on this
    /// machine has nowhere else to point. The feature this is ported from grew the same door for the
    /// same reason.
    fn with_base(sync: &SnippetSync, base: String) -> Result<Self, ShellError> {
        let mut builder = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("7Shell/", env!("CARGO_PKG_VERSION")));

        if sync.provider == SnippetProvider::GitLab {
            // A GitLab that does not accept the token answers with a redirect to its sign-in page,
            // and following it would hand the token to whatever that page asks for.
            builder = builder.redirect(reqwest::redirect::Policy::none());
        }

        let http = builder
            .build()
            .map_err(|error| ShellError::app_internal(format!("an HTTP client could not be built: {error}")))?;

        Ok(Self {
            http,
            sync: sync.clone(),
            base,
        })
    }

    /// Asks the service whether the token — and, when there is one, the snippet — is good.
    pub async fn probe(&self) -> Result<(), ShellError> {
        let request = test_request(&self.sync, &self.base)?;
        self.send(request, None).await.map(|_| ())
    }

    /// Reads the file the snippet holds.
    ///
    /// The one shape difference that matters here is GitLab's: its answer lists files as an array
    /// with a `path` rather than as an object keyed by name, and the raw address it gives is
    /// preferred over one this build would construct — the branch a snippet repository uses is
    /// `main` on current installations and `master` on older ones, and only the answer knows which.
    pub async fn read(&self) -> Result<String, ShellError> {
        let id = require_id(&self.sync)?;
        let url = snippet_url(self.sync.provider, &self.base, Some(&id));
        let body = self.send(read_request(&self.sync, &self.base)?, None).await?;

        if self.sync.provider == SnippetProvider::GitLab {
            return self.read_gitlab(&url, &body).await;
        }

        let value: serde_json::Value = serde_json::from_str(&body)
            .map_err(|error| ShellError::sync_snapshot_invalid(format!("the answer: {error}")))?;
        let (content, raw_url) = file_in(&value)?;

        match content {
            Some(text) => Ok(text),
            // A file too long to come back with the answer: the raw address is fetched through the
            // same authenticated request, because a private snippet's raw address needs the token
            // exactly as much as the answer did.
            None => {
                let raw = raw_url.ok_or_else(|| {
                    ShellError::sync_snapshot_invalid("the answer held neither the file nor an address for it")
                })?;
                self.fetch_raw(&raw).await
            }
        }
    }

    async fn read_gitlab(&self, url: &str, body: &str) -> Result<String, ShellError> {
        let value: serde_json::Value = serde_json::from_str(body)
            .map_err(|error| ShellError::sync_snapshot_invalid(format!("the answer: {error}")))?;
        let files = value
            .get("files")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| ShellError::sync_snapshot_invalid("the answer named no files"))?;
        let file = files
            .iter()
            .find(|file| file.get("path").and_then(serde_json::Value::as_str) == Some(SNIPPET_FILE_NAME))
            .ok_or_else(|| ShellError::sync_snapshot_invalid(format!("the snippet has no {SNIPPET_FILE_NAME}")))?;

        let constructed = format!("{url}/files/main/{SNIPPET_FILE_NAME}/raw");
        let raw = file
            .get("raw_url")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| constructed.clone());

        match self.fetch_raw(&raw).await {
            Err(error) if error.code() == sevenshell_error::ErrorCode::SyncHttpFailed && raw == constructed => {
                // The constructed address guessed `main`; an older installation keeps `master`.
                let master = format!("{url}/files/master/{SNIPPET_FILE_NAME}/raw");
                self.fetch_raw(&master).await
            }
            other => other,
        }
    }

    /// Writes the file, creating the gist or snippet when there is none yet, and answers with the id.
    ///
    /// What is there is looked at first: content that is not this application's envelope is somebody
    /// else's, and an id that was typed wrong should stop rather than destroy it.
    ///
    /// What is *sent* is held to the same rule, and this is where that is enforced rather than in the
    /// caller: whatever is handed in has to be this application's sealed envelope already. A snippet
    /// marked private is unlisted — anyone with the address can read it — so a caller that passed a
    /// plain snapshot would be publishing a library, and no caller is trusted to remember that.
    pub async fn write(&self, content: &str) -> Result<String, ShellError> {
        if !is_ours(content) {
            return Err(ShellError::sync_snapshot_invalid(
                "a snippet carries this application's sealed envelope and nothing else",
            ));
        }

        let updating = self.sync.snippet_id.as_deref().is_some_and(|id| !id.trim().is_empty());

        if updating {
            let existing = self.read().await?;
            if !is_ours(&existing) {
                return Err(ShellError::sync_snapshot_invalid(
                    "the snippet at that id was not written by this application, so nothing was uploaded over it",
                ));
            }
        }

        let request = write_request(&self.sync, &self.base, content)?;
        let body = self
            .send(request, Some(create_body(self.sync.provider, content, updating)))
            .await?;

        let value: serde_json::Value = serde_json::from_str(&body)
            .map_err(|error| ShellError::sync_snapshot_invalid(format!("the answer: {error}")))?;

        // A service that answers without an id leaves this build with the one it already had, which
        // is what an update does.
        response_id(&value)
            .or_else(|| self.sync.snippet_id.clone())
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| ShellError::sync_snapshot_invalid("the answer named no gist or snippet"))
    }

    /// Fetches a raw address, with the credentials the service needs for one.
    async fn fetch_raw(&self, url: &str) -> Result<String, ShellError> {
        self.send(request_for(&self.sync, "GET", url.to_owned(), None)?, None)
            .await
    }

    /// Sends one request and answers with its body, or says what went wrong.
    ///
    /// **What the service said is deliberately not in the error.** Gitee takes its token as a query
    /// parameter, so an error body can echo the address it was asked for — and an envelope is a
    /// thing people paste into bug reports. What travels instead is the status, which is what
    /// somebody can act on.
    async fn send(&self, request: SnippetRequest, body: Option<serde_json::Value>) -> Result<String, ShellError> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| ShellError::app_internal(format!("'{}': {error}", request.method)))?;
        let mut builder = self.http.request(method, &request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        if let Some(body) = body {
            // Serialized here rather than through a client feature: the content type is already one
            // of the headers this request declares, and the body is the JSON those headers promise.
            let text = serde_json::to_string(&body)
                .map_err(|error| ShellError::app_internal(format!("the request body: {error}")))?;
            builder = builder.body(text);
        }

        let response = builder.send().await.map_err(|error| {
            if error.is_timeout() {
                ShellError::sync_unreachable().with_detail("the service did not answer in time")
            } else {
                ShellError::sync_unreachable().with_detail("the request failed")
            }
        })?;

        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();

        if status == 401 || status == 403 {
            return Err(ShellError::sync_auth_failed());
        }
        if !(200..300).contains(&status) {
            return Err(ShellError::sync_http_failed(status));
        }

        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot with one session in it, for the envelope tests.
    fn sample_snapshot() -> SyncSnapshot {
        use sevenshell_types::{AuthMethod, Protocol, Session, SessionAuth, SyncSelection, TerminalSettings};

        SyncSnapshot {
            format: crate::snapshot::SNAPSHOT_FORMAT.to_owned(),
            version: crate::snapshot::SNAPSHOT_VERSION,
            exported_at: 1_700_000_000,
            app_version: "test".to_owned(),
            selection: SyncSelection::default(),
            sessions: vec![Session {
                id: "a".to_owned(),
                name: "web".to_owned(),
                group_id: None,
                protocol: Protocol::Ssh,
                host: "web.example".to_owned(),
                port: 22,
                user: "root".to_owned(),
                auth: SessionAuth::with_password(AuthMethod::Password),
                terminal: TerminalSettings::default(),
                jump: Vec::new(),
                forwards: Vec::new(),
                updated_at: 1_700_000_000,
            }],
            groups: Vec::new(),
            snippets: Vec::new(),
            words: Vec::new(),
            history: None,
            known_hosts: None,
            settings: None,
            secrets: None,
            keys: None,
        }
    }

    fn sync(provider: SnippetProvider, id: Option<&str>) -> SnippetSync {
        SnippetSync {
            provider,
            // GitLab is talked to at an installation's own address, and the tests are about the
            // requests rather than that address: one that resolves is enough.
            instance_url: (provider == SnippetProvider::GitLab).then(|| "https://gitlab.example.com".to_owned()),
            token: ServicePassword::new("token-value".to_owned()),
            snippet_id: id.map(str::to_owned),
        }
    }

    #[test]
    fn each_service_is_talked_to_where_it_lives() {
        assert_eq!(
            api_base(SnippetProvider::GitHub, None).expect("github"),
            "https://api.github.com"
        );
        assert_eq!(
            api_base(SnippetProvider::Gitee, None).expect("gitee"),
            "https://gitee.com/api/v5"
        );
        assert_eq!(
            api_base(SnippetProvider::GitLab, Some("https://gitlab.example.com/")).expect("gitlab"),
            "https://gitlab.example.com/api/v4",
            "a self-hosted installation is the normal case"
        );
    }

    #[test]
    fn a_gitlab_address_that_is_not_one_is_refused() {
        for address in [
            Some("ftp://gitlab.example.com"),
            Some("https://user:pw@gitlab.example.com"),
            Some("https://gitlab.example.com?x=1"),
            Some("https://gitlab.example.com#snippets"),
        ] {
            let error = api_base(SnippetProvider::GitLab, address).expect_err("not an address");

            assert_eq!(
                error.code(),
                sevenshell_error::ErrorCode::SyncNotConfigured,
                "{address:?}"
            );
        }
    }

    #[test]
    fn an_empty_gitlab_address_is_the_hosted_installation() {
        // An empty field on the form is not somebody refusing to name an installation; it is the one
        // most accounts are on, and the field shows that address rather than being blank.
        for address in [None, Some(""), Some("   ")] {
            assert_eq!(
                api_base(SnippetProvider::GitLab, address).expect("the hosted one"),
                format!("{GITLAB_DEFAULT_INSTANCE}/api/v4"),
                "{address:?}"
            );
        }
    }

    #[test]
    fn an_account_is_a_service_and_for_gitlab_an_installation() {
        // One token per account, and the name of the account is what both the vault's table and the
        // settings file are keyed by — so it has to be the same string for the same setup, and two
        // installations must not land on one name.
        assert_eq!(
            snippet_account(SnippetProvider::GitHub, None).expect("github"),
            "github"
        );
        assert_eq!(snippet_account(SnippetProvider::Gitee, None).expect("gitee"), "gitee");
        assert_eq!(
            snippet_account(SnippetProvider::GitHub, Some("https://api.github.com")).expect("github"),
            "github",
            "an address is not part of an account this service only has one of"
        );
        assert_eq!(
            snippet_account(SnippetProvider::GitLab, Some("https://gitlab.example.com/")).expect("gitlab"),
            "gitlab:https://gitlab.example.com",
            "the trailing slash goes, or the same installation would be two accounts"
        );
        assert_ne!(
            snippet_account(SnippetProvider::GitLab, Some("https://one.example")).expect("one"),
            snippet_account(SnippetProvider::GitLab, Some("https://two.example")).expect("two"),
            "two installations are two accounts with two tokens"
        );
    }

    #[test]
    fn a_collection_is_a_gist_or_a_snippet_and_the_name_is_the_service_s_own() {
        assert_eq!(
            snippet_url(SnippetProvider::GitHub, "https://api.github.com", None),
            "https://api.github.com/gists"
        );
        assert_eq!(
            snippet_url(SnippetProvider::Gitee, "https://gitee.com/api/v5", None),
            "https://gitee.com/api/v5/gists"
        );
        assert_eq!(
            snippet_url(SnippetProvider::GitLab, "https://gitlab.example.com/api/v4", None),
            "https://gitlab.example.com/api/v4/snippets"
        );
        assert_eq!(
            snippet_url(SnippetProvider::GitHub, "https://api.github.com", Some("abc")),
            "https://api.github.com/gists/abc"
        );
    }

    #[test]
    fn the_token_goes_where_each_service_reads_it() {
        let github = write_request(
            &sync(SnippetProvider::GitHub, Some("abc")),
            "https://api.github.com",
            "{}",
        )
        .expect("github");
        assert_eq!(github.method, "PATCH");
        assert!(
            github
                .headers
                .contains(&("authorization".to_owned(), "Bearer token-value".to_owned()))
        );
        assert!(
            github
                .headers
                .contains(&("x-github-api-version".to_owned(), "2022-11-28".to_owned()))
        );

        let gitee = write_request(
            &sync(SnippetProvider::Gitee, Some("abc")),
            "https://gitee.com/api/v5",
            "{}",
        )
        .expect("gitee");
        assert!(
            gitee.url.ends_with("?access_token=token-value"),
            "Gitee documents the token as a parameter: {}",
            gitee.url
        );
        assert!(
            !gitee.headers.iter().any(|(name, _)| name == "authorization"),
            "and not as a header"
        );

        let gitlab = write_request(
            &sync(SnippetProvider::GitLab, Some("abc")),
            "https://gitlab.example.com/api/v4",
            "{}",
        )
        .expect("gitlab");
        assert!(
            gitlab
                .headers
                .contains(&("private-token".to_owned(), "token-value".to_owned()))
        );
    }

    #[test]
    fn creating_and_updating_are_different_verbs_and_different_bodies() {
        // No id yet: everything is created with a POST.
        for provider in SnippetProvider::ALL {
            let request = write_request(&sync(*provider, None), "https://example.test/api", "{}").expect("create");
            assert_eq!(request.method, "POST", "{provider:?}");
        }

        // One that exists: GitLab updates with PUT, the other two with PATCH.
        assert_eq!(
            write_request(
                &sync(SnippetProvider::GitLab, Some("abc")),
                "https://gitlab.example.com/api/v4",
                "{}"
            )
            .expect("update")
            .method,
            "PUT"
        );
        for provider in [SnippetProvider::GitHub, SnippetProvider::Gitee] {
            assert_eq!(
                write_request(&sync(provider, Some("abc")), "https://example.test/api", "{}")
                    .expect("update")
                    .method,
                "PATCH"
            );
        }

        let gitlab_create = create_body(SnippetProvider::GitLab, "snapshot", false);
        assert_eq!(gitlab_create["visibility"], "private", "never public");
        assert_eq!(gitlab_create["files"][0]["file_path"], SNIPPET_FILE_NAME);
        assert!(
            gitlab_create["files"][0].get("action").is_none(),
            "a creation does not name an action"
        );

        let gitlab_update = create_body(SnippetProvider::GitLab, "snapshot", true);
        assert_eq!(gitlab_update["files"][0]["action"], "update");

        // The other two nest files by name, and both keep `public` false.
        for provider in [SnippetProvider::GitHub, SnippetProvider::Gitee] {
            let body = create_body(provider, "snapshot", false);
            assert_eq!(body["files"][SNIPPET_FILE_NAME]["content"], "snapshot", "{provider:?}");
            assert!(body["files"].is_object(), "{provider:?}");
            assert_eq!(body["public"], false, "{provider:?}");
        }
    }

    #[test]
    fn an_id_is_read_from_an_object_or_from_a_list() {
        assert_eq!(
            response_id(&serde_json::json!({"id": "abc123"})),
            Some("abc123".to_owned()),
            "GitHub answers with one gist"
        );
        assert_eq!(
            response_id(&serde_json::json!([{"id": 42}])),
            Some("42".to_owned()),
            "Gitee answers with a list, and its ids are numbers"
        );
        assert_eq!(response_id(&serde_json::json!({"message": "nope"})), None);
    }

    #[test]
    fn a_truncated_file_is_read_from_its_raw_address() {
        let truncated = serde_json::json!({
            "files": { SNIPPET_FILE_NAME: { "truncated": true, "raw_url": "https://example.test/raw" } }
        });
        let (content, raw) = file_in(&truncated).expect("read");
        assert_eq!(content, None, "the content is not there to read");
        assert_eq!(raw.as_deref(), Some("https://example.test/raw"));

        let complete = serde_json::json!({
            "files": { SNIPPET_FILE_NAME: { "content": "{\"format\":\"x\"}" } }
        });
        let (content, raw) = file_in(&complete).expect("read");
        assert_eq!(content.as_deref(), Some("{\"format\":\"x\"}"));
        assert_eq!(raw, None);
    }

    #[test]
    fn a_file_list_answered_as_a_string_is_unwrapped() {
        // Gitee answers with the same nesting, written out as JSON inside a string.
        let answer = serde_json::json!({
            "files": format!("{{\"{SNIPPET_FILE_NAME}\":{{\"content\":\"snapshot\"}}}}")
        });

        let (content, _) = file_in(&answer).expect("read");

        assert_eq!(content.as_deref(), Some("snapshot"));
    }

    #[test]
    fn an_envelope_is_recognised_without_the_passphrase_and_read_with_it() {
        let document = sample_snapshot();
        let text = seal(&document, "correct horse", &Default::default()).expect("sealed");

        assert!(is_ours(&text), "an upload asks this before it writes over anything");
        assert!(!is_ours("{\"somebody\":\"else's file\"}"));

        let read = open_snippet(&text, "correct horse").expect("opened");
        assert_eq!(read.contents().sessions, document.contents().sessions);
    }

    #[test]
    fn somebody_else_s_file_is_refused_rather_than_read() {
        let error =
            open_snippet("{\"files\":{\"notes.txt\":{\"content\":\"hi\"}}}", "correct horse").expect_err("not ours");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SyncSnapshotInvalid);
    }

    #[test]
    fn the_wrong_passphrase_opens_nothing() {
        let document = sample_snapshot();
        let text = seal(&document, "correct horse", &Default::default()).expect("sealed");

        let error = open_snippet(&text, "Correct Horse").expect_err("not the passphrase");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SyncPassphrase);
    }

    // -- What the client sends, against a service answering on this machine -----------------------

    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    /// One request a fake service received.
    #[derive(Debug, Clone)]
    struct Received {
        method: String,
        path: String,
        headers: HashMap<String, String>,
        body: String,
    }

    /// A code-hosting service with no TLS, on a port the operating system picks.
    ///
    /// Answers are keyed by `"METHOD /path"`; anything else is a 404, which is what a service says
    /// about a gist that is not there.
    struct Fake {
        base: String,
        seen: Arc<Mutex<Vec<Received>>>,
    }

    impl Fake {
        async fn start(answers: Vec<(String, u16, String)>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
            let base = format!("http://{}", listener.local_addr().expect("an address"));
            let seen = Arc::new(Mutex::new(Vec::new()));
            // An answer may name this service's own address — a raw file's is absolute — which is
            // only known once it is listening, so `{BASE}` stands in for it.
            let answers: HashMap<String, (u16, String)> = answers
                .into_iter()
                .map(|(key, status, body)| (key, (status, body.replace("{BASE}", &base))))
                .collect();

            let seen_for_task = Arc::clone(&seen);
            tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        return;
                    };
                    let seen = Arc::clone(&seen_for_task);
                    let answers = answers.clone();
                    tokio::spawn(async move {
                        let _ = serve(stream, seen, answers).await;
                    });
                }
            });

            Self { base, seen }
        }

        fn requests(&self) -> Vec<Received> {
            self.seen.lock().expect("seen").clone()
        }
    }

    async fn serve(
        mut stream: TcpStream,
        seen: Arc<Mutex<Vec<Received>>>,
        answers: HashMap<String, (u16, String)>,
    ) -> std::io::Result<()> {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let read = stream.read(&mut chunk).await?;
            if read == 0 {
                return Ok(());
            }
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(end) = header_end(&buffer) {
                let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
                if buffer.len() >= end + 4 + content_length(&head) {
                    break;
                }
            }
        }

        let end = header_end(&buffer).unwrap_or(buffer.len());
        let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
        let body = String::from_utf8_lossy(&buffer[end + 4..]).into_owned();

        let mut lines = head.lines();
        let mut parts = lines.next().unwrap_or_default().split_whitespace();
        let method = parts.next().unwrap_or_default().to_owned();
        let path = parts.next().unwrap_or_default().to_owned();
        let headers: HashMap<String, String> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
            .collect();

        seen.lock().expect("seen").push(Received {
            method: method.clone(),
            path: path.clone(),
            headers,
            body,
        });

        let route = path.split('?').next().unwrap_or(&path);
        let (status, answer) = answers
            .get(&format!("{method} {route}"))
            .cloned()
            .unwrap_or((404, String::new()));
        let response = format!(
            "HTTP/1.1 {status} x\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
            answer.len()
        );
        stream.write_all(response.as_bytes()).await?;
        stream.flush().await
    }

    fn header_end(buffer: &[u8]) -> Option<usize> {
        buffer.windows(4).position(|window| window == b"\r\n\r\n")
    }

    fn content_length(head: &str) -> usize {
        head.lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse().ok())
            .unwrap_or(0)
    }

    /// A client pointed at a service on this machine, for one provider.
    fn pointed(sync: &SnippetSync, base: &str) -> Snippets {
        Snippets::with_base(sync, base.to_owned()).expect("a client")
    }

    /// An account whose snippet id is `abc`.
    fn account(provider: SnippetProvider, id: &str) -> SnippetSync {
        SnippetSync {
            provider,
            instance_url: None,
            token: ServicePassword::new("token-value".to_owned()),
            snippet_id: Some(id.to_owned()),
        }
    }

    /// What a gist looks like when it holds this application's envelope.
    fn holding(content: &str) -> String {
        serde_json::json!({ "files": { SNIPPET_FILE_NAME: { "content": content } } }).to_string()
    }

    /// An envelope this application wrote, which is the only thing an upload is allowed to send.
    fn sealed() -> String {
        seal(&sample_snapshot(), "correct horse", &Default::default()).expect("sealed")
    }

    #[tokio::test]
    async fn creating_a_gist_sends_the_file_and_takes_the_id_from_the_answer() {
        let server = Fake::start(vec![("POST /gists".to_owned(), 201, "{\"id\":\"abc123\"}".to_owned())]).await;
        let sync = SnippetSync {
            provider: SnippetProvider::GitHub,
            instance_url: None,
            token: ServicePassword::new("token-value".to_owned()),
            snippet_id: None,
        };
        let envelope = sealed();

        let id = pointed(&sync, &server.base).write(&envelope).await.expect("created");

        assert_eq!(id, "abc123", "the id comes back for the next upload to patch");
        let requests = server.requests();
        assert_eq!(requests.len(), 1, "one creation, and nothing read first");
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, "/gists");
        assert_eq!(
            requests[0].headers.get("authorization").map(String::as_str),
            Some("Bearer token-value")
        );
        let body: serde_json::Value = serde_json::from_str(&requests[0].body).expect("the body is JSON");
        assert_eq!(body["files"][SNIPPET_FILE_NAME]["content"], envelope);
        assert_eq!(body["public"], false, "never a public gist");
    }

    #[tokio::test]
    async fn a_snapshot_that_is_not_sealed_is_refused_before_anything_is_sent() {
        // A snippet marked private is unlisted rather than unreadable, so what goes into one has to be
        // the sealed envelope and nothing else. The refusal is here rather than in the caller because
        // no caller may be trusted with a rule that publishes a library when it is forgotten.
        let server = Fake::start(vec![("POST /gists".to_owned(), 201, "{\"id\":\"abc123\"}".to_owned())]).await;
        let sync = SnippetSync {
            provider: SnippetProvider::GitHub,
            instance_url: None,
            token: ServicePassword::new("token-value".to_owned()),
            snippet_id: None,
        };

        let error = pointed(&sync, &server.base)
            .write(&sample_snapshot().to_json().expect("json"))
            .await
            .expect_err("a plain snapshot is not a snippet");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SyncSnapshotInvalid);
        assert!(
            server.requests().is_empty(),
            "nothing was sent, so nothing was published"
        );
    }

    #[tokio::test]
    async fn updating_reads_what_is_there_first_and_then_patches_it() {
        let envelope = seal(&sample_snapshot(), "correct horse", &Default::default()).expect("sealed");
        let server = Fake::start(vec![
            ("GET /gists/abc".to_owned(), 200, holding(&envelope)),
            ("PATCH /gists/abc".to_owned(), 200, "{\"id\":\"abc\"}".to_owned()),
        ])
        .await;
        let sync = account(SnippetProvider::GitHub, "abc");

        let id = pointed(&sync, &server.base).write(&sealed()).await.expect("updated");

        assert_eq!(id, "abc");
        let requests = server.requests();
        let shape: Vec<(&str, &str)> = requests
            .iter()
            .map(|request| (request.method.as_str(), request.path.as_str()))
            .collect();
        assert_eq!(
            shape,
            vec![("GET", "/gists/abc"), ("PATCH", "/gists/abc")],
            "what is there is read before it is written over"
        );
    }

    #[tokio::test]
    async fn a_gist_this_application_did_not_write_is_refused_and_left_alone() {
        // An id somebody typed wrong lands on content that belongs to another purpose entirely.
        let server = Fake::start(vec![(
            "GET /gists/abc".to_owned(),
            200,
            "{\"files\":{\"notes.txt\":{\"content\":\"somebody else's shopping list\"}}}".to_owned(),
        )])
        .await;
        let sync = account(SnippetProvider::GitHub, "abc");

        let error = pointed(&sync, &server.base)
            .write(&sealed())
            .await
            .expect_err("not ours");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SyncSnapshotInvalid);
        let requests = server.requests();
        assert_eq!(requests.len(), 1, "the read happened");
        assert_eq!(requests[0].method, "GET");
        assert!(
            !requests.iter().any(|request| request.method == "PATCH"),
            "and nothing was written over it"
        );
    }

    #[tokio::test]
    async fn a_truncated_file_is_fetched_from_its_raw_address_with_the_token() {
        let body = serde_json::json!({
            "files": { SNIPPET_FILE_NAME: { "truncated": true, "raw_url": "{BASE}/raw/gist-abc" } }
        })
        .to_string();
        let server = Fake::start(vec![
            ("GET /gists/abc".to_owned(), 200, body),
            ("GET /raw/gist-abc".to_owned(), 200, "the whole file".to_owned()),
        ])
        .await;
        let sync = account(SnippetProvider::GitHub, "abc");

        let text = pointed(&sync, &server.base).read().await.expect("read");

        assert_eq!(text, "the whole file", "the content came from the raw address");
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/raw/gist-abc");
        assert_eq!(
            requests[1].headers.get("authorization").map(String::as_str),
            Some("Bearer token-value"),
            "a private gist's raw address needs the token as much as the answer did"
        );
    }

    #[tokio::test]
    async fn gitlab_is_read_through_its_own_shape_and_falls_back_to_master() {
        // No raw address in the answer, so this build constructs one — and the branch it guesses is
        // wrong, which is what the fallback is for.
        let server = Fake::start(vec![
            (
                "GET /snippets/abc".to_owned(),
                200,
                serde_json::json!({
                    "files": [{ "path": SNIPPET_FILE_NAME, "raw_url": "" }]
                })
                .to_string(),
            ),
            (
                format!("GET /snippets/abc/files/main/{SNIPPET_FILE_NAME}/raw"),
                404,
                String::new(),
            ),
            (
                format!("GET /snippets/abc/files/master/{SNIPPET_FILE_NAME}/raw"),
                200,
                "the whole file".to_owned(),
            ),
        ])
        .await;
        let sync = account(SnippetProvider::GitLab, "abc");

        let text = pointed(&sync, &server.base).read().await.expect("read");

        assert_eq!(text, "the whole file");
        let requests = server.requests();
        let paths: Vec<&str> = requests.iter().map(|request| request.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "/snippets/abc",
                &format!("/snippets/abc/files/main/{SNIPPET_FILE_NAME}/raw"),
                &format!("/snippets/abc/files/master/{SNIPPET_FILE_NAME}/raw"),
            ],
            "main first, then master, which is what an older installation keeps"
        );
    }

    #[tokio::test]
    async fn a_token_the_service_refuses_is_its_own_answer() {
        let server = Fake::start(vec![("GET /user".to_owned(), 401, String::new())]).await;
        let sync = SnippetSync {
            provider: SnippetProvider::GitHub,
            instance_url: None,
            token: ServicePassword::new("stale".to_owned()),
            snippet_id: None,
        };

        let error = pointed(&sync, &server.base).probe().await.expect_err("refused");

        assert_eq!(error.code(), sevenshell_error::ErrorCode::SyncAuthFailed);
        assert_eq!(
            server.requests()[0].path,
            "/user",
            "with no snippet yet, the account is what can be checked"
        );
    }

    #[tokio::test]
    async fn gitee_is_sent_its_token_as_a_parameter_and_the_rest_of_the_api_is_the_same() {
        let server = Fake::start(vec![("POST /gists".to_owned(), 201, "[{\"id\":42}]".to_owned())]).await;
        let sync = SnippetSync {
            provider: SnippetProvider::Gitee,
            instance_url: None,
            token: ServicePassword::new("token-value".to_owned()),
            snippet_id: None,
        };

        let id = pointed(&sync, &server.base).write(&sealed()).await.expect("created");

        assert_eq!(id, "42", "Gitee answers with a list, and its ids are numbers");
        let request = &server.requests()[0];
        assert_eq!(request.path, "/gists?access_token=token-value");
        assert!(
            !request.headers.contains_key("authorization"),
            "and the token is not also a header"
        );
    }
}
