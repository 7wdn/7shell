//! In-memory stream shims that let the pull-based state machines run inside a push-based stream.
//!
//! `Receiver` and `Sender` read with `AsyncRead` and answer with `AsyncWrite`, but a terminal hands
//! its output over as chunks pushed at this layer. These two adapters bridge the directions without
//! spawning a task and without owning the terminal: the queue turns pushes into reads, and the
//! capture buffer turns the state machine's answers into bytes the caller writes into the
//! terminal's input.
//!
//! The one thing worth remembering: an empty queue answers `Pending`, not end of stream. A terminal
//! stream is almost never at its end when a chunk boundary falls in the middle of a frame, and
//! reporting `Ok(0)` there would make the protocol believe the peer hung up between two blocks.

use std::collections::VecDeque;
use std::io;
use std::mem;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Locks without letting a poisoned lock take the terminal down with it. Nothing in this module
/// calls back into user code while the lock is held, so poisoning should be impossible; recovering
/// rather than panicking is still the honest choice for a byte buffer.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Bytes pushed by the terminal and pulled by a state machine. Cloning shares the same buffer.
#[derive(Clone, Default)]
pub(crate) struct ByteQueue {
    inner: Arc<Mutex<VecDeque<u8>>>,
}

impl ByteQueue {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&self, bytes: &[u8]) {
        locked(&self.inner).extend(bytes);
    }

    /// Removes everything that is still queued, in order.
    pub(crate) fn take(&self) -> Vec<u8> {
        mem::take(&mut *locked(&self.inner)).into_iter().collect()
    }
}

/// The reading half of [`ByteQueue`], shaped for `tokio::io::AsyncRead`.
pub(crate) struct QueueReader {
    queue: ByteQueue,
}

impl QueueReader {
    pub(crate) fn new(queue: ByteQueue) -> Self {
        Self { queue }
    }
}

impl AsyncRead for QueueReader {
    fn poll_read(self: Pin<&mut Self>, _context: &mut Context<'_>, buffer: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let mut queue = locked(&this.queue.inner);
        if queue.is_empty() {
            // No bytes yet, but the stream is still open. The detour that drives this future is
            // re-polled after every push, so registering a waker would only ever wake a future
            // nobody is polling.
            return Poll::Pending;
        }
        let mut chunk = [0u8; 4096];
        let take = queue.len().min(buffer.remaining()).min(chunk.len());
        for slot in chunk.iter_mut().take(take) {
            *slot = queue.pop_front().expect("the byte count was just measured");
        }
        buffer.put_slice(&chunk[..take]);
        Poll::Ready(Ok(()))
    }
}

/// Protocol bytes a state machine wrote, waiting to be carried into the terminal's input.
#[derive(Clone, Default)]
pub(crate) struct ReplyBuffer {
    inner: Arc<Mutex<Vec<u8>>>,
}

impl ReplyBuffer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Removes everything written since the last call, in order.
    pub(crate) fn take(&self) -> Vec<u8> {
        mem::take(&mut *locked(&self.inner))
    }
}

/// The writing half of [`ReplyBuffer`], shaped for `tokio::io::AsyncWrite`.
pub(crate) struct ReplyWriter {
    buffer: ReplyBuffer,
}

impl ReplyWriter {
    pub(crate) fn new(buffer: ReplyBuffer) -> Self {
        Self { buffer }
    }
}

impl AsyncWrite for ReplyWriter {
    fn poll_write(self: Pin<&mut Self>, _context: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>> {
        // Writing into memory always takes everything, so the protocol's flush-after-every-frame
        // costs nothing and nothing can deadlock waiting for a buffer to fill.
        locked(&self.get_mut().buffer.inner).extend_from_slice(bytes);
        Poll::Ready(Ok(bytes.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
