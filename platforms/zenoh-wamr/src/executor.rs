//! Single-threaded executor for wasm32-wasip1 on WAMR.
//!
//! zenoh's API is fully async, so something has to drive `Future::poll`. Neither
//! `embassy-executor/arch-wasm` (needs the browser) nor `arch-std` (needs thread
//! park) works here. This executor is the iteration:
//!
//! ```text
//! loop {
//!     poll(root_future);
//!     if ready { break }
//!     reactor::wait();   // poll_oneoff on socket fds + timer deadline
//! }
//! ```
//!
//! Concurrentity is achieved by putting multiple futures under one root (via
//! `embassy_futures::{select,join}`), not by a multi-task spawner. That is more
//! than enough for the zenoh pub/sub examples (`session.run()` concurrent with
//! the app loop).

use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use crate::reactor;

// A no-op waker for the root future. Reads/writes and timers register *their own*
// wakers with the reactor/timer-queue; those real wakers are what get fired.
// The root poll always happens right after `reactor::wait` returns, so we never
// need the root waker itself to schedule anything.
const VTABLE: RawWakerVTable = RawWakerVTable::new(
    |_| RawWaker::new(core::ptr::null(), &VTABLE),
    |_| {},
    |_| {},
    |_| {},
);

fn noop_waker() -> Waker {
    // Safety: the vtable methods are pure no-ops and ignore the data pointer.
    unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VTABLE)) }
}

/// Drive `fut` to completion, sleeping in `poll_oneoff` whenever it is pending.
pub fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = pin!(fut);
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(out) => return out,
            Poll::Pending => reactor::wait(),
        }
    }
}
