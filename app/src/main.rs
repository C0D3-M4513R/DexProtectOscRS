#![forbid(unsafe_code, future_incompatible, clippy::unwrap_used, clippy::panic, clippy::panic_in_result_fn, clippy::unwrap_in_result, clippy::unreachable)]
#![deny(clippy::expect_used)]
#![windows_subsystem = "windows"]


mod app;
pub(crate) mod osc;

fn main() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let collector = egui_tracing::EventCollector::new();
    tracing_subscriber::registry()
        .with(tracing_subscriber::filter::EnvFilter::builder()
            .with_default_directive(tracing_core::LevelFilter::INFO.into())
            .from_env_lossy()
        )
        .with(tracing_subscriber::fmt::layer().pretty())
        .with(tracing_subscriber::filter::filter_fn(|event| {
            if let Some(module) = event.module_path() {
                let mut bool = *event.level() == tracing_core::Level::TRACE && (module.starts_with("egui") || module.starts_with("eframe"));
                bool |= (*event.level() == tracing_core::Level::DEBUG || *event.level() == tracing_core::Level::TRACE) && (module.starts_with("globset") || module.starts_with("polling") || module.starts_with("calloop"));
                !bool
            } else {
                true
            }
        }))
        .with(collector.clone())
        .init();
    log::info!("Logger initialized");
    async_main(collector);
}

#[tokio::main]
async fn async_main(collector: egui_tracing::EventCollector){
    log::info!("Tokio Runtime initialized");
    if let Some(err) = eframe::run_native(
        "DexProtectOSC-RS",
        eframe::NativeOptions::default(),
        Box::new(|cc| Ok(Box::new(app::App::new(collector, cc)))),
    )
        .err()
    {
        eprintln!(
            "Error in eframe whilst trying to start the application: {:?}",
            err
        );
    }
    println!("GUI exited. Thank you for using DexProtectOSC-RS!");
}
