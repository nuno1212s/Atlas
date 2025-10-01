use futures::{AsyncRead, AsyncWrite};
use std::io;

use std::net::SocketAddr;

use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

pub struct Socket {
    inner: Compat<TcpStream>,
}

pub struct Listener {
    inner: TcpListener,
}

pub async fn bind<A: Into<SocketAddr>>(addr: A) -> Result<Listener, io::Error> {
    let listener = TcpListener::bind(addr.into()).await.map(Listener::new)?;

    Ok(listener)
}

pub async fn connect<A: Into<SocketAddr>>(addr: A) -> Result<Socket, io::Error> {
    let socket = TcpStream::connect(addr.into())
        .await
        .map(|s| Socket::new(s.compat()))?;

    Ok(socket)
}

impl Listener {
    fn new(inner: TcpListener) -> Self {
        Listener { inner }
    }

    pub async fn accept(&self) -> Result<Socket, io::Error> {
        let socket = self
            .inner
            .accept()
            .await
            .map(|(s, _)| Socket::new(s.compat()))?;

        Ok(socket)
    }
}

impl Socket {
    fn new(inner: Compat<TcpStream>) -> Self {
        Socket { inner }
    }
}

impl AsyncRead for Socket {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for Socket {
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

/// The write half of a socket
/// We utilize the OwnedWriteHalf instead of
/// [`tokio::io::split`] as https://users.rust-lang.org/t/why-i-have-to-use-tokio-tcpstream-split-for-concurrent-read-writes/47755/3
/// suggests it is more efficient and does not require a mutex
pub struct WriteHalf {
    inner: Compat<OwnedWriteHalf>,
}

/// The read half of a socket
pub struct ReadHalf {
    inner: Compat<OwnedReadHalf>,
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

pub(super) fn split_socket(sock: Socket) -> (WriteHalf, ReadHalf) {
    let (read, write) = sock.inner.into_inner().into_split();

    (
        WriteHalf {
            inner: write.compat_write(),
        },
        ReadHalf {
            inner: read.compat(),
        },
    )
}

#[cfg(windows)]
mod sys {
    compile_error!("Sorry Windows users! Switch to the `async-std` socket backend.");
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
