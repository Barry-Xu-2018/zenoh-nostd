//! Randomness for the WAMR platform.
//!
//! zenoh needs entropy transitively (`uhlc -> rand -> getrandom`) to generate a
//! ZenohId and HLC timestamps. On `wasm32-wasip1`, getrandom's default backend
//! calls the standard WASI `random_get`, which WAMR implements — so unlike the
//! browser build we need no `js`/`custom` backend wiring.
//!
//! This helper simply exposes that backend directly for tests / sanity checks.

/// Fill `buf` with random bytes from the WASI entropy source.
pub fn fill(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    getrandom::getrandom(buf)
}

/// Return a random `u64` (used by tests to show entropy is live).
pub fn random_u64() -> u64 {
    let mut b = [0u8; 8];
    // random_get must not fail on WAMR; if it ever did, a zero seed is still a
    // valid (if poor) value and keeps this infallible for callers.
    let _ = fill(&mut b);
    u64::from_le_bytes(b)
}
