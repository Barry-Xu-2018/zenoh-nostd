//! Zenoh platform for WAMR (WebAssembly Micro Runtime), target `wasm32-wasip1`.
//!
//! Provides the pieces the browser (`zenoh-wasm`) build gets for free from the
//! JS host, rebuilt on standard WASI primitives instead:
//!
//! - [`time`] — an `embassy-time` driver over `clock_time_get`.
//! - [`rng`] — entropy over WASI `random_get`.
//! - [`executor`] — a single-threaded executor + `poll_oneoff` reactor.
//! - [`sock`] — async non-blocking TCP over WAMR's WASI socket extension.
//! - [`WamrLinkManager`] — the `ZLinkManager` zenoh sessions plug into.

use std::net::{IpAddr, SocketAddr};

use zenoh_nostd::platform::*;

pub mod executor;
pub mod reactor;
pub mod rng;
pub mod sock;
pub mod tcp;
pub mod time;
pub mod wasi;

/// Default MTU: room for IP/TCP headers on a streamed zenoh link. Matches the
/// browser-wasm choice (`u16::MAX / 2`).
const DEFAULT_MTU: u16 = u16::MAX / 2;

/// Link manager for the WAMR platform. Only outbound TCP is supported.
pub struct WamrLinkManager;

#[derive(ZLinkInfo, ZLinkTx, ZLinkRx, ZLink)]
#[zenoh(ZLink = (WamrLinkTx<'link>, WamrLinkRx<'link>))]
pub enum WamrLink {
    Tcp(tcp::WamrTcpLink),
}

#[derive(ZLinkInfo, ZLinkTx)]
pub enum WamrLinkTx<'link> {
    Tcp(tcp::WamrTcpLinkTx<'link>),
}

#[derive(ZLinkInfo, ZLinkRx)]
pub enum WamrLinkRx<'link> {
    Tcp(tcp::WamrTcpLinkRx<'link>),
}

impl ZLinkManager for WamrLinkManager {
    type Link<'a>
        = WamrLink
    where
        Self: 'a;

    async fn connect(
        &self,
        endpoint: Endpoint<'_>,
    ) -> core::result::Result<Self::Link<'_>, LinkError> {
        let protocol = endpoint.protocol();
        let address = endpoint.address();

        match protocol.as_str() {
            "tcp" => {
                let dst = SocketAddr::try_from(address)?;
                let (octets, port) = match dst.ip() {
                    IpAddr::V4(v4) => (v4.octets(), dst.port()),
                    IpAddr::V6(_) => {
                        // WAMR socket spike only verified IPv4 layouts.
                        return Err(LinkError::CouldNotConnect);
                    }
                };
                let stream = sock::WamrTcp::connect(octets, port).map_err(|_| LinkError::CouldNotConnect)?;
                Ok(WamrLink::Tcp(tcp::WamrTcpLink::new(stream, DEFAULT_MTU)))
            }
            _ => zenoh::zbail!(LinkError::CouldNotParseProtocol),
        }
    }

    async fn listen(&self, _: Endpoint<'_>) -> core::result::Result<Self::Link<'_>, LinkError> {
        // WAMR guest apps make outbound connections to a router/peer.
        zenoh::zbail!(LinkError::CouldNotListen)
    }
}
