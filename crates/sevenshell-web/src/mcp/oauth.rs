//! OAuth for external MCP servers.
//!
//! The flow is the MCP authorization profile: discover the protected resource's authorization
//! server, register a public client when the server offers registration, run authorization code +
//! PKCE through the system browser, exchange the code at the loopback callback, and keep the access
//! token in the same vault entry as the server's other credentials. A background refresh renews the
//! token before it expires.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::Url;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::{Value, json};
use sevenshell_error::ShellError;
use sevenshell_mcp_client::McpClientManager;
use sevenshell_types::{McpServerConfig, McpServerTransport};
use sevenshell_vault::Vault;
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;

use super::{StoredOAuth, apply_secrets, load_stored, save_stored};

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);
const REFRESH_SKEW_SECONDS: u64 = 120;

pub struct PendingOAuth {
    pub server_id: String,
    verifier: String,
    redirect_uri: String,
    token_endpoint: String,
    client_id: String,
    resource: Option<String>,
    created_at: u64,
}

impl PendingOAuth {
    pub fn expired(&self) -> bool {
        now().saturating_sub(self.created_at) > 600
    }
}

struct Endpoints {
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: Option<String>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

fn client() -> Result<reqwest::Client, ShellError> {
    reqwest::Client::builder()
        .timeout(DISCOVERY_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| ShellError::app_internal(format!("the OAuth client could not be built: {error}")))
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                utf8_percent_encode(key, NON_ALPHANUMERIC),
                utf8_percent_encode(value, NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

async fn get_json(client: &reqwest::Client, url: &str) -> Result<Value, ShellError> {
    let response = client
        .get(url)
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| ShellError::app_internal(format!("OAuth discovery failed: {error}")))?;
    if !response.status().is_success() {
        return Err(ShellError::app_internal(format!(
            "OAuth discovery answered status {}",
            response.status().as_u16()
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| ShellError::app_internal(format!("OAuth discovery could not be read: {error}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| ShellError::app_internal(format!("OAuth discovery is not JSON: {error}")))
}

async fn post_json(client: &reqwest::Client, url: &str, body: &Value) -> Result<Value, ShellError> {
    let response = client
        .post(url)
        .header(ACCEPT, "application/json")
        .header(CONTENT_TYPE, "application/json")
        .body(
            serde_json::to_vec(body)
                .map_err(|error| ShellError::app_internal(format!("the registration body is not JSON: {error}")))?,
        )
        .send()
        .await
        .map_err(|error| ShellError::app_internal(format!("OAuth registration failed: {error}")))?;
    if !response.status().is_success() {
        return Err(ShellError::app_internal(format!(
            "OAuth registration answered status {}",
            response.status().as_u16()
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| ShellError::app_internal(format!("OAuth registration could not be read: {error}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| ShellError::app_internal(format!("OAuth registration is not JSON: {error}")))
}

async fn post_form(client: &reqwest::Client, url: &str, pairs: &[(&str, &str)]) -> Result<Value, ShellError> {
    let response = client
        .post(url)
        .header(ACCEPT, "application/json")
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(form(pairs))
        .send()
        .await
        .map_err(|error| ShellError::app_internal(format!("the token endpoint could not be reached: {error}")))?;
    if !response.status().is_success() {
        return Err(ShellError::app_internal(format!(
            "the token endpoint answered status {}",
            response.status().as_u16()
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| ShellError::app_internal(format!("the token answer could not be read: {error}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| ShellError::app_internal(format!("the token answer is not JSON: {error}")))
}

async fn discover(
    client: &reqwest::Client,
    server: &McpServerConfig,
) -> Result<(Endpoints, Option<String>), ShellError> {
    let url = server
        .url
        .as_deref()
        .filter(|url| !url.trim().is_empty())
        .ok_or_else(|| ShellError::app_internal("this server has no URL to authorize"))?;
    let parsed =
        Url::parse(url).map_err(|error| ShellError::app_internal(format!("the server URL is not valid: {error}")))?;
    let origin = parsed.origin().ascii_serialization();
    let resource = url.to_owned();

    let mut authorization_servers: Vec<String> = Vec::new();
    let path_suffix = parsed.path().trim_end_matches('/');
    for candidate in [
        format!("{origin}/.well-known/oauth-protected-resource"),
        if path_suffix.is_empty() || path_suffix == "/" {
            String::new()
        } else {
            format!("{origin}/.well-known/oauth-protected-resource{path_suffix}")
        },
    ] {
        if candidate.is_empty() {
            continue;
        }
        if let Ok(metadata) = get_json(client, &candidate).await {
            if let Some(servers) = metadata.get("authorization_servers").and_then(Value::as_array) {
                authorization_servers = servers.iter().filter_map(Value::as_str).map(str::to_owned).collect();
            }
            break;
        }
    }

    let issuer = authorization_servers.first().cloned().unwrap_or_else(|| origin.clone());
    let issuer = issuer.trim_end_matches('/').to_owned();
    let mut metadata = None;
    for candidate in [
        format!("{issuer}/.well-known/oauth-authorization-server"),
        format!("{issuer}/.well-known/openid-configuration"),
    ] {
        if let Ok(answer) = get_json(client, &candidate).await {
            metadata = Some(answer);
            break;
        }
    }
    let metadata =
        metadata.ok_or_else(|| ShellError::app_internal("the authorization server metadata could not be read"))?;
    let authorization_endpoint = metadata
        .get("authorization_endpoint")
        .and_then(Value::as_str)
        .ok_or_else(|| ShellError::app_internal("the authorization server has no authorization endpoint"))?
        .to_owned();
    let token_endpoint = metadata
        .get("token_endpoint")
        .and_then(Value::as_str)
        .ok_or_else(|| ShellError::app_internal("the authorization server has no token endpoint"))?
        .to_owned();
    let registration_endpoint = metadata
        .get("registration_endpoint")
        .and_then(Value::as_str)
        .map(str::to_owned);

    Ok((
        Endpoints {
            authorization_endpoint,
            token_endpoint,
            registration_endpoint,
        },
        Some(resource),
    ))
}

async fn ensure_client(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    redirect_uri: &str,
    stored: &super::StoredSecrets,
) -> Result<String, ShellError> {
    if let Some(oauth) = &stored.oauth
        && !oauth.client_id.is_empty()
    {
        return Ok(oauth.client_id.clone());
    }
    let Some(registration_endpoint) = endpoints.registration_endpoint.as_deref() else {
        return Err(ShellError::app_internal(
            "the authorization server offers no client registration and no client id is stored",
        ));
    };
    let registration = post_json(
        client,
        registration_endpoint,
        &json!({
            "client_name": "7Shell",
            "redirect_uris": [redirect_uri],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none"
        }),
    )
    .await?;
    registration
        .get("client_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ShellError::app_internal("the registration answer has no client id"))
}

pub async fn begin(
    vault: &Vault,
    server: &McpServerConfig,
    web_port: u16,
) -> Result<(String, String, PendingOAuth), ShellError> {
    if !matches!(server.transport, McpServerTransport::Http | McpServerTransport::Sse) {
        return Err(ShellError::app_internal("OAuth applies to HTTP and SSE servers only"));
    }
    let client = client()?;
    let (endpoints, resource) = discover(&client, server).await?;
    let redirect_uri = format!("http://127.0.0.1:{web_port}/oauth/callback");
    let stored = load_stored(vault, &server.id);
    let client_id = ensure_client(&client, &endpoints, &redirect_uri, &stored).await?;

    let mut verifier_bytes = [0u8; 32];
    getrandom::fill(&mut verifier_bytes)
        .map_err(|error| ShellError::app_internal(format!("PKCE randomness failed: {error}")))?;
    let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = uuid::Uuid::new_v4().simple().to_string();

    let mut authorization = Url::parse(&endpoints.authorization_endpoint)
        .map_err(|error| ShellError::app_internal(format!("the authorization endpoint is not a URL: {error}")))?;
    {
        let mut query = authorization.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", &client_id);
        query.append_pair("redirect_uri", &redirect_uri);
        query.append_pair("code_challenge", &challenge);
        query.append_pair("code_challenge_method", "S256");
        query.append_pair("state", &state);
        if let Some(resource) = resource.as_deref() {
            query.append_pair("resource", resource);
        }
    }

    let pending = PendingOAuth {
        server_id: server.id.clone(),
        verifier,
        redirect_uri,
        token_endpoint: endpoints.token_endpoint,
        client_id,
        resource,
        created_at: now(),
    };
    Ok((authorization.to_string(), state, pending))
}

pub async fn complete(
    vault: &Vault,
    manager: &McpClientManager,
    pending: PendingOAuth,
    code: &str,
) -> Result<String, ShellError> {
    let client = client()?;
    let mut pairs = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", pending.redirect_uri.as_str()),
        ("client_id", pending.client_id.as_str()),
        ("code_verifier", pending.verifier.as_str()),
    ];
    if let Some(resource) = pending.resource.as_deref() {
        pairs.push(("resource", resource));
    }
    let answer = post_form(&client, &pending.token_endpoint, &pairs).await?;
    let (access_token, refresh_token, expires_at) = parse_token(&answer)?;

    let mut stored = load_stored(vault, &pending.server_id);
    stored.oauth = Some(StoredOAuth {
        access_token,
        refresh_token,
        expires_at,
        token_endpoint: pending.token_endpoint,
        client_id: pending.client_id,
        resource: pending.resource,
    });
    save_stored(vault, &pending.server_id, &stored)?;
    apply_secrets(vault, manager, &manager.configured());
    Ok(pending.server_id)
}

fn parse_token(answer: &Value) -> Result<(String, Option<String>, u64), ShellError> {
    let access_token = answer
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| ShellError::app_internal("the token answer has no access token"))?
        .to_owned();
    let refresh_token = answer
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_owned);
    let lifetime = answer.get("expires_in").and_then(Value::as_u64).unwrap_or(3_600);
    Ok((
        access_token,
        refresh_token,
        now().saturating_add(lifetime.saturating_sub(30)),
    ))
}

async fn refresh(vault: &Vault, server_id: &str) -> Result<bool, ShellError> {
    let stored = load_stored(vault, server_id);
    let Some(oauth) = stored.oauth.clone() else {
        return Ok(false);
    };
    let Some(refresh_token) = oauth.refresh_token.clone() else {
        return Ok(false);
    };
    let client = client()?;
    let mut pairs = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.as_str()),
        ("client_id", oauth.client_id.as_str()),
    ];
    if let Some(resource) = oauth.resource.as_deref() {
        pairs.push(("resource", resource));
    }
    let answer = post_form(&client, &oauth.token_endpoint, &pairs).await?;
    let (access_token, new_refresh, expires_at) = parse_token(&answer)?;
    let mut updated = load_stored(vault, server_id);
    updated.oauth = Some(StoredOAuth {
        access_token,
        refresh_token: new_refresh.or(Some(refresh_token)),
        expires_at,
        token_endpoint: oauth.token_endpoint,
        client_id: oauth.client_id,
        resource: oauth.resource,
    });
    save_stored(vault, server_id, &updated)?;
    Ok(true)
}

/// Renews an access token shortly before it expires. Runs for the lifetime of the process.
pub fn spawn_refresh(vault: Arc<Vault>, manager: Arc<McpClientManager>, events: broadcast::Sender<String>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let servers = manager.configured();
            let mut refreshed: Vec<String> = Vec::new();
            for server in &servers {
                let stored = load_stored(&vault, &server.id);
                let Some(oauth) = stored.oauth.as_ref() else {
                    continue;
                };
                if oauth.expires_at > now().saturating_add(REFRESH_SKEW_SECONDS) {
                    continue;
                }
                match refresh(&vault, &server.id).await {
                    Ok(true) => refreshed.push(server.id.clone()),
                    Ok(false) => {}
                    Err(error) => log::warn!("the OAuth token for `{}` could not be renewed: {error}", server.id),
                }
            }
            if !refreshed.is_empty() {
                apply_secrets(&vault, &manager, &manager.configured());
                // An open HTTP session still carries the header it was opened with, so a renewed
                // token only takes effect after the session is replaced.
                for server_id in refreshed {
                    if let Ok(status) = manager.refresh(&server_id).await {
                        let _ = events.send(json!({ "event": "mcp:server_status", "payload": status }).to_string());
                    }
                }
            }
        }
    });
}
