//! The command line the bridge is started with, and where its endpoint file comes from.
//!
//! Three inputs decide the endpoint file, and this is their whole order: `--endpoint-file`, then
//! `SEVENSHELL_ENDPOINT_FILE`, then `mcp-endpoint.json` in this installation's configuration
//! directory. The environment variable is not a convenience: an MCP client's configuration is a
//! JSON fragment rather than a shell command line, so setting a key in its `env` map is the only
//! way such a fragment can point the bridge somewhere without knowing the platform's layout — and
//! that is exactly what the snippet the desktop shows does.
//!
//! Parsing lives here rather than in `main` so the precedence can be tested by calling it. A `main`
//! that reads the process environment directly can only be tested by spawning a child, which is a
//! test of the operating system as much as of this code.

use std::ffi::OsString;
use std::path::PathBuf;

use sevenshell_api::ENDPOINT_FILE_NAME;

/// The variable a client's configuration can set instead of passing `--endpoint-file`.
pub const ENDPOINT_FILE_ENV: &str = "SEVENSHELL_ENDPOINT_FILE";

/// The name `--version` prints.
const PROGRAM: &str = "sshell-mcp";

/// What the arguments asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Serve { endpoint_file: Option<PathBuf> },
    Help,
    Version,
}

impl Command {
    /// Reads the arguments after the program name.
    ///
    /// An error is a sentence for stderr. The caller prints it, prints the usage and exits 2 — the
    /// code a supervisor reads as "this launch was configured wrong", which is not the same thing
    /// as a session that ran and ended.
    pub fn parse<I>(arguments: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = OsString>,
    {
        let mut endpoint_file = None;
        let mut arguments = arguments.into_iter();

        while let Some(argument) = arguments.next() {
            let Some(text) = argument.to_str() else {
                return Err("an argument is not valid UTF-8".to_owned());
            };
            match text {
                "--help" | "-h" => return Ok(Self::Help),
                "--version" | "-V" => return Ok(Self::Version),
                "--endpoint-file" => {
                    let Some(path) = arguments.next().filter(|path| !path.is_empty()) else {
                        return Err("--endpoint-file needs a path".to_owned());
                    };
                    endpoint_file = Some(PathBuf::from(path));
                }
                _ if text.starts_with("--endpoint-file=") => {
                    let path = &text["--endpoint-file=".len()..];
                    if path.is_empty() {
                        return Err("--endpoint-file needs a path".to_owned());
                    }
                    endpoint_file = Some(PathBuf::from(path));
                }
                other => return Err(format!("unknown argument `{other}`")),
            }
        }

        Ok(Self::Serve { endpoint_file })
    }
}

/// The path the command line named or the environment named, before the search for a default.
///
/// An empty environment variable reads as unset: a client configuration that expanded to nothing is
/// not a request for a file named `""`, and looking for one would turn a configuration mistake into
/// a confusing "not available" from the desktop instead of "not configured".
pub fn endpoint_file_override(
    command_line: Option<PathBuf>,
    environment: &dyn Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    command_line.or_else(|| {
        environment(ENDPOINT_FILE_ENV)
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
    })
}

/// The usage text. Printed to stdout when it was asked for, to stderr when it follows an error.
pub fn usage() -> String {
    format!(
        "usage: {PROGRAM} [--endpoint-file <path>]\n\
         \n\
         Speaks the Model Context Protocol on standard input and output and forwards tool\n\
         calls to the 7Shell desktop's loopback control endpoint. The desktop is never started\n\
         by this program: without it, the tool list is still answered and a call says why not.\n\
         \n\
         Options:\n\
         \x20 --endpoint-file <path>  The file the desktop writes when it starts listening.\n\
         \x20                         Defaults to ${ENDPOINT_FILE_ENV}, then to\n\
         \x20                         {ENDPOINT_FILE_NAME} in this machine's configuration directory.\n\
         \x20 -h, --help              Print this and exit.\n\
         \x20 -V, --version           Print the version and exit.\n"
    )
}

/// The version line.
pub fn version() -> String {
    format!("{PROGRAM} {}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Command, String> {
        Command::parse(arguments.iter().map(OsString::from))
    }

    fn environment(variable: Option<&str>) -> impl Fn(&str) -> Option<OsString> {
        let variable = variable.map(OsString::from);
        move |name| {
            if name == ENDPOINT_FILE_ENV {
                variable.clone()
            } else {
                None
            }
        }
    }

    #[test]
    fn no_arguments_means_serve_with_the_default_endpoint() {
        assert_eq!(parse(&[]), Ok(Command::Serve { endpoint_file: None }));
    }

    #[test]
    fn the_endpoint_file_is_accepted_with_a_space_or_an_equals_sign() {
        assert_eq!(
            parse(&["--endpoint-file", "somewhere/else.json"]),
            Ok(Command::Serve {
                endpoint_file: Some(PathBuf::from("somewhere/else.json"))
            })
        );
        assert_eq!(
            parse(&["--endpoint-file=somewhere/else.json"]),
            Ok(Command::Serve {
                endpoint_file: Some(PathBuf::from("somewhere/else.json"))
            })
        );
    }

    #[test]
    fn help_and_version_are_their_own_commands() {
        assert_eq!(parse(&["--help"]), Ok(Command::Help));
        assert_eq!(parse(&["-h"]), Ok(Command::Help));
        assert_eq!(parse(&["--version"]), Ok(Command::Version));
        assert_eq!(parse(&["-V"]), Ok(Command::Version));
    }

    #[test]
    fn an_unknown_argument_is_named_in_the_refusal() {
        let message = parse(&["--verbose"]).expect_err("an unknown argument is refused");
        assert!(message.contains("--verbose"), "{message}");
    }

    #[test]
    fn an_endpoint_file_without_a_value_is_refused_rather_than_read_from_the_environment() {
        assert!(parse(&["--endpoint-file"]).is_err());
        assert!(parse(&["--endpoint-file="]).is_err());
    }

    #[test]
    fn the_command_line_wins_over_the_environment() {
        let chosen = endpoint_file_override(
            Some(PathBuf::from("from-command-line.json")),
            &environment(Some("from-environment.json")),
        );
        assert_eq!(chosen, Some(PathBuf::from("from-command-line.json")));
    }

    #[test]
    fn the_environment_is_used_when_the_command_line_names_nothing() {
        let chosen = endpoint_file_override(None, &environment(Some("from-environment.json")));
        assert_eq!(chosen, Some(PathBuf::from("from-environment.json")));
    }

    #[test]
    fn an_empty_environment_variable_reads_as_unset() {
        assert_eq!(endpoint_file_override(None, &environment(Some(""))), None);
        assert_eq!(endpoint_file_override(None, &environment(None)), None);
    }

    #[test]
    fn the_usage_names_both_ways_of_moving_the_endpoint_file() {
        let usage = usage();
        assert!(usage.contains("--endpoint-file"), "{usage}");
        assert!(usage.contains(ENDPOINT_FILE_ENV), "{usage}");
        assert!(usage.contains(ENDPOINT_FILE_NAME), "{usage}");
    }

    #[test]
    fn the_version_line_names_the_program_and_this_build() {
        let version = version();
        assert!(version.starts_with(PROGRAM), "{version}");
        assert!(version.contains(env!("CARGO_PKG_VERSION")), "{version}");
    }
}
