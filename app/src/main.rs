#![forbid(unsafe_code, future_incompatible, clippy::unwrap_used, clippy::panic, clippy::panic_in_result_fn, clippy::unwrap_in_result, clippy::unreachable)]
#![deny(clippy::expect_used)]
#![windows_subsystem = "windows"]

use std::ops::{Deref, DerefMut};
use std::sync::Arc;
#[cfg(feature = "tray")]
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use eframe::{Frame, Storage};
use egui::{Context, RawInput, Ui, Visuals};

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
#[cfg(feature = "tray")]
static QUIT:parking_lot::Mutex<bool> = parking_lot::Mutex::new(false);
#[cfg(feature = "tray")]
static IS_OPEN:AtomicBool = AtomicBool::new(false);
#[cfg(feature = "tray")]
static OPEN:tokio::sync::Notify = tokio::sync::Notify::const_new();
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
    #[cfg(feature = "tray")]
    {
        let tray_icon = tray_icon::Icon::from_rgba(icon.rgba.clone(), icon.width, icon.height).expect("Failed to load tray-icon");
        let menu = tray_icon::menu::Menu::new();
        let open = tray_icon::menu::MenuItem::new("Open", true, None);
        let quit = tray_icon::menu::MenuItem::new("Quit", true, None);
        menu.append_items(&[&open, &quit]).expect("Failed to build menu");

        let _icon = match tray_icon::TrayIconBuilder::new()
            .with_icon(tray_icon)
            .with_menu(Box::new(menu))
            .build()
        {
            Ok(icon) => icon,
            Err(err) => {
                log::error!("Failed to spawn Tray: {err}");
                return;
            }
        };

        {
            let open = open.into_id();
            let quit = quit.into_id();
            tray_icon::menu::MenuEvent::set_event_handler(Some(move |v:tray_icon::menu::MenuEvent|{
                if v.id == quit {
                    *QUIT.lock() = true;
                }
                if v.id == quit || v.id == open {
                    OPEN.notify_one();
                }
            }))
        }
    }

    struct WrapApp<T>(T);
    impl<D: eframe::App, T:Deref<Target = D> + DerefMut> eframe::App for WrapApp<T> {
        fn logic(&mut self, ctx: &Context, frame: &mut Frame) {
            D::logic(&mut *self.0, ctx, frame)
        }

        fn ui(&mut self, ui: &mut Ui, frame: &mut Frame) {
            D::ui(&mut *self.0, ui, frame)
        }

        fn update(&mut self, ctx: &Context, frame: &mut Frame) {
            #[allow(deprecated)]
            D::update(&mut *self.0, ctx, frame)
        }

        fn save(&mut self, storage: &mut dyn Storage) {
            D::save(&mut *self.0, storage)
        }

        fn on_exit(&mut self) {
            D::on_exit(&mut *self.0)
        }

        fn auto_save_interval(&self) -> Duration {
            D::auto_save_interval(&*self.0)
        }

        fn clear_color(&self, visuals: &Visuals) -> [f32; 4] {
            D::clear_color(&*self.0, visuals)
        }

        fn persist_egui_memory(&self) -> bool {
            D::persist_egui_memory(&*self.0)
        }

        fn raw_input_hook(&mut self, ctx: &Context, raw_input: &mut RawInput) {
            D::raw_input_hook(&mut *self.0, ctx, raw_input)
        }
    }
    let app = std::cell::OnceCell::new();
    loop{
        let collector = collector.clone();
        if let Some(err) = eframe::run_native(
            "DexProtectOSC-RS",
            eframe::NativeOptions{
                viewport: egui::ViewportBuilder::default()
                    .with_icon(icon.clone()),
                ..Default::default()
            },
            Box::new(|cc| {
                #[cfg(feature = "tray")]
                {
                    IS_OPEN.store(true, std::sync::atomic::Ordering::Release);
                }
                let app = runtime.block_on(app.get_or_init(
                    ||Arc::new(tokio::sync::Mutex::new(app::App::new(collector, cc, runtime.clone())))
                ).clone().lock_owned());
                Ok(Box::new(WrapApp(app)))
            }),
        )
            .err()
        {
            eprintln!(
                "Error in eframe whilst trying to start the application: {:?}",
                err
            );
        }
        #[cfg(feature = "tray")]
        {
            IS_OPEN.store(false, std::sync::atomic::Ordering::Release);
            if *QUIT.lock() {
                break;
            }
            runtime.block_on(OPEN.notified());
            if *QUIT.lock() {
                break;
            }
        }
        #[cfg(not(feature = "tray"))]
        break;
    }

    println!("GUI exited. Thank you for using DexProtectOSC-RS!");
}
