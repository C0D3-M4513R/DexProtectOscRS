use std::sync::Arc;
use rosc::OscTime;

/// An *osc packet* can contain an *osc message* or a bundle of nested messages
/// which is called *osc bundle*.
#[derive(Clone, Debug, PartialEq)]
pub enum OscPacket {
    Message(Arc<rosc::OscMessage>),
    Bundle(OscBundle),
}

/// An OSC bundle contains zero or more OSC packets
/// and a time tag. The contained packets *should* be
/// applied at the given time tag.
#[derive(Clone, Debug, PartialEq)]
pub struct OscBundle {
    pub timetag: OscTime,
    pub content: Arc<[Arc<OscPacket>]>,
}

impl From<rosc::OscBundle> for OscBundle {
    fn from(value: rosc::OscBundle) -> Self {
        Self{
            timetag: value.timetag,
            content: value.content.into_iter().map(|m|Arc::new(OscPacket::from(m))).collect()
        }
    }
}

impl From<&rosc::OscBundle> for OscBundle {
    fn from(value: &rosc::OscBundle) -> Self {
        Self{
            timetag: value.timetag,
            content: value.content.iter().map(|m|Arc::new(OscPacket::from(m))).collect()
        }
    }
}

impl From<&OscBundle> for rosc::OscBundle {
    fn from(value: &OscBundle) -> Self {
        Self{
            timetag: value.timetag,
            content: value.content.iter().map(|m|rosc::OscPacket::from(m.as_ref())).collect(),
        }
    }
}
impl From<&rosc::OscPacket> for OscPacket {
    fn from(value: &rosc::OscPacket) -> Self {
        match value {
            rosc::OscPacket::Message(v) => OscPacket::Message(Arc::new(v.clone())),
            rosc::OscPacket::Bundle(b) => OscPacket::Bundle(OscBundle::from(b)),
        }
    }
}

impl From<rosc::OscPacket> for OscPacket {
    fn from(value: rosc::OscPacket) -> Self {
        match value {
            rosc::OscPacket::Message(v) => OscPacket::Message(Arc::new(v)),
            rosc::OscPacket::Bundle(b) => OscPacket::Bundle(OscBundle::from(b)),
        }
    }
}

impl From<&OscPacket> for rosc::OscPacket {
    fn from(value: &OscPacket) -> Self {
        match value {
            OscPacket::Message(v) => rosc::OscPacket::Message(v.as_ref().clone()),
            OscPacket::Bundle(b) => rosc::OscPacket::Bundle(rosc::OscBundle::from(b)),
        }
    }
}