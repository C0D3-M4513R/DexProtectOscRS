use std::collections::VecDeque;
use std::fmt::{Debug, Formatter};
use std::ops::{Deref, DerefMut, IndexMut};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use egui::{Ui, Widget};
use serde_derive::{Deserialize, Serialize};
use tokio::time::Instant;
use crate::{Args};
use crate::osc::OscCreateData;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AppData{
    logs_visible: bool,
    auto_connect_launch: bool,
    path:String,
    dex_use_bundles: bool,
    #[cfg(feature = "oscquery")]
    use_oscquery: bool,
    recv_ip:String,
    osc_recv_port: u16,
    send_ip:String,
    osc_send_port: u16,
    max_message_size: usize,
    osc_multiplexer_enabled: bool,
    osc_multiplexer_parse_packets: bool,
    dex_protect_enabled: bool,
    osc_multiplexer_sockets: Vec<(String, u16)>,
    osc_create_data: OscCreateData,
    #[cfg(feature = "tray")]
    quit_to_tray: bool,
}

impl AppData {
    pub fn merge_data(mut self, data:&Args) -> Self {
        #[cfg(feature = "tray")]
        if data.start_minimized { self.auto_connect_launch = true; }
        #[cfg(feature = "oscquery")]
        if let Some(use_oscquery) = data.osc.use_oscquery { self.use_oscquery = use_oscquery; }
        if let Some(recv) = data.osc.recv { self.recv_ip = recv.ip().to_string(); self.osc_recv_port = recv.port(); }
        if let Some(send) = data.osc.send { self.send_ip = send.ip().to_string(); self.osc_send_port = send.port(); }
        if let Some(max_message_size) = data.osc.max_message_size { self.max_message_size = max_message_size; }
        if let Some(dex_protect_enabled) = data.osc.dex_protect_enabled { self.dex_protect_enabled = dex_protect_enabled; }
        if let Some(dex_use_bundles) = data.osc.dex_use_bundles { self.dex_use_bundles = dex_use_bundles; }
        if let Some(path) = &data.osc.path { if let Some(path) = path.to_str() { self.path = path.to_string() } else { log::warn!("The specified path '{}' cannot be converted to a UTF-8 String", path.display())} }
        if let Some(osc_multiplexer_sockets) = &data.osc.osc_multiplexer_sockets { self.osc_multiplexer_sockets = osc_multiplexer_sockets.iter().map(|v|(v.ip().to_string(), v.port())).collect(); }
        if let Some(osc_multiplexer_parse_packets) = data.osc.osc_multiplexer_parse_packets { self.osc_multiplexer_parse_packets = osc_multiplexer_parse_packets; }
        self
    }
}

pub struct App<'a>{
    collector:egui_tracing::EventCollector,
    data: AppData,

    #[cfg(all(feature = "file_dialog", not(target_arch = "wasm32")))]
    file_picker_thread: Option<tokio::task::JoinHandle<Option<PathBuf>>>,

    osc_multiplexer_port_popup: Option<Box<PopupFunc<'a>>>,
    stop_osc: Option<tokio::sync::oneshot::Sender<()>>,
    osc_thread: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    popups: VecDeque<Box<PopupFunc<'a>>>,
    runtime: Arc<tokio::runtime::Runtime>,
    #[cfg(feature = "tray")]
    quit: Arc<parking_lot::Mutex<crate::State>>,
}
impl<'a> Debug for App<'a>{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("App");
        debug.field("collector", &self.collector)
            .field("data",&self.data);
        #[cfg(all(feature = "file_dialog", not(target_arch = "wasm32")))]
        debug.field("file_picker_thread.is_some()", &self.file_picker_thread.is_some());
        debug
            .field("osc_multiplexer_port_popup.is_some()", &self.osc_multiplexer_port_popup.is_some())
            .field("stop_osc", &self.stop_osc)
            .field("osc_thread", &self.osc_thread)
            .field("popups.len()", &self.popups.len())
            .finish()
    }
}
impl<'a> Deref for App<'a>{
    type Target = AppData;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
impl<'a> DerefMut for App<'a>{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}
impl Default for AppData{
    fn default() -> Self {
        Self{
            logs_visible: false,
            auto_connect_launch: true,
            path: "".to_string(),
            dex_use_bundles: false,
            #[cfg(feature = "oscquery")]
            use_oscquery: false,
            recv_ip:"0.0.0.0".to_string(),
            osc_recv_port: crate::osc::OSC_RECV_PORT,
            send_ip:"127.0.0.1".to_string(),
            osc_send_port: crate::osc::OSC_SEND_PORT,
            max_message_size: crate::osc::OSC_RECV_BUFFER_SIZE,
            osc_multiplexer_enabled: false,
            osc_multiplexer_parse_packets: false,
            dex_protect_enabled: true,
            osc_multiplexer_sockets: Vec::new(),
            osc_create_data: OscCreateData::default(),
            quit_to_tray: true,
        }
    }
}

impl<'a> TryFrom<&App<'a>> for OscCreateData {
    type Error = std::net::AddrParseError;

    fn try_from(value: &App<'a>) -> Result<Self, Self::Error> {
        Ok(OscCreateData{
            #[cfg(feature = "oscquery")]
            use_oscquery: value.use_oscquery,
            recv: std::net::SocketAddr::new(std::net::IpAddr::from_str(value.recv_ip.as_str())?, value.osc_recv_port),
            send: std::net::SocketAddr::new(std::net::IpAddr::from_str(value.send_ip.as_str())?, value.osc_send_port),
            max_message_size: value.max_message_size,
            dex_protect_enabled: value.dex_protect_enabled,
            dex_use_bundles: value.dex_use_bundles,
            path: PathBuf::from(&value.path),
            osc_multiplexer_sockets: if value.osc_multiplexer_enabled {
                let mut vec = Vec::with_capacity(value.osc_multiplexer_sockets.len());
                for (ip, port) in &value.osc_multiplexer_sockets {
                    vec.push(std::net::SocketAddr::new(std::net::IpAddr::from_str(ip.as_str())?, *port));
                }
                vec
            } else {Vec::new()},
            osc_multiplexer_parse_packets: value.osc_multiplexer_parse_packets,
        })
    }
}

impl<'a> App<'a> {
    /// Called once before the first frame.
    pub fn new(args: crate::Args, quit_mut: Arc<parking_lot::Mutex<crate::State>>, collector: egui_tracing::EventCollector, cc: &eframe::CreationContext<'_>, runtime: Arc<tokio::runtime::Runtime>) -> Self {
        // This is also where you can customize the look and feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

        // Load previous app state (if any).
        // Note that you must enable the `persistence` feature for this to work.

        let data:AppData = if let Some(storage) = cc.storage {
            eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default()
        }else {
            Default::default()
        };
        let data = data.merge_data(&args);


        #[cfg(not(debug_assertions))]
        log::info!("You are running a release build. Some log statements were disabled.");

        let mut slf = Self {
            collector,
            data,
            #[cfg(all(feature = "file_dialog", not(target_arch = "wasm32")))]
            file_picker_thread: None,
            osc_multiplexer_port_popup: None,
            stop_osc: None,
            osc_thread: None,
            popups: Default::default(),
            runtime,
            #[cfg(feature="tray")]
            quit: quit_mut
        };

        if slf.auto_connect_launch{
            slf.spawn_osc_from_creation_data();
        }
        slf
    }

    fn has_file_picker_thread(&self)->bool{
        #[cfg(all(feature = "file_dialog", not(target_arch = "wasm32")))]
        return self.file_picker_thread.is_some();
        #[cfg(not(all(feature = "file_dialog", not(target_arch = "wasm32"))))]
        false
    }

    fn handle_join_error(
        &mut self,
        error: &tokio::task::JoinError,
        title: impl Into<egui::WidgetText> + 'a,
    ) {
        self.handle_display_popup("An unknown error occurred while logging out.", error, title);
    }

    fn handle_display_popup<D: std::fmt::Display>(
        &mut self,
        label: impl Into<egui::WidgetText> + 'a,
        error: &D,
        title: impl Into<egui::WidgetText> + 'a,
    ) {
        let error_string = error.to_string();
        let label = label.into().clone();
        self.popups.push_front(popup_creator(title, move |_, ui| {
            ui.label(label.clone());
            ui.label("Some developer information below:");
            ui.label(&error_string);
        }));
    }

    fn stop_osc(&mut self) -> Option<tokio::task::JoinHandle<anyhow::Result<()>>> {
        log::info!("Stopping Osc Thread");
        if let Some(osc_thread) = self.osc_thread.take() {
            if self.stop_osc.take().map(|v|v.send(()).is_err()).unwrap_or(true)
            {
                osc_thread.abort();
                log::info!("OSC Thread abort sent");
            }
            Some(osc_thread)
        } else {
            None
        }
    }
    fn spawn_osc_from_creation_data(&mut self){
        log::info!("Trying to connect to OSC on IP '{}'", self.osc_create_data.recv);
        let osc_create_data = self.osc_create_data.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.stop_osc = Some(tx);
        self.osc_thread = Some(self.runtime.spawn(crate::osc::create_and_start_osc(osc_create_data, rx)));
    }

    fn check_osc_thread(&mut self, ctx: &egui::Context){
        if let Some(osc_thread) = self.osc_thread.take() {
            if osc_thread.is_finished(){
                log::error!("OSC Thread finished unexpectedly");
                self.join_osc_thread(ctx, osc_thread, false);
            }else{
                self.osc_thread = Some(osc_thread);
            }
        }
    }
    fn join_osc_thread(&mut self, ctx: &egui::Context, osc_thread: tokio::task::JoinHandle<anyhow::Result<()>>, expect_exit: bool) {
        match self.runtime.block_on(osc_thread){
            Ok(Ok(())) => {
                log::info!("OSC Thread finished");
                if expect_exit {return;}
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                let time = Instant::now();
                self.popups.push_back(popup_creator(
                    "OSC Thread Exited",
                    move |_, ui| {
                        ui.label("The OSC Thread (the one that communicates with VRChat) exited unexpectedly.");
                        ui.label(format!("This happened {:.1} ago. (this updates only when you move your mouse or something changes)", time.elapsed().as_secs_f32()));
                    })
                )
            }
            Ok(Err(e)) => {
                log::warn!("Error in OSC Thread: {}",e);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                self.handle_display_popup("Osc Error:", &e, "Error in Osc");
            }
            Err(e) => {
                log::error!("Panic in OSC Thread: {}", e);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                self.handle_join_error(&e, "Critical Error in Osc");
            }
        }
    }
    fn dex_protect_ui(&mut self, ui:&mut egui::Ui){
        ui.heading("DexProtect:");
        ui.horizontal(|ui|{
            ui.checkbox(&mut self.dex_use_bundles, "Use Osc Bundles: ");
            ui.hyperlink_to("This is known to cause issues with VRChat and to NOT WORK.", "https://feedback.vrchat.com/bug-reports/p/inconsistent-handling-of-osc-packets-inside-osc-bundles-and-osc-packages");
        });
        ui.horizontal(|ui|{
            ui.label("Keys Folder: ");
            #[cfg_attr(not(all(feature = "file_dialog", not(target_arch = "wasm32"))), allow(unused_variables))]
                let resp = ui.add_enabled(
                !self.has_file_picker_thread(),
                egui::TextEdit::singleline(&mut self.path)
            );
            #[cfg(all(feature = "file_dialog", not(target_arch = "wasm32")))]
            {
                if self.file_picker_thread.is_some(){
                    resp.on_hover_text("A Dialogue to Pick a Folder is currently open.");
                }
            }

            #[cfg(not(all(feature = "file_dialog", not(target_arch = "wasm32"))))]
            ui.label("(No Browse available. Copy and Paste the Path from your File Browser or type it in manually)");
            #[cfg(all(feature = "file_dialog", not(target_arch = "wasm32")))]
            {
                let mut resp = ui.add_enabled(self.file_picker_thread.is_none(), egui::Button::new("Browse"));
                if !resp.enabled(){
                    resp = resp.on_hover_text("A Dialogue to Pick a Folder is currently open. Please use that one.");
                }
                if resp.clicked(){
                    self.file_picker_thread = Some(self.runtime.spawn(async{
                        rfd::AsyncFileDialog::new()
                            .pick_folder()
                            .await
                            .map(|f|f.path().to_path_buf())
                    }));
                }
                if let Some(file_picker_thread) = self.file_picker_thread.take(){
                    if file_picker_thread.is_finished(){
                        match self.runtime.block_on(file_picker_thread) {
                            Ok(Some(path)) => {
                                self.path = path.to_string_lossy().to_string();
                                log::info!("Picked Folder: '{}' (potential replacements due to non UTF-8 characters) ", self.path);
                            },
                            Ok(None) => log::info!("No Folder Picked."),
                            Err(e) => {
                                log::error!("Panic whist picking a Folder: {}", e);
                                self.handle_join_error(&e, "Critical Error whilst picking a Folder");
                            }
                        }
                    }else{
                        self.file_picker_thread = Some(file_picker_thread);
                    }
                }
            }
        });
        ui.add_space(10.)
    }
    fn multiplexer_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Osc Multiplexer:");
        ui.label("All messages Received from the Osc Receive Port will be forwarded to the Ports specified in the list below.");
        ui.label("This allows you to use multiple Osc Applications, that need to Receive Messages, at the same time.");

        ui.checkbox(&mut self.osc_multiplexer_parse_packets, "Parse Packets and Ignore Packets that can't be parsed");
        if ui.add_enabled(self.osc_multiplexer_port_popup.is_none(), egui::Button::new("Manage Ports")).clicked() {
            self.osc_multiplexer_port_popup = Some(popup_creator_collapsible("Osc Multiplexer Ports:", true, |app, ui|{
                let mut i = 0;
                while i < app.osc_multiplexer_sockets.len(){
                    ui.horizontal(|ui|{
                        let (ip, port) = app.osc_multiplexer_sockets.index_mut(i);
                        ui.label(format!("Osc Forward Ip {}: ", i));
                        ui.text_edit_singleline(ip);
                        ui.label(format!("Osc Forward Port {}: ", i));
                        ui.add(egui::DragValue::new(port));
                        if ui.button("Delete")
                            .on_hover_text("Delete this Port from the list, and replaces it with the last one.")
                            .clicked()
                        {
                            app.osc_multiplexer_sockets.swap_remove(i);
                        }

                    });
                    i+=1;
                }
                if ui.button("Add Port").clicked() {
                    app.osc_multiplexer_sockets.push(("127.0.0.1".to_string(), 0));
                }
            }));
        }
        ui.add_space(10.)
    }

    fn osc_control_ui(&mut self, ui: &mut egui::Ui){
        #[cfg(feature = "tray")]
        {
            ui.heading("Generic Controls:");
            ui.horizontal(|ui|{
                ui.label("Quit when pressing exit (instead of Hiding to Tray): ");
                ui.checkbox(&mut self.data.quit_to_tray, ());
            });
            if ui.button("Quit Immediately").clicked() {
                *self.quit.lock() = crate::State::Quitting;
                ui.send_viewport_cmd(egui::ViewportCommand::Close);
                ui.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
            }
        }
        ui.add_space(16.);

        ui.heading("Generic Osc Controls:");
        #[cfg(feature = "oscquery")]
        ui.horizontal(|ui|{
            ui.checkbox(&mut self.use_oscquery, "Use OscQuery: ");
            ui.label("OscQuery is known to have several deficiencies.");
            ui.hyperlink_to("Issue #1", "https://vrchat.canny.io/bug-reports/p/oscquery-json-ghost-parameters");
            ui.hyperlink_to("Issue #2", "https://vrchat.canny.io/bug-reports/p/oscquery-not-properly-filtering-data");
            ui.hyperlink_to("Issue #3", "https://vrchat.canny.io/bug-reports/p/oscquery-provides-wrong-values-for-avatar-parameters-until-they-are-changed");
        });
        #[cfg(feature = "oscquery")]
        let oscquery = self.use_oscquery;
        #[cfg(not(feature = "oscquery"))]
        let oscquery = false;

        ui.add_enabled_ui(!oscquery, |ui|{
            ui.horizontal(|ui|{
                ui.label("Receive IP:");
                ui.text_edit_singleline(&mut self.recv_ip);
            });
            ui.horizontal(|ui|{
                ui.label("OSC Receive Port:");
                ui.add(egui::DragValue::new(&mut self.osc_recv_port));
                if ui.button("Reset to Default").clicked() {
                    self.osc_recv_port = crate::osc::OSC_RECV_PORT;
                }
            });
            ui.horizontal(|ui|{
                ui.label("Send IP:");
                ui.text_edit_singleline(&mut self.send_ip);
            });
            ui.horizontal(|ui|{
                ui.label("OSC Send Port:");
                ui.add(egui::DragValue::new(&mut self.osc_send_port));
                if ui.button("Reset to Default").clicked() {
                    self.osc_send_port = crate::osc::OSC_SEND_PORT;
                }
            });
        });
        ui.horizontal(|ui|{
            ui.label("Osc Max Message Size:");
            egui::DragValue::new(&mut self.max_message_size)
                .speed(1)
                .range(1..=usize::try_from(isize::MAX).unwrap_or(usize::MAX))
                .ui(ui);
            if ui.button("Reset to Default").clicked() {
                self.max_message_size = crate::osc::OSC_RECV_BUFFER_SIZE;
            }
        });
        ui.label("Please note that the Settings in the Ui will only be applied after you Reconnect/Connect.");
        ui.horizontal(|ui|{
            if ui.button(if self.osc_thread.is_some() {"Reconnect"} else {"Connect"}).clicked() {
                if let Some(thread) = self.stop_osc() {
                    self.join_osc_thread(ui.ctx(), thread, true);
                }
                match OscCreateData::try_from(&*self) {
                    Ok(osc_create_data) => {
                        self.osc_create_data = osc_create_data;
                        self.spawn_osc_from_creation_data();
                    },
                    Err(e) => {
                        log::error!("\"{}\" is not a valid IP-Address. Rust error: \"{}\"",self.recv_ip,  e);
                        self.handle_display_popup(format!("\"{}\" is not a valid IP-Address", self.recv_ip), &e, "Error Parsing IP-Address")
                    }
                }
            }
            if self.osc_thread.is_some() && ui.button("Disconnect").clicked() {
                self.stop_osc();
            }
            ui.checkbox(&mut self.auto_connect_launch, "Auto-Connect on Launch");
        });
        ui.add_space(10.);
    }
}

impl<'a> eframe::App for App<'a> {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.data.quit_to_tray && ctx.input(|v|v.viewport().close_requested()) {
            *self.quit.lock() = crate::State::Quitting;
        }
        self.check_osc_thread(ctx);
    }
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            //create immutable copies
            let dex_protect_enabled = self.dex_protect_enabled;
            let osc_multiplexer_enabled = self.osc_multiplexer_enabled;
            let logs_visible = self.logs_visible;
            let mut strip_builder = egui_extras::StripBuilder::new(ui);
            strip_builder = strip_builder.size(egui_extras::Size::exact(130.))
                .size(egui_extras::Size::exact(25.));
            if dex_protect_enabled {
                strip_builder = strip_builder.size(egui_extras::Size::exact(80.));
            }
            if osc_multiplexer_enabled {
                strip_builder = strip_builder.size(egui_extras::Size::exact(90.));
            }
            if logs_visible {
                //FIXME(egui_tracing): Using a Size of Remaining causes issue:
                // - https://github.com/grievouz/egui_tracing/issues/48
                // - https://github.com/grievouz/egui_tracing/issues/47
                strip_builder = strip_builder.size(egui_extras::Size::exact(500.));
            }
            strip_builder.vertical(|mut strip| {
                strip.cell(|ui|{
                    self.osc_control_ui(ui);
                });
                strip.cell(|ui| {
                    ui.horizontal(|ui|{
                        if ui.button(if self.logs_visible {"Hide Logs"} else { "Show Logs"}).clicked() {
                        self.logs_visible = !self.logs_visible;
                        }
                        ui.checkbox(&mut self.dex_protect_enabled, "Enable DexProtectOSC");
                        ui.checkbox(&mut self.osc_multiplexer_enabled, "Enable Osc Multiplexer (allows for multiple Osc send applications) ");
                    });
                });
                if dex_protect_enabled {
                    strip.cell(|ui|{
                        self.dex_protect_ui(ui);
                    });
                }
                if osc_multiplexer_enabled {
                    strip.cell(|ui|{
                        self.multiplexer_ui(ui);
                    });
                }
                if logs_visible {
                    strip.cell(|ui|{
                        ui.add(egui_tracing::Logs::new(self.collector.clone()));
                    });
                }
            });

        });

        if let Some(mut popup) = self.osc_multiplexer_port_popup.take() {
            if popup(self, ui, frame) {
                self.osc_multiplexer_port_popup = Some(popup);
            }
        }
        self.popups = core::mem::take(&mut self.popups).into_iter().filter_map(|mut popup|{
            if popup(self, ui, frame) {
                Some(popup)
            }else{
                None
            }
        }).collect();
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage,eframe::APP_KEY, &self.data)
    }
}
impl<'a> Drop for App<'a> {
    fn drop(&mut self) {
        if let Some(jh) = self.stop_osc() {
            match self.runtime.block_on(jh) {
                Ok(Ok(())) => {},
                Ok(Err(e)) => {
                    log::error!("OSC thread reported an error: {e}");
                },
                Err(e) => {
                    log::error!("Osc Thread Panicked whilst stopping: {e}");
                }
            }
        }
        if let Some(jh) = self.file_picker_thread.take() {
            jh.abort();
            match self.runtime.block_on(jh) {
                Ok(_) => {},
                Err(e) => {
                    log::error!("File Picker Thread Panicked whilst stopping: {e}");
                }
            }
        }
    }
}
type PopupFunc<'a> = dyn FnMut(&'_ mut App,&'_ mut egui::Ui, &'_ mut eframe::Frame) -> bool + 'a + Send;

fn get_id() -> u64 {
    static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}

fn popup_creator<'a>(
    title: impl Into<egui::WidgetText> + 'a,
    add_content: impl FnMut(&mut App, &mut egui::Ui) + 'a + Send,
) -> Box<PopupFunc<'a>> {
    popup_creator_collapsible(title, false, add_content)
}

fn popup_creator_collapsible<'a>(
    title: impl Into<egui::WidgetText> + 'a,
    collapsible: bool,
    mut add_content: impl FnMut(&mut App, &mut egui::Ui) + 'a + Send,
) -> Box<PopupFunc<'a>> {
    let title = title.into();
    let id = get_id();
    let mut open = true;
    Box::new(move |app:&'_ mut App, ui: &'_ mut egui::Ui, _: &'_ mut eframe::Frame| {
        egui::Window::new(title.clone())
            .resizable(false)
            .collapsible(collapsible)
            .open(&mut open)
            .id(egui::Id::new(id))
            .show(ui.ctx(), |ui|add_content(app,ui));
        open
    })
}
