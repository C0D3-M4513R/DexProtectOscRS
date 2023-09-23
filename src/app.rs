use std::collections::VecDeque;
use std::fmt::{Debug, Display, Formatter};
#[cfg(all(feature = "file_dialog", not(target_arch = "wasm32")))]
use std::path::PathBuf;
use eframe::{Frame, Storage};
use egui::{Context};
use serde_derive::{Deserialize, Serialize};
use tokio::time::Instant;
use crate::get_runtime;
use crate::osc::OscCreateData;

#[derive(Deserialize, Serialize)]
pub struct App<'a>{
    #[serde(skip)]
    collector:egui_tracing::EventCollector,
    pub(crate) auto_connect_launch: bool,
    pub(crate) ip:String,
    pub(crate) unapplied_changes:bool,
    pub(crate) path:String,
    #[cfg(all(feature = "file_dialog", not(target_arch = "wasm32")))]
    #[serde(skip)]
    file_picker_thread: Option<tokio::task::JoinHandle<Option<PathBuf>>>,
    pub(crate) osc_recv_port: u16,
    pub(crate) osc_send_port: u16,
    #[serde(skip)]
    osc_thread: Option<tokio::task::JoinHandle<Result<(),OSCError>>>,
    osc_create_data: OscCreateData,
    #[serde(skip)]
    popups: VecDeque<Box<PopupFunc<'a>>>,
}
impl<'a> Debug for App<'a>{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("App");
        debug.field("collector",&self.collector)
            .field("auto_connect_launch",&self.auto_connect_launch)
            .field("ip", &self.ip)
            .field("unapplied_changes", &self.unapplied_changes)
            .field("path", &self.path);
        #[cfg(all(feature = "file_dialog", not(target_arch = "wasm32")))]
        debug.field("file_picker_thread.is_some()", &self.file_picker_thread.is_some());
        debug.field("osc_recv_port", &self.osc_recv_port)
            .field("osc_send_port", &self.osc_send_port)
            .field("popups.len()", &self.popups.len())
            .finish()
    }
}
impl<'a> Default for App<'a>{
    fn default() -> Self {
        Self{
            collector:egui_tracing::EventCollector::new(),
            auto_connect_launch: true,
            ip:"127.0.0.1".to_string(),
            unapplied_changes: false,
            path: "".to_string(),
            #[cfg(all(feature = "file_dialog", not(target_arch = "wasm32")))]
            file_picker_thread: None,
            osc_recv_port: crate::osc::OSC_RECV_PORT,
            osc_send_port: crate::osc::OSC_SEND_PORT,
            osc_thread: None,
            osc_create_data: OscCreateData::default(),
            popups: VecDeque::new(),
        }
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
        self.popups.push_front(popup_creator(title, move |ui| {
            ui.label(label.clone());
            ui.label("Some developer information below:");
            ui.label(&error_string);
            ui.button("Close").clicked()
        }));
    }

    fn spawn_osc_from_creation_data(&mut self){
        log::info!("Trying to connect to OSC on IP '{}'", self.osc_create_data.ip);
        self.osc_thread = Some(tokio::spawn(start_osc(self.osc_create_data.clone())));
    }
}

async fn start_osc(osc_create_data: OscCreateData) -> Result<(),OSCError>{
    let osc = match crate::osc::Osc::new(&osc_create_data).await{
        Ok(v) => v,
        Err(e) => {
            return Err(OSCError::StdIo(e));
        }
    };
    osc.listen().await;

    Ok(())
}

#[derive(Debug)]
enum OSCError{
    StdIo(std::io::Error)
}
impl Display for OSCError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self{
            OSCError::StdIo(e) => write!(f,"OSCError::StdIo({})", e)
        }
    }
}

impl std::error::Error for OSCError {

}

impl<'a> eframe::App for App<'a> {
    fn update(&mut self, ctx: &Context, frame: &mut Frame) {
        if let Some(osc_thread) = self.osc_thread.take() {
            if osc_thread.is_finished(){
                match get_runtime().block_on(osc_thread){
                    Ok(Ok(())) => {
                        log::error!("OSC Thread finished unexpectedly");
                        let time = Instant::now();
                        self.popups.push_back(popup_creator(
                            "OSC Thread Exited",
                            move |ui| {
                                ui.label("The OSC Thread (the one that communicates with VRChat) exited unexpectedly.");
                                ui.label(format!("This happened {:.1} ago. (this updates only when you move your mouse or something changes)", time.elapsed().as_secs_f32()));
                                ui.button("Close").clicked()
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
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::Resize::default()
                .resizable(true)
                .min_width(ctx.screen_rect().size().x-20.)
                .max_size(egui::vec2(ctx.screen_rect().size().x-20.,f32::INFINITY))
                .show(ui,|ui|
                    ui.add(egui_tracing::Logs::new(self.collector.clone()))
                );
            ui.add_space(10.);
            ui.horizontal(|ui|{
                ui.label("IP:");
                ui.text_edit_singleline(&mut self.ip)
            }).inner;
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
                            self.file_picker_thread = Some(file_picker_thread)
                        }
                    }
                }
            });

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
        });

        let mut i = 0;
        while i < self.popups.len() {
            let popup = &mut self.popups[i];
            if popup(ctx, frame) {
                self.popups.remove(i);
            } else {
                i += 1;
            }
        }
    }

    fn save(&mut self, storage: &mut dyn Storage) {
        eframe::set_value(storage,eframe::APP_KEY, self)
    }
}
type PopupFunc<'a> = dyn Fn(&'_ egui::Context, &'_ mut eframe::Frame) -> bool + 'a;

fn get_id() -> u64 {
    static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}
fn popup_creator<'a>(
    title: impl Into<egui::WidgetText> + 'a,
    add_content: impl Fn(&mut egui::Ui) -> bool + 'a,
) -> Box<PopupFunc<'a>> {
    let title = title.into();
    let id = get_id();
    Box::new(move |ctx: &'_ egui::Context, _: &'_ mut eframe::Frame| {
        let mut clicked = false;
        egui::Window::new(title.clone())
            .resizable(false)
            .collapsible(false)
            .id(egui::Id::new(id))
            .show(ctx, |ui| {
                clicked = add_content(ui);
            });
        clicked
    })
}
