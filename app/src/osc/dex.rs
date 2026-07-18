use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::ops::{Deref, DerefMut, Index, Shr};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use aes::cipher::{BlockModeDecrypt, KeyIvInit};
use rosc::{OscBundle, OscMessage, OscPacket, OscType};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use unicode_bom::Bom;
use super::OscSender;
use super::OscCreateData;

const DEX_KEY_WAIT_MS:u64 = 1_500;
const DEX_KEY_WAIT_RETRIES:u64 = 5;
const DEX_KEY_WAIT_DESC:&'static str = const {
    const fn get_fractionals(wait_ms: u64) -> u64 {
        let mut fractionals = wait_ms%1000;
        while fractionals % 10 == 0 {
            fractionals /= 10;
        }

        fractionals
    }
    const SECONDS:u64 = DEX_KEY_WAIT_MS/1000;
    const FRACTIONAL:u64 = get_fractionals(DEX_KEY_WAIT_MS);

    const_format::formatc!("{SECONDS}.{FRACTIONAL} seconds")
};

const DEX_KEY_MAX_WAIT_DESC:&'static str = const {
    const fn get_fractionals(wait_ms: u64) -> u64 {
        let mut fractionals = wait_ms%1000;
        while fractionals % 10 == 0 {
            fractionals /= 10;
        }

        fractionals
    }
    const SECONDS:u64 = DEX_KEY_WAIT_MS*DEX_KEY_WAIT_RETRIES/1000;
    const FRACTIONAL:u64 = get_fractionals(DEX_KEY_WAIT_MS*DEX_KEY_WAIT_RETRIES);

    const_format::formatc!("{SECONDS}.{FRACTIONAL} seconds")
};

type Detect = Option<(tokio::sync::oneshot::Sender<()>, JoinHandle<()>)>;
#[derive(Debug)]
pub(super) struct DexOscHandler {
    path: Arc<std::path::Path>,
    dex_use_bundles: bool,
    osc: OscSender,
    key_params_outstanding_confirmations: arc_swap::ArcSwap<HashMap<String, OscType>>,
    key_params: arc_swap::ArcSwapOption<HashMap<String, OscType>>,
    current_params: arc_swap::ArcSwap<HashMap<Arc<str>, OscType>>,
    detect: Mutex<Detect>
}

impl DexOscHandler {
    pub fn new(osc_create_data: &OscCreateData, osc: OscSender) -> Self {
        Self {
            path: Arc::from(osc_create_data.path.clone()),
            dex_use_bundles: osc_create_data.dex_use_bundles,
            osc,
            key_params_outstanding_confirmations: arc_swap::ArcSwap::new(Arc::new(HashMap::new())),
            key_params: arc_swap::ArcSwapOption::empty(),
            current_params: arc_swap::ArcSwap::new(Arc::new(HashMap::new())),
            detect: Mutex::new(None)
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ArcDexOscHandler(pub Arc<DexOscHandler>);
impl From<DexOscHandler> for ArcDexOscHandler {
    fn from(value: DexOscHandler) -> Self {
        Self(Arc::new(value))
    }
}
impl Deref for ArcDexOscHandler {
    type Target = Arc<DexOscHandler>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for ArcDexOscHandler {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(feature = "compile_time_key_include")]
static KEYS: phf::Map<&'static str, &'static [u8]> = ::app_macro::include_tree!("../../../keys");

impl<I> network_handler::ArbitraryHandler<&'_ [&'_ OscMessage], I> for ArcDexOscHandler
{
    type Output = Vec<Pin<Box<dyn Future<Output = ()> + Send>>>;
    fn handle(&mut self, message: &'_ [&'_ OscMessage], _: I) -> Self::Output {
        let mut out = Vec::new();
        for message in message {
            if message.addr.eq_ignore_ascii_case(super::VRCHAT_AVATAR_CHANGE) {
                let mut id = None;
                for i in &message.args{
                    match i {
                        OscType::String(s) => {
                            if id.is_none(){
                                id = Some(s);
                            }else{
                                unrecognized_avatar_change(&message.args);
                                continue;
                            }
                        }
                        _ => {
                            unrecognized_avatar_change(&message.args);
                            continue;
                        }
                    }
                }
                if let Some(id) = id {
                    log::info!("Got Avatar Change to {id}");
                    let clone = self.clone();
                    out.push(Box::pin(clone.handle_avatar_change_osc(Arc::from(id.as_str()))) as Pin<Box<dyn Future<Output = ()> + Send>>);
                }else{
                    log::error!("No avatar id was found for the '/avatar/change' message. This is unexpected and might be a change to VRChat's OSC messages.");
                }
            } else if message.addr.starts_with("/avatar/parameters/") {
                if message.args.len() > 1 {
                    log::error!("An Avatar Key parameter at the path '{}' was set to multiple values. Currently this is unexpected. Values: {:?}", message.addr, message.args);
                }
                let first = match message.args.get(0) {
                    None => {
                        log::error!("An Avatar Key parameter at the path '{}' was set to no values. Currently this is unexpected.", message.addr);
                        continue;
                    }
                    Some(v) => v.clone(),
                };

                self.current_params.rcu(|p|{
                    let mut map = HashMap::clone(p);
                    map.insert(Arc::from(message.addr.as_str()), first.clone());
                    map
                });
                let params = self.key_params_outstanding_confirmations.load();
                match params.get(&message.addr) {
                    None => {
                        #[cfg(all(debug_assertions, feature="debug_log"))]
                        {
                            log::trace!("Got a non-avatar-key parameter set: {}", message.addr);
                        }
                    }
                    Some(val) => {
                        if first != *val {
                            #[cfg(not(all(debug_assertions, feature="debug_log")))]
                            log::error!("An Avatar Key parameter at the path '{}' was set to a different value or type than the key", message.addr);
                            #[cfg(all(debug_assertions, feature="debug_log"))]
                            log::error!("An Avatar Key parameter at the path '{}' was set to a different value or type than the key. (was {first:?}, expected {val:?})", message.addr);
                        } else {
                            {
                                drop(params);
                                self.key_params_outstanding_confirmations.rcu(|p|{
                                    let mut map = HashMap::clone(p);
                                    map.remove(&message.addr);
                                    map
                                });
                            }
                            #[cfg(not(all(debug_assertions, feature="debug_log")))]
                            log::debug!("Got a avatar-key parameter set");
                            #[cfg(all(debug_assertions, feature="debug_log"))]
                            log::debug!("Got a avatar-key parameter set: {message:?}");
                        }
                    }
                }
                let params = self.key_params.load();
                match params.as_ref() {
                    Some(params) => {
                        match params.get(&message.addr) {
                            None => {
                                #[cfg(all(debug_assertions, feature="debug_log"))]
                                {
                                    log::trace!("Got a non-avatar-key parameter set: {}", message.addr);
                                }
                            }
                            Some(val) => {
                                if first != *val {
                                    #[cfg(not(all(debug_assertions, feature="debug_log")))]
                                    log::error!("An Avatar Key parameter at the path '{}' was set to a different value or type than the key", message.addr);
                                    #[cfg(all(debug_assertions, feature="debug_log"))]
                                    log::error!("An Avatar Key parameter at the path '{}' was set to a different value or type than the key. (was {first:?}, expected {val:?})", message.addr);
                                } else {
                                    #[cfg(not(all(debug_assertions, feature="debug_log")))]
                                    log::debug!("Got a avatar-key parameter set");
                                    #[cfg(all(debug_assertions, feature="debug_log"))]
                                    log::debug!("Got a avatar-key parameter set: {message:?}");
                                }
                            }
                        }
                    }
                    None => {}
                }

            }else{
                #[cfg(all(debug_assertions, feature="debug_log"))]
                log::trace!("Uninteresting OSC Message for DexProtect: {:?}", message)
            }

        }

        out
    }
}
impl Drop for DexOscHandler {
    fn drop(&mut self) {
        let detect = self.detect.get_mut();
        tracing::debug!("DexOscHandler got dropped");
        if let Some((tx, jh)) = detect.take() {
            let span = tracing::warn_span!("DexOscHandler wasn't stopped, before it being dropped! Trying to stop it.");
            let _span = span.enter();
            let _ = tx.send(());
            jh.abort();
        }
    }
}
impl ArcDexOscHandler {
    async fn handle_avatar_change_osc(self, id: Arc<str>) {
        let names = match &self.osc{
            #[cfg(feature = "oscquery")]
            OscSender::OscQuery { query, ..} => {
                match query.get_parameter(super::VRCHAT_AVATAR_CHANGE, super::ALL_VRCHAT_CLIENTS).await {
                    Ok(v) => Some(v.into_iter()
                        .filter_map(|(name, node)|{
                            match node.value.unwrap_or_default().get(0) {
                                Some(vrchat_osc::models::OscValue::String(v)) => {
                                    if v.as_str() == id.as_ref() {
                                        Some(Arc::<str>::from(name))
                                    } else {
                                        None
                                    }
                                }
                                _ => None
                            }
                        }).collect()),
                    Err(err) => {
                        log::warn!("Failed to get current avatar information from vrchat clients: {err}");
                        return;
                    }
                }
            }
            _ => None
        };
        self.handle_avatar_change(id, names).await
    }
    pub async fn stop(&self) -> tokio::sync::MutexGuard<'_, Detect> {
        log::debug!("Stopping previous Key-Apply thread and deleting previous Key info");
        self.key_params_outstanding_confirmations.store(Arc::new(HashMap::new()));
        log::debug!("Deleted Outstanding Key-Information that hasn't yet been applied.");
        self.key_params.store(None);
        log::debug!("Deleted Key-Information.");
        let mut detect = self.detect.lock().await;
        if let Some((tx, jh)) = detect.take() {
            if let Err(_) = tx.send(()) {
                jh.abort();
            }
            match jh.await {
                Ok(()) => {},
                Err(err) => {
                    tracing::error!("Failed to join Avatar Unlock Watcher, because the thread panicked?: {err}")
                }
            }
        }
        log::debug!("Stopped previous Key-Apply thread");
        detect
    }
    pub async fn handle_avatar_change(self, id: Arc<str>, names: Option<Arc<[Arc<str>]>>) {
        let mut detect = self.stop().await;
        let mut params = HashMap::new();
        let mut key = Vec::new();
        {
            let potentially_decrypted = {
                #[cfg(feature = "compile_time_key_include")]
                {
                    KEYS.get(&*id).map(|v|v.to_vec())
                }
                #[cfg(not(feature = "compile_time_key_include"))]
                {
                    None
                }
            };
            let potentially_decrypted = match potentially_decrypted {
                Some(v) => v,
                None => {
                    let mut path = self.path.to_path_buf();
                    if path.file_name().is_some() {
                        path.push(id.as_ref());
                    }
                    path.set_file_name(id.as_ref());
                    path.set_extension("key");
                    match tokio::fs::read(path.as_path()).await{
                        Ok(v) => v,
                        Err(e) => {
                            if e.kind() == std::io::ErrorKind::NotFound{
                                log::info!("No key detected for avatar ID {id} at {}, not unlocking.\nAssuming that the following error actually means the file doesn't exist and not just a directory along the way:\n {e}", path.display());
                                return;
                            }
                            log::error!("Failed to read the Avatar id '{}' from the Avatar Folder: {}.", id, e);
                            return;
                        }
                    }
                }
            };

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
            #[cfg(all(debug_assertions, feature="debug_log"))]
            log::debug!("Decoded Avatar id '{}' Key file: '{}'", id, decoded);
            decoded = decoded.replace(",", ".");
            #[cfg(all(debug_assertions, feature="debug_log"))]
            log::debug!("Decoded Avatar id '{}' post processed Key file: '{}'", id, decoded);
            // #[cfg(not(windows))] //Todo: Is this all os's aside from windows or just a unix/linux thing?
            let decoded = if let Some(new) = decoded.strip_suffix("\x02\x02") {
                #[cfg(all(debug_assertions, feature="debug_log"))] //TODO: Why does this happen?
                log::warn!("Keyfile has a suspicious 0x0202 at the end of the keyfile. Removing.");
                new
            } else {
                decoded.as_str()
            };
            let split:Vec<&str> = decoded.split("|").collect();
            let len = if split.len()%2 == 0 {
                split.len()
            }else{
                log::error!("Found an uneven amount of keys in the Avatar id '{id}' key file.\n This is highly unusual and suggests corruption in the key file. \n You should suggest reporting this in the Discord for DexProtect.\n All bets are off from here on out, if unlocking will actually work.");
                split.len()-1
            };
            params.reserve(len/2);
            key.reserve_exact(len/2);
            let mut i = 0;
            while i < len {
                let string = format!("/avatar/parameters/{}", split[i+1]);
                let type_;
                let float = split[i];
                if let Some(index) = float.find("."){
                    #[cfg(all(debug_assertions, feature="debug_log"))]
                    log::trace!("Decoding float: {string}:{float}");
                    let (whole_str, part_str) = float.split_at(index);
                    let mut part_string = part_str.to_string();
                    part_string.remove(0);
                    #[cfg(all(debug_assertions, feature="debug_log"))]
                    log::trace!("Decoding float: {}, whole: {}, part:{}", float,whole_str, part_string);
                    let whole = match decode_number(whole_str, &id){
                        Some(v) => v,
                        None => return
                    };
                    let part = match decode_number(part_string.as_str(), &id){
                        Some(v) => v,
                        None => return
                    };
                    let part_digits = part_string.len() as u32;

                    let amount = whole as f32 + part as f32/(10.0f32.powf(part_digits as f32));
                    type_ = OscType::Float(amount);
                }else {
                    #[cfg(all(debug_assertions, feature="debug_log"))]
                    log::trace!("Decoding int: {string}:{float}");
                    let whole = match decode_number(float, &id){
                        Some(v) => v,
                        None => return
                    };
                    let part = 0;
                    let part_digits = 0;
                    let amount = whole as f32 + part as f32/(10.0f32.powf(part_digits as f32));

                    // type_ = OscType::Int(whole.cast_signed());
                    type_ = OscType::Float(amount);
                }
                params.insert(string.clone(), type_.clone());
                let msg = OscPacket::Message(OscMessage{
                    addr: string.clone(),
                    args: vec![type_],
                });
                key.push(msg);
                i+=2;
            }
        }
        {
            let mut js = tokio::task::JoinSet::new();
            send_key(&mut js, self.osc.clone(), key, names.clone(), self.dex_use_bundles);
            wait_all_js(&mut js).await;
        }
        log::info!("A Key for the Avatar id '{}' was detected and decoded. The Avatar has been attempted to be Unlocked.", id);
        params.shrink_to_fit();
        let params = Arc::new(params);
        self.key_params_outstanding_confirmations.store(params.clone());
        self.key_params.store(Some(params.clone()));
        let slf = self.clone();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        let jh = tokio::task::spawn(async move {
            let mut js = tokio::task::JoinSet::new();
            let mut apply_success = false;
            let mut apply_tries = 0;
            macro_rules! success {
                () => {
                    if !apply_success {
                        slf.key_params_outstanding_confirmations.store(Arc::new(HashMap::new()));
                        #[allow(unused_assignments)]
                        {
                            apply_success = true;
                        }
                    }
                }
            }
            macro_rules! reapply_key {
                () => {
                    let mut key = Vec::new();
                    {
                        let params = slf.key_params_outstanding_confirmations.load();
                        let params_ref = &*params;

                        if params_ref.is_empty() {
                            drop(params);
                            log::trace!("All Avatar Keys have been supplied after {apply_tries}*{DEX_KEY_WAIT_DESC}.");
                            log::debug!("Osc DexProtect Key-Apply thread finished. Exiting.");
                            success!();
                            return;
                        }

                        let len = params_ref.len();
                        key.reserve_exact(len);
                        for (name, type_) in params_ref.iter() {
                            key.push(OscPacket::Message(OscMessage{
                                addr: name.clone(),
                                args: vec![type_.clone()],
                            }))
                        }

                        #[cfg(all(debug_assertions, feature="debug_log"))]
                        {
                            let params = params_ref.iter()
                                .map(|(k, v)|format!("\r\n\t{k}\t{v:?}"))
                                .collect::<String>();
                            log::error!("The Avatar Key has not been fully applied after {apply_tries}*{DEX_KEY_WAIT_DESC}. There are {len} avatar keys, that were not applied. {params}");
                        }
                        #[cfg(not(all(debug_assertions, feature="debug_log")))]
                        {
                            log::error!("The Avatar Key has not been fully applied after {apply_tries}*{DEX_KEY_WAIT_DESC}. There are {len} avatar keys, that were not applied.");
                        }
                    }

                    if !key.is_empty() {
                        send_key(&mut js, slf.osc.clone(), key, names.clone(), slf.dex_use_bundles);
                        wait_all_js(&mut js).await;
                    }
                };
            }
            let mut timer = tokio::time::interval(tokio::time::Duration::from_millis(DEX_KEY_WAIT_MS));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop{
                tokio::select! {
                    biased;
                    _ = &mut rx => {
                        log::debug!("Osc DexProtect Key-Apply thread got terminate signal. Exiting.");
                        break;
                    }
                    _ = timer.tick() => {
                        if !apply_success {
                            if apply_tries <= DEX_KEY_WAIT_RETRIES {
                                apply_tries += 1;
                                reapply_key!();
                            } else {
                                log::error!("Giving up on unlocking after {DEX_KEY_MAX_WAIT_DESC}.");
                            }
                            continue;
                        }

                        let outstanding_params = {
                            let current_params = slf.current_params.load();
                            let out = params.iter()
                                .filter(|(key, value)|current_params.get(key.as_str()) != Some(value))
                                .map(|(k, v)|(k.clone(), v.clone()))
                                .collect::<HashMap<_, _>>();
                            drop(current_params);
                            out
                        };

                        if !outstanding_params.is_empty() {
                            log::info!("Detected avatar parameters deviating from avatar Key. Re-Unlocking!");
                            slf.key_params_outstanding_confirmations.store(Arc::new(outstanding_params));
                            apply_success = false;
                            apply_tries = 0;
                            reapply_key!();
                        }
                    }
                }
            }
        });
        *detect = Some((tx, jh));

        log::debug!("Initial Avatar Change handling done")
    }
}

fn send_key(js: &mut tokio::task::JoinSet<anyhow::Result<()>>, osc: OscSender, key: Vec<OscPacket>, names: Option<Arc<[Arc<str>]>>, use_bundles: bool) {
    if use_bundles {
        log::warn!("You are using Osc Bundles. This can cause issues with newer style keys and VRChat.\nSee https://feedback.vrchat.com/bug-reports/p/inconsistent-handling-of-osc-packets-inside-osc-bundles-and-osc-packages .");
        js.spawn(osc.send(OscPacket::Bundle(OscBundle{
            timetag: rosc::OscTime{
                seconds: 0,
                fractional: 1
            },
            content: key.clone()
        }), names));
    } else {
        for msg in key {
            let osc = osc.clone();
            js.spawn(osc.send(msg, names.clone()));
        }
    }
}
async fn wait_all_js(js: &mut tokio::task::JoinSet<anyhow::Result<()>>) {
    while let Some(v) = js.join_next().await {
        match v{
            Ok(Ok(_)) => {}
            Ok(Err(err)) => {
                log::error!("Failed to send osc message: {err}");
            }
            Err(err) => {
                log::error!("Panicked whilst sending data: {err}");
            }
        }
    }
}

#[derive(Copy, Clone, Debug, thiserror::Error)]
enum DecryptError{
    #[error("DecryptError:InvalidLength({0})")]
    InvalidLength(#[from] aes::cipher::InvalidLength),
    #[error("DecryptError:UnpadError({0})")]
    UnpadError(#[from] aes::cipher::block_padding::Error),
}

//Sorry for those people wanting to build this themselves.
//If I were to commit the Key and IV, it would defeat the entire purpose.
//Consider this a crackme challenge, under the terms that you do not redistribute those keys.
//
//Here is a checksum, to see if you have the same file as me:
//openssl dgst -sha3-512 app/src/osc/dex_key.rs
// SHA3-512(app/src/osc/dex_key.rs)= 7c5bc2a6fbf44e13010c2e1f14e215266b4a27737eebab46512f695b174481a5c9ea7a224ead0009d4e0b12f12f4cf2e2b385c5da9434f324ab024fb74db7036
//
//And here is the template of the dex_key.rs file. Just fill in the missing ??.
/*
#[allow(dead_code)]
const KEY: [u8; 32] = [0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??];
#[allow(dead_code)]
const IV: [u8;16] = [0x??u8, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??, 0x??];
*/
#[cfg(not(feature = "no_decryption_keys"))]
include!("dex_key.rs");
#[cfg(feature = "no_decryption_keys")]
const KEY: [u8; 32] = [0; 32];
#[cfg(feature = "no_decryption_keys")]
const IV: [u8;16] = [0; 16];


fn decrpyt(mut file: Vec<u8>) -> (Vec<u8>, Option<DecryptError>) {
    match cbc::Decryptor::<aes::Aes256>::new_from_slices(
            &KEY,
            &IV
        ).map_err(DecryptError::from)
        .and_then(|aes|aes.decrypt_padded::<cbc::cipher::block_padding::Pkcs7>(file.as_mut_slice()).map_err(DecryptError::from)) {
        Ok(_) => (file, None),
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
