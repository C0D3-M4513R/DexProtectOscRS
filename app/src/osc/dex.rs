use std::collections::VecDeque;
use std::future::Future;
use std::ops::{Index, Shr};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use aes::cipher::KeyIvInit;
use cbc::cipher::BlockDecryptMut;
use rosc::{OscBundle, OscMessage, OscPacket, OscType};
use unicode_bom::Bom;
use super::OscSender;
use super::OscCreateData;

#[derive(Clone)]
pub(super) struct DexOscHandler {
    path: Arc<std::path::Path>,
    dex_use_bundles: bool,
    osc: Arc<OscSender>,
}

impl DexOscHandler {
    pub fn new(osc_create_data: &OscCreateData, osc: Arc<OscSender>) -> Self {
        Self {
            path: Arc::from(osc_create_data.path.clone()),
            dex_use_bundles: osc_create_data.dex_use_bundles,
            osc,
        }
    }
}

impl osc_handler::MessageHandler for DexOscHandler
{
    type Fut = futures::future::Either<core::future::Ready<Self::Output>,Pin<Box<dyn Future<Output = Self::Output> + Send>>>;
    type Output = ();

    fn handle(&mut self, message: Arc<OscMessage>) -> Self::Fut {
        if message.addr.eq_ignore_ascii_case("/avatar/change") {
            let mut id = None;
            for i in &message.args{
                match i {
                    OscType::String(s) => {
                        if id.is_none(){
                            id = Some(s);
                        }else{
                            unrecognized_avatar_change(&message.args);
                            return futures::future::Either::Left(core::future::ready(()));
                        }
                    }
                    _ => {
                        unrecognized_avatar_change(&message.args);
                        return futures::future::Either::Left(core::future::ready(()));
                    }
                }
            }
            if let Some(id) = id {
                let clone = self.clone();
                return futures::future::Either::Right(Box::pin(clone.handle_avatar_change(Arc::from(id.as_str()))))
            }else{
                log::error!("No avatar id was found for the '/avatar/change' message. This is unexpected and might be a change to VRChat's OSC messages.")
            }
        }else{
            #[cfg(debug_assertions)]
            log::trace!("Uninteresting OSC Message for DexProtect: {:?}", message)
        }
        futures::future::Either::Left(core::future::ready(()))
    }
}

impl DexOscHandler {
    async fn handle_avatar_change(self, id: Arc<str>) {
        let mut path = self.path.to_path_buf();
        if path.file_name().is_some() {
            path.push(id.as_ref());
        }
        path.set_file_name(id.as_ref());
        path.set_extension("key");
        match tokio::fs::read(path).await{
            Ok(potentially_decrypted) => {
                let (v, err) = decrpyt(potentially_decrypted);
                if let Some(err) = err {
                    log::error!("Failed to decrypt the Key for the Avatar id '{id}'. Trying to treat the key as an unencrypted legacy Key.\n Error: {err}");
                }
                let mut decoded = match vecu8_to_str(v){
                    Some(v) => v,
                    None => {
                        log::error!("Failed to decode the Avatar id '{}' Key file. Refusing to unlock.", id);
                        return;
                    }
                };
                #[cfg(debug_assertions)]
                log::debug!("Decoded Avatar id '{}' Key file: '{}'", id, decoded);
                let mut key:Vec<rosc::OscPacket> = Vec::new();
                decoded = decoded.replace(",", ".");
                #[cfg(debug_assertions)]
                log::debug!("Decoded Avatar id '{}' post processed Key file: '{}'", id, decoded);
                let split:Vec<&str> = decoded.split("|").collect();
                let len = if split.len()%2 == 0 {
                    split.len()
                }else{
                    log::error!("Found an uneven amount of keys in the Avatar id '{id}' key file.\n This is highly unusual and suggests corruption in the key file. \n You should suggest reporting this in the Discord for DexProtect.\n All bets are off from here on out, if unlocking will actually work.");
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
                        whole = match decode_number(whole_str, &id){
                            Some(v) => v,
                            None => return
                        };
                        part = match decode_number(part_string.as_str(), &id){
                            Some(v) => v,
                            None => return
                        };
                        part_digits = part_string.len() as u32;
                    }else {
                        whole = match decode_number(float, &id){
                            Some(v) => v,
                            None => return
                        };
                        part = 0;
                        part_digits = 0;
                    }
                    let amount = whole as f32 + part as f32/(10.0f32.powf(part_digits as f32));
                    if self.dex_use_bundles {
                        key.push(OscPacket::Message(OscMessage{
                            addr: format!("/avatar/parameters/{}", split[i+1]),
                            args: vec![OscType::Float(amount)],
                        }));
                    }else {
                        if let Ok(v) = self.osc.send_message_with_logs(&OscPacket::Message(OscMessage{
                            addr: format!("/avatar/parameters/{}", split[i+1]),
                            args: vec![OscType::Float(amount)],
                        })) {
                            let _ = v.await;
                        };
                    }
                    i+=2;
                }
                if self.dex_use_bundles {
                    log::warn!("You are using Osc Bundles. This can cause issues with newer style keys and VRChat.\nSee https://feedback.vrchat.com/bug-reports/p/inconsistent-handling-of-osc-packets-inside-osc-bundles-and-osc-packages .");
                    if let Ok(v) = self.osc.send_message_with_logs(&OscPacket::Bundle(OscBundle{
                        timetag: rosc::OscTime{
                            seconds: 0,
                            fractional: 1
                        },
                        content: key
                    })){
                        let _ = v.await;
                    };
                }
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

#[derive(Copy, Clone, Debug, thiserror::Error)]
enum DecryptError{
    #[error("DecryptError:InvalidLength({0})")]
    InvalidLength(#[from] aes::cipher::InvalidLength),
    #[error("DecryptError:UnpadError({0})")]
    UnpadError(#[from] aes::cipher::block_padding::UnpadError),
}

//Sorry for those people wanting to build this themselves.
//If I were to commit the Key and IV, it would defeat the entire purpose.
//Consider this a crackme challenge, under the terms that you do not redistribute those keys.
#[cfg(not(feature = "no_decryption_keys"))]
include!("dex_key.rs");
#[cfg(feature = "no_decryption_keys")]
const KEY: [u8; 32] = [0; 32];
#[cfg(feature = "no_decryption_keys")]
const IV: [u8;16] = [0; 16];


fn decrpyt(file: Vec<u8>) -> (Vec<u8>, Option<DecryptError>) {
    match cbc::Decryptor::<aes::Aes256>::new_from_slices(
            &KEY,
            &IV
        ).map_err(DecryptError::from)
        .and_then(|aes|aes.decrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(file.as_slice()).map_err(DecryptError::from)) {
        Ok(v) => (v, None),
        Err(err) => (file, Some(err)),
    }
}

fn unrecognized_avatar_change(arg:&Vec<OscType>){
    log::error!("Received a OSC Message with the address /avatar/change but the first argument was not a string.\n This is unexpected and there might have been a change to VRChat's OSC messages.\n Extraneous Argument: {:#?}", arg);
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
//        Bom::Null => {
//             log::debug!("No BOM Detected. Assuming UTF-16LE.");
//             let utf16_buf = vecu8_to_vecu16(v,false);
//             log::debug!("Decoded {} u16 values.", utf16_buf.len());
//             utf16_buf_to_str(utf16_buf)
//         }
            log::debug!("No BOM Detected. Assuming UTF-8.");
            match String::from_utf8(v.into()) {
                Ok(v) => Some(v),
                Err(_) => None,
            }
        }
        Bom::Bocu1 => None,
        Bom::Gb18030 => None,
        Bom::Scsu => None,
        Bom::UtfEbcdic => None,
        Bom::Utf1 => None,
        Bom::Utf7 => {
            //https://en.wikipedia.org/wiki/UTF-7
            //> UTF-7 has never been an official standard of the Unicode Consortium.
            //> It is known to have security issues, which is why software has been changed to disable its use.
            //> It is prohibited in HTML 5.
            //
            //And I guess so will I.
            log::debug!("Actively ignoring UTF-7 file");
            None
        },
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
        log::debug!("Uneven amount of bytes read.");
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
