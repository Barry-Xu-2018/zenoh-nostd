//! Reactor: tracks which socket fds have pending readers/writers, and arms
//! `poll_oneoff` subscriptions for them. The executor sleeps in
//! [`wait`] until a socket is ready or a timer deadline fires.
//!
//! Single-threaded WASM: a `std::sync::Mutex` is enough; it is never contended.

use core::task::Waker;
use std::sync::Mutex;

use crate::time::{self, ticks_to_nanos};
use crate::wasi::{self, Event, Fd, Subscription, SubscriptionClock, SubscriptionFdReadwrite, SubscriptionPayload};

/// Maximum simultaneous interest entries (one per open socket half is plenty).
const MAX_INTERESTS: usize = 16;
/// subscriptions = up to 2 per interest (read+write) + 1 clock.
const MAX_SUBS: usize = MAX_INTERESTS * 2 + 1;

struct Interest {
    fd: Fd,
    read: Option<Waker>,
    write: Option<Waker>,
}

static INTERESTS: Mutex<Vec<Interest>> = Mutex::new(Vec::new());

/// Register interest in readability on `fd`. The waker is fired when
/// `poll_oneoff` reports the fd is ready to read.
pub fn register_read(fd: Fd, waker: &Waker) {
    register(fd, true, waker);
}

/// Register interest in writability on `fd`.
pub fn register_write(fd: Fd, waker: &Waker) {
    register(fd, false, waker);
}

/// Drop all interest registrations for `fd` (called when the socket is closed).
pub fn unregister(fd: Fd) {
    let mut g = INTERESTS.lock().unwrap();
    g.retain(|i| i.fd != fd);
}

fn register(fd: Fd, read: bool, waker: &Waker) {
    let mut g = INTERESTS.lock().unwrap();
    if let Some(slot) = g.iter_mut().find(|i| i.fd == fd) {
        let dst = if read { &mut slot.read } else { &mut slot.write };
        match dst {
            Some(existing) if existing.will_wake(waker) => {}
            _ => *dst = Some(waker.clone()),
        }
        return;
    }
    if g.len() >= MAX_INTERESTS {
        // Evict the oldest interest by waking it (task will re-register).
        if let Some(old) = g.first_mut() {
            if let Some(w) = old.read.take() {
                w.wake();
            }
            if let Some(w) = old.write.take() {
                w.wake();
            }
        }
        g.remove(0);
    }
    let mut interest = Interest {
        fd,
        read: None,
        write: None,
    };
    if read {
        interest.read = Some(waker.clone());
    } else {
        interest.write = Some(waker.clone());
    }
    g.push(interest);
}

/// Fire any already-due timers, then block in `poll_oneoff` until either a
/// registered fd is ready or the next timer deadline arrives.
///
/// Returns after waking at least one fd waker, one timer waker, or on a
/// `poll_oneoff` error (caller re-polls the root future either way).
pub fn wait() {
    // Drain timers that have already expired (their wakers get fired here).
    let next_ticks = time::next_timer_expiration();

    // Snapshot current interests so I can drop the lock before the blocking call.
    let snapshot: Vec<(Fd, bool, bool)> = {
        let g = INTERESTS.lock().unwrap();
        g.iter()
            .map(|i| (i.fd, i.read.is_some(), i.write.is_some()))
            .collect()
    };

    let mut subs: [Subscription; MAX_SUBS] = unsafe { core::mem::zeroed() };
    let mut nsubs = 0usize;

    for (fd, want_read, want_write) in snapshot {
        if want_read && nsubs < MAX_SUBS {
            // userdata low bit 0 => read interest for this fd's slot-index style id.
            // I pack: high 32 bits unused, low 31 bits = fd as u32, bit 31 unused,
            // and bit 0 of userdata encodes direction: 0=read, 1=write.
            // Simpler: userdata = (fd as u64) << 1 | dir.
            subs[nsubs] = fd_sub(fd, wasi::EVENTTYPE_FD_READ, (fd as u64) << 1);
            nsubs += 1;
        }
        if want_write && nsubs < MAX_SUBS {
            subs[nsubs] = fd_sub(fd, wasi::EVENTTYPE_FD_WRITE, ((fd as u64) << 1) | 1);
            nsubs += 1;
        }
    }

    // Arm a clock subscription if a future timer is pending.
    //
    // IMPORTANT: WAMR (iwasm 2.4.x) does NOT implement absolute sleep on the
    // monotonic clock (`SUBSCRIPTION_CLOCK_ABSTIME` → "Unimplemented absolute
    // sleep on monotonic clock" and returns immediately → busy spin). Always
    // use a REATIVE timeout = max(0, deadline - now).
    if next_ticks != u64::MAX && nsubs < MAX_SUBS {
        let now_ns = time::monotonic_nanos();
        let deadline_ns = ticks_to_nanos(next_ticks);
        let rel_ns = deadline_ns.saturating_sub(now_ns).max(1); // at least 1ns
        subs[nsubs] = Subscription {
            userdata: u64::MAX, // sentinel: clock event
            tag: wasi::EVENTTYPE_CLOCK,
            _pad: [0; 7],
            payload: SubscriptionPayload {
                clock: SubscriptionClock {
                    id: wasi::CLOCKID_MONOTONIC,
                    _pad0: [0; 4],
                    timeout: rel_ns,
                    precision: 1_000, // 1 µs
                    flags: 0,         // relative
                    _pad1: [0; 6],
                },
            },
        };
        nsubs += 1;
    }

    // Nothing to wait for (no sockets, no timers). Yield briefly so I don't
    // busy-spin if the root future soft-parked without registering anything.
    if nsubs == 0 {
        // Relative 1ms sleep via a clock subscription with flags=0.
        subs[0] = Subscription {
            userdata: u64::MAX,
            tag: wasi::EVENTTYPE_CLOCK,
            _pad: [0; 7],
            payload: SubscriptionPayload {
                clock: SubscriptionClock {
                    id: wasi::CLOCKID_MONOTONIC,
                    _pad0: [0; 4],
                    timeout: 1_000_000, // 1 ms relative
                    precision: 1_000,
                    flags: 0, // relative
                    _pad1: [0; 6],
                },
            },
        };
        nsubs = 1;
    }

    let mut events: [Event; MAX_SUBS] = unsafe { core::mem::zeroed() };
    let mut nevents: usize = 0;
    let e = unsafe { wasi::poll_oneoff(subs.as_ptr(), events.as_mut_ptr(), nsubs, &mut nevents) };
    if e != wasi::ERRNO_SUCCESS {
        // Still drain timers so a bad poll doesn't leave deadlines stuck forever.
        let _ = time::next_timer_expiration();
        return;
    }

    // Wake fd interests that became ready.
    {
        let mut g = INTERESTS.lock().unwrap();
        for ev in events.iter().take(nevents) {
            if ev.userdata == u64::MAX {
                // clock event; timers are drained below.
                continue;
            }
            let is_write = (ev.userdata & 1) != 0;
            let fd = (ev.userdata >> 1) as Fd;
            if let Some(slot) = g.iter_mut().find(|i| i.fd == fd) {
                let w = if is_write {
                    slot.write.take()
                } else {
                    slot.read.take()
                };
                if let Some(waker) = w {
                    waker.wake();
                }
            }
        }
        // Drop empty interest rows.
        g.retain(|i| i.read.is_some() || i.write.is_some());
    }

    // Fire any timers whose deadline is now past (includes the one I slept for).
    let _ = time::next_timer_expiration();
}

fn fd_sub(fd: Fd, tag: u8, userdata: u64) -> Subscription {
    Subscription {
        userdata,
        tag,
        _pad: [0; 7],
        payload: SubscriptionPayload {
            fd_readwrite: SubscriptionFdReadwrite {
                fd,
                _pad: [0; 28],
            },
        },
    }
}
