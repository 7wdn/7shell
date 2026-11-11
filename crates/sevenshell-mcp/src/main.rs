//! `sshell-mcp`: the program an MCP client launches.
//!
//! Standard input and output are the protocol and nothing else while a session runs. Everything
//! this program has to say about itself — usage after an error, a file it could not read, a runtime
//! that would not start — goes to stderr, because one stray line on stdout is a client parsing
//! something that is not JSON. The two exceptions are `--help` and `--version`: they are asked for
//! by a person before any session exists, and a help text a pipe cannot carry is not help.
//!
//! Nothing is launched to make a call work. The endpoint file is read, the call is posted to the
//! loopback address it names, and a desktop that is not there is reported as such — the session
//! still completes with exit code 0, because the bridge did its job and the client decides what an
//! unavailable desktop means for it.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use sevenshell_mcp::{Command, HttpToolClient, McpServer, endpoint_file_override, endpoint_path};

fn main() -> ExitCode {
    let command = match Command::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("sshell-mcp: {message}");
            eprintln!("{}", sevenshell_mcp::cli::usage());
            return ExitCode::from(2);
        }
    };

    match command {
        Command::Help => {
            println!("{}", sevenshell_mcp::cli::usage());
            ExitCode::SUCCESS
        }
        Command::Version => {
            println!("{}", sevenshell_mcp::cli::version());
            ExitCode::SUCCESS
        }
        Command::Serve { endpoint_file } => {
            // The one place the process environment is read: everything below takes the resolved
            // path, so the precedence is stated once and the paths below it stay testable.
            let endpoint_file = endpoint_file_override(endpoint_file, &|key| std::env::var_os(key));
            serve(endpoint_file)
        }
    }
}

fn serve(endpoint_file: Option<PathBuf>) -> ExitCode {
    let endpoint_file = match endpoint_path(endpoint_file) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("sshell-mcp: {}", error.message);
            return ExitCode::FAILURE;
        }
    };

    if let Err(error) = std::fs::metadata(&endpoint_file) {
        // Reported and not fatal: the file can appear when the desktop starts, and a client that is
        // already connected should get an answer to its call rather than a bridge that died before
        // the handshake. The call itself says what is missing when it is made.
        eprintln!(
            "sshell-mcp: no control endpoint file at {} ({error}); \
             a tool call will report that the desktop is not available",
            endpoint_file.display()
        );
    }

    let client = match HttpToolClient::new(endpoint_file) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("sshell-mcp: {}", error.message);
            return ExitCode::FAILURE;
        }
    };

    // The endpoint's value is the effective limit; the server's own cap stays out of the way.
    let server = Arc::new(McpServer::new(Arc::new(client), usize::MAX));

    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("sshell-mcp: the runtime could not be started: {error}");
            return ExitCode::FAILURE;
        }
    };

    runtime.block_on(async move {
        // The two standard streams are the session: it ends when the client closes its end, which is
        // how a client that has gone away stops the bridge without a signal.
        match server.serve(tokio::io::stdin(), tokio::io::stdout()).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("sshell-mcp: standard input could not be read: {error}");
                ExitCode::FAILURE
            }
        }
    })
}
