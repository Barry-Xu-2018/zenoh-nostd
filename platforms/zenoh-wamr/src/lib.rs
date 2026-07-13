//! Zenoh platform for WAMR (WebAssembly Micro Runtime), target `wasm32-wasip1`.
//!
//! Provides the pieces the browser (`zenoh-wasm`) build gets for free from the
//! JS host, rebuilt on standard WASI primitives instead:
//!
//! - [`time`] — an `embassy-time` driver over `clock_time_get`.
//! - [`rng`]  — entropy over WASI `random_get`.
//! - [`executor`] — (later) a single-threaded executor + `poll_oneoff` reactor.
//! - the `ZLinkManager` (later) — TCP over WAMR's WASI socket extension.

pub mod rng;
pub mod time;
pub mod wasi;
