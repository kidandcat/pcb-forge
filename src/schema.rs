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
    #[serde(default)]
    pub options: OptionsDef,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BoardDef {
    #[serde(default = "default_aspect_ratio")]
    pub aspect_ratio: f64,
    #[serde(default = "default_layers")]
    pub layers: u32,
    #[serde(default = "default_trace_width")]
    pub trace_width: f64,
    #[serde(default = "default_clearance")]
    pub clearance: f64,
    #[serde(default)]
    pub footprint_lib: Option<String>,
}

fn default_aspect_ratio() -> f64 {
    1.0
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

/// AI-tunable options for placement and scoring.
#[derive(Debug, Clone, Deserialize)]
pub struct OptionsDef {
    /// Courtyard area multiplier for board sizing (1.5 = tight, 2.5 = spacious).
    #[serde(default = "default_density")]
    pub density: f64,
    /// Base component spacing multiplier (0.5 = tight, 2.0 = spread).
    #[serde(default = "default_spacing")]
    pub spacing: f64,
    /// Number of placement variants to generate and compare (1-50).
    #[serde(default = "default_placement_variants")]
    pub placement_variants: usize,
    /// Score penalty per mm² of board area (higher = favors smaller boards).
    #[serde(default = "default_board_penalty")]
    pub board_penalty: f64,
    /// Score penalty per mm of trace length (higher = favors shorter traces).
    #[serde(default = "default_trace_penalty")]
    pub trace_penalty: f64,
    /// Score penalty per via (higher = favors fewer layer changes).
    #[serde(default = "default_via_penalty")]
    pub via_penalty: f64,
    /// Score reward per successfully routed net.
    #[serde(default = "default_net_reward")]
    pub net_reward: f64,
}

impl Default for OptionsDef {
    fn default() -> Self {
        Self {
            density: default_density(),
            spacing: default_spacing(),
            placement_variants: default_placement_variants(),
            board_penalty: default_board_penalty(),
            trace_penalty: default_trace_penalty(),
            via_penalty: default_via_penalty(),
            net_reward: default_net_reward(),
        }
    }
}

fn default_density() -> f64 { 2.0 }
fn default_spacing() -> f64 { 1.0 }
fn default_placement_variants() -> usize { 10 }
fn default_board_penalty() -> f64 { 0.5 }
fn default_trace_penalty() -> f64 { 0.1 }
fn default_via_penalty() -> f64 { 50.0 }
fn default_net_reward() -> f64 { 1000.0 }

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

/// Runtime options (mirrors OptionsDef after parsing).
#[derive(Debug, Clone, Serialize)]
pub struct Options {
    pub density: f64,
    pub spacing: f64,
    pub placement_variants: usize,
    pub board_penalty: f64,
    pub trace_penalty: f64,
    pub via_penalty: f64,
    pub net_reward: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self::from(&OptionsDef::default())
    }
}

impl From<&OptionsDef> for Options {
    fn from(def: &OptionsDef) -> Self {
        Self {
            density: def.density,
            spacing: def.spacing,
            placement_variants: def.placement_variants,
            board_penalty: def.board_penalty,
            trace_penalty: def.trace_penalty,
            via_penalty: def.via_penalty,
            net_reward: def.net_reward,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Board {
    pub width: f64,
    pub height: f64,
    pub aspect_ratio: f64,
    pub layers: u32,
    pub trace_width: f64,
    pub clearance: f64,
    pub components: Vec<Component>,
    pub nets: Vec<Net>,
    pub options: Options,
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
