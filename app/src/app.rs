use std::collections::VecDeque;
use std::convert::Infallible;
use std::fmt::{Debug, Formatter};
use std::ops::IndexMut;
use std::path::PathBuf;
use std::str::FromStr;
use egui::Widget;
use serde_derive::{Deserialize, Serialize};
use tokio::time::Instant;
use crate::get_runtime;
use crate::osc::OscCreateData;

#[derive(Deserialize, Serialize)]
#[serde(default)]
pub struct App<'a>{
    logs_visible: bool,
    #[serde(skip)]
    collector:egui_tracing::EventCollector,
    auto_connect_launch: bool,
    ip:String,
    path:String,
    #[cfg(all(feature = "file_dialog", not(target_arch = "wasm32")))]
    #[serde(skip)]
    file_picker_thread: Option<tokio::task::JoinHandle<Option<PathBuf>>>,
    dex_use_bundles: bool,
    osc_recv_port: u16,
    osc_send_port: u16,
    max_message_size: usize,
    osc_multiplexer_enabled: bool,
    osc_multiplexer_parse_packets: bool,
    dex_protect_enabled: bool,
    osc_multiplexer_rev_port: Vec<u16>,
    #[serde(skip)]
    osc_multiplexer_port_popup: Option<Box<PopupFunc<'a>>>,
    #[serde(skip)]
    osc_thread: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
    #[serde(skip)]
    osc_join_set: Option<tokio::task::JoinSet<Infallible>>,
    osc_create_data: OscCreateData,
    #[serde(skip)]
    popups: VecDeque<Box<PopupFunc<'a>>>,
}
impl<'a> Debug for App<'a>{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("App");
        debug.field("logs_visible", &self.logs_visible)
            .field("collector",&self.collector)
            .field("auto_connect_launch",&self.auto_connect_launch)
            .field("ip", &self.ip)
            .field("path", &self.path);
        #[cfg(all(feature = "file_dialog", not(target_arch = "wasm32")))]
        debug.field("file_picker_thread.is_some()", &self.file_picker_thread.is_some());
        debug
            .field("dex_use_bundles", &self.dex_use_bundles)
            .field("osc_recv_port", &self.osc_recv_port)
            .field("osc_send_port", &self.osc_send_port)
            .field("max_message_size", &self.max_message_size)
            .field("osc_multiplexer_enabled", &self.osc_multiplexer_enabled)
            .field("dex_protect_enabled", &self.dex_protect_enabled)
            .field("osc_multiplexer_rev_port", &self.osc_multiplexer_rev_port)
            .field("osc_thread", &self.osc_thread)
            .field("osc_join_set", &self.osc_join_set)
            .field("osc_create_data", &self.osc_create_data)
            .field("popups.len()", &self.popups.len())
            .finish()
    }
}
impl<'a> Default for App<'a>{
    fn default() -> Self {
        Self{
            logs_visible: false,
            collector:egui_tracing::EventCollector::new(),
            auto_connect_launch: true,
            ip:"127.0.0.1".to_string(),
            path: "".to_string(),
            #[cfg(all(feature = "file_dialog", not(target_arch = "wasm32")))]
            file_picker_thread: None,
            dex_use_bundles: false,
            osc_recv_port: crate::osc::OSC_RECV_PORT,
            osc_send_port: crate::osc::OSC_SEND_PORT,
            max_message_size: osc_handler::OSC_RECV_BUFFER_SIZE,
            osc_multiplexer_enabled: false,
            osc_multiplexer_parse_packets: false,
            dex_protect_enabled: true,
            osc_multiplexer_rev_port: Vec::new(),
            osc_multiplexer_port_popup: None,
            osc_thread: None,
            osc_join_set: None,
            osc_create_data: OscCreateData::default(),
            popups: VecDeque::new(),
        }
    }
}

impl<'a> TryFrom<&App<'a>> for OscCreateData {
    type Error = std::net::AddrParseError;

    fn try_from(value: &App<'a>) -> Result<Self, Self::Error> {
        Ok(OscCreateData{
            ip: std::net::IpAddr::from_str(value.ip.as_str())?,
            recv_port: value.osc_recv_port,
            send_port: value.osc_send_port,
            max_message_size: value.max_message_size,
            dex_protect_enabled: value.dex_protect_enabled,
            dex_use_bundles: value.dex_use_bundles,
            path: PathBuf::from(&value.path),
            osc_multiplexer_rev_port: if value.osc_multiplexer_enabled {value.osc_multiplexer_rev_port.clone()} else {Vec::new()},
            osc_multiplexer_parse_packets: value.osc_multiplexer_parse_packets,
        })
    }
}

impl<'a> App<'a> {
    /// Called once before the first frame.
    pub fn new(collector: egui_tracing::EventCollector, cc: &eframe::CreationContext<'_>) -> Self {
        // This is also where you can customize the look and feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

        // Load previous app state (if any).
        // Note that you must enable the `persistence` feature for this to work.

        let mut slf:App = if let Some(storage) = cc.storage {
            eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default()
        }else {
            Default::default()
        };

        #[cfg(not(debug_assertions))]
        log::info!("You are running a release build. Some log statements were disabled.");
        slf.collector = collector;
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

    fn spawn_osc_from_creation_data(&mut self){
        log::info!("Trying to connect to OSC on IP '{}'", self.osc_create_data.ip);
        let osc_create_data = self.osc_create_data.clone();
        self.osc_thread = Some(tokio::spawn(async move {
            let mut js = crate::osc::create_and_start_osc(&osc_create_data).await?;
            log::info!("Successfully connected to OSC and started all Handlers.");
            loop{
                match js.join_next().await {
                    Some(Ok(_)) => {
                        log::error!("Joined a Task that should never finish. This should never happen.\nIs there a bug in the rust language, or is the developer just stupid?");
                    },
                    Some(Err(e)) => {
                        log::error!("Panic in OSC Thread: {}", e);
                        return Err(std::io::Error::new(std::io::ErrorKind::Other,e))
                    },
                    None => return Ok(()),
                }
            }
        }));
    }

    fn check_osc_thread(&mut self){
        if let Some(osc_thread) = self.osc_thread.take() {
            if osc_thread.is_finished(){
                match get_runtime().block_on(osc_thread){
                    Ok(Ok(())) => {
                        log::error!("OSC Thread finished unexpectedly");
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
                        self.handle_display_popup("Osc Error:", &e, "Error in Osc");
                    }
                    Err(e) => {
                        log::error!("Panic in OSC Thread: {}", e);
                        self.handle_join_error(&e, "Critical Error in Osc");
                    }
                }
            }else{
                self.osc_thread = Some(osc_thread);
            }
        }
    }
    fn dex_protect_ui(&mut self, ui:&mut egui::Ui){
        ui.heading("DexProtect:");
        ui.horizontal(|ui|{
            ui.checkbox(&mut self.dex_use_bundles, "Use Osc Bundles: ");
            ui.hyperlink_to("This is known to cause issues with VRChat.", "https://feedback.vrchat.com/bug-reports/p/inconsistent-handling-of-osc-packets-inside-osc-bundles-and-osc-packages");
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
                    self.file_picker_thread = Some(get_runtime().spawn(async{
                        rfd::AsyncFileDialog::new()
                            .pick_folder()
                            .await
                            .map(|f|f.path().to_path_buf())
                    }));
                }
                if let Some(file_picker_thread) = self.file_picker_thread.take(){
                    if file_picker_thread.is_finished(){
                        match get_runtime().block_on(file_picker_thread) {
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

        ui.checkbox(&mut self.osc_multiplexer_parse_packets, "Parse Packets and Ignore Packets that can't be parsed. (it is recommended to enable this. Currently if disabled, some parts of packets might be sent more than once.)");
        if ui.add_enabled(self.osc_multiplexer_port_popup.is_none(), egui::Button::new("Manage Ports")).clicked() {
            self.osc_multiplexer_port_popup = Some(popup_creator_collapsible("Osc Multiplexer Ports:", true, |app, ui|{
                let mut i = 0;
                while i < app.osc_multiplexer_rev_port.len(){
                    ui.horizontal(|ui|{
                        ui.label(format!("Osc Forward Port {}: ", i));
                        ui.add(egui::DragValue::new(app.osc_multiplexer_rev_port.index_mut(i)));
                        if ui.button("Delete")
                            .on_hover_text("Delete this Port from the list, and replaces it with the last one.")
                            .clicked()
                        {
                            app.osc_multiplexer_rev_port.swap_remove(i);
                        }

                    });
                    i+=1;
                }
                if ui.button("Add Port").clicked() {
                    app.osc_multiplexer_rev_port.push(0);
                }
            }));
        }
        ui.add_space(10.)
    }

    fn osc_control_ui(&mut self, ui: &mut egui::Ui){
        ui.heading("Generic Osc Controls:");
        ui.horizontal(|ui|{
            ui.label("IP:");
            ui.text_edit_singleline(&mut self.ip);
        });
        ui.horizontal(|ui|{
            ui.label("OSC Receive Port:");
            ui.add(egui::DragValue::new(&mut self.osc_recv_port));
            if ui.button("Reset to Default").clicked() {
                self.osc_recv_port = crate::osc::OSC_RECV_PORT;
            }
        });
        ui.horizontal(|ui|{
            ui.label("OSC Send Port:");
            ui.add(egui::DragValue::new(&mut self.osc_send_port));
            if ui.button("Reset to Default").clicked() {
                self.osc_send_port = crate::osc::OSC_SEND_PORT;
            }
        });
        ui.horizontal(|ui|{
            ui.label("Osc Max Message Size:");
            egui::DragValue::new(&mut self.max_message_size)
                .speed(1)
                .range(1..=usize::try_from(isize::MAX).unwrap_or(usize::MAX))
                .ui(ui);
        });
        ui.label("Please note that the Settings in the Ui will only be applied after you Reconnect/Connect.");
        ui.horizontal(|ui|{
            if ui.button(if self.osc_thread.is_some() {"Reconnect"} else {"Connect"}).clicked() {
                if let Some(osc_thread) = self.osc_thread.take(){
                    log::info!("OSC Thread is already running and a Reconnect was requested. Aborting OSC thread.");
                    osc_thread.abort();
                    log::info!("OSC Thread aborted");
                }
                match OscCreateData::try_from(&*self) {
                    Ok(osc_create_data) => {
                        self.osc_create_data = osc_create_data;
                        self.spawn_osc_from_creation_data();
                    },
                    Err(e) => {
                        log::error!("\"{}\" is not a valid IP-Address. Rust error: \"{}\"",self.ip,  e);
                        self.handle_display_popup(format!("\"{}\" is not a valid IP-Address", self.ip),&e,"Error Parsing IP-Address")
                    }
                }
            }
            if self.osc_thread.is_some() && ui.button("Disconnect").clicked() {
                if let Some(osc_thread) = self.osc_thread.take(){
                    log::info!("OSC Thread is already running and a Disconnect was requested. Aborting OSC thread.");
                    osc_thread.abort();
                    log::info!("OSC Thread aborted");
                }
            }
            ui.checkbox(&mut self.auto_connect_launch, "Auto-Connect on Launch");
        });
        ui.add_space(10.);
    }
}

impl<'a> eframe::App for App<'a> {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.check_osc_thread();
        egui::CentralPanel::default().show(ctx, |ui| {
            //create immutable copies
            let dex_protect_enabled = self.dex_protect_enabled;
            let osc_multiplexer_enabled = self.osc_multiplexer_enabled;
            let logs_visible = self.logs_visible;
            let mut strip_builder = egui_extras::StripBuilder::new(ui);
            if dex_protect_enabled {
                strip_builder = strip_builder.size(egui_extras::Size::exact(80.));
            }
            if osc_multiplexer_enabled {
                strip_builder = strip_builder.size(egui_extras::Size::exact(90.));
            }
            strip_builder = strip_builder.size(egui_extras::Size::exact(130.))
                .size(egui_extras::Size::exact(25.));
            if logs_visible {
                strip_builder = strip_builder.size(egui_extras::Size::remainder());
            }
            strip_builder.vertical(|mut strip| {
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
                if logs_visible {
                    strip.cell(|ui|{
                        ui.add(egui_tracing::Logs::new(self.collector.clone()));
                    });
                }
            });

        });

        if let Some(mut popup) = self.osc_multiplexer_port_popup.take() {
            if popup(self, ctx, frame) {
                self.osc_multiplexer_port_popup = Some(popup);
            }
        }
        self.popups = core::mem::take(&mut self.popups).into_iter().filter_map(|mut popup|{
            if popup(self, ctx, frame) {
                Some(popup)
            }else{
                None
            }
        }).collect();
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage,eframe::APP_KEY, self)
    }
}
type PopupFunc<'a> = dyn FnMut(&'_ mut App,&'_ egui::Context, &'_ mut eframe::Frame) -> bool + 'a;

fn get_id() -> u64 {
    static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}

fn popup_creator<'a>(
    title: impl Into<egui::WidgetText> + 'a,
    add_content: impl FnMut(&mut App, &mut egui::Ui) + 'a,
) -> Box<PopupFunc<'a>> {
    popup_creator_collapsible(title, false, add_content)
}

fn popup_creator_collapsible<'a>(
    title: impl Into<egui::WidgetText> + 'a,
    collapsible: bool,
    mut add_content: impl FnMut(&mut App, &mut egui::Ui) + 'a,
) -> Box<PopupFunc<'a>> {
    let title = title.into();
    let id = get_id();
    let mut open = true;
    Box::new(move |app:&'_ mut App,ctx: &'_ egui::Context, _: &'_ mut eframe::Frame| {
        egui::Window::new(title.clone())
            .resizable(false)
            .collapsible(collapsible)
            .open(&mut open)
            .id(egui::Id::new(id))
            .show(ctx, |ui|add_content(app,ui));
        open
    })
}
