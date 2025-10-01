use crate::socket::{MioListener, MioSocket};

use std::io;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::ops::{Deref, DerefMut};

pub struct Socket {
    inner: TcpStream,
}

pub struct Listener {
    inner: TcpListener,
}

pub fn bind<A: Into<SocketAddr>>(addr: A) -> Result<Listener, io::Error> {
    let listener = TcpListener::bind(addr.into()).map(Listener::new)?;

    Ok(listener)
}

pub fn connect<A: Into<SocketAddr>>(addr: A) -> Result<Socket, io::Error> {
    let socket = TcpStream::connect(addr.into()).map(Socket::new)?;

    Ok(socket)
}

impl Listener {
    fn new(inner: TcpListener) -> Self {
        Listener { inner }
    }

    pub fn accept(&self) -> Result<Socket, io::Error> {
        let socket = self.inner.accept().map(|(s, _)| Socket::new(s))?;

        Ok(socket)
    }
}

impl Socket {
    fn new(inner: TcpStream) -> Self {
        Socket { inner }
    }
}
impl Deref for Socket {
    type Target = TcpStream;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for Socket {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl Write for Socket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl Read for Socket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        std::io::Read::read(&mut self.inner, buf)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        std::io::Read::read_exact(&mut self.inner, buf)
    }
}

impl From<Socket> for MioSocket {
    fn from(value: Socket) -> Self {
        value
            .inner
            .set_nonblocking(true)
            .expect("Failed to set non-blocking");

        MioSocket {
            inner: mio::net::TcpStream::from_std(value.inner),
        }
    }
}

impl From<Listener> for MioListener {
    fn from(value: Listener) -> Self {
        value
            .inner
            .set_nonblocking(true)
            .expect("Failed to set non-blocking");

        MioListener {
            inner: mio::net::TcpListener::from_std(value.inner),
        }
    }
}

pub struct WriteHalf {
    inner: Socket,
}

pub struct ReadHalf {
    inner: Socket,
}

impl Read for ReadHalf {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Write for WriteHalf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

pub(super) fn split(socket: Socket) -> (WriteHalf, ReadHalf) {
    let new_socket = socket.inner.try_clone().expect("Failed to split socket");

    (
        WriteHalf {
            inner: Socket { inner: new_socket },
        },
        ReadHalf { inner: socket },
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
