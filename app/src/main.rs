#![forbid(unsafe_code, future_incompatible, clippy::unwrap_used, clippy::panic, clippy::panic_in_result_fn, clippy::unwrap_in_result, clippy::unreachable)]
#![deny(clippy::expect_used)]
#![windows_subsystem = "windows"]

use std::sync::Arc;

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
struct Image{
    width: u32,
    height: u32,
    rgba: &'static [u8],
}
impl From<Image> for egui::IconData {
    fn from(value: Image) -> Self {
        Self {
            width: value.width,
            height: value.height,
            rgba: Vec::from(value.rgba),
        }
    }
}
const ICON_BYTES:Image = ::app_macro::include_image!("../../images/app.png");
fn async_main(collector: egui_tracing::EventCollector){
    let runtime = Arc::new(tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
    );
    log::info!("Tokio Runtime initialized");
    let icon = Arc::<egui::IconData>::new(ICON_BYTES.into());
    if let Some(err) = eframe::run_native(
        "DexProtectOSC-RS",
        eframe::NativeOptions{
            viewport: egui::ViewportBuilder::default()
                .with_icon(icon.clone()),
            ..Default::default()
        },
        Box::new(|cc| {
            Ok(Box::new(app::App::new(collector.clone(), cc, runtime.clone())))
        }),
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
