//! Just enough SOCKS5 to carry what a programme asks for.
//!
//! RFC 1928 is a big protocol; this is the part a dynamic forward needs and nothing else:
//!
//! * **No authentication.** The listener is on this machine and the client is a programme its user
//!   started. The credentials a SOCKS5 server would check are the operating system's business, and
//!   the ones that matter to the *server* are already the SSH session's.
//! * **CONNECT only.** `BIND` and `UDP ASSOCIATE` are for protocols this build does not carry, and
//!   a refusal that says so is better than a connection that quietly behaves like something else.
//! * **All three address kinds.** A name is passed through to the far side to resolve, which is the
//!   point of a tunnel: the name means what it means *there*, not here.
//!
//! Every refusal is written back before the connection closes. A client that gets a bare close
//! reports a protocol error, which tells whoever is debugging nothing at all.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// How long a client has to say what it wants. A connection that opens and then says nothing is
/// held open by the operating system, not by anybody's intentions.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// SOCKS5's version byte, in every message of the protocol.
const VERSION: u8 = 0x05;

/// What a client asked for, once it has been understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Reach this address from the far side and carry the bytes.
    Connect { host: String, port: u16 },
}

/// Why a request cannot be served. Each one has a code the client is told, because "it did not
/// work" and "that is not something I do" are different answers for whoever is configuring this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The client does not speak SOCKS5, or offers no method this server has.
    Greeting,
    /// The request was well formed but asked for something this build does not do.
    Command,
    /// The address kind is not one of the three the protocol defines.
    AddressKind,
    /// The destination could not be reached from the far side.
    Unreachable,
}

impl Refusal {
    /// The reply code the client is told.
    fn code(self) -> u8 {
        match self {
            // `0x07` is "command not supported" and covers the greeting and the request alike: both
            // mean "this is not a conversation this server has".
            Self::Greeting | Self::Command => 0x07,
            Self::AddressKind => 0x08,
            Self::Unreachable => 0x04,
        }
    }
}

/// Reads the greeting and the request, and writes back every refusal it can decide on its own.
///
/// What was asked for is understood here; whether it can be *carried* is not yet known, because
/// that needs a channel to the far side. So the answers are two moments: this call writes the
/// refusals it knows about (a stranger at the door, a method it does not have, a request it does
/// not serve), and then the caller writes exactly one more — [`grant`] when the destination was
/// reached, or [`refuse`] when it was not. A client told "granted" before the channel existed would
/// send its request into a connection that is already over.
pub async fn read_request(stream: &mut TcpStream) -> Result<Request, Refusal> {
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, converse(stream)).await {
        Ok(result) => result,
        // A client that said nothing is refused the way an unsupported request is: the connection
        // ends here either way, and this path must not hang on somebody's half-open socket.
        Err(_) => Err(Refusal::Greeting),
    }
}

/// Tells the client the destination was reached and the bytes can start.
pub async fn grant(stream: &mut TcpStream) -> std::io::Result<()> {
    write_reply(stream, 0x00).await
}

/// Tells the client why it cannot be carried.
pub async fn refuse(stream: &mut TcpStream, refusal: Refusal) {
    // Best effort on purpose: the connection is being given up, and a client that has already gone
    // away is not a reason to say anything else.
    let _ = write_reply(stream, refusal.code()).await;
}

async fn converse(stream: &mut TcpStream) -> Result<Request, Refusal> {
    read_greeting(stream).await?;
    let request = read_request_bytes(stream).await;
    if let Err(refusal) = &request {
        refuse(stream, *refusal).await;
    }
    request
}

/// `VER NMETHODS METHODS…`, answered with the one method this server has: none.
async fn read_greeting(stream: &mut TcpStream) -> Result<(), Refusal> {
    let mut head = [0u8; 2];
    stream.read_exact(&mut head).await.map_err(|_| Refusal::Greeting)?;
    if head[0] != VERSION {
        return Err(Refusal::Greeting);
    }

    let mut methods = vec![0u8; usize::from(head[1])];
    stream.read_exact(&mut methods).await.map_err(|_| Refusal::Greeting)?;
    if !methods.contains(&0x00) {
        // `0xFF` is "no method acceptable": the client is told rather than left guessing.
        let _ = stream.write_all(&[VERSION, 0xFF]).await;
        return Err(Refusal::Greeting);
    }

    stream.write_all(&[VERSION, 0x00]).await.map_err(|_| Refusal::Greeting)
}

/// `VER CMD RSV ATYP ADDR PORT`, of which this build serves `CMD = CONNECT`.
async fn read_request_bytes(stream: &mut TcpStream) -> Result<Request, Refusal> {
    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await.map_err(|_| Refusal::Command)?;
    if head[0] != VERSION || head[2] != 0x00 {
        return Err(Refusal::Command);
    }
    if head[1] != 0x01 {
        return Err(Refusal::Command);
    }

    let host = read_address(stream, head[3]).await?;
    let mut port = [0u8; 2];
    stream.read_exact(&mut port).await.map_err(|_| Refusal::Command)?;

    Ok(Request::Connect {
        host,
        port: u16::from_be_bytes(port),
    })
}

async fn read_address(stream: &mut TcpStream, kind: u8) -> Result<String, Refusal> {
    match kind {
        // A literal address, read out the way it would be written.
        0x01 => {
            let mut octets = [0u8; 4];
            stream.read_exact(&mut octets).await.map_err(|_| Refusal::AddressKind)?;
            Ok(std::net::Ipv4Addr::from(octets).to_string())
        }
        0x04 => {
            let mut octets = [0u8; 16];
            stream.read_exact(&mut octets).await.map_err(|_| Refusal::AddressKind)?;
            Ok(std::net::Ipv6Addr::from(octets).to_string())
        }
        // A name, handed on as a name: it is the far side's DNS that decides what it means.
        0x03 => {
            let mut length = [0u8; 1];
            stream.read_exact(&mut length).await.map_err(|_| Refusal::AddressKind)?;
            let mut name = vec![0u8; usize::from(length[0])];
            stream.read_exact(&mut name).await.map_err(|_| Refusal::AddressKind)?;
            String::from_utf8(name).map_err(|_| Refusal::AddressKind)
        }
        _ => Err(Refusal::AddressKind),
    }
}

/// `VER REP RSV ATYP BND.ADDR BND.PORT`, with a bound address of "nothing in particular".
///
/// The address is optional for a `CONNECT` reply and clients ignore it, so this sends the IPv4
/// all-zero form rather than pretending to know which interface the connection leaves from.
pub async fn write_reply(stream: &mut TcpStream, code: u8) -> std::io::Result<()> {
    stream.write_all(&[VERSION, code, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
    stream.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// Runs the handshake against a scripted client, and answers with what it concluded.
    async fn handshake_with(client_bytes: &[u8]) -> (Result<Request, Refusal>, Vec<u8>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let address = listener.local_addr().expect("the address");
        let script = client_bytes.to_vec();

        let serving = tokio::spawn(async move {
            let (mut server, _peer) = listener.accept().await.expect("a connection");
            // Refusals are written by `read_request` itself; a granted request is written here,
            // which is where the caller would write it once the tunnel exists.
            let result = read_request(&mut server).await;
            if result.is_ok() {
                grant(&mut server).await.expect("the grant is written");
            }

            // Read out whatever the client sent before closing. A socket closed with unread data in
            // its buffer is reset rather than finished, and the client would see a connection error
            // instead of the answer it was just given — which is a property of this harness, not of
            // the protocol, and would otherwise hide the answer it is here to check.
            let mut discarded = [0u8; 256];
            let _ = tokio::time::timeout(Duration::from_millis(200), async {
                while let Ok(read) = server.read(&mut discarded).await {
                    if read == 0 {
                        break;
                    }
                }
            })
            .await;

            result
        });

        let mut client = TcpStream::connect(address).await.expect("connect");
        client.write_all(&script).await.expect("the script is sent");
        client.flush().await.expect("flushed");

        let mut reply = vec![0u8; 64];
        let read = tokio::time::timeout(Duration::from_secs(5), client.read(&mut reply))
            .await
            .expect("the server answers")
            .expect("bytes");
        reply.truncate(read);

        (serving.await.expect("the server finished"), reply)
    }

    fn greeting() -> Vec<u8> {
        vec![0x05, 0x01, 0x00]
    }

    #[tokio::test]
    async fn a_connect_request_is_read_as_a_name_and_a_port() {
        let mut script = greeting();
        script.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, 0x0b]);
        script.extend_from_slice(b"example.com");
        script.extend_from_slice(&443u16.to_be_bytes());

        let (result, reply) = handshake_with(&script).await;

        assert_eq!(
            result,
            Ok(Request::Connect {
                host: "example.com".to_owned(),
                port: 443
            }),
            "a name is handed on as a name, for the far side to resolve"
        );
        // Both messages arrive together: the greeting's answer, then the request's.
        assert_eq!(
            reply,
            vec![0x05, 0x00, 0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0],
            "the greeting is accepted, then the connection is granted"
        );
    }

    #[tokio::test]
    async fn literal_addresses_are_read_in_both_families() {
        let mut ipv4 = greeting();
        ipv4.extend_from_slice(&[0x05, 0x01, 0x00, 0x01, 127, 0, 0, 1]);
        ipv4.extend_from_slice(&22u16.to_be_bytes());
        let (result, _) = handshake_with(&ipv4).await;
        assert_eq!(
            result,
            Ok(Request::Connect {
                host: "127.0.0.1".to_owned(),
                port: 22
            })
        );

        let mut ipv6 = greeting();
        ipv6.extend_from_slice(&[0x05, 0x01, 0x00, 0x04]);
        ipv6.extend_from_slice(&std::net::Ipv6Addr::LOCALHOST.octets());
        ipv6.extend_from_slice(&80u16.to_be_bytes());
        let (result, _) = handshake_with(&ipv6).await;
        assert_eq!(
            result,
            Ok(Request::Connect {
                host: "::1".to_owned(),
                port: 80
            })
        );
    }

    #[tokio::test]
    async fn a_command_this_build_does_not_carry_is_refused_as_one() {
        // `BIND` is 0x02. A client asking for it has to be told, not left with a closed socket.
        let mut script = greeting();
        script.extend_from_slice(&[0x05, 0x02, 0x00, 0x01, 127, 0, 0, 1]);
        script.extend_from_slice(&22u16.to_be_bytes());

        let (result, reply) = handshake_with(&script).await;

        assert_eq!(result, Err(Refusal::Command));
        assert_eq!(&reply[2..6], &[0x05, 0x07, 0x00, 0x01], "command not supported");
    }

    #[tokio::test]
    async fn an_unknown_address_kind_is_refused_as_one() {
        let mut script = greeting();
        script.extend_from_slice(&[0x05, 0x01, 0x00, 0x09]);

        let (result, reply) = handshake_with(&script).await;

        assert_eq!(result, Err(Refusal::AddressKind));
        assert_eq!(&reply[2..6], &[0x05, 0x08, 0x00, 0x01], "address type not supported");
    }

    #[tokio::test]
    async fn a_client_that_offers_no_method_this_server_has_is_refused() {
        let (result, reply) = handshake_with(&[0x05, 0x01, 0x02]).await;

        assert_eq!(result, Err(Refusal::Greeting));
        assert_eq!(reply, vec![0x05, 0xFF], "no acceptable methods");
    }

    #[tokio::test]
    async fn something_that_is_not_socks5_at_all_is_refused() {
        let (result, reply) = handshake_with(b"GET / HTTP/1.1\r\n\r\n").await;

        assert_eq!(result, Err(Refusal::Greeting));
        assert!(
            reply.is_empty() || reply[0] == 0x05,
            "a stranger is given no answer: there is no version to refuse in"
        );
    }
}
