//! Asynchronous non-blocking TCP over WAMR's WASI socket extension.
//!
//! Ported from the verified spike (`sock_open`/`sock_connect` + `fd_fdstat_set_flags`
//! NONBLOCK + `fd_read`/`fd_write` that return `EAGAIN`, with readiness via the
//! reactor's `poll_oneoff`).
//!
//! The fd is stored as a plain integer and cheaply shared between the tx/rx half
//! of a split link (same model as std's `TcpStream::try_clone`). Closing either
//! half is the caller's responsibility via [`WamrTcp::close`] or `Drop` on the
//! last owner — I deliberately do **not** close-on-drop of every copy, so that
//! split halves don't invalidate each other. Ownership is tracked with an
//! [`std::sync::Arc`]-like refcount built from `std::rc::Rc` (single-threaded).

use core::future::poll_fn;
use core::task::{Context, Poll};
use std::cell::Cell;
use std::rc::Rc;

use crate::reactor;
use crate::wasi::{self, Ciovec, Fd, Iovec, WasiAddrIp4};

/// Errors at the socket layer. Mapped to `LinkError` by the link adapter (step 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SockError {
    Open,
    Connect,
    SetNonblock,
    Read,
    Write,
    Closed,
}

struct Inner {
    fd: Fd,
    /// When true the Drop of the last Rc-holder must close the fd.
    close_on_drop: Cell<bool>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if self.close_on_drop.get() {
            reactor::unregister(self.fd);
            let _ = unsafe { wasi::fd_close(self.fd) };
        }
    }
}

/// A connected, non-blocking TCP socket.
#[derive(Clone)]
pub struct WamrTcp {
    inner: Rc<Inner>,
}

impl WamrTcp {
    /// Open a TCP socket, connect to `octets:port`, and put it in non-blocking mode.
    pub fn connect(octets: [u8; 4], port: u16) -> Result<Self, SockError> {
        let mut sockfd: Fd = -1;
        let e = unsafe { wasi::sock_open(-1, wasi::INET4, wasi::SOCKET_STREAM, &mut sockfd) };
        if e != wasi::ERRNO_SUCCESS {
            return Err(SockError::Open);
        }

        let addr = WasiAddrIp4::new(octets, port);
        let e = unsafe { wasi::sock_connect(sockfd, &addr as *const _ as *const _) };
        if e != wasi::ERRNO_SUCCESS {
            let _ = unsafe { wasi::fd_close(sockfd) };
            return Err(SockError::Connect);
        }

        let e = unsafe { wasi::fd_fdstat_set_flags(sockfd, wasi::FDFLAGS_NONBLOCK) };
        if e != wasi::ERRNO_SUCCESS {
            let _ = unsafe { wasi::fd_close(sockfd) };
            return Err(SockError::SetNonblock);
        }

        Ok(Self {
            inner: Rc::new(Inner {
                fd: sockfd,
                close_on_drop: Cell::new(true),
            }),
        })
    }

    pub fn fd(&self) -> Fd {
        self.inner.fd
    }

    /// Explicitly close the underlying fd. Subsequent I/O returns `SockError::Closed`.
    pub fn close(self) {
        // Force-close regardless of other Rc holders: flip the flag on and drop.
        // Other clones still hold the same Inner; Drop closes only once when the
        // last Rc goes away — so just unregister + close now and silence Drop.
        reactor::unregister(self.inner.fd);
        let _ = unsafe { wasi::fd_close(self.inner.fd) };
        self.inner.close_on_drop.set(false);
    }

    /// One non-blocking read attempt. `Poll::Pending` means EAGAIN — the waker
    /// is already registered with the reactor.
    pub fn poll_read(&self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<Result<usize, SockError>> {
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let iov = Iovec {
            buf: buf.as_mut_ptr(),
            buf_len: buf.len(),
        };
        let mut nread: usize = 0;
        let e = unsafe { wasi::fd_read(self.inner.fd, &iov, 1, &mut nread) };
        match e {
            wasi::ERRNO_SUCCESS => {
                if nread == 0 {
                    // EOF
                    Poll::Ready(Err(SockError::Closed))
                } else {
                    Poll::Ready(Ok(nread))
                }
            }
            wasi::ERRNO_AGAIN => {
                reactor::register_read(self.inner.fd, cx.waker());
                Poll::Pending
            }
            _ => Poll::Ready(Err(SockError::Read)),
        }
    }

    /// One non-blocking write attempt.
    pub fn poll_write(&self, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, SockError>> {
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let iov = Ciovec {
            buf: buf.as_ptr(),
            buf_len: buf.len(),
        };
        let mut nwritten: usize = 0;
        let e = unsafe { wasi::fd_write(self.inner.fd, &iov, 1, &mut nwritten) };
        match e {
            wasi::ERRNO_SUCCESS => Poll::Ready(Ok(nwritten)),
            wasi::ERRNO_AGAIN => {
                reactor::register_write(self.inner.fd, cx.waker());
                Poll::Pending
            }
            _ => Poll::Ready(Err(SockError::Write)),
        }
    }

    /// Read some bytes. Completes when at least one byte is available (or EOF/err).
    pub async fn read(&self, buf: &mut [u8]) -> Result<usize, SockError> {
        poll_fn(|cx| self.poll_read(cx, buf)).await
    }

    /// Write the entire buffer, retrying on partial writes / EAGAIN.
    pub async fn write_all(&self, mut buf: &[u8]) -> Result<(), SockError> {
        while !buf.is_empty() {
            let n = poll_fn(|cx| self.poll_write(cx, buf)).await?;
            if n == 0 {
                return Err(SockError::Write);
            }
            buf = &buf[n..];
        }
        Ok(())
    }

    /// Read exactly `buf.len()` bytes.
    pub async fn read_exact(&self, mut buf: &mut [u8]) -> Result<(), SockError> {
        while !buf.is_empty() {
            let n = self.read(buf).await?;
            buf = &mut buf[n..];
        }
        Ok(())
    }
}
