//! `embassy-time` driver backed by WASI.
//!
//! `embassy_time::{Instant, Timer, Duration}` need a global [`Driver`] that can
//! answer "what time is it" and "wake this waker at time T". We provide both on
//! top of WASI:
//!
//! - `now()` reads the monotonic clock via `clock_time_get(CLOCKID_MONOTONIC)`.
//! - `schedule_wake()` records the waker in a small timer queue. The queue is
//!   drained by [`next_timer_expiration`], which the executor calls each turn to
//!   learn the next deadline (so it can arm a `poll_oneoff` clock subscription)
//!   and to wake any timers that have already expired.
//!
//! `TICK_HZ` is configured to 1_000_000 (see Cargo.toml), i.e. one tick == one
//! microsecond, so converting the nanosecond WASI clock is a divide-by-1000.

use core::task::Waker;
use std::sync::Mutex;

use embassy_time_queue_utils::Queue;

use crate::wasi;

/// Nanoseconds per embassy tick (TICK_HZ = 1_000_000 => 1 tick = 1000 ns).
const NANOS_PER_TICK: u64 = 1_000_000_000 / embassy_time_driver::TICK_HZ;

/// Read the WASI monotonic clock, in nanoseconds since an arbitrary epoch.
pub fn monotonic_nanos() -> u64 {
    let mut ns: u64 = 0;
    // precision 0 = "give me the best you have". Errors are not expected for a
    // monotonic clock; fall back to 0 to honour the Driver contract (never fail).
    let e = unsafe { wasi::clock_time_get(wasi::CLOCKID_MONOTONIC, 0, &mut ns) };
    if e != wasi::ERRNO_SUCCESS { 0 } else { ns }
}

/// The global timer queue. A `std::sync::Mutex` gives us the `Sync` that the
/// `Driver` trait demands; on single-threaded WASM it is never contended.
static QUEUE: Mutex<Queue> = Mutex::new(Queue::new());

struct WamrTimeDriver;

impl embassy_time_driver::Driver for WamrTimeDriver {
    fn now(&self) -> u64 {
        monotonic_nanos() / NANOS_PER_TICK
    }

    fn schedule_wake(&self, at: u64, waker: &Waker) {
        // If the deadline is already in the past this returns true and the timer
        // is dequeued+woken on the next `next_expiration` (the executor polls it
        // every turn), so no wake is lost.
        QUEUE.lock().unwrap().schedule_wake(at, waker);
    }
}

embassy_time_driver::time_driver_impl!(static DRIVER: WamrTimeDriver = WamrTimeDriver);

/// Wake every timer whose deadline has passed and return the next deadline (in
/// embassy ticks), or `u64::MAX` if no timers are pending.
///
/// The executor calls this each turn to (a) fire due timers and (b) decide how
/// long to sleep in `poll_oneoff`.
pub fn next_timer_expiration() -> u64 {
    let now = monotonic_nanos() / NANOS_PER_TICK;
    QUEUE.lock().unwrap().next_expiration(now)
}

/// Convert an embassy tick deadline into an absolute nanosecond timestamp on the
/// WASI monotonic clock (for arming a `poll_oneoff` clock subscription).
pub fn ticks_to_nanos(ticks: u64) -> u64 {
    ticks.saturating_mul(NANOS_PER_TICK)
}
