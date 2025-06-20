use nix::errno::Errno;
use std::str::FromStr;
use std::{
    fmt, fs, io,
    net::{AddrParseError, IpAddr, SocketAddr, ToSocketAddrs},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    pin::Pin,
    str,
    task::*,
    time::Duration,
};

use log::*;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufStream, ReadBuf},
    net::{TcpListener, TcpStream, UnixListener, UnixStream},
    time::timeout,
};

/// Unify behavior of tcp & unix socket listener
pub enum UnifyListener {
    Tcp(TcpListener),
    Unix(UnixListener),
}

/// Unify behavior of tcp & unix ddr
pub enum UnifyAddr {
    Socket(SocketAddr),
    Path(PathBuf),
}

impl std::fmt::Display for UnifyAddr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Socket(s) => write!(f, "{}", s),
            Self::Path(p) => write!(f, "{}", p.display()),
        }
    }
}
impl std::fmt::Debug for UnifyAddr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Socket(s) => write!(f, "{}", s),
            Self::Path(p) => write!(f, "{}", p.display()),
        }
    }
}
impl std::clone::Clone for UnifyAddr {
    fn clone(&self) -> Self {
        match self {
            Self::Socket(s) => UnifyAddr::Socket(s.clone()),
            Self::Path(p) => UnifyAddr::Path(p.clone()),
        }
    }
}
impl std::str::FromStr for UnifyAddr {
    type Err = AddrParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.as_bytes()[0] as char == '/' {
            return Ok(Self::Path(PathBuf::from(s)));
        }
        match s.parse::<SocketAddr>() {
            Ok(a) => Ok(Self::Socket(a)),
            // Can't directly resolve the IP, try to resolve it through the domain name.
            // If multiple IP addresses are resolved, only the first result is taken
            Err(e) => match s.to_socket_addrs() {
                Ok(mut _v) => match _v.next() {
                    Some(a) => Ok(Self::Socket(a)),
                    None => Err(e),
                },
                Err(_) => Err(e),
            },
        }
    }
}

impl std::cmp::PartialEq<str> for UnifyAddr {
    fn eq(&self, other: &str) -> bool {
        match self {
            Self::Socket(s) => {
                match other.parse::<SocketAddr>() {
                    Ok(addr) => *s == addr,
                    Err(_) => {
                        // compatibility case: ‘other’ is IpAddr
                        match other.parse::<IpAddr>() {
                            Ok(addr) => s.ip() == addr,
                            Err(_) => false,
                        }
                    }
                }
            }
            Self::Path(p) => p == Path::new(other),
        }
    }
}

const ZERO_TIME: Duration = Duration::from_secs(0);

/// Unify behavior of tcp & unix stream
pub enum UnifyStream {
    Tcp(TcpStream),
    Unix(UnixStream),
}

impl UnifyListener {
    pub async fn bind(addr: &UnifyAddr) -> Result<Self, io::Error> {
        match addr {
            UnifyAddr::Socket(_addr) => match TcpListener::bind(_addr).await {
                Ok(l) => Ok(UnifyListener::Tcp(l)),
                Err(e) => Err(e),
            },
            UnifyAddr::Path(path) => {
                if path.exists() {
                    fs::remove_file(path)?;
                }
                let path_dup_str = format!("{}{}", path.to_str().unwrap(), "_dup");
                let path_dup = Path::new(&path_dup_str);
                if path_dup.exists() {
                    fs::remove_file(&path_dup)?;
                }
                match UnixListener::bind(&path_dup) {
                    Ok(l) => {
                        if let Err(e) = fs::hard_link(path_dup, path) {
                            error!(
                                "hard_link {:?}->{:?} error: {:?}",
                                path_dup.to_str(),
                                path.to_str(),
                                e
                            );
                            return Err(e);
                        }
                        if let Err(e) = fs::set_permissions(path, fs::Permissions::from_mode(0o666))
                        {
                            error!("cannot get metadata of {:?}: {:?}", path.to_str(), e);
                        }
                        Ok(UnifyListener::Unix(l))
                    }
                    Err(e) => Err(e),
                }
            }
        }
    }

    #[inline]
    pub async fn accept(&mut self) -> Result<UnifyStream, io::Error> {
        match self {
            UnifyListener::Tcp(l) => match l.accept().await {
                Ok((tcpsock, _)) => return Ok(UnifyStream::Tcp(tcpsock)),
                Err(e) => return Err(e),
            },
            UnifyListener::Unix(l) => match l.accept().await {
                Ok((unixsock, _)) => return Ok(UnifyStream::Unix(unixsock)),
                Err(e) => return Err(e),
            },
        }
    }

    #[inline]
    pub fn poll_accept(
        &self, ctx: &mut Context,
    ) -> Poll<std::io::Result<(UnifyStream, UnifyAddr)>> {
        match self {
            UnifyListener::Tcp(l) => {
                if let Poll::Ready(r) = l.poll_accept(ctx) {
                    match r {
                        Ok((socker, addr)) => {
                            return Poll::Ready(Ok((
                                UnifyStream::Tcp(socker),
                                UnifyAddr::Socket(addr),
                            )));
                        }
                        Err(e) => return Poll::Ready(Err(e)),
                    }
                }
            }
            UnifyListener::Unix(l) => {
                if let Poll::Ready(r) = l.poll_accept(ctx) {
                    match r {
                        Ok((socker, addr)) => {
                            let addr = UnifyAddr::Path(
                                addr.as_pathname().unwrap_or(Path::new("")).to_path_buf(),
                            );
                            return Poll::Ready(Ok((UnifyStream::Unix(socker), addr)));
                        }
                        Err(e) => return Poll::Ready(Err(e)),
                    }
                }
            }
        }
        Poll::Pending
    }
}

impl std::fmt::Display for UnifyListener {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Tcp(l) => match l.local_addr() {
                Ok(addr) => {
                    return write!(f, "listener {}", addr);
                }
                Err(_) => {
                    return write!(f, "tcp listener unknown");
                }
            },
            Self::Unix(l) => match l.local_addr() {
                Ok(addr) => {
                    return write!(f, "listener {}", addr.as_pathname().unwrap().display());
                }
                Err(_) => {
                    return write!(f, "unix listener unknown");
                }
            },
        }
    }
}

impl UnifyStream {
    #[inline(always)]
    pub async fn connect(addr: &UnifyAddr) -> Result<Self, io::Error> {
        match addr {
            UnifyAddr::Socket(_addr) => match TcpStream::connect(_addr).await {
                Ok(stream) => Ok(UnifyStream::Tcp(stream)),
                Err(e) => Err(e),
            },
            UnifyAddr::Path(path) => match UnixStream::connect(path).await {
                Ok(stream) => Ok(UnifyStream::Unix(stream)),
                Err(e) => Err(e),
            },
        }
    }

    #[inline(always)]
    pub async fn connect_timeout(
        addr: &UnifyAddr, connect_timeout: Duration,
    ) -> Result<Self, io::Error> {
        if connect_timeout == ZERO_TIME {
            UnifyStream::connect(addr).await
        } else {
            match addr {
                UnifyAddr::Socket(_addr) => {
                    match timeout(connect_timeout, TcpStream::connect(_addr)).await {
                        Ok(connect_result) => match connect_result {
                            Ok(stream) => Ok(UnifyStream::Tcp(stream)),
                            Err(e) => Err(e),
                        },
                        Err(e) => Err(e.into()),
                    }
                }
                UnifyAddr::Path(path) => {
                    match timeout(connect_timeout, UnixStream::connect(path)).await {
                        Ok(connect_result) => match connect_result {
                            Ok(stream) => Ok(UnifyStream::Unix(stream)),
                            Err(e) => Err(e),
                        },
                        Err(e) => Err(e.into()),
                    }
                }
            }
        }
    }

    pub async fn close(&mut self) -> std::io::Result<()> {
        match self {
            UnifyStream::Tcp(l) => l.shutdown().await,
            UnifyStream::Unix(l) => l.shutdown().await,
        }
    }

    #[inline(always)]
    pub async fn read_exact_timeout(
        &mut self, dst: &mut [u8], read_timeout: Duration,
    ) -> Result<usize, io::Error> {
        if read_timeout == ZERO_TIME {
            return self.read_exact(dst).await;
        } else {
            match timeout(read_timeout, self.read_exact(dst)).await {
                Ok(o) => match o {
                    Ok(l) => return Ok(l),
                    Err(e2) => return Err(e2),
                },
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
    }

    #[inline(always)]
    pub async fn write_timeout(
        &mut self, dst: &[u8], write_timeout: Duration,
    ) -> Result<(), io::Error> {
        if write_timeout == ZERO_TIME {
            return self.write_all(dst).await;
        } else {
            match timeout(write_timeout, self.write_all(dst)).await {
                Ok(o) => match o {
                    Ok(_) => return Ok(()),
                    Err(e2) => return Err(e2),
                },
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
    }

    #[inline(always)]
    pub async fn flush_timeout(&mut self, write_timeout: Duration) -> Result<(), io::Error> {
        if write_timeout == ZERO_TIME {
            return self.flush().await;
        } else {
            match timeout(write_timeout, self.flush()).await {
                Ok(o) => match o {
                    Ok(_) => return Ok(()),
                    Err(e2) => return Err(e2),
                },
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
    }
}

impl std::fmt::Display for UnifyStream {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Tcp(l) => match l.local_addr() {
                Ok(addr) => {
                    return write!(f, "listener {}", addr);
                }
                Err(_) => {
                    return write!(f, "tcp listener unknown");
                }
            },
            Self::Unix(l) => match l.local_addr() {
                Ok(addr) => {
                    return write!(f, "listener {}", addr.as_pathname().unwrap().display());
                }
                Err(_) => {
                    return write!(f, "unix listener unknown");
                }
            },
        }
    }
}

impl AsyncRead for UnifyStream {
    #[inline(always)]
    fn poll_read(
        self: Pin<&mut Self>, cx: &mut Context, buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match Pin::get_mut(self) {
            UnifyStream::Tcp(l) => {
                return Pin::new(l).poll_read(cx, buf);
            }
            UnifyStream::Unix(l) => {
                return Pin::new(l).poll_read(cx, buf);
            }
        }
    }
}

impl AsyncWrite for UnifyStream {
    #[inline(always)]
    fn poll_write(
        self: Pin<&mut Self>, cx: &mut Context, buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        match Pin::get_mut(self) {
            UnifyStream::Tcp(l) => {
                return Pin::new(l).poll_write(cx, buf);
            }
            UnifyStream::Unix(l) => {
                return Pin::new(l).poll_write(cx, buf);
            }
        }
    }

    #[inline(always)]
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), io::Error>> {
        match Pin::get_mut(self) {
            UnifyStream::Tcp(l) => {
                return Pin::new(l).poll_flush(cx);
            }
            UnifyStream::Unix(l) => {
                return Pin::new(l).poll_flush(cx);
            }
        }
    }

    #[inline(always)]
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), io::Error>> {
        match Pin::get_mut(self) {
            UnifyStream::Tcp(l) => {
                return Pin::new(l).poll_shutdown(cx);
            }
            UnifyStream::Unix(l) => {
                return Pin::new(l).poll_shutdown(cx);
            }
        }
    }
}

/// Buffered IO for UnifyStream
pub struct UnifyBufStream {
    reader_buf_size: usize,
    writer_buf_size: usize,
    buf_stream: BufStream<UnifyStream>,
}

impl UnifyBufStream {
    #[inline(always)]
    pub fn new(unify_stream: UnifyStream) -> Self {
        Self {
            reader_buf_size: 8 * 1024,
            writer_buf_size: 8 * 1024,
            buf_stream: BufStream::with_capacity(8 * 1024, 8 * 1024, unify_stream),
        }
    }

    #[inline(always)]
    pub fn with_capacity(
        reader_buf_size: usize, writer_buf_size: usize, unify_stream: UnifyStream,
    ) -> Self {
        Self {
            reader_buf_size,
            writer_buf_size,
            buf_stream: BufStream::with_capacity(reader_buf_size, writer_buf_size, unify_stream),
        }
    }

    #[inline(always)]
    pub async fn close(&mut self) -> std::io::Result<()> {
        self.buf_stream.shutdown().await
    }

    #[inline(always)]
    pub async fn read_exact(&mut self, dst: &mut [u8]) -> Result<usize, io::Error> {
        self.buf_stream.read_exact(dst).await
    }

    #[inline(always)]
    pub async fn read_exact_timeout(
        &mut self, dst: &mut [u8], read_timeout: Duration,
    ) -> Result<usize, io::Error> {
        if read_timeout == ZERO_TIME {
            return self.buf_stream.read_exact(dst).await;
        } else {
            match timeout(read_timeout, self.buf_stream.read_exact(dst)).await {
                Ok(o) => match o {
                    Ok(l) => return Ok(l),
                    Err(e2) => return Err(e2),
                },
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
    }

    #[inline(always)]
    pub async fn write_all(&mut self, dst: &[u8]) -> Result<(), io::Error> {
        self.buf_stream.write_all(dst).await
    }

    #[inline(always)]
    pub async fn write_timeout(
        &mut self, dst: &[u8], write_timeout: Duration,
    ) -> Result<(), io::Error> {
        if write_timeout == ZERO_TIME {
            return self.buf_stream.write_all(dst).await;
        } else {
            match timeout(write_timeout, self.buf_stream.write_all(dst)).await {
                Ok(o) => match o {
                    Ok(_) => return Ok(()),
                    Err(e2) => return Err(e2),
                },
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
    }

    #[inline(always)]
    pub async fn flush_timeout(&mut self, write_timeout: Duration) -> Result<(), io::Error> {
        if write_timeout == ZERO_TIME {
            return self.buf_stream.flush().await;
        } else {
            match timeout(write_timeout, self.buf_stream.flush()).await {
                Ok(o) => match o {
                    Ok(_) => return Ok(()),
                    Err(e2) => return Err(e2),
                },
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
    }
}

impl std::fmt::Display for UnifyBufStream {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        return write!(
            f,
            "UnifyBufStream reader_buf_size:{} writer_buf_size:{}, stream:{:#}",
            self.reader_buf_size,
            self.writer_buf_size,
            self.buf_stream.get_ref()
        );
    }
}

pub async fn listen_on_addr(addr: &str) -> std::io::Result<UnifyListener> {
    match UnifyAddr::from_str(addr) {
        Err(_) => {
            error!("Fail to parse addr {:?}", addr);
            return Err(Errno::EFAULT.into());
        }
        Ok(listen_addr) => match UnifyListener::bind(&listen_addr).await {
            Ok(listener) => {
                info!("listen on {:?}", addr);
                return Ok(listener);
            }
            Err(e) => {
                error!("Fail to bind on addr {:?}: {:?}", listen_addr, e);
                return Err(e);
            }
        },
    }
}

#[cfg(test)]
mod tests {

    #[allow(unused_imports)]
    use std::str::FromStr;

    use super::*;

    #[test]
    fn test_addr_compare() {
        let uaddr_unify = UnifyAddr::from_str("127.0.0.1:18555").expect("parse address error");
        let uaddr_with_port = "127.0.0.1:18555";
        let uaddr_without_port = "127.0.0.1";
        let uaddr_wrong_ip_without_port = "127.0.0.2";
        let uaddr_wrong_ip_with_port = "127.0.0.2:18555";
        let uaddr_wrong_port = "127.0.0.1:18888";

        assert!(uaddr_unify == *uaddr_with_port);
        assert!(uaddr_unify == *uaddr_without_port);
        assert!(uaddr_unify != *uaddr_wrong_ip_without_port);
        assert!(uaddr_unify != *uaddr_wrong_ip_with_port);
        assert!(uaddr_unify != *uaddr_wrong_port);
    }
}
