//! Sockets backed by Linux `io_uring`, via the `rio` crate.
//!
//! # Why the socket owns its buffers
//!
//! `io_uring` hands a buffer to the kernel for the whole duration of an
//! operation, but [`AsyncRead::poll_read`] only lends its slice for the duration
//! of a single call, and a pending operation has to survive across polls. The
//! two cannot be reconciled directly, so each socket owns a heap staging buffer:
//! the kernel reads into / writes out of that, and the caller's slice is filled
//! by copying.
//!
//! # Why extending the completion lifetime is sound
//!
//! [`rio::Completion`]'s lifetime parameter is a pure `PhantomData` marker — the
//! struct stores no pointer to either the buffer or the file, only the ring and
//! an SQE id. It exists to stop the buffer being freed while the kernel still
//! owns it. Two invariants let us uphold that by hand instead:
//!
//! * the staging buffer is a `Vec<u8>`, so its heap address is stable even when
//!   the socket itself is moved; and
//! * `Completion::drop` blocks until the kernel has finished the operation, and
//!   in every variant below `completion` is declared *before* `buf`, so it is
//!   dropped first.

use std::future::Future;
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};

use futures::io::{AsyncRead, AsyncWrite};
use rio::{Completion, Rio, Uring};
use socket2::{Domain, Protocol, Socket as SSocket, Type};

/// Ordering of uring operations; `Link` waits for previous ops on the same
/// submission chain to finish before executing the requested one.
const ORD: rio::Ordering = rio::Ordering::Link;

/// Size of the staging buffer handed to the kernel per submission. The socket
/// halves are additionally wrapped in `BufReader`/`BufWriter` by the parent
/// module, so this only needs to be large enough to keep the ring busy.
const IO_BUFFER_SIZE: usize = 64 * 1024;

static RIO: OnceLock<Rio> = OnceLock::new();

/// Initializes the global `io_uring` instance.
pub fn init() -> Result<(), io::Error> {
    let ring = rio::new()?;

    RIO.set(ring).map_err(|_| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the io_uring instance was already initialized",
        )
    })
}

/// Releases the global `io_uring` instance.
///
/// The ring is deliberately kept for the lifetime of the process: sockets may
/// still hold in-flight completions that refer to it, and tearing it down while
/// any of them are live would be unsound. Reclaiming it early bought nothing,
/// since `init` can only be called once anyway.
pub fn drop() -> Result<(), io::Error> {
    Ok(())
}

#[inline(always)]
fn ring() -> &'static Uring {
    RIO.get().expect("Linux io_uring wasn't initialized")
}

fn new_buffer() -> Vec<u8> {
    vec![0u8; IO_BUFFER_SIZE]
}

/// See the module docs for why `completion` precedes `buf` in every variant.
enum ReadState {
    Idle(Vec<u8>),
    Pending {
        completion: Completion<'static, usize>,
        buf: Vec<u8>,
    },
    Ready {
        buf: Vec<u8>,
        filled: usize,
        consumed: usize,
    },
    /// Only reachable if a previous poll panicked while the state was taken out.
    Poisoned,
}

enum WriteState {
    Idle(Vec<u8>),
    Pending {
        completion: Completion<'static, usize>,
        buf: Vec<u8>,
    },
    Poisoned,
}

pub struct Socket {
    inner: TcpStream,
    read: ReadState,
    write: WriteState,
}

pub struct Listener {
    inner: TcpListener,
}

impl Socket {
    fn new(inner: TcpStream) -> Self {
        Socket {
            inner,
            read: ReadState::Idle(new_buffer()),
            write: WriteState::Idle(new_buffer()),
        }
    }
}

fn poisoned(direction: &'static str) -> io::Error {
    io::Error::other(format!(
        "rio socket {direction} state was poisoned by an earlier panic"
    ))
}

/// Submits a receive into `buf`, detaching the completion's lifetime.
///
/// # Safety invariants
///
/// Upheld by [`ReadState::Pending`]'s field order and by `buf` being heap
/// allocated; see the module documentation.
fn submit_recv(stream: &TcpStream, mut buf: Vec<u8>) -> ReadState {
    // rio derives the submission length from the iovec, so open the buffer back
    // up to the full staging size. Capacity survives truncation, so this does
    // not reallocate.
    buf.resize(IO_BUFFER_SIZE, 0);

    let completion = unsafe {
        std::mem::transmute::<Completion<'_, usize>, Completion<'static, usize>>(
            ring().recv_ordered(stream, &buf, ORD),
        )
    };

    ReadState::Pending { completion, buf }
}

/// Submits a send of `buf[..len]`, detaching the completion's lifetime.
///
/// # Safety invariants
///
/// As [`submit_recv`], but for [`WriteState::Pending`].
fn submit_send(stream: &TcpStream, mut buf: Vec<u8>, len: usize) -> WriteState {
    // Only the first `len` bytes are to be sent, and the submission length comes
    // from the vector, so trim it to match.
    buf.truncate(len);

    let completion = unsafe {
        std::mem::transmute::<Completion<'_, usize>, Completion<'static, usize>>(
            ring().send_ordered(stream, &buf, ORD),
        )
    };

    WriteState::Pending { completion, buf }
}

impl AsyncRead for Socket {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();

        loop {
            match std::mem::replace(&mut this.read, ReadState::Poisoned) {
                ReadState::Poisoned => return Poll::Ready(Err(poisoned("read"))),

                ReadState::Ready {
                    buf,
                    filled,
                    consumed,
                } => {
                    if consumed >= filled {
                        // Drained; fall through and submit a fresh receive.
                        this.read = ReadState::Idle(buf);
                        continue;
                    }

                    let n = std::cmp::min(filled - consumed, out.len());
                    out[..n].copy_from_slice(&buf[consumed..consumed + n]);

                    let consumed = consumed + n;
                    this.read = if consumed == filled {
                        ReadState::Idle(buf)
                    } else {
                        ReadState::Ready {
                            buf,
                            filled,
                            consumed,
                        }
                    };

                    return Poll::Ready(Ok(n));
                }

                ReadState::Idle(buf) => {
                    if out.is_empty() {
                        this.read = ReadState::Idle(buf);
                        return Poll::Ready(Ok(0));
                    }

                    this.read = submit_recv(&this.inner, buf);
                }

                ReadState::Pending {
                    mut completion,
                    buf,
                } => match Pin::new(&mut completion).poll(cx) {
                    Poll::Ready(Ok(0)) => {
                        // The peer closed the connection.
                        this.read = ReadState::Idle(buf);
                        return Poll::Ready(Ok(0));
                    }
                    Poll::Ready(Ok(filled)) => {
                        this.read = ReadState::Ready {
                            buf,
                            filled,
                            consumed: 0,
                        };
                    }
                    Poll::Ready(Err(err)) => {
                        this.read = ReadState::Idle(buf);
                        return Poll::Ready(Err(err));
                    }
                    Poll::Pending => {
                        this.read = ReadState::Pending { completion, buf };
                        return Poll::Pending;
                    }
                },
            }
        }
    }
}

impl AsyncWrite for Socket {
    /// Accepts bytes into the staging buffer and submits them. Like any buffered
    /// writer, a byte reported as written here is only queued: a failure on the
    /// underlying send surfaces on a later `poll_write` or `poll_flush`.
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();

        loop {
            match std::mem::replace(&mut this.write, WriteState::Poisoned) {
                WriteState::Poisoned => return Poll::Ready(Err(poisoned("write"))),

                WriteState::Idle(mut buf) => {
                    if data.is_empty() {
                        this.write = WriteState::Idle(buf);
                        return Poll::Ready(Ok(0));
                    }

                    // `submit_send` truncated it on the previous round.
                    buf.resize(IO_BUFFER_SIZE, 0);

                    let n = std::cmp::min(data.len(), buf.len());
                    buf[..n].copy_from_slice(&data[..n]);

                    this.write = submit_send(&this.inner, buf, n);

                    return Poll::Ready(Ok(n));
                }

                // A submission is already in flight; it has to land before the
                // buffer can be reused, otherwise the two would interleave.
                WriteState::Pending {
                    mut completion,
                    buf,
                } => match Pin::new(&mut completion).poll(cx) {
                    Poll::Ready(Ok(_)) => {
                        this.write = WriteState::Idle(buf);
                    }
                    Poll::Ready(Err(err)) => {
                        this.write = WriteState::Idle(buf);
                        return Poll::Ready(Err(err));
                    }
                    Poll::Pending => {
                        this.write = WriteState::Pending { completion, buf };
                        return Poll::Pending;
                    }
                },
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();

        match std::mem::replace(&mut this.write, WriteState::Poisoned) {
            WriteState::Poisoned => Poll::Ready(Err(poisoned("write"))),
            WriteState::Idle(buf) => {
                this.write = WriteState::Idle(buf);
                Poll::Ready(Ok(()))
            }
            WriteState::Pending {
                mut completion,
                buf,
            } => match Pin::new(&mut completion).poll(cx) {
                Poll::Ready(result) => {
                    this.write = WriteState::Idle(buf);
                    Poll::Ready(result.map(|_| ()))
                }
                Poll::Pending => {
                    this.write = WriteState::Pending { completion, buf };
                    Poll::Pending
                }
            },
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_flush(cx)
    }
}

// bind won't actually be asynchronous, but we'll only call it once
// throughout the library, anyway
pub async fn bind<A: Into<SocketAddr>>(addr: A) -> Result<Listener, io::Error> {
    let inner = TcpListener::bind(addr.into())?;

    Ok(Listener { inner })
}

pub async fn connect<A: Into<SocketAddr>>(addr: A) -> Result<Socket, io::Error> {
    let addr = addr.into();

    let domain = match addr {
        SocketAddr::V4(_) => Domain::IPV4,
        SocketAddr::V6(_) => Domain::IPV6,
    };

    let socket = SSocket::new(domain, Type::STREAM, Some(Protocol::TCP))?;

    ring().connect(&socket, &addr, ORD).await?;

    Ok(Socket::new(socket.into()))
}

impl Listener {
    pub async fn accept(&self) -> Result<Socket, io::Error> {
        let stream = ring().accept(&self.inner).await?;

        Ok(Socket::new(stream))
    }
}

/// The write half of a socket
pub struct WriteHalf {
    inner: Socket,
}

/// The read half of a socket
pub struct ReadHalf {
    inner: Socket,
}

impl AsyncWrite for WriteHalf {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_close(cx)
    }
}

impl AsyncRead for ReadHalf {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

/// `io_uring` has no equivalent of tokio's ownership split, so the descriptor is
/// duplicated instead: each half drives its own independent submissions against
/// the same connection.
pub(super) fn split_socket(sock: Socket) -> (WriteHalf, ReadHalf) {
    let read_stream = sock
        .inner
        .try_clone()
        .expect("Failed to duplicate the socket descriptor while splitting it");

    (
        WriteHalf { inner: sock },
        ReadHalf {
            inner: Socket::new(read_stream),
        },
    )
}

#[cfg(unix)]
mod sys {
    use std::os::unix::io::{AsRawFd, RawFd};

    impl AsRawFd for super::Socket {
        fn as_raw_fd(&self) -> RawFd {
            self.inner.as_raw_fd()
        }
    }

    impl AsRawFd for super::Listener {
        fn as_raw_fd(&self) -> RawFd {
            self.inner.as_raw_fd()
        }
    }
}
