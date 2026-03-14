use anyhow::{Context, Result};
use std::path::Path;

use crate::footprint;
use crate::schema::*;

/// Default KiCad footprint library paths to try (in order).
const DEFAULT_LIB_PATHS: &[&str] = &[
    ".pcb/cache/gitlab.com/kicad/libraries/kicad-footprints/9.0.3",
    ".pcb-forge/libraries/kicad-footprints",
    ".local/share/kicad/9.0/footprints",
];

pub fn parse_circuit(path: &Path) -> Result<Board> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read circuit file: {}", path.display()))?;

    let def: CircuitDefinition =
        toml::from_str(&content).with_context(|| "Failed to parse TOML circuit definition")?;

    build_board(def)
}

fn resolve_lib_path(configured: &Option<String>) -> Option<std::path::PathBuf> {
    // 1. Use configured path if provided
    if let Some(p) = configured {
        let expanded = shellexpand(p);
        let path = std::path::PathBuf::from(&expanded);
        if path.exists() {
            return Some(path);
        }
    }

    // 2. Check KICAD_FOOTPRINT_DIR env var
    if let Ok(env_path) = std::env::var("KICAD_FOOTPRINT_DIR") {
        let path = std::path::PathBuf::from(&env_path);
        if path.exists() {
            return Some(path);
        }
    }

    // 3. Try default paths relative to home
    if let Some(home) = dirs_home() {
        for default in DEFAULT_LIB_PATHS {
            let path = home.join(default);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

fn shellexpand(p: &str) -> String {
    if p.starts_with('~') {
        if let Some(home) = dirs_home() {
            return p.replacen('~', &home.to_string_lossy(), 1);
        }
    }
    p.to_string()
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

fn build_board(def: CircuitDefinition) -> Result<Board> {
    let lib_path = resolve_lib_path(&def.board.footprint_lib);
    if let Some(ref lp) = lib_path {
        println!("  Footprint library: {}", lp.display());
    } else {
        println!("  Warning: No KiCad footprint library found. Using fallback footprints.");
    }

    let mut components = Vec::new();

    for (name, comp_def) in &def.components {
        let ref_des = generate_ref_des(&comp_def.footprint, components.len());

        let pins: Vec<Pin> = comp_def
            .pins
            .iter()
            .map(|(pin_name, pin_def)| {
                let (number, pin_type) = match pin_def {
                    PinDef::Numeric(n) => (n.to_string(), PinType::Passive),
                    PinDef::Named(s) => (s.clone(), PinType::Passive),
                    PinDef::Detailed(d) => (d.number.clone(), d.pin_type.clone()),
                };
                Pin {
                    name: pin_name.clone(),
                    number,
                    pin_type,
                    x: 0.0,
                    y: 0.0,
                }
            })
            .collect();

        // Load footprint data
        let footprint_data = load_footprint_for_component(
            &comp_def.footprint,
            &lib_path,
            &name,
            pins.len(),
        );

        // Apply real pad positions to pins
        let pins = apply_pad_positions(pins, &footprint_data);

        components.push(Component {
            ref_des,
            name: name.clone(),
            footprint: comp_def.footprint.clone(),
            value: comp_def.value.clone(),
            lcsc: comp_def.lcsc.clone(),
            pins,
            description: comp_def.description.clone(),
            footprint_data: Some(footprint_data),
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
        });
    }

    let mut nets = Vec::new();

    // Build nets from explicit net definitions
    for net_def in &def.nets {
        let pin_refs: Vec<PinRef> = net_def
            .pins
            .iter()
            .filter_map(|p| {
                let parts: Vec<&str> = p.splitn(2, '.').collect();
                if parts.len() == 2 {
                    Some(PinRef {
                        component: parts[0].to_string(),
                        pin: parts[1].to_string(),
                    })
                } else {
                    eprintln!("Warning: invalid pin reference '{}', expected 'component.pin'", p);
                    None
                }
            })
            .collect();

        nets.push(Net {
            name: net_def.name.clone(),
            pins: pin_refs,
        });
    }

    // Build power nets
    if let Some(power) = &def.power {
        if !power.vcc.is_empty() {
            let pin_refs: Vec<PinRef> = power
                .vcc
                .iter()
                .filter_map(|p| {
                    let parts: Vec<&str> = p.splitn(2, '.').collect();
                    if parts.len() == 2 {
                        Some(PinRef {
                            component: parts[0].to_string(),
                            pin: parts[1].to_string(),
                        })
                    } else {
                        None
                    }
                })
                .collect();
            nets.push(Net {
                name: "VCC3V3".to_string(),
                pins: pin_refs,
            });
        }
        if !power.gnd.is_empty() {
            let pin_refs: Vec<PinRef> = power
                .gnd
                .iter()
                .filter_map(|p| {
                    let parts: Vec<&str> = p.splitn(2, '.').collect();
                    if parts.len() == 2 {
                        Some(PinRef {
                            component: parts[0].to_string(),
                            pin: parts[1].to_string(),
                        })
                    } else {
                        None
                    }
                })
                .collect();
            nets.push(Net {
                name: "GND".to_string(),
                pins: pin_refs,
            });
        }
    }

    Ok(Board {
        width: def.board.width.unwrap_or(0.0),
        height: def.board.height.unwrap_or(0.0),
        layers: def.board.layers,
        trace_width: def.board.trace_width,
        clearance: def.board.clearance,
        components,
        nets,
    })
}

fn load_footprint_for_component(
    footprint_ref: &str,
    lib_path: &Option<std::path::PathBuf>,
    name: &str,
    pin_count: usize,
) -> footprint::FootprintData {
    if let Some(lib) = lib_path {
        if let Some(fp_path) = footprint::resolve_footprint_path(footprint_ref, lib) {
            match footprint::load_footprint(&fp_path) {
                Ok(data) => {
                    let sig_pads = data.signal_pads().len();
                    println!("    {} → loaded {} ({} pads)", name, footprint_ref, sig_pads);
                    return data;
                }
                Err(e) => {
                    eprintln!("    {} → failed to parse {}: {}", name, footprint_ref, e);
                }
            }
        } else {
            eprintln!("    {} → not found: {}, using fallback", name, footprint_ref);
        }
    }

    println!("    {} → fallback footprint ({} pads)", name, pin_count);
    footprint::generate_fallback(name, pin_count)
}

/// Match pin numbers to pad positions from the footprint data.
fn apply_pad_positions(mut pins: Vec<Pin>, fp: &footprint::FootprintData) -> Vec<Pin> {
    let signal_pads = fp.signal_pads();

    for pin in &mut pins {
        if let Some(pad) = signal_pads.iter().find(|p| p.number == pin.number) {
            pin.x = pad.at_x;
            pin.y = pad.at_y;
        }
    }

    pins
}

fn generate_ref_des(footprint: &str, index: usize) -> String {
    let fp_lower = footprint.to_lowercase();
    let prefix = if fp_lower.contains("led") {
        "D"
    } else if fp_lower.contains("capacitor") || fp_lower.contains("/c_") {
        "C"
    } else if fp_lower.contains("resistor") || fp_lower.contains("/r_") {
        "R"
    } else if fp_lower.contains("connector") || fp_lower.contains("usb") || fp_lower.contains("jst") {
        "J"
    } else if fp_lower.contains("button") || fp_lower.contains("switch") || fp_lower.contains("sw_") {
        "SW"
    } else if fp_lower.contains("inductor") {
        "L"
    } else if fp_lower.contains("crystal") || fp_lower.contains("oscillator") {
        "Y"
    } else {
        "U"
    };
    format!("{}{}", prefix, index + 1)
}
