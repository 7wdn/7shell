//! The person's OpenSSH configuration, reduced to what a session can use.
//!
//! The parser lives in `sevenshell-ssh`; this module is the one place that knows *where* the file
//! is and turns an alias into the values the interface may see. Identity files stay here: the
//! picker gets an alias, an address, an account and a port, and never a local path.

use sevenshell_ssh::openssh::{ParsedConfig, ResolvedHost};
use sevenshell_types::SshConfigHost;

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
            if resolved.proxy_jump.is_some() {
                note(&mut unsupported, "ProxyJump");
            }
            if resolved.proxy_command.is_some() {
                note(&mut unsupported, "ProxyCommand");
            }
            SshConfigHost {
                host: resolved.address(&alias),
                user: resolved.user,
                port: resolved.port,
                unsupported,
                alias,
            }
        })
        .collect()
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
    fn a_proxy_is_named_in_the_warning_list() {
        let config = sevenshell_ssh::openssh::parse(
            "Host web\n  HostName 10.0.0.5\n  ProxyJump bastion\n  ProxyCommand ssh -W %h:%p bastion\n",
        );
        let hosts = hosts_from(&config);

        assert!(hosts[0].unsupported.iter().any(|name| name == "ProxyJump"));
        assert!(hosts[0].unsupported.iter().any(|name| name == "ProxyCommand"));
    }

    #[test]
    fn wildcard_defaults_do_not_become_aliases() {
        let config = sevenshell_ssh::openssh::parse("Host *\n  User ops\nHost db-*\n  Port 5432\n");
        assert!(hosts_from(&config).is_empty());
    }
}
