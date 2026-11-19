//! The person's OpenSSH configuration, reduced to what a session can use.
//!
//! The parser lives in `sevenshell-ssh`; this module is the one place that knows *where* the file
//! is and turns an alias into the values the interface may see. Identity files stay here: the
//! picker gets an alias, an address, an account and a port, and never a local path.

use sevenshell_ssh::openssh::{JumpStep, ParsedConfig, ResolvedHost, parse_proxy_jump};
use sevenshell_types::SshConfigHost;

/// The port a hop uses when neither the `ProxyJump` value nor its `Host` block names one.
const DEFAULT_JUMP_PORT: u16 = 22;

/// How many hops a `ProxyJump` chain may name before it is treated as a mistake.
const MAX_PROXY_HOPS: usize = 16;

/// One hop between this machine and the target, as `ProxyJump` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyHop {
    /// The name as written, which is what a message about this hop should show.
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// The hop's own `Host` block when one matched; its `IdentityFile` values live here.
    pub config: Option<ResolvedHost>,
}

/// Why a `ProxyJump` chain cannot be followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyChainError {
    /// A hop that is not `[user@]host[:port]`, as written.
    Invalid(String),
    /// A chain that revisits a hop, rendered in the order it was walked.
    Loop(String),
    /// A hop's own configuration uses a directive this build will not ignore.
    Directive(String),
}

/// Every concrete alias the configuration names, with the values it resolves to.
///
/// A machine without OpenSSH, or without a config file, has no hosts — an empty list is the answer
/// there, not an error. Directives a connection cannot honour are carried in `unsupported` so the
/// picker can say why an entry will refuse rather than hiding it.
pub fn hosts() -> Vec<SshConfigHost> {
    config().map(|config| hosts_from(&config)).unwrap_or_default()
}

/// The parsed configuration of this machine, when it has one.
pub fn config() -> Option<ParsedConfig> {
    let path = sevenshell_platform::openssh_config_path()?;
    let home = sevenshell_platform::home_dir()?;
    Some(sevenshell_ssh::openssh::load(&path, &home))
}

/// Resolves one alias against a parsed configuration. `None` when the file could not be read.
pub fn resolve(alias: &str) -> Option<ResolvedHost> {
    config().map(|config| config.resolve(alias))
}

/// The safe projection of a parsed configuration, for tests and for [`hosts`].
pub fn hosts_from(config: &ParsedConfig) -> Vec<SshConfigHost> {
    config
        .aliases()
        .into_iter()
        .map(|alias| {
            let resolved = config.resolve(&alias);
            let mut unsupported = resolved.unsupported.clone();
            if resolved.proxy_command.is_some() {
                note(&mut unsupported, "ProxyCommand");
            }
            // A value the connection could not parse is carried as unsupported rather than shown
            // as a route: the picker disables the row, so nobody saves a session that refuses.
            let proxy_jump = resolved
                .proxy_jump
                .as_deref()
                .and_then(|value| match parse_proxy_jump(value) {
                    Ok(steps) if steps.is_empty() => None,
                    Ok(_) => Some(value.to_owned()),
                    Err(_) => {
                        note(&mut unsupported, "ProxyJump");
                        None
                    }
                });
            SshConfigHost {
                host: resolved.address(&alias),
                user: resolved.user,
                port: resolved.port,
                unsupported,
                proxy_jump,
                alias,
            }
        })
        .collect()
}

/// Every hop between this machine and a target whose alias names a `ProxyJump`.
///
/// The order is the order connections are made in: the outermost jump first, the one reached
/// directly, and the target's own jump last. A hop may itself have a `ProxyJump` block; that chain
/// is walked outward the same way, because dialling the nearer host directly is exactly the route
/// the configuration was written to avoid. `fallback_user` answers for a hop that names no
/// account and has no `Host` block.
pub fn proxy_hops(
    config: &ParsedConfig,
    resolved: &ResolvedHost,
    fallback_user: &str,
) -> Result<Vec<ProxyHop>, ProxyChainError> {
    let Some(value) = resolved.proxy_jump.as_deref() else {
        return Ok(Vec::new());
    };
    let steps = parse_proxy_jump(value).map_err(ProxyChainError::Invalid)?;

    let mut hops = Vec::new();
    let mut path = Vec::new();
    for step in steps {
        collect_hop(config, &step, fallback_user, &mut hops, &mut path)?;
    }
    Ok(hops)
}

fn collect_hop(
    config: &ParsedConfig,
    step: &JumpStep,
    fallback_user: &str,
    hops: &mut Vec<ProxyHop>,
    path: &mut Vec<String>,
) -> Result<(), ProxyChainError> {
    let name = step.host.to_ascii_lowercase();
    path.push(name.clone());
    if path.len() > MAX_PROXY_HOPS || path.iter().filter(|entry| **entry == name).count() > 1 {
        return Err(ProxyChainError::Loop(path.join(" -> ")));
    }

    let resolved = config.resolve(&step.host);
    let entry = resolved.matched.then_some(resolved);

    if let Some(entry) = &entry {
        if entry.proxy_command.is_some() {
            return Err(ProxyChainError::Directive("ProxyCommand".to_owned()));
        }
        if let Some(directive) = entry.unsupported.first() {
            return Err(ProxyChainError::Directive(directive.clone()));
        }
        if let Some(next) = entry.proxy_jump.as_deref() {
            let steps = parse_proxy_jump(next).map_err(ProxyChainError::Invalid)?;
            for nested in steps {
                collect_hop(config, &nested, fallback_user, hops, path)?;
            }
        }
    }

    let (host, port, user) = match &entry {
        Some(entry) => (
            entry.host_name.clone().unwrap_or_else(|| step.host.clone()),
            step.port.or(entry.port).unwrap_or(DEFAULT_JUMP_PORT),
            step.user
                .clone()
                .or_else(|| entry.user.clone())
                .unwrap_or_else(|| fallback_user.to_owned()),
        ),
        None => (
            step.host.clone(),
            step.port.unwrap_or(DEFAULT_JUMP_PORT),
            step.user.clone().unwrap_or_else(|| fallback_user.to_owned()),
        ),
    };

    hops.push(ProxyHop {
        name: step.host.clone(),
        host,
        port,
        user,
        config: entry,
    });
    path.pop();
    Ok(())
}

fn note(list: &mut Vec<String>, name: &str) {
    if !list.iter().any(|existing| existing == name) {
        list.push(name.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_is_projected_without_its_identity_files() {
        let config = sevenshell_ssh::openssh::parse(
            "Host web\n  HostName 10.0.0.5\n  User deploy\n  Port 2222\n  IdentityFile ~/.ssh/id_web\n",
        );
        let hosts = hosts_from(&config);

        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].alias, "web");
        assert_eq!(hosts[0].host, "10.0.0.5");
        assert_eq!(hosts[0].user.as_deref(), Some("deploy"));
        assert_eq!(hosts[0].port, Some(2222));
        assert!(hosts[0].unsupported.is_empty());
    }

    #[test]
    fn a_proxy_jump_is_a_route_rather_than_a_warning() {
        let config = sevenshell_ssh::openssh::parse("Host web\n  HostName 10.0.0.5\n  ProxyJump bastion\n");
        let hosts = hosts_from(&config);

        assert!(hosts[0].unsupported.is_empty());
        assert_eq!(hosts[0].proxy_jump.as_deref(), Some("bastion"));
    }

    #[test]
    fn a_proxy_command_is_still_named_in_the_warning_list() {
        let config = sevenshell_ssh::openssh::parse(
            "Host web\n  HostName 10.0.0.5\n  ProxyJump bastion\n  ProxyCommand ssh -W %h:%p bastion\n",
        );
        let hosts = hosts_from(&config);

        assert!(hosts[0].unsupported.iter().any(|name| name == "ProxyCommand"));
        assert!(!hosts[0].unsupported.iter().any(|name| name == "ProxyJump"));
    }

    #[test]
    fn a_malformed_proxy_jump_leaves_the_row_unselectable() {
        let config = sevenshell_ssh::openssh::parse("Host web\n  ProxyJump bastion:\n");
        let hosts = hosts_from(&config);

        assert!(hosts[0].unsupported.iter().any(|name| name == "ProxyJump"));
        assert_eq!(hosts[0].proxy_jump, None);
    }

    #[test]
    fn a_proxy_route_is_walked_outward_and_reads_each_hop_block() {
        let config = sevenshell_ssh::openssh::parse(
            "Host web\n  HostName 10.0.0.5\n  User deploy\n  ProxyJump gateway\n\
             Host gateway\n  HostName 10.0.0.9\n  User ops\n  Port 2200\n  ProxyJump jumpbox\n\
             Host jumpbox\n  HostName 10.0.0.1\n",
        );
        let resolved = config.resolve("web");
        let hops = proxy_hops(&config, &resolved, &resolved.user.clone().unwrap_or_default()).expect("a route");

        assert_eq!(
            hops.iter().map(|hop| hop.host.as_str()).collect::<Vec<_>>(),
            ["10.0.0.1", "10.0.0.9"],
            "the outermost jump is connected to first"
        );
        assert_eq!(hops[1].user, "ops");
        assert_eq!(hops[1].port, 2200);
        assert_eq!(hops[0].user, "deploy", "a hop with no account takes the leaf's");
    }

    #[test]
    fn an_explicit_user_and_port_beat_the_hop_block() {
        let config = sevenshell_ssh::openssh::parse(
            "Host web\n  ProxyJump root@10.0.0.9:2022\nHost 10.0.0.9\n  User ops\n  Port 2200\n",
        );
        let resolved = config.resolve("web");
        let hops = proxy_hops(&config, &resolved, "deploy").expect("a route");

        assert_eq!(hops.len(), 1);
        assert_eq!(hops[0].user, "root");
        assert_eq!(hops[0].port, 2022);
    }

    #[test]
    fn a_proxy_cycle_is_refused_instead_of_dialling_around_it() {
        let config = sevenshell_ssh::openssh::parse("Host web\n  ProxyJump gateway\nHost gateway\n  ProxyJump web\n");
        let resolved = config.resolve("web");
        let error = proxy_hops(&config, &resolved, "deploy").expect_err("a loop");

        assert_eq!(error, ProxyChainError::Loop("gateway -> web -> gateway".to_owned()));
    }

    #[test]
    fn a_proxy_command_on_a_hop_is_still_refused() {
        let config =
            sevenshell_ssh::openssh::parse("Host web\n  ProxyJump gateway\nHost gateway\n  ProxyCommand nc %h %p\n");
        let resolved = config.resolve("web");
        let error = proxy_hops(&config, &resolved, "deploy").expect_err("a command");

        assert_eq!(error, ProxyChainError::Directive("ProxyCommand".to_owned()));
    }

    #[test]
    fn a_hop_that_is_a_literal_address_uses_the_default_port() {
        let config = sevenshell_ssh::openssh::parse("Host web\n  ProxyJump 10.1.1.1\n");
        let resolved = config.resolve("web");
        let hops = proxy_hops(&config, &resolved, "deploy").expect("a route");

        assert_eq!(hops[0].host, "10.1.1.1");
        assert_eq!(hops[0].port, 22);
        assert_eq!(hops[0].config, None);
    }

    #[test]
    fn a_none_value_means_the_configuration_names_no_route() {
        let config = sevenshell_ssh::openssh::parse("Host web\n  ProxyJump none\n");
        let resolved = config.resolve("web");
        assert!(proxy_hops(&config, &resolved, "deploy").expect("no route").is_empty());
        assert_eq!(hosts_from(&config)[0].proxy_jump, None);
    }

    #[test]
    fn wildcard_defaults_do_not_become_aliases() {
        let config = sevenshell_ssh::openssh::parse("Host *\n  User ops\nHost db-*\n  Port 5432\n");
        assert!(hosts_from(&config).is_empty());
    }
}
