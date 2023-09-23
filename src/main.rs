#![forbid(unsafe_code, future_incompatible, clippy::unwrap_used, clippy::panic, clippy::panic_in_result_fn, clippy::unwrap_in_result, clippy::unreachable)]
#![deny(clippy::expect_used)]

use std::sync::OnceLock;
use tokio::runtime::{Builder, Runtime};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod app;
pub(crate) mod osc;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
fn get_runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        #[allow(clippy::expect_used)]
        Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to initialize tokio runtime")
    })
}

fn main() {
    let collector = egui_tracing::EventCollector::default();
    tracing_subscriber::registry()
        .with(collector.clone())
        .init();
    log::info!("Logger initialized");
    let rt = get_runtime();
    let _a = rt.enter(); // "_" as a variable name immediately drops the value, causing no tokio runtime to be registered. "_a" does not.
    log::info!("Tokio Runtime initialized");
    let native_options = eframe::NativeOptions::default();
    if let Some(err) = eframe::run_native(
        "DexProtectOSC-RS",
        native_options,
        Box::new(|cc| Box::new(app::App::new(collector, cc))),
    )
        .err()
    {
        eprintln!(
            "Error in eframe whilst trying to start the application: {}",
            err
        );
    }
    println!("GUI exited. Thank you for using DexProtectOSC-RS!");
}
