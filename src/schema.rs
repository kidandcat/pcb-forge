use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::footprint::FootprintData;

#[derive(Debug, Clone, Deserialize)]
pub struct CircuitDefinition {
    pub board: BoardDef,
    #[serde(default)]
    pub components: HashMap<String, ComponentDef>,
    #[serde(default)]
    pub nets: Vec<NetDef>,
    #[serde(default)]
    pub power: Option<PowerDef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BoardDef {
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
    #[serde(default = "default_layers")]
    pub layers: u32,
    #[serde(default = "default_trace_width")]
    pub trace_width: f64,
    #[serde(default = "default_clearance")]
    pub clearance: f64,
    #[serde(default)]
    pub footprint_lib: Option<String>,
}

fn default_layers() -> u32 {
    2
}
fn default_trace_width() -> f64 {
    0.25
}
fn default_clearance() -> f64 {
    0.25
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComponentDef {
    pub footprint: String,
    pub value: String,
    #[serde(default)]
    pub lcsc: Option<String>,
    #[serde(default)]
    pub pins: HashMap<String, PinDef>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub rotation: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum PinDef {
    Numeric(u32),
    Named(String),
    Detailed(PinDetail),
}

#[derive(Debug, Clone, Deserialize)]
pub struct PinDetail {
    pub number: String,
    #[serde(default = "default_pin_type")]
    pub pin_type: PinType,
}

fn default_pin_type() -> PinType {
    PinType::Passive
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PinType {
    Input,
    Output,
    Bidirectional,
    Power,
    Passive,
    OpenCollector,
    OpenEmitter,
    NotConnected,
}

impl PinType {
    pub fn to_kicad_str(&self) -> &str {
        match self {
            PinType::Input => "input",
            PinType::Output => "output",
            PinType::Bidirectional => "bidirectional",
            PinType::Power => "power_in",
            PinType::Passive => "passive",
            PinType::OpenCollector => "open_collector",
            PinType::OpenEmitter => "open_emitter",
            PinType::NotConnected => "no_connect",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetDef {
    pub name: String,
    pub pins: Vec<String>, // "component.pin"
}

#[derive(Debug, Clone, Deserialize)]
pub struct PowerDef {
    #[serde(default)]
    pub vcc: Vec<String>,
    #[serde(default)]
    pub gnd: Vec<String>,
}

// Runtime structures after parsing
#[derive(Debug, Clone, Serialize)]
pub struct Component {
    pub ref_des: String,
    pub name: String,
    pub footprint: String,
    pub value: String,
    pub lcsc: Option<String>,
    pub pins: Vec<Pin>,
    pub description: Option<String>,
    pub footprint_data: Option<FootprintData>,
    // Placement
    pub x: f64,
    pub y: f64,
    pub rotation: f64,
    pub manually_placed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pin {
    pub name: String,
    pub number: String,
    pub pin_type: PinType,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Net {
    pub name: String,
    pub pins: Vec<PinRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PinRef {
    pub component: String,
    pub pin: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Board {
    pub width: f64,
    pub height: f64,
    pub layers: u32,
    pub trace_width: f64,
    pub clearance: f64,
    pub components: Vec<Component>,
    pub nets: Vec<Net>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    FCu,
    BCu,
    FMask,
    BMask,
    FSilkS,
    BSilkS,
    EdgeCuts,
}

impl Layer {
    pub fn name(&self) -> &str {
        match self {
            Layer::FCu => "F.Cu",
            Layer::BCu => "B.Cu",
            Layer::FMask => "F.Mask",
            Layer::BMask => "B.Mask",
            Layer::FSilkS => "F.SilkS",
            Layer::BSilkS => "B.SilkS",
            Layer::EdgeCuts => "Edge.Cuts",
        }
    }
}
