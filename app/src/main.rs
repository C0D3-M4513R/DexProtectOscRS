#![forbid(future_incompatible, clippy::unwrap_used, clippy::panic, clippy::panic_in_result_fn, clippy::unwrap_in_result, clippy::unreachable)]
#![deny(clippy::expect_used)]
#![windows_subsystem = "windows"]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use clap::Parser;
use eframe::{Frame, Storage, UserEvent};
use egui::{Context, RawInput, Ui, Visuals};
use serde_derive::{Deserialize, Serialize};
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::platform::run_on_demand::EventLoopExtRunOnDemand;
use winit::window::WindowId;

#[cfg(feature = "gui")]
mod app;
pub(crate) mod osc;
#[cfg(feature = "gui")]
mod icon;
#[derive(Debug, Clone, clap_derive::Parser)]
#[command(version, about, long_about = None)]
/// An osc multiplexer and program for to unlock DexProtect'ed Avatar
struct Args{
    #[cfg(feature = "gui")]
    #[clap(short, long, action = clap::ArgAction::Set, default_value_t = true)]
    /// Should the application be started with a gui?
    gui: bool,
    #[cfg(feature = "tray")]
    #[clap(short = 'm', long)]
    /// Should the application be started minimized to tray? (on error it will show itself)
    start_minimized: bool,
    #[clap(flatten)]
    osc: OscCreateData
}

#[derive(Debug, Default, Clone,Serialize,Deserialize, clap_derive::Parser)]
#[serde(default)]
pub struct OscCreateData {
    #[cfg(feature = "oscquery")]
    #[clap(long)]
    /// Should OscQuery be used to establish an Osc connection?
    /// WARNING: OscQuery might not be as reliable as Osc!
    pub use_oscquery: Option<bool>,
    #[clap(short, long)]
    /// Ip and Port to Receive Osc Data on
    pub recv: Option<SocketAddr>,
    #[clap(short, long)]
    /// Ip and Port to Send Osc Data to
    pub send: Option<SocketAddr>,
    #[clap(long)]
    /// The maximum buffer size for osc messages
    pub max_message_size: Option<usize>,
    #[clap(long)]
    /// Should the Dex Protect handler be enabled?
    pub dex_protect_enabled: Option<bool>,
    #[clap(long)]
    /// Should the Dex Protect handler send one OscBundle?
    /// WARNING: KNOWN BROKEN! See https://feedback.vrchat.com/bug-reports/p/inconsistent-handling-of-osc-packets-inside-osc-bundles-and-osc-packages
    pub dex_use_bundles: Option<bool>,
    #[clap(short, long)]
    /// Path to the folder of Dex Protect Keys. By default, this is located at '%USERPROFILE%\Documents\DexProtect'
    pub path: Option<PathBuf>,
    #[clap(long = "multiplexer_sockets")]
    /// Ip and Port addresses to forward received Osc Packets to
    pub osc_multiplexer_sockets: Option<Vec<SocketAddr>>,
    #[clap(long = "multiplexer_parse_packets")]
    /// Should packets be parsed and then serialized again, before being forwarded, or should the raw received bytes be blindly forwarded?
    pub osc_multiplexer_parse_packets: Option<bool>,
}

impl Default for Args {
    fn default() -> Self {
        Self{
            #[cfg(feature = "gui")]
            gui: true,
            #[cfg(feature = "tray")]
            start_minimized: false,
            osc: OscCreateData::default(),
        }
    }
}


fn attach_console() -> anyhow::Result<()>{
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        unsafe {
            use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
            if let Ok(()) = AttachConsole(ATTACH_PARENT_PROCESS) {
                set_console_handles()?;
            }
        }
    }

    Ok(())
}
fn alloc_console() -> anyhow::Result<bool>{
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        return Ok(false);
    }
    #[cfg(windows)]
    {
        unsafe {
            use windows::Win32::System::Console::{AllocConsole};
            if let Ok(()) = AllocConsole() {
                set_console_handles()?;
                return Ok(true);
            }
        }
    }

    Ok(false)
}

#[cfg(windows)]
unsafe fn set_console_handles() -> anyhow::Result<()> {
    {
        unsafe {
            use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS, SetStdHandle, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE, STD_INPUT_HANDLE};
            use windows::Win32::Foundation::{HANDLE};
            use windows::Win32::System::Console::{GetConsoleMode, SetConsoleMode, CONSOLE_MODE, ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING};


            use std::os::windows::io::AsRawHandle;

            if let Ok(()) = AttachConsole(ATTACH_PARENT_PROCESS) {
                let conout = core::mem::ManuallyDrop::new(std::fs::OpenOptions::new()
                    .write(true)
                    .open("CONOUT$")
                    .map_err(|err|anyhow::format_err!("Failed to open special file 'CONOUT$': {err}"))?);
                let conout_handle = HANDLE(conout.as_raw_handle() as _);
                SetStdHandle(STD_OUTPUT_HANDLE, conout_handle)?;
                SetStdHandle(STD_ERROR_HANDLE, conout_handle)?;
                {
                    let mut mode = CONSOLE_MODE::default();
                    GetConsoleMode(conout_handle, &mut mode)?;
                    SetConsoleMode(conout_handle, mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING)?;
                }

                let conin = core::mem::ManuallyDrop::new(std::fs::OpenOptions::new()
                    .write(true)
                    .open("CONIN$")
                    .map_err(|err|anyhow::format_err!("Failed to open special file 'CONIN$': {err}"))?);
                let conin_handle = HANDLE(conin.as_raw_handle() as _);
                SetStdHandle(STD_INPUT_HANDLE, conin_handle)?;
                {
                    let mut mode = CONSOLE_MODE::default();
                    GetConsoleMode(conin_handle, &mut mode)?;
                    SetConsoleMode(conin_handle, mode | ENABLE_VIRTUAL_TERMINAL_INPUT)?;
                }
            }
        }
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    attach_console()?;

    let collector;

    let args = Args::parse();

    #[cfg(feature = "gui")]
    {
        collector = if args.gui{
            Some(egui_tracing::EventCollector::new())
        } else {
            None
        };
    }
    #[cfg(not(feature = "gui"))]
    {
        collector = ();
    }

    init_logging(&collector)?;

    tracing::info!("Logger initialized");
    async_main(args, collector)
}
#[cfg(feature = "gui")]
type Collector = Option<egui_tracing::EventCollector>;
#[cfg(not(feature = "gui"))]
type Collector = ();

fn init_logging(collector: &Collector) -> anyhow::Result<()> {
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    #[cfg(feature = "gui")]
    {
        if let Some(collector) = collector {
            let env_filter =
                tracing_subscriber::filter::EnvFilter::builder()
                    .with_default_directive(tracing::level_filters::LevelFilter::INFO.into())
                    .from_env_lossy();
            tracing_subscriber::registry()
                .with(
                    tracing_subscriber::fmt::layer()
                        .pretty()
                        .with_filter(env_filter.clone())
                )
                .with(
                    tracing_subscriber::filter::filter_fn(|event| {
                        if let Some(module) = event.module_path() {
                            let mut bool = *event.level() == tracing::Level::TRACE && (module.starts_with("egui") || module.starts_with("eframe"));
                            bool |= (*event.level() == tracing::Level::DEBUG || *event.level() == tracing::Level::TRACE) && (module.starts_with("globset") || module.starts_with("polling") || module.starts_with("calloop"));
                            !bool
                        } else {
                            true
                        }
                    }).and_then(collector.clone())
                        .with_filter(env_filter)
                )
                .init();

            return Ok(());
        }
    }

    let _ = collector;
    {
        let v = alloc_console()?;
        tracing_subscriber::registry()
            .with(
                {
                    let mut fmt =
                        tracing_subscriber::fmt::layer()
                            .pretty();
                    if v {
                        println!("AllocConsole was used. AllocConsole doesn't want to handle ANSI, so disabling ansi");
                        fmt.set_ansi(false);
                    }
                    fmt.with_filter(
                        tracing_subscriber::filter::EnvFilter::builder()
                            .with_default_directive(tracing::level_filters::LevelFilter::INFO.into())
                            .from_env_lossy()
                    )
                }
            )
            .init();
        tracing::info!("Initialized Logging.")
    }

    Ok(())
}
fn async_main(args: Args, collector: Collector) -> anyhow::Result<()> {
    #[cfg(not(feature = "gui"))]
    {
        let _ = collector;
    }
    let runtime = Arc::new(tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
    );
    log::info!("Tokio Runtime initialized");

    #[cfg(feature = "gui")]
    {
        if let Some(collector) = collector {
            let mut event_loop = winit::event_loop::EventLoop::with_user_event()
                .build()?;

            let quit_mut = Arc::new(parking_lot::Mutex::new(false));
            let app_data = Arc::new(parking_lot::Mutex::new(None));
            #[cfg(feature="tray")]
            let cc = Arc::new(tokio::sync::Mutex::new(None::<egui::Context>));

            {
                let quit_mut = quit_mut.clone();
                let cc = cc.clone();
                runtime.spawn(async move {
                    if let Err(err) = tokio::signal::ctrl_c().await {
                        log::error!("Failed to listen for Ctrl-C: {err}");
                    }
                    tracing::info!("Received Ctrl-C. Exiting!");
                    *quit_mut.lock() = true;
                    if let Some(cc) = &*cc.lock().await {
                        cc.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            }

            let open = Arc::new(std::sync::Condvar::new());

            struct App<'a>{
                cc: Arc<parking_lot::Mutex<Option<egui::Context>>>,
                quit_mut: Arc<parking_lot::Mutex<bool>>,
                open_var: Arc<std::sync::Condvar>,
                #[cfg(feature="tray")]
                icon: bool,
                app: eframe::EframeWinitApplication<'a>,
            }
            impl<'a> winit::application::ApplicationHandler<eframe::UserEvent> for App<'a> {
                fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
                    #[cfg(feature="tray")]
                    {
                        if cause == winit::event::StartCause::Init && !self.icon {
                            self.icon = true;
                            let ctx = self.cc.clone();
                            let open_var = self.open_var.clone();
                            let icon = &crate::icon::ICON_BYTES;
                            let tray_icon = tray_icon::Icon::from_rgba(icon.rgba.to_vec(), icon.width, icon.height).expect("Failed to load tray-icon");
                            let menu = tray_icon::menu::Menu::new();
                            let open = tray_icon::menu::MenuItem::new("Open", true, None);
                            let quit = tray_icon::menu::MenuItem::new("Quit", true, None);
                            menu.append_items(&[&open, &quit]).expect("Failed to build menu");

                            let _ = match tray_icon::TrayIconBuilder::new()
                                .with_icon(tray_icon)
                                .with_menu(Box::new(menu))
                                .build()
                            {
                                Ok(icon) => icon,
                                Err(err) => {
                                    log::error!("Failed to spawn Tray: {err}");
                                    panic!("Failed to spawn Tray: {err}");
                                }
                            };

                            {
                                let quit_mut = self.quit_mut.clone();
                                let open = open.into_id();
                                let quit = quit.into_id();
                                tray_icon::menu::MenuEvent::set_event_handler(Some(move |v:tray_icon::menu::MenuEvent|{
                                    let ctx = ctx.lock();
                                    if v.id == quit {
                                        *quit_mut.lock() = true;
                                        open_var.notify_all();
                                        if let Some(ctx) = &*ctx {
                                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                        }
                                    }
                                    if v.id == open {
                                        open_var.notify_all();
                                        if let Some(ctx) = &*ctx {
                                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                                        }
                                    }
                                }))
                            }
                        }
                    }
                    self.app.new_events(event_loop, cause);
                }

                fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                    self.app.resumed(event_loop);
                }

                fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
                    self.app.user_event(event_loop, event);
                }

                fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
                    self.app.window_event(event_loop, window_id, event);
                }

                fn device_event(&mut self, event_loop: &ActiveEventLoop, device_id: DeviceId, event: DeviceEvent) {
                    self.app.device_event(event_loop, device_id, event);
                }

                fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
                    self.app.about_to_wait(event_loop);
                }

                fn suspended(&mut self, event_loop: &ActiveEventLoop) {
                    self.app.suspended(event_loop);
                }

                fn exiting(&mut self, event_loop: &ActiveEventLoop) {
                    self.app.exiting(event_loop);
                }

                fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
                    self.app.memory_warning(event_loop);
                }
            }
            let cc = Arc::new(parking_lot::Mutex::new(None));

            macro_rules! start_app {
                ()=>{
                    eframe::create_native(
                        "DexProtectOSC-RS",
                        eframe::NativeOptions{
                            viewport: egui::ViewportBuilder::default()
                                .with_icon(Arc::<egui::IconData>::new(icon::ICON_BYTES.into())),
                            ..Default::default()
                        },
                        Box::new(|cc_r| {
                            *cc.lock() = Some(cc_r.egui_ctx.clone());
                            let cc = cc_r;

                            #[cfg(feature = "tray")]
                            {
                                FIRST_START.call_once(||{
                                    if !args.start_minimized
                                    {
                                        cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                    }
                                })
                            }
                            let data = {
                                let mut data = app_data.lock();
                                data.get_or_insert_with(||Arc::new(parking_lot::Mutex::new(app::App::new(args.clone(), quit_mut.clone(), collector.clone(), cc, runtime.clone())))).clone()
                            };
                            Ok(Box::new(Wrap(data.lock_arc())))
                        }),
                        &event_loop,
                    )
                }
            }

            let mut app = App {
                cc: cc.clone(),
                quit_mut: quit_mut.clone(),
                open_var: open.clone(),
                app: start_app!(),
                #[cfg(feature="tray")]
                icon: false,
            };
            let open_mtx = std::sync::Mutex::new(true);
            let mut lock = open_mtx.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            struct Wrap<T>(T);
            impl<T> eframe::App for Wrap<T>
            where
                T: core::ops::DerefMut,
                T::Target: eframe::App
            {
                fn logic(&mut self, ctx: &Context, frame: &mut Frame) {
                    self.0.logic(ctx, frame)
                }

                fn ui(&mut self, ui: &mut Ui, frame: &mut Frame) {
                    self.0.logic(ui, frame)
                }

                fn save(&mut self, _storage: &mut dyn Storage) {
                    self.0.save(_storage)
                }

                fn on_exit(&mut self) {
                    self.0.on_exit()
                }

                fn auto_save_interval(&self) -> Duration {
                    self.0.auto_save_interval()
                }

                fn clear_color(&self, _visuals: &Visuals) -> [f32; 4] {
                    self.0.clear_color(_visuals)
                }

                fn persist_egui_memory(&self) -> bool {
                    self.0.persist_egui_memory()
                }

                fn raw_input_hook(&mut self, _ctx: &Context, _raw_input: &mut RawInput) {
                    self.0.raw_input_hook(_ctx, _raw_input)
                }
            }

            #[cfg(feature = "tray")]
            static FIRST_START:std::sync::Once = std::sync::Once::new();

            loop {
                if *quit_mut.lock() {
                    break;
                }
                if *lock {
                    app.app = start_app!();
                }
                event_loop.run_app_on_demand(&mut app)?;
                *cc.lock() = None;
                lock = open.wait(lock).unwrap_or_else(std::sync::PoisonError::into_inner);
            }

            println!("GUI exited. Thank you for using DexProtectOSC-RS!");
            return Ok(());
        }
    }

    {
        let data = osc::OscCreateData::default()
            .merge_data(args.osc);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let join = runtime.spawn(async {

            if let Err(err) = tokio::signal::ctrl_c().await {
                tracing::error!("Error waiting for Ctrl-C signal: {err}");
            }
            tracing::info!("received ctrl-c signal");
            if let Err(_) = tx.send(()) {
                tracing::error!("Failed to inform Osc Handler of exit request");
            }
        });
        runtime.block_on(async {
            if let Err(err) = osc::create_and_start_osc(data, rx).await {
                tracing::error!("Error in OscHandler thread: {err}");
            }
        });
        tracing::info!("OscHandler thread exited");
        join.abort();
        tracing::info!("Stopping Runtime");
        drop(runtime);
        tracing::info!("Stopped Runtime");
        Ok(())
    }
}
