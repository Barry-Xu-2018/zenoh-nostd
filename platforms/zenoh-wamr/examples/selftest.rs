//! Standalone sanity check for the WAMR time driver + rng, runnable under iwasm:
//!
//!   cargo build --target wasm32-wasip1 --example selftest
//!   iwasm target/wasm32-wasip1/debug/examples/selftest.wasm
//!
//! Prints `SELFTEST-OK` on success, or `SELFTEST-FAIL ...` and a nonzero exit.
//! It drives a real `embassy_time::Timer` future by hand (busy-waiting on the
//! monotonic clock) so it validates schedule_wake + next_expiration without
//! needing the executor, which is built in the next step.

use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use embassy_time::{Duration, Instant, Timer};
use zenoh_wamr::{rng, time};

// A no-op waker: we poll in a busy loop, so we never actually need to be woken.
const VTABLE: RawWakerVTable = RawWakerVTable::new(
    |_| RawWaker::new(core::ptr::null(), &VTABLE),
    |_| {},
    |_| {},
    |_| {},
);
fn noop_waker() -> Waker {
    unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VTABLE)) }
}

fn fail(msg: &str) -> ! {
    println!("SELFTEST-FAIL {msg}");
    std::process::exit(1);
}

fn main() {
    // --- rng: entropy is live and varies ---
    let a = rng::random_u64();
    let b = rng::random_u64();
    println!("rng: a={a:#018x} b={b:#018x}");
    if a == 0 && b == 0 {
        fail("rng returned all zeros");
    }
    if a == b {
        fail("rng returned identical values twice");
    }

    // --- monotonic clock advances ---
    let t0 = time::monotonic_nanos();
    let mut spin = 0u64;
    while time::monotonic_nanos() == t0 {
        spin += 1;
        if spin > 100_000_000 {
            fail("monotonic clock did not advance");
        }
    }
    let t1 = time::monotonic_nanos();
    println!("clock: t0={t0} t1={t1} (advanced by {} ns)", t1 - t0);

    // --- embassy_time::Instant uses our driver's now() ---
    let i0 = Instant::now();
    let i1 = Instant::now();
    if i1 < i0 {
        fail("Instant::now went backwards");
    }
    println!("instant: i0={} i1={} ticks", i0.as_ticks(), i1.as_ticks());

    // --- drive a real Timer future to completion ---
    // Timer::after registers a deadline via schedule_wake; we poll it, confirm
    // it is Pending with a future deadline reported by next_timer_expiration,
    // busy-wait past that deadline, then confirm it resolves Ready.
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    let target = Duration::from_millis(50);
    let start = Instant::now();
    let mut timer = pin!(Timer::after(target));

    match timer.as_mut().poll(&mut cx) {
        Poll::Ready(()) => fail("Timer resolved immediately (before its deadline)"),
        Poll::Pending => {}
    }

    let next = time::next_timer_expiration();
    if next == u64::MAX {
        fail("no timer deadline registered after polling Timer");
    }
    println!("timer: registered deadline at {next} ticks");

    // Busy-wait past the deadline, pumping the queue so the waker path runs.
    loop {
        let _ = time::next_timer_expiration();
        match timer.as_mut().poll(&mut cx) {
            Poll::Ready(()) => break,
            Poll::Pending => {}
        }
        if Instant::now().duration_since(start) > Duration::from_secs(5) {
            fail("Timer never resolved within 5s");
        }
    }

    let elapsed = Instant::now().duration_since(start);
    println!("timer: resolved after {} ms", elapsed.as_millis());
    if elapsed < target {
        fail("Timer resolved before its deadline elapsed");
    }

    println!("SELFTEST-OK time driver + rng work on WAMR");
}
