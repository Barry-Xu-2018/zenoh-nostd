//! TCP link adapter: wraps [`crate::sock::WamrTcp`] in the `ZLink*` traits that
//! zenoh's transport layer expects. Mirrors `platforms/zenoh-std/src/tcp.rs`.

use core::marker::PhantomData;

use zenoh_nostd::platform::*;

use crate::sock::{SockError, WamrTcp};

fn map_err(e: SockError) -> LinkError {
    match e {
        SockError::Open | SockError::Connect | SockError::SetNonblock => LinkError::CouldNotConnect,
        SockError::Write => LinkError::LinkTxFailed,
        SockError::Read | SockError::Closed => LinkError::LinkRxFailed,
    }
}

/// Full duplex TCP link.
pub struct WamrTcpLink {
    stream: WamrTcp,
    mtu: u16,
}

impl WamrTcpLink {
    pub fn new(stream: WamrTcp, mtu: u16) -> Self {
        Self { stream, mtu }
    }
}

/// Transmit half (shares the same underlying fd via `WamrTcp`'s Rc).
///
/// The lifetime parameter exists so the top-level `WamrLinkTx<'link>` enum (as
/// required by the `ZLink` derive) can carry one. TCP halves are owned clones of
/// the socket handle, not borrows, so the lifetime is otherwise unused.
pub struct WamrTcpLinkTx<'a> {
    stream: WamrTcp,
    mtu: u16,
    _phantom: PhantomData<&'a ()>,
}

/// Receive half.
pub struct WamrTcpLinkRx<'a> {
    stream: WamrTcp,
    mtu: u16,
    _phantom: PhantomData<&'a ()>,
}

impl ZLinkInfo for WamrTcpLink {
    fn mtu(&self) -> u16 {
        self.mtu
    }
    fn is_streamed(&self) -> bool {
        true
    }
}
impl ZLinkInfo for WamrTcpLinkTx<'_> {
    fn mtu(&self) -> u16 {
        self.mtu
    }
    fn is_streamed(&self) -> bool {
        true
    }
}
impl ZLinkInfo for WamrTcpLinkRx<'_> {
    fn mtu(&self) -> u16 {
        self.mtu
    }
    fn is_streamed(&self) -> bool {
        true
    }
}

impl ZLinkTx for WamrTcpLink {
    async fn write_all(&mut self, buffer: &[u8]) -> core::result::Result<(), LinkError> {
        self.stream.write_all(buffer).await.map_err(map_err)
    }
}
impl ZLinkTx for WamrTcpLinkTx<'_> {
    async fn write_all(&mut self, buffer: &[u8]) -> core::result::Result<(), LinkError> {
        self.stream.write_all(buffer).await.map_err(map_err)
    }
}

impl ZLinkRx for WamrTcpLink {
    async fn read(&mut self, buffer: &mut [u8]) -> core::result::Result<usize, LinkError> {
        self.stream.read(buffer).await.map_err(map_err)
    }
    async fn read_exact(&mut self, buffer: &mut [u8]) -> core::result::Result<(), LinkError> {
        self.stream.read_exact(buffer).await.map_err(map_err)
    }
}
impl ZLinkRx for WamrTcpLinkRx<'_> {
    async fn read(&mut self, buffer: &mut [u8]) -> core::result::Result<usize, LinkError> {
        self.stream.read(buffer).await.map_err(map_err)
    }
    async fn read_exact(&mut self, buffer: &mut [u8]) -> core::result::Result<(), LinkError> {
        self.stream.read_exact(buffer).await.map_err(map_err)
    }
}

impl ZLink for WamrTcpLink {
    type Tx<'a> = WamrTcpLinkTx<'a>;
    type Rx<'a> = WamrTcpLinkRx<'a>;

    fn split(&mut self) -> (Self::Tx<'_>, Self::Rx<'_>) {
        (
            WamrTcpLinkTx {
                stream: self.stream.clone(),
                mtu: self.mtu,
                _phantom: PhantomData,
            },
            WamrTcpLinkRx {
                stream: self.stream.clone(),
                mtu: self.mtu,
                _phantom: PhantomData,
            },
        )
    }
}
