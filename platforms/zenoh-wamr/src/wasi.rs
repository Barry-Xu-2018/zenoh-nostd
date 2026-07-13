//! Raw WASI + WAMR socket-extension FFI bindings.
//!
//! These target the `wasi_snapshot_preview1` import module as implemented by
//! WAMR (WebAssembly Micro Runtime). Socket creation/connection use WAMR's
//! socket extension (`sock_open` / `sock_connect`); byte transfer uses the
//! standard preview1 `fd_read` / `fd_write`; readiness is driven by
//! `poll_oneoff`. All verified against iwasm 2.4.5.
//!
//! Signatures and struct layouts confirmed empirically:
//! - `__wasi_addr_t` (IPv4): `kind: i32 = 0` @0, `n0..n3: u8` @4-7,
//!   `port: u16` (host byte order) @8, total size 12.
//! - non-blocking `fd_read` returns `EAGAIN` (6) when no data is ready.

#![allow(dead_code)]

use core::ffi::c_void;

pub type Errno = u16;
pub type Fd = i32;

// errno values we care about (preview1)
pub const ERRNO_SUCCESS: Errno = 0;
pub const ERRNO_AGAIN: Errno = 6;

// address family / socket type (WAMR socket extension)
pub const INET4: i32 = 0;
pub const INET6: i32 = 1;
pub const SOCKET_STREAM: i32 = 1;
pub const SOCKET_DGRAM: i32 = 0;

// __wasi_fdflags_t
pub const FDFLAGS_NONBLOCK: u16 = 0x0004;

// __wasi_eventtype_t
pub const EVENTTYPE_CLOCK: u8 = 0;
pub const EVENTTYPE_FD_READ: u8 = 1;
pub const EVENTTYPE_FD_WRITE: u8 = 2;

// __wasi_clockid_t
pub const CLOCKID_MONOTONIC: u32 = 1;

// __wasi_subclockflags_t
pub const SUBSCRIPTION_CLOCK_ABSTIME: u16 = 1;

#[repr(C)]
pub struct Ciovec {
    pub buf: *const u8,
    pub buf_len: usize,
}

#[repr(C)]
pub struct Iovec {
    pub buf: *mut u8,
    pub buf_len: usize,
}

/// `__wasi_addr_t` for the IPv4 case. WAMR reads `kind` first; for IPv4 the
/// four address octets follow, then the port in host byte order.
#[repr(C)]
pub struct WasiAddrIp4 {
    pub kind: i32,
    pub n0: u8,
    pub n1: u8,
    pub n2: u8,
    pub n3: u8,
    pub port: u16,
}

impl WasiAddrIp4 {
    pub fn new(octets: [u8; 4], port: u16) -> Self {
        Self {
            kind: 0, // IPv4
            n0: octets[0],
            n1: octets[1],
            n2: octets[2],
            n3: octets[3],
            port,
        }
    }
}

// --- poll_oneoff subscription / event structs (preview1) ---
//
// The subscription union is laid out with an 8-byte tag word followed by the
// event-specific payload. We over-size the payload to the largest variant
// (clock) so a single struct works for both fd_read and clock subscriptions.

#[repr(C)]
pub struct Subscription {
    pub userdata: u64,
    /// __wasi_eventtype_t
    pub tag: u8,
    pub _pad: [u8; 7],
    /// union payload (largest variant is the clock subscription)
    pub payload: SubscriptionPayload,
}

#[repr(C)]
pub union SubscriptionPayload {
    pub fd_readwrite: SubscriptionFdReadwrite,
    pub clock: SubscriptionClock,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SubscriptionFdReadwrite {
    pub fd: Fd,
    pub _pad: [u8; 28],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SubscriptionClock {
    pub id: u32,
    pub _pad0: [u8; 4],
    pub timeout: u64,
    pub precision: u64,
    pub flags: u16,
    pub _pad1: [u8; 6],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Event {
    pub userdata: u64,
    pub error: Errno,
    pub type_: u8,
    pub _pad: [u8; 5],
    pub fd_readwrite_nbytes: u64,
    pub fd_readwrite_flags: u16,
    pub _rest: [u8; 6],
}

impl Default for Event {
    fn default() -> Self {
        // Safety: all-zero is a valid bit pattern for these POD fields.
        unsafe { core::mem::zeroed() }
    }
}

#[link(wasm_import_module = "wasi_snapshot_preview1")]
unsafe extern "C" {
    #[link_name = "sock_open"]
    pub fn sock_open(poolfd: Fd, af: i32, socktype: i32, sockfd: *mut Fd) -> Errno;

    #[link_name = "sock_connect"]
    pub fn sock_connect(fd: Fd, addr: *const c_void) -> Errno;

    #[link_name = "fd_write"]
    pub fn fd_write(fd: Fd, iovs: *const Ciovec, iovs_len: usize, nwritten: *mut usize) -> Errno;

    #[link_name = "fd_read"]
    pub fn fd_read(fd: Fd, iovs: *const Iovec, iovs_len: usize, nread: *mut usize) -> Errno;

    #[link_name = "fd_close"]
    pub fn fd_close(fd: Fd) -> Errno;

    #[link_name = "fd_fdstat_set_flags"]
    pub fn fd_fdstat_set_flags(fd: Fd, flags: u16) -> Errno;

    #[link_name = "poll_oneoff"]
    pub fn poll_oneoff(
        subs: *const Subscription,
        events: *mut Event,
        nsubs: usize,
        nevents: *mut usize,
    ) -> Errno;

    #[link_name = "clock_time_get"]
    pub fn clock_time_get(id: u32, precision: u64, time: *mut u64) -> Errno;
}
