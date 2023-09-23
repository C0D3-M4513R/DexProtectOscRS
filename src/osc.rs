use std::collections::VecDeque;
use async_recursion::async_recursion;
use std::io;
use std::net::{AddrParseError, IpAddr, Ipv4Addr};
use std::ops::{Index, Shr};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::SystemTime;
use rosc::{OscBundle, OscMessage, OscPacket, OscType};
use serde_derive::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use unicode_bom::Bom;
use crate::app::App;

pub const OSC_RECV_PORT:u16 = 9001;
pub const OSC_SEND_PORT:u16 = 9000;
pub(crate) struct Osc {
    bundles: Vec<OscBundle>,
    osc_recv:UdpSocket,
    osc_send:UdpSocket,
    path:PathBuf,
}

impl Osc{
    pub async fn new(osc_create_data: &OscCreateData) -> io::Result<Self> {
        let osc_send = match bind_and_connect_udp(osc_create_data.ip, 0, osc_create_data.send_port,"send").await{
            Ok(v) => v,
            Err(e) => {
                log::warn!("Failed to Bind and/or connect the OSC UDP send socket: {}", e);
                Err(e)?
            }
        };
        log::info!("About to Bind OSC UDP receive Socket to {}:{}", osc_create_data.ip,osc_create_data.recv_port);
        let osc_recv = match UdpSocket::bind((osc_create_data.ip,osc_create_data.recv_port)).await{
            Ok(v) => v,
            Err(e) => {
                log::warn!("Failed to Bind and/or connect the OSC UDP receive socket: {}", e);
                Err(e)?
            }
        };
        log::info!("Bound OSC UDP receive Socket.");

        Ok(Osc{
            bundles: Vec::new(),
            osc_send,
            osc_recv,
            path: osc_create_data.path.clone()
        })
    }

    pub async fn listen(mut self) -> ! {
        loop{
            self.check_osc_bundles().await;
            let mut buf = [0u8;8192];
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
    }
    async fn send_message(&self, message: &OscPacket){
        #[cfg(debug_assertions)]
        log::trace!("Sending OSC Message: {:#?}", message);
        match rosc::encoder::encode(message) {
            Ok(v) => match self.osc_send.send(v.as_slice()).await {
                Ok(_) => return,
                Err(e) => log::error!("Failed to send a OSC Message: {}, Encoded Packet was: {:#x?}, Osc Message was: {:#?}",e,v.as_slice(), message)
            },
            Err(e) => log::error!("Failed to encode a OSC Message: {}, Packet was: {:#?}",e, message)
        }
    }

    async fn check_osc_bundles(&mut self){
        let mut i = 0;
        while i < self.bundles.len() {
            let element = self.bundles.index(i);
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

    #[async_recursion]
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
                    let float = split.index(i);
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
                        addr: format!("/avatar/parameters/{}", split.index(i+1)),
                        args: vec![OscType::Float(amount)],
                    }));
                    i+=2;
                }
                self.send_message(&OscPacket::Bundle(OscBundle{
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

async fn bind_and_connect_udp(ip:IpAddr, bind_port:u16, connect_port:u16, way:&str) -> io::Result<UdpSocket> {
    log::info!("About to Bind OSC UDP {} Socket on port {}", way,bind_port);
    let udp_sock = UdpSocket::bind((ip,bind_port)).await?;
    log::info!("Bound OSC UDP {} Socket. About to connect to {}:{}.", way,ip,connect_port);
    udp_sock.connect((ip,connect_port)).await?;
    log::info!("Connected OSC UDP {} Socket to {}:{}.", way,ip,connect_port);
    Ok(udp_sock)
}

#[derive(Debug, Clone,Serialize,Deserialize)]
pub(crate) struct OscCreateData {
    pub ip: IpAddr,
    pub recv_port:u16,
    pub send_port:u16,
    pub path: PathBuf,
}

impl Default for OscCreateData {
    fn default() -> Self {
        OscCreateData{
            ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            recv_port: OSC_RECV_PORT,
            send_port: OSC_SEND_PORT,
            path: PathBuf::new()
        }
    }
}


impl<'a> TryFrom<&App<'a>> for OscCreateData {
    type Error = AddrParseError;

    fn try_from(value: &App<'a>) -> Result<Self, Self::Error> {
        Ok(OscCreateData{
            ip: IpAddr::from_str(value.ip.as_str())?,
            recv_port: value.osc_recv_port,
            send_port: value.osc_send_port,
            path: PathBuf::from(&value.path)
        })
    }
}