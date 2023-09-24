use std::collections::VecDeque;
use std::convert::Infallible;
use std::ops::{Index, Shr};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::SystemTime;
use rosc::{OscBundle, OscMessage, OscPacket, OscType};
use tokio::net::UdpSocket;
use unicode_bom::Bom;
use super::OscSender;
use super::OscCreateData;

pub(super) struct DexOsc {
    bundles: Vec<OscBundle>,
    osc_recv: UdpSocket,
    path:PathBuf,
    osc: Arc<OscSender>,
}

impl DexOsc {
    pub async fn new(osc_create_data: &OscCreateData, osc:Arc<OscSender>) -> std::io::Result<Self> {
        log::info!("About to Bind OSC UDP receive Socket to {}:{}", osc_create_data.ip,osc_create_data.recv_port);
        let osc_recv = match UdpSocket::bind((osc_create_data.ip,osc_create_data.recv_port)).await{
            Ok(v) => v,
            Err(e) => {
                log::warn!("Failed to Bind and/or connect the OSC UDP receive socket: {}", e);
                Err(e)?
            }
        };
        log::info!("Bound OSC UDP receive Socket.");

        Ok(DexOsc {
            bundles: Vec::new(),
            osc_recv,
            path: osc_create_data.path.clone(),
            osc
        })
    }

    pub fn listen(mut self, js:&mut tokio::task::JoinSet<Infallible>) {
        js.spawn(async move {
            loop {
                self.check_osc_bundles().await;
                let mut buf = [0u8; super::OSC_RECV_BUFFER_SIZE];
                match self.osc_recv.recv(&mut buf).await {
                    Err(e) => {
                        log::error!("Error receiving udp packet. Skipping Packet: {}",e);
                        continue;
                    }
                    Ok(size) => {
                        #[cfg(debug_assertions)]
                        log::trace!("Received UDP Packet with size {} ",size);
                        match rosc::decoder::decode_udp(&buf[..size]) {
                            Err(e) => {
                                log::error!("Error decoding udp packet into an OSC Packet: {}", e);
                                #[cfg(debug_assertions)]
                                log::trace!("Packet contents were: {:#X?}",&buf[..size]);
                                continue;
                            }
                            Ok((_, packet)) => self.handle_packet(packet).await
                        }
                    }
                };
            }
        });
    }

    async fn check_osc_bundles(&mut self){
        let mut i = 0;
        while i < self.bundles.len() {
            let element = &self.bundles[i];
            if SystemTime::from(element.timetag) < SystemTime::now() {
                let content = self.bundles.swap_remove(i).content;
                self.apply_packets(content).await;
            }else{
                i+=1;
            }
        }
    }

    async fn apply_packets(&mut self, packets:Vec<OscPacket>){
        for i in packets{
            self.handle_packet(i).await;
        }
    }

    #[async_recursion::async_recursion]
    async fn handle_packet(&mut self, packet: OscPacket){
        match packet {
            OscPacket::Message(msg) => {
                #[cfg(debug_assertions)]
                log::trace!("Got a OSC Packet: {}: {:?}", msg.addr, msg.args);
                self.handle_message(msg).await;
            }
            OscPacket::Bundle(bundle) => {
                if bundle.timetag.seconds == 0 && bundle.timetag.fractional == 1{
                    self.apply_packets(bundle.content).await;
                    return;
                }
                log::debug!("Got a OSC Bundle to be applied in {}.{}s {:?}", bundle.timetag.seconds, bundle.timetag.fractional, bundle.timetag.fractional);
                self.bundles.push(bundle);
            }
        }
    }

    async fn handle_message(&self, message: OscMessage){
        if message.addr.eq_ignore_ascii_case("/avatar/change") {
            let mut id = None;
            for i in &message.args{
                match i {
                    OscType::String(s) => {
                        if id.is_none(){
                            id = Some(s);
                        }else{
                            unrecognized_avatar_change(&message.args);
                            return;
                        }
                    }
                    _ => {
                        unrecognized_avatar_change(&message.args);
                    }
                }
            }
            if let Some(id) = id{
                self.handle_avatar_change(id).await;
            }else{
                log::error!("No avatar id was found for the '/avatar/change' message. This is unexpected and might be a change to VRChat's OSC messages.")
            }
        }
    }

    async fn handle_avatar_change(&self, id: &String) {
        let mut path = self.path.clone();
        path.push(id);
        path.set_extension("key");
        match tokio::fs::read(path).await{
            Ok(v) => {
                let mut v = match vecu8_to_str(v){
                    Some(v) => v,
                    None => {
                        log::error!("Failed to decode the Avatar id '{}' Key file. Refusing to unlock.", id);
                        return;
                    }
                };
                #[cfg(debug_assertions)]
                log::debug!("Decoded Avatar id '{}' Key file: '{}'", id, v);
                let mut key = Vec::new();
                v = v.replace(",",".");
                let split:Vec<&str> = v.split("|").collect();
                let len = if split.len()%2 == 0 {
                    split.len()
                }else{
                    log::error!("Found an uneven amount of keys in the Avatar id '{}' key file.\n This is highly unusual and suggests corruption in the key file. \n You should suggest reporting this in the Discord for DexProtect.\n All bets are off from here on out, if unlocking will actually work.", id);
                    split.len()-1
                };
                let mut i = 0;
                while i < len {
                    let float = split[i];
                    log::trace!("Decoding float: {}", float);
                    let whole:u32;
                    let part:u32;
                    let part_digits:u32;
                    if let Some(index) = float.find("."){
                        let (whole_str, part_str) = float.split_at(index);
                        let mut part_string = part_str.to_string();
                        part_string.remove(0);
                        log::trace!("Decoding float: {}, whole: {}, part:{}", float,whole_str, part_string);
                        whole = match decode_number(whole_str, id){
                            Some(v) => v,
                            None => return
                        };
                        part = match decode_number(part_string.as_str(), id){
                            Some(v) => v,
                            None => return
                        };
                        part_digits = part_string.len() as u32;
                    }else {
                        whole = match decode_number(float, id){
                            Some(v) => v,
                            None => return
                        };
                        part = 0;
                        part_digits = 0;
                    }
                    let amount = whole as f32 + part as f32/(10.0f32.powf(part_digits as f32));
                    key.push(OscPacket::Message(OscMessage{
                        addr: format!("/avatar/parameters/{}", split[i+1]),
                        args: vec![OscType::Float(amount)],
                    }));
                    i+=2;
                }
                self.osc.send_message_with_logs(&OscPacket::Bundle(OscBundle{
                    timetag: rosc::OscTime{
                        seconds: 0,
                        fractional: 1
                    },
                    content: key
                })).await;
                log::info!("Avatar Change Detected to Avatar id '{}'. Key was detected, has been decoded and the Avatar has been Unlocked.", id);
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound{
                    log::info!("No key detected for avatar ID {}, not unlocking.\nAssuming that the following error actually means the file doesn't exist and not just a directory along the way:\n {}", id, e);
                    return;
                }
                log::error!("Failed to read the Avatar id '{}' from the Avatar Folder: {}.", id, e);
            }
        }

    }
}

fn decode_number(number:&str, id:&str) -> Option<u32> {
    match u32::from_str(number){
        Ok(v) => Some(v),
        Err(e) => {
            log::error!("Error whilst decoding part of the Key for the Avatar id '{}': {}.\n Refusing to unlock.", id, e);
            None
        }
    }
}
fn vecu8_to_str(v:Vec<u8>) -> Option<String> {
    let bom = unicode_bom::Bom::from(v.as_slice());
    match bom {
        Bom::Null => {
            log::debug!("No BOM Detected. Assuming UTF-8.");
            let mut vec_deque = VecDeque::from(v);
            vec_deque.pop_front();
            vec_deque.pop_front();
            vec_deque.pop_front();
            match String::from_utf8(vec_deque.into()) {
                Ok(v) => Some(v),
                Err(_) => None,
            }
        }
        Bom::Bocu1 => None,
        Bom::Gb18030 => None,
        Bom::Scsu => None,
        Bom::UtfEbcdic => None,
        Bom::Utf1 => None,
        Bom::Utf7 => None,
        Bom::Utf8 => {
            log::debug!("Detected UTF-8 file.");
            let mut vec_deque = VecDeque::from(v);
            vec_deque.pop_front();
            vec_deque.pop_front();
            vec_deque.pop_front();
            match String::from_utf8(vec_deque.into()) {
                Ok(v) => Some(v),
                Err(_) => None,
            }
        }
        Bom::Utf16Be => {
            log::debug!("Detected UTF-16Be file.");
            let mut utf16_buf = VecDeque::from(vecu8_to_vecu16(v, true));
            utf16_buf.pop_front();
            log::debug!("Decoded {} u16 values.", utf16_buf.len());
            utf16_buf_to_str(utf16_buf.into())
        }
        Bom::Utf16Le => {
            log::debug!("Detected UTF-16Le file.");
            let mut utf16_buf = VecDeque::from(vecu8_to_vecu16(v,false));
            utf16_buf.pop_front();
            log::debug!("Decoded {} u16 values.", utf16_buf.len());
            utf16_buf_to_str(utf16_buf.into())
        }
        Bom::Utf32Be => None,
        Bom::Utf32Le => None,
    }
}
fn vecu8_to_vecu16(v:Vec<u8>, be:bool) -> Vec<u16>{
    log::debug!("Got {} bytes.", v.len());
    let mut utf16buf:Vec<u16> = Vec::new();
    let mut i = 0;
    let len = if v.len()%2 == 0 {
        v.len()
    } else {
        log::debug!("Uneven amount of bytes read from key file.");
        v.len()-1
    };
    while i < len{
        utf16buf.push(if be {(*v.index(i) as u16).shr(8) | (*v.index(i+1) as u16)} else {(*v.index(i+1) as u16).shr(8) | (*v.index(i) as u16)});
        i+=2;
    }
    if len != v.len() {
        log::info!("Reappending last byte.");
        utf16buf.push(*v.index(len) as u16);
    }
    log::debug!("Converted to {} u16 values.", utf16buf.len());
    utf16buf
}
fn utf16_buf_to_str(v:Vec<u16>) -> Option<String>{
    let mut string = String::new();
    for i in char::decode_utf16(v){
        match i {
            Ok(v)=>string.push(v),
            Err(_) => return None,
        }
    }
    return Some(string);
}
fn unrecognized_avatar_change(arg:&Vec<OscType>){
    log::error!("Received a OSC Message with the address /avatar/change but the first argument was not a string.\n This is unexpected and there might have been a change to VRChat's OSC messages.\n Extraneous Argument: {:#?}", arg);
}
