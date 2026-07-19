//! End-to-end test of the WAMR executor + async TCP socket + timer, under iwasm.
//!
//! Host side (run first):
//!   python3 -c '
//! import socket
//! s=socket.socket(); s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
//! s.bind(("127.0.0.1",9876)); s.listen(1)
//! print("listening on 9876", flush=True)
//! c,_=s.accept(); data=c.recv(64); c.sendall(b"echo:"+data); c.close()
//! '
//!
//! Then:
//!   cargo build --target wasm32-wasip1 --example exec_selftest
//!   iwasm --addr-pool=0.0.0.0/0 --allow-resolve \
//!       target/wasm32-wasip1/debug/examples/exec_selftest.wasm

use embassy_futures::select::{select, Either};
use embassy_time::{Duration, Instant, Timer};
use zenoh_wamr::executor;
use zenoh_wamr::sock::WamrTcp;

fn fail(msg: &str) -> ! {
    println!("EXEC-SELFTEST-FAIL {msg}");
    std::process::exit(1);
}

async fn run() {
    // --- timer alone, driven by block_on + poll_oneoff clock subscription ---
    let start = Instant::now();
    Timer::after(Duration::from_millis(40)).await;
    let elapsed = Instant::now().duration_since(start);
    println!("timer-only: slept {} ms", elapsed.as_millis());
    if elapsed < Duration::from_millis(40) {
        fail("timer returned too early");
    }
    if elapsed > Duration::from_millis(500) {
        fail("timer took unreasonably long (reactor not sleeping?)");
    }

    // --- TCP connect + write + read, concurrent with a safety-timeout timer ---
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(9876);

    println!("tcp: connecting to 127.0.0.1:{port}");
    let tcp = match WamrTcp::connect([127, 0, 0, 1], port) {
        Ok(t) => t,
        Err(e) => fail(&format!("connect failed: {e:?}")),
    };
    println!("tcp: connected fd={}", tcp.fd());

    if let Err(e) = tcp.write_all(b"ping").await {
        fail(&format!("write_all failed: {e:?}"));
    }
    println!("tcp: wrote ping");

    let mut buf = [0u8; 32];
    // Race the read against a 2s watchdog. If the reactor fails to wake on
    // FD_READ, triggers an explicit error on timeout instead of hanging the system indefinitely.
    let read_result = select(
        tcp.read(&mut buf),
        Timer::after(Duration::from_secs(2)),
    )
    .await;

    let n = match read_result {
        Either::First(Ok(n)) => n,
        Either::First(Err(e)) => fail(&format!("read failed: {e:?}")),
        Either::Second(()) => fail("read timed out after 2s (FD_READ reactor broken?)"),
    };

    let got = &buf[..n];
    println!("tcp: read {n} bytes: {:?}", core::str::from_utf8(got));
    if got != b"echo:ping" {
        fail(&format!("unexpected payload: {got:?}"));
    }

    // Drop closes the fd (last owner).
    drop(tcp);

    println!("EXEC-SELFTEST-OK executor + socket + timer work on WAMR");
}

fn main() {
    executor::block_on(run());
}
