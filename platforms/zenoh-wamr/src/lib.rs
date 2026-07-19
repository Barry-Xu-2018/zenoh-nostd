//! Zenoh platform for WAMR (WebAssembly Micro Runtime), target `wasm32-wasip1`.
//!
//! Provides the pieces the browser (`zenoh-wasm`) build gets for free from the
//! JS host, rebuilt on standard WASI primitives instead:
//!
//! - [`time`] — an `embassy-time` driver over `clock_time_get`.
//! - [`rng`] — entropy over WASI `random_get`.
//! - [`executor`] — a single-threaded executor + `poll_oneoff` reactor.
//! - [`sock`] — async non-blocking TCP over WAMR's WASI socket extension.
//! - the `ZLinkManager` (later).

pub mod executor;
pub mod reactor;
pub mod rng;
pub mod sock;
pub mod time;
pub mod wasi;
