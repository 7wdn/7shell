//! The key agent running on this machine, asked to sign instead of holding a key here.
//!
//! An agent is why somebody can have ten servers and no private key on disk: the key never leaves
//! the agent's process, and what this crate does with it is ask for a signature over the exchange
//! the server sent. Nothing here reads key material, because there is none to read — which is also
//! why agent authentication is offered at all.
//!
//! Two ways in, and the difference is which agent is running rather than which platform this is:
//! the environment names one (`$SSH_AUTH_SOCK`), and on Windows OpenSSH's agent is reached through
//! a named pipe when nothing did. Pageant speaks a different protocol entirely and is answered as
//! unsupported, by name, rather than being tried through the wrong door.

use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use sevenshell_error::ShellError;

use crate::route::BoxedStream;

/// An agent that answered, and the identities it holds.
pub(crate) struct Agent {
    client: AgentClient<BoxedStream>,
}

impl Agent {
    /// Asks the agent for its identities. An agent holding none is not an error here: it is an
    /// answer, and the caller decides what to say about it.
    pub(crate) async fn identities(&mut self) -> Result<Vec<AgentIdentity>, ShellError> {
        self.client.request_identities().await.map_err(|error| {
            ShellError::ssh_auth_failed("the key agent", 0, "agent")
                .with_detail(format!("the agent could not be asked for its keys: {error}"))
        })
    }

    pub(crate) fn client(&mut self) -> &mut AgentClient<BoxedStream> {
        &mut self.client
    }
}

/// The public keys the machine's agent holds, as `authorized_keys` lines.
///
/// Public by design: it is what an agent is for — the keys are public, the signing is not — and it
/// is the one thing a caller needs to answer "what would this agent authenticate with". Nothing
/// here can sign; the signature happens inside the connection that needs it.
pub async fn agent_public_keys() -> Result<Vec<String>, ShellError> {
    let mut agent = connect().await?;
    let identities = agent.identities().await?;
    Ok(identities
        .iter()
        .filter_map(|identity| identity.public_key().to_openssh().ok())
        .collect())
}

/// Connects to the agent the environment names, or to OpenSSH's on Windows.
pub(crate) async fn connect() -> Result<Agent, ShellError> {
    let stream = environment_agent_stream().await?;
    Ok(Agent {
        client: AgentClient::connect(stream),
    })
}

/// The stream to the agent, or why there is none.
async fn environment_agent_stream() -> Result<BoxedStream, ShellError> {
    // `$SSH_AUTH_SOCK` first on every platform: a setup that names an agent means that one, and on
    // Windows that is also how a forwarded agent arrives.
    if let Some(path) = std::env::var_os("SSH_AUTH_SOCK").filter(|value| !value.is_empty()) {
        return open_socket(std::path::Path::new(&path)).await;
    }

    #[cfg(windows)]
    {
        // OpenSSH's agent, which is what Windows ships and what the ssh command line uses when
        // nothing else is configured.
        let pipe = r"\.\pipe\openssh-ssh-agent";
        let stream = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(pipe)
            .map_err(|error| no_agent(&format!("the OpenSSH agent's pipe could not be opened: {error}")))?;
        Ok(Box::new(stream))
    }

    #[cfg(unix)]
    {
        Err(no_agent("SSH_AUTH_SOCK is not set"))
    }
}

#[cfg(unix)]
async fn open_socket(path: &std::path::Path) -> Result<BoxedStream, ShellError> {
    let stream = tokio::net::UnixStream::connect(path)
        .await
        .map_err(|error| no_agent(&format!("{} could not be reached: {error}", path.display())))?;
    Ok(Box::new(stream))
}

#[cfg(windows)]
async fn open_socket(path: &std::path::Path) -> Result<BoxedStream, ShellError> {
    // A Windows build can still be pointed at a socket by a forwarded agent, and the path there is
    // a named pipe rather than a file.
    let stream = tokio::net::windows::named_pipe::ClientOptions::new()
        .open(path.as_os_str())
        .map_err(|error| no_agent(&format!("{} could not be reached: {error}", path.display())))?;
    Ok(Box::new(stream))
}

/// No agent to ask, said as a failure of the authentication rather than of the application: the
/// server never saw an attempt, and the person has a thing to start.
fn no_agent(detail: &str) -> ShellError {
    ShellError::ssh_auth_failed("the key agent", 0, "agent").with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A minimal agent: it answers `REQUEST_IDENTITIES` and nothing else.
    ///
    /// Written to be *wrong* if this crate reads the answer wrongly, which is the point: the
    /// signature half of the protocol is the library's and is tested there, while the half this
    /// crate owns is turning an agent's answer into the keys a caller can see.
    async fn fake_agent(mut stream: tokio::net::TcpStream, key: &russh::keys::PublicKey) {
        let request = read_message(&mut stream).await;
        assert_eq!(request.first(), Some(&11), "SSH_AGENTC_REQUEST_IDENTITIES");

        let blob = key.to_bytes().expect("the key encodes");
        let mut reply = vec![12u8]; // SSH_AGENT_IDENTITIES_ANSWER
        reply.extend_from_slice(&1u32.to_be_bytes());
        reply.extend_from_slice(&(blob.len() as u32).to_be_bytes());
        reply.extend_from_slice(&blob);
        reply.extend_from_slice(&4u32.to_be_bytes());
        reply.extend_from_slice(b"test");

        write_message(&mut stream, &reply).await;
    }

    /// The agent protocol's framing: a length, then the message. Written out here rather than
    /// assumed, because getting it wrong is exactly what this test is for.
    async fn read_message(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
        let mut length = [0u8; 4];
        stream.read_exact(&mut length).await.expect("the length arrives");
        let mut payload = vec![0u8; u32::from_be_bytes(length) as usize];
        stream.read_exact(&mut payload).await.expect("the message arrives");
        payload
    }

    async fn write_message(stream: &mut tokio::net::TcpStream, payload: &[u8]) {
        let mut framed = (payload.len() as u32).to_be_bytes().to_vec();
        framed.extend_from_slice(payload);
        stream.write_all(&framed).await.expect("the answer is written");
        stream.flush().await.expect("and flushed");
    }

    /// A listener the test can connect to, and the agent serving it.
    async fn listening_agent() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        // A fixed key rather than a generated one: the test is about reading an answer, and a key
        // that differs every run would make a failure harder to reproduce, not easier.
        let key = russh::keys::PublicKey::from(russh::keys::ssh_key::public::Ed25519PublicKey([7u8; 32]));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let address = listener.local_addr().expect("the address");
        let task = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.expect("a connection");
            fake_agent(stream, &key).await;
        });
        (address, task)
    }

    #[tokio::test]
    async fn an_agents_answer_becomes_the_keys_it_holds() {
        let (address, serving) = listening_agent().await;
        let stream = tokio::net::TcpStream::connect(address).await.expect("connect");

        let mut agent = Agent {
            client: AgentClient::connect(Box::new(stream)),
        };

        let identities = agent.identities().await.expect("the agent answers");

        assert_eq!(identities.len(), 1);
        let key = identities[0].public_key();
        assert_eq!(key.algorithm(), russh::keys::Algorithm::Ed25519);
        assert_eq!(identities[0].comment(), "test");
        serving.await.expect("the agent finished");
    }

    #[tokio::test]
    async fn an_agent_that_holds_nothing_answers_with_nothing() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let address = listener.local_addr().expect("the address");
        let serving = tokio::spawn(async move {
            let (mut stream, _peer) = listener.accept().await.expect("a connection");
            let request = read_message(&mut stream).await;
            assert_eq!(request.first(), Some(&11), "SSH_AGENTC_REQUEST_IDENTITIES");

            let mut reply = vec![12u8];
            reply.extend_from_slice(&0u32.to_be_bytes());
            write_message(&mut stream, &reply).await;
        });

        let stream = tokio::net::TcpStream::connect(address).await.expect("connect");
        let mut agent = Agent {
            client: AgentClient::connect(Box::new(stream)),
        };

        assert!(
            agent.identities().await.expect("an answer").is_empty(),
            "an agent holding no keys is an answer, not an error"
        );
        serving.await.expect("the agent finished");
    }
}
