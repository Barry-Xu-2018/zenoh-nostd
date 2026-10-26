#![cfg_attr(feature = "esp32s3", no_std)]
#![cfg_attr(feature = "esp32s3", no_main)]
#![cfg_attr(feature = "wasm", no_main)]

use zenoh_examples::*;
use zenoh_nostd::session::*;

const VERSION: &str = env!("Z_PUB_VERSION");

/// Fixed-size buffer so the payload can be formatted without `alloc`.
struct PayloadBuf {
    buf: [u8; 128],
    len: usize,
}

impl PayloadBuf {
    fn new(counter: u32) -> Self {
        use core::fmt::Write;
        let mut p = Self { buf: [0; 128], len: 0 };
        let _ = write!(p, "Hello, from no-std! #{} (version {})", counter, VERSION);
        p
    }

    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl core::fmt::Write for PayloadBuf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let end = self.len + s.len();
        if end > self.buf.len() {
            return Err(core::fmt::Error);
        }
        self.buf[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

#[cfg(not(feature = "wamr"))]
#[embassy_executor::task]
async fn session_task(session: &'static Session<'static, ExampleConfig>) {
    if let Err(e) = session.run().await {
        zenoh::error!("Error in session task: {}", e);
    }
}

#[cfg(not(feature = "wamr"))]
async fn entry(spawner: embassy_executor::Spawner) -> zenoh::ZResult<()> {
    #[cfg(feature = "log")]
    env_logger::init();

    zenoh::info!("zenoh-nostd z_pub example, version: {}", VERSION);

    let config = init_session_example(&spawner).await;
    let session = if LISTEN {
        zenoh::listen!(ExampleConfig: config, Endpoint::try_from(ENDPOINT)?)
    } else {
        zenoh::connect!(ExampleConfig: config, Endpoint::try_from(ENDPOINT)?)
    };

    spawner.spawn(session_task(session)).unwrap();

    let publisher = session
        .declare_publisher(zenoh::keyexpr::new("demo/example")?)
        .finish()
        .await?;

    let mut counter: u32 = 0;
    loop {
        let next = counter.wrapping_add(1);
        let payload = PayloadBuf::new(next);
        let payload = payload.as_bytes();
        if publisher.put(payload).finish().await.is_ok() {
            counter = next;
            zenoh::info!(
                "[Publisher] Sent PUT ('{}': '{}')",
                publisher.keyexpr().as_str(),
                core::str::from_utf8(payload).unwrap()
            );
        }

        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
    }
}

/// WAMR path: single root future under our `block_on` + `poll_oneoff` reactor.
/// `session.run()` and the publish loop are composed with `join` (no Spawner).
#[cfg(feature = "wamr")]
async fn entry_wamr() -> zenoh::ZResult<()> {
    #[cfg(feature = "log")]
    env_logger::init();

    zenoh::info!("zenoh-nostd z_pub example (wamr), version: {}", VERSION);

    let config = init_session_example_wamr();
    let session = if LISTEN {
        zenoh::listen!(ExampleConfig: config, Endpoint::try_from(ENDPOINT)?)
    } else {
        zenoh::connect!(ExampleConfig: config, Endpoint::try_from(ENDPOINT)?)
    };

    let publisher = session
        .declare_publisher(zenoh::keyexpr::new("demo/example")?)
        .finish()
        .await?;

    embassy_futures::join::join(session.run(), async {
        let mut counter: u32 = 0;
        loop {
            let next = counter.wrapping_add(1);
            let payload = PayloadBuf::new(next);
            let payload = payload.as_bytes();
            if publisher.put(payload).finish().await.is_ok() {
                counter = next;
                zenoh::info!(
                    "[Publisher] #{} Sent PUT (version {}) ('{}': '{}')",
                    counter,
                    VERSION,
                    publisher.keyexpr().as_str(),
                    core::str::from_utf8(payload).unwrap()
                );
            }
            embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
        }
    })
    .await
    .0
    .map_err(|e| e.into())
}

#[cfg(not(feature = "wamr"))]
#[cfg_attr(feature = "std", embassy_executor::main)]
#[cfg_attr(feature = "wasm", embassy_executor::main)]
#[cfg_attr(feature = "esp32s3", esp_rtos::main)]
async fn main(spawner: embassy_executor::Spawner) {
    if let Err(e) = entry(spawner).await {
        zenoh::error!("Error in main: {}", e);
    }

    zenoh::info!("Exiting main");
}

#[cfg(feature = "wamr")]
fn main() {
    if let Err(e) = zenoh_wamr::executor::block_on(entry_wamr()) {
        zenoh::error!("Error in main: {}", e);
    }
    zenoh::info!("Exiting main");
}

#[cfg(feature = "esp32s3")]
mod esp32s3_app {
    use esp_hal::rng::Rng;
    pub use esp_println as _;
    use getrandom::{Error, register_custom_getrandom};

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        zenoh_nostd::session::zenoh::error!("Panic: {}", info);

        loop {}
    }

    extern crate alloc;

    esp_bootloader_esp_idf::esp_app_desc!();

    register_custom_getrandom!(getrandom_custom);
    pub fn getrandom_custom(bytes: &mut [u8]) -> Result<(), Error> {
        Rng::new().read(bytes);
        Ok(())
    }
}
