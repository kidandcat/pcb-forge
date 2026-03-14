use anyhow::Result;
use std::collections::{BTreeSet, HashMap};
use std::io::{BufReader, Cursor};

use crate::footprint::PadData;
use crate::router::{RoutedNet, TraceSegment, Via};
use crate::schema::Board;

use topola::autorouter::execution::Command;
use topola::autorouter::invoker::Invoker;
use topola::autorouter::selection::PinSelection;
use topola::autorouter::{Autorouter, PlanarAutorouteOptions, PresortBy};
use topola::board::edit::BoardEdit;
use topola::board::AccessMesadata;
use topola::drawing::graph::{GetMaybeNet, MakePrimitiveRef, PrimitiveIndex};
use topola::drawing::primitive::MakePrimitiveShape;
use topola::geometry::primitive::PrimitiveShape;
use topola::geometry::shape::AccessShape;
use topola::geometry::GetLayer;
use topola::graph::GenericIndex;
use topola::layout::via::ViaWeight;
use topola::router::RouterOptions;
use topola::specctra::design::SpecctraDesign;
use topola::specctra::mesadata::SpecctraMesadata;
use topola::stepper::TimeoutOptions;

/// Conversion: pcb-forge uses mm, Specctra DSN uses um.
const MM_TO_UM: f64 = 1000.0;
const UM_TO_MM: f64 = 0.001;

/// Power nets that should be copper pour, not routed traces.
fn is_power_pour_net(name: &str) -> bool {
    name == "GND" || name == "VCC3V3"
}

/// Route the board using Topola's topological autorouter.
pub fn route_with_topola(board: &Board) -> Result<Vec<RoutedNet>> {
    // Step 1: Generate Specctra DSN
    let dsn = generate_dsn(board);

    // Step 2: Load into Topola
    let cursor = Cursor::new(dsn.as_bytes());
    let bufread = BufReader::new(cursor);
    let design = SpecctraDesign::load(bufread)
        .map_err(|e| anyhow::anyhow!("Failed to parse generated DSN: {:?}", e))?;
    let mut recorder = BoardEdit::new();
    let topola_board = design.make_board(&mut recorder);

    // Step 3: Create autorouter
    let autorouter = Autorouter::new(topola_board)
        .map_err(|e| anyhow::anyhow!("Autorouter init failed: {:?}", e))?;
    let mut invoker = Invoker::new(autorouter);

    // Step 4: Route on each layer (with panic protection)
    let layer_count = invoker
        .autorouter()
        .board()
        .layout()
        .drawing()
        .layer_count();

    for layer in 0..layer_count {
        let selection = PinSelection::new_select_layer(invoker.autorouter().board(), layer);
        if selection.selectors().count() == 0 {
            continue;
        }

        let options = PlanarAutorouteOptions {
            principal_layer: layer,
            presort_by: PresortBy::RatlineIntersectionCountAndLength,
            permutate: false,
            router: RouterOptions {
                routed_band_width: board.trace_width * MM_TO_UM,
                wrap_around_bands: true,
                squeeze_through_under_bends: true,
            },
            timeout: TimeoutOptions {
                initial: 5.0,
                progress_bonus: 0.01,
            },
        };

        // Topola can panic on certain placements - catch and continue
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            invoker.execute(Command::Autoroute(selection, options))
        }));

        match result {
            Ok(Ok(())) => eprintln!("  Topola: layer {} routed successfully", layer),
            Ok(Err(e)) => eprintln!("  Topola: layer {} routing: {:?}", layer, e),
            Err(_) => eprintln!("  Topola: layer {} routing panicked, partial results may be available", layer),
        }
    }

    // Step 5: Extract traces
    let routed = extract_traces(invoker.autorouter(), board);
    Ok(routed)
}

/// Generate a Specctra DSN string from a pcb-forge Board.
fn generate_dsn(board: &Board) -> String {
    let mut dsn = String::new();

    // Header
    dsn.push_str("(pcb pcb_forge\n");
    dsn.push_str("  (parser\n");
    dsn.push_str("    (string_quote \")\n");
    dsn.push_str("    (space_in_quoted_tokens on)\n");
    dsn.push_str("    (host_cad \"pcb-forge\")\n");
    dsn.push_str("    (host_version \"0.1.0\")\n");
    dsn.push_str("  )\n");
    dsn.push_str("  (resolution um 10)\n");
    dsn.push_str("  (unit um)\n");

    // Structure
    dsn.push_str("  (structure\n");
    dsn.push_str(
        "    (layer F.Cu\n      (type signal)\n      (property\n        (index 0)\n      )\n    )\n",
    );
    if board.layers >= 2 {
        dsn.push_str("    (layer B.Cu\n      (type signal)\n      (property\n        (index 1)\n      )\n    )\n");
    }

    // Boundary
    let bw = board.width * MM_TO_UM;
    let bh = board.height * MM_TO_UM;
    dsn.push_str(&format!(
        "    (boundary\n      (path pcb 0  0 0  {} 0  {} {}  0 {}  0 0)\n    )\n",
        bw, bw, bh, bh
    ));

    // Via padstack reference
    let via_name = "Via[0-1]_600:300_um";
    dsn.push_str(&format!("    (via \"{}\")\n", via_name));

    // Rules
    let clearance_um = board.clearance * MM_TO_UM;
    let trace_w_um = board.trace_width * MM_TO_UM;
    dsn.push_str(&format!(
        "    (rule\n      (width {})\n      (clearance {})\n    )\n",
        trace_w_um, clearance_um
    ));
    dsn.push_str("  )\n");

    // Placement
    dsn.push_str("  (placement\n");
    let mut images: Vec<(String, Vec<usize>)> = Vec::new();
    {
        let mut image_map: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, comp) in board.components.iter().enumerate() {
            image_map
                .entry(comp.footprint.clone())
                .or_default()
                .push(i);
        }
        let mut keys: Vec<String> = image_map.keys().cloned().collect();
        keys.sort();
        for k in keys {
            images.push((k.clone(), image_map.remove(&k).unwrap()));
        }
    }
    for (fp, indices) in &images {
        dsn.push_str(&format!("    (component {}\n", dsn_quote(fp)));
        for &i in indices {
            let comp = &board.components[i];
            let x = comp.x * MM_TO_UM;
            let y = comp.y * MM_TO_UM;
            dsn.push_str(&format!(
                "      (place {} {:.1} {:.1} front {:.1} (PN {}))\n",
                comp.ref_des,
                x,
                y,
                comp.rotation,
                dsn_quote(&comp.value)
            ));
        }
        dsn.push_str("    )\n");
    }
    dsn.push_str("  )\n");

    // Library
    dsn.push_str("  (library\n");

    // Track padstacks we need to define
    let mut padstack_defs: HashMap<String, String> = HashMap::new();

    for (fp, indices) in &images {
        dsn.push_str(&format!("    (image {}\n", dsn_quote(fp)));

        // Use first component as representative for footprint definition
        let comp = &board.components[indices[0]];
        if let Some(fp_data) = &comp.footprint_data {
            // Outline (courtyard/fab lines)
            for line in &fp_data.lines {
                if line.layer.contains("CrtYd") || line.layer.contains("Fab") {
                    let w = line.width * MM_TO_UM;
                    let sx = line.start.0 * MM_TO_UM;
                    let sy = line.start.1 * MM_TO_UM;
                    let ex = line.end.0 * MM_TO_UM;
                    let ey = line.end.1 * MM_TO_UM;
                    dsn.push_str(&format!(
                        "      (outline (path signal {} {} {} {} {}))\n",
                        w, sx, sy, ex, ey
                    ));
                }
            }

            // Pads
            for pad in fp_data.signal_pads() {
                let ps_name = make_padstack_name(pad);
                padstack_defs
                    .entry(ps_name.clone())
                    .or_insert_with(|| make_padstack_def(pad));

                let px = pad.at_x * MM_TO_UM;
                let py = pad.at_y * MM_TO_UM;
                dsn.push_str(&format!(
                    "      (pin {} {} {} {})\n",
                    ps_name, pad.number, px, py
                ));
            }
        }

        dsn.push_str("    )\n");
    }

    // Padstack definitions
    let mut ps_keys: Vec<String> = padstack_defs.keys().cloned().collect();
    ps_keys.sort();
    for name in &ps_keys {
        dsn.push_str(&format!("    {}\n", padstack_defs[name]));
    }

    // Via padstack
    dsn.push_str(&format!("    (padstack \"{}\"\n", via_name));
    dsn.push_str("      (shape (circle F.Cu 600))\n");
    dsn.push_str("      (shape (circle B.Cu 600))\n");
    dsn.push_str("      (attach off)\n");
    dsn.push_str("    )\n");

    dsn.push_str("  )\n");

    // Network: include ALL nets (including power pour) so Topola treats their pads
    // as proper obstacles with clearance. Power pour nets are routed by Topola for
    // pad association, but their traces are filtered out in extraction (copper zones
    // handle the actual connections).
    dsn.push_str("  (network\n");
    for net in &board.nets {
        dsn.push_str(&format!("    (net {}\n      (pins", dsn_quote(&net.name)));
        for pin_ref in &net.pins {
            if let Some(comp) = board.components.iter().find(|c| c.name == pin_ref.component) {
                if let Some(pin) = comp.pins.iter().find(|p| p.name == pin_ref.pin) {
                    dsn.push_str(&format!(" {}-{}", comp.ref_des, pin.number));
                }
            }
        }
        dsn.push_str(")\n    )\n");
    }

    // Net class (all nets)
    let net_names: Vec<String> = board.nets.iter()
        .map(|n| dsn_quote(&n.name)).collect();
    dsn.push_str(&format!(
        "    (class kicad_default \"\" {}\n",
        net_names.join(" ")
    ));
    dsn.push_str(&format!(
        "      (circuit\n        (use_via {})\n      )\n",
        via_name
    ));
    dsn.push_str(&format!(
        "      (rule\n        (width {})\n        (clearance {})\n      )\n",
        trace_w_um, clearance_um
    ));
    dsn.push_str("    )\n");
    dsn.push_str("  )\n");

    // Wiring (empty)
    dsn.push_str("  (wiring\n  )\n");

    dsn.push_str(")\n");

    dsn
}

/// Generate a padstack name based on pad properties.
fn make_padstack_name(pad: &PadData) -> String {
    let w = (pad.size_w * MM_TO_UM) as i64;
    let h = (pad.size_h * MM_TO_UM) as i64;
    if pad.pad_type == "thru_hole" {
        let d = pad.drill.map(|d| (d * MM_TO_UM) as i64).unwrap_or(300);
        format!("Pad_THT_{}x{}_D{}", w, h, d)
    } else {
        format!("Pad_SMD_{}x{}", w, h)
    }
}

/// Generate a padstack definition for a pad.
fn make_padstack_def(pad: &PadData) -> String {
    let hw = pad.size_w * MM_TO_UM / 2.0;
    let hh = pad.size_h * MM_TO_UM / 2.0;
    let name = make_padstack_name(pad);

    if pad.pad_type == "thru_hole" {
        format!(
            "(padstack {}\n      (shape (rect F.Cu {} {} {} {}))\n      (shape (rect B.Cu {} {} {} {}))\n      (attach off)\n    )",
            name, -hw, -hh, hw, hh, -hw, -hh, hw, hh
        )
    } else {
        let layer = if pad.layers.iter().any(|l| l.contains("B.Cu")) {
            "B.Cu"
        } else {
            "F.Cu"
        };
        format!(
            "(padstack {}\n      (shape (rect {} {} {} {} {}))\n      (attach off)\n    )",
            name, layer, -hw, -hh, hw, hh
        )
    }
}

/// Quote a string for DSN format if it contains special characters.
fn dsn_quote(s: &str) -> String {
    if s.contains(' ')
        || s.contains('(')
        || s.contains(')')
        || s.contains('"')
        || s.contains('-')
    {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// Extract routed traces from Topola's board into pcb-forge format.
fn extract_traces(
    autorouter: &Autorouter<SpecctraMesadata>,
    pcb_board: &Board,
) -> Vec<RoutedNet> {
    let board = autorouter.board();
    let drawing = board.layout().drawing();
    let mesadata = board.mesadata();

    // net_index → (net_name, segments, vias)
    let mut net_traces: HashMap<usize, (String, Vec<TraceSegment>, Vec<Via>)> = HashMap::new();
    let mut visited_vias: BTreeSet<GenericIndex<ViaWeight>> = BTreeSet::new();

    for index in drawing.primitive_nodes() {
        let primitive = index.primitive_ref(drawing);

        let Some(net) = primitive.maybe_net() else {
            continue;
        };

        let net_name = mesadata
            .net_netname(net)
            .unwrap_or("unknown")
            .to_string();

        let entry = net_traces
            .entry(net)
            .or_insert_with(|| (net_name, Vec::new(), Vec::new()));

        let layer_name = mesadata.layer_layername(primitive.layer()).unwrap_or("F.Cu");
        let layer_idx: u8 = if layer_name == "B.Cu" { 1 } else { 0 };

        match index {
            // Only extract routed (loose) traces, not fixed pad geometry
            PrimitiveIndex::SeqLooseSeg(_) | PrimitiveIndex::LoneLooseSeg(_) => {
                if let PrimitiveShape::Seg(seg) = primitive.shape() {
                    entry.1.push(TraceSegment {
                        start: (seg.from.x() * UM_TO_MM, seg.from.y() * UM_TO_MM),
                        end: (seg.to.x() * UM_TO_MM, seg.to.y() * UM_TO_MM),
                        layer: layer_idx,
                        width: seg.width * UM_TO_MM,
                    });
                }
            }
            PrimitiveIndex::LooseBend(_) => {
                if let PrimitiveShape::Bend(bend) = primitive.shape() {
                    let points: Vec<_> = bend.render_discretization(64).collect();
                    for window in points.windows(2) {
                        entry.1.push(TraceSegment {
                            start: (window[0].x() * UM_TO_MM, window[0].y() * UM_TO_MM),
                            end: (window[1].x() * UM_TO_MM, window[1].y() * UM_TO_MM),
                            layer: layer_idx,
                            width: bend.width * UM_TO_MM,
                        });
                    }
                }
            }
            PrimitiveIndex::FixedDot(dot) => {
                if let Some(via) = board.layout().fixed_dot_via(dot) {
                    if visited_vias.insert(via) {
                        if let PrimitiveShape::Dot(dot_shape) = primitive.shape() {
                            entry.2.push(Via {
                                x: dot_shape.center().x() * UM_TO_MM,
                                y: dot_shape.center().y() * UM_TO_MM,
                                drill: 0.3,
                                size: dot_shape.circle.r * 2.0 * UM_TO_MM,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Diagnostic logging
    let mut total_segs = 0usize;
    let mut total_bends = 0usize;
    let mut total_vias = 0usize;
    for (_, (name, segments, vias)) in &net_traces {
        let bend_count = segments.iter().filter(|s| {
            // Bend-derived segments are non-axis-aligned (diagonal)
            let dx = (s.end.0 - s.start.0).abs();
            let dy = (s.end.1 - s.start.1).abs();
            dx > 0.001 && dy > 0.001
        }).count();
        let seg_count = segments.len() - bend_count;
        total_segs += seg_count;
        total_bends += bend_count;
        total_vias += vias.len();
        if !segments.is_empty() || !vias.is_empty() {
            eprintln!(
                "  Topola net '{}': {} straight segs, {} bend segs, {} vias",
                name, seg_count, bend_count, vias.len()
            );
        }
    }
    eprintln!(
        "  Topola totals: {} straight segs, {} bend segs, {} vias",
        total_segs, total_bends, total_vias
    );

    // Build output matching pcb-forge net order.
    // Power pour nets (GND, VCC3V3) are included in DSN for obstacle handling but
    // their traces are discarded here — copper zones handle those connections.
    let mut routed: Vec<RoutedNet> = Vec::new();
    for net in &pcb_board.nets {
        if is_power_pour_net(&net.name) {
            // Power pour nets: no traces (copper zones handle them)
            routed.push(RoutedNet {
                name: net.name.clone(),
                segments: Vec::new(),
                vias: Vec::new(),
            });
        } else if let Some((_, segments, vias)) = net_traces
            .values()
            .find(|(name, _, _)| name == &net.name)
        {
            routed.push(RoutedNet {
                name: net.name.clone(),
                segments: segments.clone(),
                vias: vias.clone(),
            });
        } else {
            routed.push(RoutedNet {
                name: net.name.clone(),
                segments: Vec::new(),
                vias: Vec::new(),
            });
        }
    }

    routed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::footprint::{FootprintData, PadData};
    use crate::schema::{Board, Component, Net, Pin, PinRef, PinType};

    /// Create a minimal board for DSN testing.
    fn make_test_board() -> Board {
        let make_pad = |num: &str, x: f64, y: f64| PadData {
            number: num.to_string(),
            pad_type: "smd".to_string(),
            shape: "rect".to_string(),
            at_x: x,
            at_y: y,
            size_w: 0.8,
            size_h: 0.5,
            layers: vec!["F.Cu".to_string()],
            drill: None,
        };

        Board {
            width: 30.0,
            height: 20.0,
            layers: 2,
            trace_width: 0.25,
            clearance: 0.25,
            components: vec![
                Component {
                    ref_des: "U1".to_string(),
                    name: "chip".to_string(),
                    footprint: "Package_SO:SOIC-8".to_string(),
                    value: "IC1".to_string(),
                    lcsc: None,
                    pins: vec![
                        Pin { name: "A".to_string(), number: "1".to_string(), pin_type: PinType::Passive, x: -1.27, y: -0.635 },
                        Pin { name: "B".to_string(), number: "2".to_string(), pin_type: PinType::Passive, x: -1.27, y: 0.635 },
                    ],
                    description: None,
                    footprint_data: Some(FootprintData {
                        name: "SOIC-8".to_string(),
                        pads: vec![make_pad("1", -1.27, -0.635), make_pad("2", -1.27, 0.635)],
                        lines: vec![],
                    }),
                    x: 10.0,
                    y: 10.0,
                    rotation: 0.0,
                    manually_placed: false,
                },
                Component {
                    ref_des: "R1".to_string(),
                    name: "res".to_string(),
                    footprint: "Resistor_SMD:R_0402".to_string(),
                    value: "10K".to_string(),
                    lcsc: None,
                    pins: vec![
                        Pin { name: "P1".to_string(), number: "1".to_string(), pin_type: PinType::Passive, x: -0.5, y: 0.0 },
                        Pin { name: "P2".to_string(), number: "2".to_string(), pin_type: PinType::Passive, x: 0.5, y: 0.0 },
                    ],
                    description: None,
                    footprint_data: Some(FootprintData {
                        name: "R_0402".to_string(),
                        pads: vec![make_pad("1", -0.5, 0.0), make_pad("2", 0.5, 0.0)],
                        lines: vec![],
                    }),
                    x: 20.0,
                    y: 10.0,
                    rotation: 0.0,
                    manually_placed: false,
                },
            ],
            nets: vec![
                Net {
                    name: "SIG1".to_string(),
                    pins: vec![
                        PinRef { component: "chip".to_string(), pin: "A".to_string() },
                        PinRef { component: "res".to_string(), pin: "P1".to_string() },
                    ],
                },
            ],
        }
    }

    /// Verify that generated DSN contains all required sections.
    #[test]
    fn test_dsn_structure() {
        let board = make_test_board();
        let dsn = generate_dsn(&board);

        assert!(dsn.contains("(pcb pcb_forge"), "Missing PCB header");
        assert!(dsn.contains("(parser"), "Missing parser section");
        assert!(dsn.contains("(resolution um 10)"), "Missing resolution");
        assert!(dsn.contains("(structure"), "Missing structure section");
        assert!(dsn.contains("(layer F.Cu"), "Missing F.Cu layer");
        assert!(dsn.contains("(layer B.Cu"), "Missing B.Cu layer");
        assert!(dsn.contains("(boundary"), "Missing boundary");
        assert!(dsn.contains("(placement"), "Missing placement");
        assert!(dsn.contains("(library"), "Missing library");
        assert!(dsn.contains("(network"), "Missing network");
        assert!(dsn.contains("(wiring"), "Missing wiring section");
    }

    /// Verify that DSN clearance matches board clearance.
    #[test]
    fn test_dsn_clearance_value() {
        let board = make_test_board();
        let dsn = generate_dsn(&board);

        let expected_clearance_um = board.clearance * MM_TO_UM;
        let clearance_str = format!("(clearance {})", expected_clearance_um);
        assert!(
            dsn.contains(&clearance_str),
            "DSN should contain clearance={}, got DSN:\n{}",
            expected_clearance_um,
            dsn
        );
    }

    /// Verify that DSN trace width matches board trace_width.
    #[test]
    fn test_dsn_trace_width() {
        let board = make_test_board();
        let dsn = generate_dsn(&board);

        let expected_width_um = board.trace_width * MM_TO_UM;
        let width_str = format!("(width {})", expected_width_um);
        assert!(
            dsn.contains(&width_str),
            "DSN should contain width={}",
            expected_width_um
        );
    }

    /// Verify that padstack names are deterministic and correctly formed.
    #[test]
    fn test_padstack_names() {
        let pad = PadData {
            number: "1".to_string(),
            pad_type: "smd".to_string(),
            shape: "rect".to_string(),
            at_x: 0.0,
            at_y: 0.0,
            size_w: 0.8,
            size_h: 0.5,
            layers: vec!["F.Cu".to_string()],
            drill: None,
        };
        let name = make_padstack_name(&pad);
        assert_eq!(name, "Pad_SMD_800x500", "SMD padstack name incorrect");

        let tht_pad = PadData {
            number: "1".to_string(),
            pad_type: "thru_hole".to_string(),
            shape: "circle".to_string(),
            at_x: 0.0,
            at_y: 0.0,
            size_w: 1.5,
            size_h: 1.5,
            layers: vec!["F.Cu".to_string(), "B.Cu".to_string()],
            drill: Some(0.8),
        };
        let tht_name = make_padstack_name(&tht_pad);
        assert_eq!(tht_name, "Pad_THT_1500x1500_D800", "THT padstack name incorrect");
    }

    /// Verify generated DSN can be parsed by Topola.
    #[test]
    fn test_dsn_parseable_by_topola() {
        let board = make_test_board();
        let dsn = generate_dsn(&board);

        let cursor = std::io::Cursor::new(dsn.as_bytes());
        let bufread = std::io::BufReader::new(cursor);
        let result = SpecctraDesign::load(bufread);

        assert!(
            result.is_ok(),
            "Generated DSN should be parseable by Topola: {:?}",
            result.err()
        );
    }

    /// Verify that DSN roundtrip produces a valid Topola board.
    #[test]
    fn test_dsn_roundtrip_creates_board() {
        let board = make_test_board();
        let dsn = generate_dsn(&board);

        let cursor = std::io::Cursor::new(dsn.as_bytes());
        let bufread = std::io::BufReader::new(cursor);
        let design = SpecctraDesign::load(bufread).expect("DSN should parse");

        let mut recorder = topola::board::edit::BoardEdit::new();
        let topola_board = design.make_board(&mut recorder);

        // Verify board has primitives
        let prim_count = topola_board.layout().drawing().primitive_nodes().count();
        assert!(
            prim_count > 0,
            "Topola board should have primitives after loading DSN, got {}",
            prim_count
        );
    }

    /// Verify that dsn_quote escapes special characters correctly.
    #[test]
    fn test_dsn_quote() {
        assert_eq!(dsn_quote("simple"), "simple");
        assert_eq!(dsn_quote("with space"), "\"with space\"");
        assert_eq!(dsn_quote("with-dash"), "\"with-dash\"");
        assert_eq!(dsn_quote("has(paren)"), "\"has(paren)\"");
    }

    /// Verify boundary dimensions match board size.
    #[test]
    fn test_dsn_boundary_dimensions() {
        let board = make_test_board();
        let dsn = generate_dsn(&board);

        let bw = board.width * MM_TO_UM;
        let bh = board.height * MM_TO_UM;
        let boundary_str = format!("{} 0  {} {}  0 {}", bw, bw, bh, bh);
        assert!(
            dsn.contains(&boundary_str),
            "DSN boundary should match board dimensions ({}x{} um)",
            bw,
            bh
        );
    }

    /// Verify that net pins are correctly referenced in DSN.
    #[test]
    fn test_dsn_net_pin_references() {
        let board = make_test_board();
        let dsn = generate_dsn(&board);

        // SIG1 connects chip.A (pin 1) and res.P1 (pin 1)
        assert!(dsn.contains("(net SIG1"), "Missing net SIG1");
        assert!(dsn.contains("U1-1"), "Missing pin reference U1-1 for chip.A");
        assert!(dsn.contains("R1-1"), "Missing pin reference R1-1 for res.P1");
    }
}
