//! Reaching a target through another connection, and answering a local port from one.
//!
//! These are the same primitive twice. A channel opened on an established session behaves like a
//! socket — it is a byte stream to a `host:port` as seen from the far side — so a second SSH
//! connection can run over it, and a listener on this machine can feed bytes into it. That is all
//! a jump host is, and all `ssh -L` is.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use sevenshell_error::ShellError;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::SshSession;

/// A stream to somewhere: a socket, or a channel inside a connection.
pub trait TunnelStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> TunnelStream for T {}

/// A stream whose concrete type a chain of hops makes unknowable at the call site.
pub type BoxedStream = Box<dyn TunnelStream>;

/// How a connection reaches its target.
#[derive(Default)]
pub enum Route {
    /// Straight to the target.
    #[default]
    Direct,
    /// Through an established session, which may itself have been reached through another one.
    Through {
        hop: Arc<SshSession>,
        host: String,
        port: u16,
    },
}

impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Direct => f.write_str("Route::Direct"),
            Self::Through { host, port, .. } => write!(f, "Route::Through({host}:{port})"),
        }
    }
}

impl Route {
    /// The sessions this route depends on, in the order they were opened.
    ///
    /// A connection opened over a channel lives exactly as long as the connection that carries it,
    /// so the caller has to hold on to the hops. This is how it knows what to hold.
    pub fn upstream(&self) -> Vec<Arc<SshSession>> {
        match self {
            Self::Direct => Vec::new(),
            Self::Through { hop, .. } => {
                let mut chain = vec![Arc::clone(hop)];
                chain.extend(hop.upstream.iter().cloned());
                chain
            }
        }
    }

    /// What the route is, for a message a person reads.
    pub fn describe(&self) -> Option<String> {
        match self {
            Self::Direct => None,
            Self::Through { host, port, .. } => Some(format!("{host}:{port}")),
        }
    }
}

/// A local port answered from the far side of a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalForward {
    pub bind_host: String,
    /// Zero asks the system for a free port, which is what a test or a temporary tunnel wants.
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
}

/// A forward that is up.
///
/// Dropping this stops nothing: the listener runs on a task of its own, so the only way out is
/// [`TunnelHandle::stop`], and the only way to be sure is to await it.
pub struct TunnelHandle {
    /// What is actually listening. Worth reading back: the caller may have asked for port zero.
    pub listener: SocketAddr,
    connections: Arc<AtomicU64>,
    failures: Arc<AtomicU64>,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl TunnelHandle {
    /// Connections that were carried to the far side.
    pub fn connections(&self) -> u64 {
        self.connections.load(Ordering::Relaxed)
    }

    /// Connections that could not be carried: the far side refused, or the session ended.
    ///
    /// Counted rather than logged, because it is the one number that explains "the tunnel is up
    /// but nothing works", and a library has no business writing to somebody's log.
    pub fn failures(&self) -> u64 {
        self.failures.load(Ordering::Relaxed)
    }

    /// Stops the listener and waits for it to finish.
    pub async fn stop(self) {
        let _ = self.stop.send(());
        let _ = self.task.await;
    }
}

impl SshSession {
    /// Opens a channel to `host:port` as seen from the far side, as a plain byte stream.
    pub async fn open_tunnel_stream(&self, host: &str, port: u16) -> Result<BoxedStream, ShellError> {
        let channel = self.open_direct_tcpip_channel(host, port).await?;
        Ok(Box::new(channel.into_stream()))
    }

    /// Listens on `forward.bind_host:forward.bind_port` and answers every connection from
    /// `forward.target_host:forward.target_port` on the far side of this session.
    pub async fn forward_local(self: &Arc<Self>, forward: LocalForward) -> Result<TunnelHandle, ShellError> {
        let listener = TcpListener::bind((forward.bind_host.as_str(), forward.bind_port))
            .await
            .map_err(|error| {
                ShellError::app_internal(format!(
                    "the forward could not listen on {}:{}: {error}",
                    forward.bind_host, forward.bind_port
                ))
            })?;

        let bound = listener
            .local_addr()
            .map_err(|error| ShellError::app_internal(format!("the listening address could not be read: {error}")))?;

        let connections = Arc::new(AtomicU64::new(0));
        let failures = Arc::new(AtomicU64::new(0));
        let (stop, mut stopped) = oneshot::channel();
        let session = Arc::clone(self);

        let counting = Arc::clone(&connections);
        let failing = Arc::clone(&failures);
        let target = forward.clone();

        let task = tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    _ = &mut stopped => break,
                    accepted = listener.accept() => accepted,
                };

                let Ok((mut local, _peer)) = accepted else {
                    // The listener itself is gone; nothing is left to wait for.
                    break;
                };

                let session = Arc::clone(&session);
                let counting = Arc::clone(&counting);
                let failing = Arc::clone(&failing);
                let target = target.clone();

                // Each connection is its own task, so one slow transfer cannot hold up the next
                // connection. A failure is counted: a tunnel that is up but carrying nothing is
                // exactly the state somebody needs explained.
                tokio::spawn(async move {
                    match session
                        .open_tunnel_stream(&target.target_host, target.target_port)
                        .await
                    {
                        Ok(mut remote) => {
                            counting.fetch_add(1, Ordering::Relaxed);
                            if tokio::io::copy_bidirectional(&mut local, &mut remote).await.is_err() {
                                failing.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Err(_) => {
                            failing.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
            }
        });

        Ok(TunnelHandle {
            listener: bound,
            connections,
            failures,
            stop,
            task,
        })
    }

    /// Listens on `forward.bind_host:forward.bind_port` and speaks SOCKS5: every connection says
    /// where it wants to go, and that destination is reached from the far side of this session.
    ///
    /// The listener, the counting and the stopping are the same as a local forward's — a dynamic
    /// forward is one whose destination is decided per connection rather than saved with it, and
    /// nothing else about it is different. Where it does differ is the order of the answers: the
    /// client is told "granted" only after the channel exists, so a destination that cannot be
    /// reached is refused with a reason instead of a connection that dies on first use.
    pub async fn forward_dynamic(self: &Arc<Self>, forward: LocalForward) -> Result<TunnelHandle, ShellError> {
        let listener = TcpListener::bind((forward.bind_host.as_str(), forward.bind_port))
            .await
            .map_err(|error| {
                ShellError::app_internal(format!(
                    "the forward could not listen on {}:{}: {error}",
                    forward.bind_host, forward.bind_port
                ))
            })?;

        let bound = listener
            .local_addr()
            .map_err(|error| ShellError::app_internal(format!("the listening address could not be read: {error}")))?;

        let connections = Arc::new(AtomicU64::new(0));
        let failures = Arc::new(AtomicU64::new(0));
        let (stop, mut stopped) = oneshot::channel();
        let session = Arc::clone(self);

        let counting = Arc::clone(&connections);
        let failing = Arc::clone(&failures);

        let task = tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    _ = &mut stopped => break,
                    accepted = listener.accept() => accepted,
                };

                let Ok((local, _peer)) = accepted else {
                    break;
                };

                let session = Arc::clone(&session);
                let counting = Arc::clone(&counting);
                let failing = Arc::clone(&failing);

                tokio::spawn(async move {
                    let mut local = local;
                    let request = match crate::socks5::read_request(&mut local).await {
                        Ok(request) => request,
                        Err(_) => {
                            // The refusal has already been written by the handshake.
                            failing.fetch_add(1, Ordering::Relaxed);
                            return;
                        }
                    };

                    let crate::socks5::Request::Connect { host, port } = request;
                    match session.open_tunnel_stream(&host, port).await {
                        Ok(mut remote) => {
                            if crate::socks5::grant(&mut local).await.is_err() {
                                // The client gave up between asking and being told.
                                return;
                            }
                            counting.fetch_add(1, Ordering::Relaxed);
                            if tokio::io::copy_bidirectional(&mut local, &mut remote).await.is_err() {
                                failing.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Err(_) => {
                            failing.fetch_add(1, Ordering::Relaxed);
                            crate::socks5::refuse(&mut local, crate::socks5::Refusal::Unreachable).await;
                        }
                    }
                });
            }
        });

        Ok(TunnelHandle {
            listener: bound,
            connections,
            failures,
            stop,
            task,
        })
    }
}
