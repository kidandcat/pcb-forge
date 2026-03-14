use anyhow::Result;
use gerber_types::*;
use gerber_types::Polarity;
use std::collections::HashMap;
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::router::RoutedNet;
use crate::schema::{Board, Layer};

const COORD_FMT_INT: u8 = 4;
const COORD_FMT_DEC: u8 = 6;

fn coord_format() -> CoordinateFormat {
    CoordinateFormat::new(
        ZeroOmission::Leading,
        CoordinateMode::Absolute,
        COORD_FMT_INT,
        COORD_FMT_DEC,
    )
}

fn coords(x_mm: f64, y_mm: f64) -> Coordinates {
    let fmt = coord_format();
    let scale = 10_i64.pow(COORD_FMT_DEC as u32);
    Coordinates::new(
        CoordinateNumber::new((x_mm * scale as f64) as i64),
        CoordinateNumber::new((y_mm * scale as f64) as i64),
        fmt,
    )
}

fn comment(text: &str) -> Command {
    FunctionCode::GCode(GCode::Comment(CommentContent::String(text.to_string()))).into()
}

fn header_commands() -> Vec<Command> {
    vec![
        ExtendedCode::CoordinateFormat(coord_format()).into(),
        ExtendedCode::Unit(Unit::Millimeters).into(),
    ]
}

fn eof() -> Command {
    FunctionCode::MCode(MCode::EndOfFile).into()
}

fn select_aperture(code: i32) -> Command {
    FunctionCode::DCode(DCode::SelectAperture(code)).into()
}

fn flash(x: f64, y: f64) -> Command {
    DCode::Operation(Operation::Flash(Some(coords(x, y)))).into()
}

fn move_to(x: f64, y: f64) -> Command {
    DCode::Operation(Operation::Move(Some(coords(x, y)))).into()
}

fn line_to(x: f64, y: f64) -> Command {
    DCode::Operation(Operation::Interpolate(Some(coords(x, y)), None)).into()
}

fn linear_mode() -> Command {
    FunctionCode::GCode(GCode::InterpolationMode(InterpolationMode::Linear)).into()
}

fn region_on() -> Command {
    FunctionCode::GCode(GCode::RegionMode(true)).into()
}

fn region_off() -> Command {
    FunctionCode::GCode(GCode::RegionMode(false)).into()
}

fn polarity_dark() -> Command {
    ExtendedCode::LoadPolarity(Polarity::Dark).into()
}

fn polarity_clear() -> Command {
    ExtendedCode::LoadPolarity(Polarity::Clear).into()
}

fn write_commands(commands: Vec<Command>, path: &Path) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let mut writer = BufWriter::new(file);
    commands.serialize(&mut writer)?;
    writer.flush()?;
    Ok(())
}

/// Collect unique aperture sizes from all components' real pads.
/// Returns a map from (width_um, height_um) -> aperture code, starting from 11.
fn collect_pad_apertures(board: &Board) -> HashMap<(i64, i64), i32> {
    let mut sizes = HashMap::new();
    let mut next_code = 11;

    for comp in &board.components {
        if let Some(fp) = &comp.footprint_data {
            for pad in &fp.pads {
                if pad.number.is_empty() && !pad.layers.iter().any(|l| l.contains("Cu")) {
                    continue;
                }
                let key = (
                    (pad.size_w * 1000.0).round() as i64,
                    (pad.size_h * 1000.0).round() as i64,
                );
                sizes.entry(key).or_insert_with(|| {
                    let code = next_code;
                    next_code += 1;
                    code
                });
            }
        }
    }

    sizes
}

/// Check if a pad on a component belongs to a specific net.
fn is_pad_on_net(comp: &crate::schema::Component, pad_number: &str, board: &Board, net_name: &str) -> bool {
    let pin = comp.pins.iter().find(|p| p.number == pad_number);
    if let Some(pin) = pin {
        board.nets.iter().any(|net| {
            net.name == net_name
                && net.pins.iter().any(|pr| pr.component == comp.name && pr.pin == pin.name)
        })
    } else {
        false
    }
}

pub fn generate_gerbers(
    board: &Board,
    routed_nets: &[RoutedNet],
    output_dir: &Path,
) -> Result<()> {
    std::fs::create_dir_all(output_dir)?;

    generate_copper_layer(board, routed_nets, output_dir, Layer::FCu)?;
    generate_copper_layer(board, routed_nets, output_dir, Layer::BCu)?;
    generate_mask_layer(board, output_dir, Layer::FMask)?;
    generate_mask_layer(board, output_dir, Layer::BMask)?;
    generate_silk_layer(board, output_dir, Layer::FSilkS)?;
    generate_silk_layer(board, output_dir, Layer::BSilkS)?;
    generate_edge_cuts(board, output_dir)?;
    generate_drill_file(board, routed_nets, output_dir)?;

    Ok(())
}

fn layer_filename(layer: Layer) -> &'static str {
    match layer {
        Layer::FCu => "pcb-forge-F_Cu.gtl",
        Layer::BCu => "pcb-forge-B_Cu.gbl",
        Layer::FMask => "pcb-forge-F_Mask.gts",
        Layer::BMask => "pcb-forge-B_Mask.gbs",
        Layer::FSilkS => "pcb-forge-F_SilkS.gto",
        Layer::BSilkS => "pcb-forge-B_SilkS.gbo",
        Layer::EdgeCuts => "pcb-forge-Edge_Cuts.gm1",
    }
}

/// Get absolute pad positions for a component using real footprint data.
fn real_pad_positions(comp: &crate::schema::Component) -> Vec<(f64, f64, f64, f64)> {
    if let Some(fp) = &comp.footprint_data {
        fp.signal_pads()
            .iter()
            .map(|pad| {
                (
                    comp.x + pad.at_x,
                    comp.y + pad.at_y,
                    pad.size_w,
                    pad.size_h,
                )
            })
            .collect()
    } else {
        let pin_count = comp.pins.len();
        let body_w = 8.0_f64.max(pin_count as f64 * 1.0);
        let half = (pin_count + 1) / 2;

        comp.pins
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let (px, py) = if i < half {
                    let py = -(half as f64 - 1.0) * 1.27 / 2.0 + i as f64 * 1.27;
                    (-(body_w / 2.0 - 0.5), py)
                } else {
                    let ri = i - half;
                    let right_count = pin_count - half;
                    let py = -(right_count as f64 - 1.0) * 1.27 / 2.0 + ri as f64 * 1.27;
                    (body_w / 2.0 - 0.5, py)
                };
                (comp.x + px, comp.y + py, 1.5, 0.6)
            })
            .collect()
    }
}

fn generate_copper_layer(
    board: &Board,
    routed_nets: &[RoutedNet],
    output_dir: &Path,
    layer: Layer,
) -> Result<()> {
    let target_layer: u8 = match layer {
        Layer::FCu => 0,
        Layer::BCu => 1,
        _ => return Ok(()),
    };

    let is_bcu = matches!(layer, Layer::BCu);

    let mut commands = Vec::new();
    commands.push(comment(&format!("Layer: {}", layer.name())));
    commands.extend(header_commands());

    // D10: signal trace aperture (0.25mm)
    commands.push(
        ExtendedCode::ApertureDefinition(ApertureDefinition {
            code: 10,
            aperture: Aperture::Circle(Circle {
                diameter: 0.25,
                hole_diameter: None,
            }),
        })
        .into(),
    );

    // D100: power trace aperture (0.5mm)
    commands.push(
        ExtendedCode::ApertureDefinition(ApertureDefinition {
            code: 100,
            aperture: Aperture::Circle(Circle {
                diameter: 0.5,
                hole_diameter: None,
            }),
        })
        .into(),
    );

    // Collect pad apertures
    let pad_apertures = collect_pad_apertures(board);
    for (&(w_um, h_um), &code) in &pad_apertures {
        let w = w_um as f64 / 1000.0;
        let h = h_um as f64 / 1000.0;
        if (w - h).abs() < 0.001 {
            commands.push(
                ExtendedCode::ApertureDefinition(ApertureDefinition {
                    code,
                    aperture: Aperture::Circle(Circle {
                        diameter: w,
                        hole_diameter: None,
                    }),
                })
                .into(),
            );
        } else {
            commands.push(
                ExtendedCode::ApertureDefinition(ApertureDefinition {
                    code,
                    aperture: Aperture::Rectangle(Rectangular {
                        x: w,
                        y: h,
                        hole_diameter: None,
                    }),
                })
                .into(),
            );
        }
    }

    // --- Copper pour on B.Cu (GND ground plane) ---
    if is_bcu {
        let pour_clearance = 0.3; // mm clearance around non-GND features

        // Draw filled region covering entire board (dark polarity)
        commands.push(region_on());
        commands.push(move_to(0.0, 0.0));
        commands.push(linear_mode());
        commands.push(line_to(board.width, 0.0));
        commands.push(line_to(board.width, board.height));
        commands.push(line_to(0.0, board.height));
        commands.push(line_to(0.0, 0.0));
        commands.push(region_off());

        // Clear areas around non-GND through-hole pads
        commands.push(polarity_clear());
        for comp in &board.components {
            if let Some(fp) = &comp.footprint_data {
                for pad in fp.signal_pads() {
                    // Only clear around thru-hole pads (they appear on B.Cu)
                    if pad.pad_type != "thru_hole" {
                        continue;
                    }
                    if is_pad_on_net(comp, &pad.number, board, "GND") {
                        continue; // GND pads connect to the pour
                    }
                    let cx = comp.x + pad.at_x;
                    let cy = comp.y + pad.at_y;
                    let hw = pad.size_w / 2.0 + pour_clearance;
                    let hh = pad.size_h / 2.0 + pour_clearance;
                    commands.push(region_on());
                    commands.push(move_to(cx - hw, cy - hh));
                    commands.push(linear_mode());
                    commands.push(line_to(cx + hw, cy - hh));
                    commands.push(line_to(cx + hw, cy + hh));
                    commands.push(line_to(cx - hw, cy + hh));
                    commands.push(line_to(cx - hw, cy - hh));
                    commands.push(region_off());
                }
            }
        }

        // Clear around B.Cu traces that are not GND
        for net in routed_nets {
            let is_gnd = net.name == "GND";
            if is_gnd {
                continue;
            }
            for seg in &net.segments {
                if seg.layer == 1 {
                    // Clear a rectangle around the trace segment
                    let dx = seg.end.0 - seg.start.0;
                    let dy = seg.end.1 - seg.start.1;
                    let len = (dx * dx + dy * dy).sqrt();
                    if len < 0.001 {
                        continue;
                    }
                    let hw = seg.width / 2.0 + pour_clearance;
                    // Use perpendicular expansion
                    let nx = -dy / len * hw;
                    let ny = dx / len * hw;
                    commands.push(region_on());
                    commands.push(move_to(seg.start.0 + nx, seg.start.1 + ny));
                    commands.push(linear_mode());
                    commands.push(line_to(seg.end.0 + nx, seg.end.1 + ny));
                    commands.push(line_to(seg.end.0 - nx, seg.end.1 - ny));
                    commands.push(line_to(seg.start.0 - nx, seg.start.1 - ny));
                    commands.push(line_to(seg.start.0 + nx, seg.start.1 + ny));
                    commands.push(region_off());
                }
            }
        }

        // Clear around vias that are not GND
        for net in routed_nets {
            if net.name == "GND" {
                continue;
            }
            for via in &net.vias {
                let r = via.size / 2.0 + pour_clearance;
                // Approximate circle with octagon
                commands.push(region_on());
                let n = 8;
                for i in 0..=n {
                    let a = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
                    let px = via.x + r * a.cos();
                    let py = via.y + r * a.sin();
                    if i == 0 {
                        commands.push(move_to(px, py));
                        commands.push(linear_mode());
                    } else {
                        commands.push(line_to(px, py));
                    }
                }
                commands.push(region_off());
            }
        }

        // Restore dark polarity
        commands.push(polarity_dark());
    }

    // Flash pads
    for comp in &board.components {
        if let Some(fp) = &comp.footprint_data {
            for pad in fp.signal_pads() {
                // On B.Cu, only flash thru-hole pads
                if is_bcu && pad.pad_type != "thru_hole" {
                    continue;
                }
                let key = (
                    (pad.size_w * 1000.0).round() as i64,
                    (pad.size_h * 1000.0).round() as i64,
                );
                if let Some(&code) = pad_apertures.get(&key) {
                    commands.push(select_aperture(code));
                    commands.push(flash(comp.x + pad.at_x, comp.y + pad.at_y));
                }
            }
        } else if !is_bcu {
            for (px, py, _w, _h) in real_pad_positions(comp) {
                let key = (1500_i64, 600_i64);
                if let Some(&code) = pad_apertures.get(&key) {
                    commands.push(select_aperture(code));
                    commands.push(flash(px, py));
                }
            }
        }
    }

    // Traces
    commands.push(linear_mode());
    let mut current_aperture = 0;
    for net in routed_nets {
        for seg in &net.segments {
            if seg.layer == target_layer {
                let needed_aperture = if (seg.width - 0.5).abs() < 0.01 { 100 } else { 10 };
                if needed_aperture != current_aperture {
                    commands.push(select_aperture(needed_aperture));
                    current_aperture = needed_aperture;
                }
                commands.push(move_to(seg.start.0, seg.start.1));
                commands.push(line_to(seg.end.0, seg.end.1));
            }
        }
    }

    commands.push(eof());
    write_commands(commands, &output_dir.join(layer_filename(layer)))
}

fn generate_mask_layer(board: &Board, output_dir: &Path, layer: Layer) -> Result<()> {
    let mut commands = Vec::new();
    commands.push(comment(&format!("Layer: {}", layer.name())));
    commands.extend(header_commands());

    let mask_expansion = 0.1;
    let pad_apertures = collect_pad_apertures(board);
    let mut mask_apertures = HashMap::new();
    let mut next_code = 10;

    for (&(w_um, h_um), _) in &pad_apertures {
        let w = w_um as f64 / 1000.0 + mask_expansion;
        let h = h_um as f64 / 1000.0 + mask_expansion;
        let key = (
            (w * 1000.0).round() as i64,
            (h * 1000.0).round() as i64,
        );
        mask_apertures.entry(key).or_insert_with(|| {
            let code = next_code;
            next_code += 1;
            code
        });
    }

    for (&(w_um, h_um), &code) in &mask_apertures {
        let w = w_um as f64 / 1000.0;
        let h = h_um as f64 / 1000.0;
        commands.push(
            ExtendedCode::ApertureDefinition(ApertureDefinition {
                code,
                aperture: Aperture::Rectangle(Rectangular {
                    x: w,
                    y: h,
                    hole_diameter: None,
                }),
            })
            .into(),
        );
    }

    for comp in &board.components {
        if let Some(fp) = &comp.footprint_data {
            for pad in fp.signal_pads() {
                let w = pad.size_w + mask_expansion;
                let h = pad.size_h + mask_expansion;
                let key = (
                    (w * 1000.0).round() as i64,
                    (h * 1000.0).round() as i64,
                );
                if let Some(&code) = mask_apertures.get(&key) {
                    commands.push(select_aperture(code));
                    commands.push(flash(comp.x + pad.at_x, comp.y + pad.at_y));
                }
            }
        }
    }

    commands.push(eof());
    write_commands(commands, &output_dir.join(layer_filename(layer)))
}

fn generate_silk_layer(board: &Board, output_dir: &Path, layer: Layer) -> Result<()> {
    let mut commands = Vec::new();
    commands.push(comment(&format!("Layer: {}", layer.name())));
    commands.extend(header_commands());

    // D10: silkscreen line aperture (0.15mm)
    commands.push(
        ExtendedCode::ApertureDefinition(ApertureDefinition {
            code: 10,
            aperture: Aperture::Circle(Circle {
                diameter: 0.15,
                hole_diameter: None,
            }),
        })
        .into(),
    );

    // D11: text stroke aperture (0.12mm for ref des)
    commands.push(
        ExtendedCode::ApertureDefinition(ApertureDefinition {
            code: 11,
            aperture: Aperture::Circle(Circle {
                diameter: 0.12,
                hole_diameter: None,
            }),
        })
        .into(),
    );

    commands.push(select_aperture(10));

    if matches!(layer, Layer::FSilkS) {
        for comp in &board.components {
            if let Some(fp) = &comp.footprint_data {
                // Draw real silkscreen lines from footprint
                let silk_lines: Vec<_> = fp
                    .lines
                    .iter()
                    .filter(|l| l.layer.contains("SilkS"))
                    .collect();

                for line in &silk_lines {
                    commands.push(move_to(
                        comp.x + line.start.0,
                        comp.y + line.start.1,
                    ));
                    commands.push(linear_mode());
                    commands.push(line_to(comp.x + line.end.0, comp.y + line.end.1));
                }

                // Pin 1 marker
                if let Some(first_pad) = fp.signal_pads().first() {
                    commands.push(flash(
                        comp.x + first_pad.at_x - first_pad.size_w / 2.0 - 0.5,
                        comp.y + first_pad.at_y,
                    ));
                }
            } else {
                // Fallback: draw component outline
                let pin_count = comp.pins.len();
                let body_w = 8.0_f64.max(pin_count as f64 * 1.0);
                let body_h = 6.0_f64.max(pin_count as f64 * 0.8);
                let hw = (body_w - 1.0) / 2.0;
                let hh = (body_h - 1.0) / 2.0;

                let corners = [
                    (comp.x - hw, comp.y - hh),
                    (comp.x + hw, comp.y - hh),
                    (comp.x + hw, comp.y + hh),
                    (comp.x - hw, comp.y + hh),
                    (comp.x - hw, comp.y - hh),
                ];

                commands.push(move_to(corners[0].0, corners[0].1));
                commands.push(linear_mode());
                for corner in &corners[1..] {
                    commands.push(line_to(corner.0, corner.1));
                }

                commands.push(flash(comp.x - hw - 0.5, comp.y - hh + 0.5));
            }

            // Reference designator text (0.8mm height)
            commands.push(select_aperture(11));
            let text_size = 0.8;
            // Position text above component
            let comp_top = if let Some(fp) = &comp.footprint_data {
                let (_, min_y, _, _) = fp.courtyard_bounds();
                comp.y + min_y - 0.5
            } else {
                comp.y - 4.0
            };
            let text_width = text_total_width(&comp.ref_des, text_size);
            let text_x = comp.x - text_width / 2.0;
            let text_y = comp_top - text_size;
            render_text_commands(&mut commands, &comp.ref_des, text_x, text_y, text_size);
            commands.push(select_aperture(10));
        }
    }

    commands.push(eof());
    write_commands(commands, &output_dir.join(layer_filename(layer)))
}

fn generate_edge_cuts(board: &Board, output_dir: &Path) -> Result<()> {
    let mut commands = Vec::new();
    commands.push(comment("Layer: Edge.Cuts"));
    commands.extend(header_commands());

    commands.push(
        ExtendedCode::ApertureDefinition(ApertureDefinition {
            code: 10,
            aperture: Aperture::Circle(Circle {
                diameter: 0.05,
                hole_diameter: None,
            }),
        })
        .into(),
    );

    commands.push(select_aperture(10));

    let corners = [
        (0.0, 0.0),
        (board.width, 0.0),
        (board.width, board.height),
        (0.0, board.height),
        (0.0, 0.0),
    ];

    commands.push(move_to(corners[0].0, corners[0].1));
    commands.push(linear_mode());
    for corner in &corners[1..] {
        commands.push(line_to(corner.0, corner.1));
    }

    commands.push(eof());
    write_commands(commands, &output_dir.join(layer_filename(Layer::EdgeCuts)))
}

fn generate_drill_file(board: &Board, routed_nets: &[RoutedNet], output_dir: &Path) -> Result<()> {
    let output_path = output_dir.join("pcb-forge.drl");
    let mut file = std::fs::File::create(&output_path)?;

    writeln!(file, "M48")?;
    writeln!(file, ";DRILL file pcb-forge")?;
    writeln!(file, ";FORMAT={{-:-/ absolute / metric / decimal}}")?;
    writeln!(file, "FMAT,2")?;
    writeln!(file, "METRIC")?;

    // Collect all drill sizes
    let mut drill_sizes: Vec<f64> = Vec::new();

    // Through-hole pad drills
    for comp in &board.components {
        if let Some(fp) = &comp.footprint_data {
            for pad in fp.signal_pads() {
                if pad.pad_type == "thru_hole" {
                    if let Some(d) = pad.drill {
                        if !drill_sizes.iter().any(|&s| (s - d).abs() < 0.001) {
                            drill_sizes.push(d);
                        }
                    }
                }
            }
        }
    }

    // Via drills
    let via_drill = 0.3;
    let has_vias = routed_nets.iter().any(|n| !n.vias.is_empty());
    if has_vias && !drill_sizes.iter().any(|&s| (s - via_drill).abs() < 0.001) {
        drill_sizes.push(via_drill);
    }

    if drill_sizes.is_empty() {
        drill_sizes.push(via_drill);
    }
    drill_sizes.sort_by(|a, b| a.partial_cmp(b).unwrap());

    // Define tools
    for (i, &size) in drill_sizes.iter().enumerate() {
        writeln!(file, "T{}C{:.3}", i + 1, size)?;
    }
    writeln!(file, "%")?;

    // Drill through-hole pads
    for (i, &size) in drill_sizes.iter().enumerate() {
        writeln!(file, "T{}", i + 1)?;

        // Through-hole pad hits
        for comp in &board.components {
            if let Some(fp) = &comp.footprint_data {
                for pad in fp.signal_pads() {
                    if pad.pad_type == "thru_hole" {
                        if let Some(d) = pad.drill {
                            if (d - size).abs() < 0.001 {
                                writeln!(file, "X{:.3}Y{:.3}", comp.x + pad.at_x, comp.y + pad.at_y)?;
                            }
                        }
                    }
                }
            }
        }

        // Via hits
        if (via_drill - size).abs() < 0.001 {
            for net in routed_nets {
                for via in &net.vias {
                    writeln!(file, "X{:.3}Y{:.3}", via.x, via.y)?;
                }
            }
        }
    }

    writeln!(file, "M30")?;
    Ok(())
}

// ── Stroke font for silkscreen text ────────────────────────────────

/// Get line segments for a character in a 0..4 x 0..6 coordinate space.
fn char_strokes(ch: char) -> Vec<((f64, f64), (f64, f64))> {
    match ch.to_ascii_uppercase() {
        'A' => vec![((0.0,6.0),(0.0,2.0)),((0.0,2.0),(2.0,0.0)),((2.0,0.0),(4.0,2.0)),((4.0,2.0),(4.0,6.0)),((0.0,4.0),(4.0,4.0))],
        'B' => vec![((0.0,0.0),(0.0,6.0)),((0.0,0.0),(3.0,0.0)),((3.0,0.0),(4.0,1.0)),((4.0,1.0),(3.0,3.0)),((0.0,3.0),(3.0,3.0)),((3.0,3.0),(4.0,4.5)),((4.0,4.5),(3.0,6.0)),((3.0,6.0),(0.0,6.0))],
        'C' => vec![((4.0,1.0),(3.0,0.0)),((3.0,0.0),(1.0,0.0)),((1.0,0.0),(0.0,1.0)),((0.0,1.0),(0.0,5.0)),((0.0,5.0),(1.0,6.0)),((1.0,6.0),(3.0,6.0)),((3.0,6.0),(4.0,5.0))],
        'D' => vec![((0.0,0.0),(0.0,6.0)),((0.0,0.0),(3.0,0.0)),((3.0,0.0),(4.0,1.0)),((4.0,1.0),(4.0,5.0)),((4.0,5.0),(3.0,6.0)),((3.0,6.0),(0.0,6.0))],
        'E' => vec![((4.0,0.0),(0.0,0.0)),((0.0,0.0),(0.0,6.0)),((0.0,6.0),(4.0,6.0)),((0.0,3.0),(3.0,3.0))],
        'F' => vec![((4.0,0.0),(0.0,0.0)),((0.0,0.0),(0.0,6.0)),((0.0,3.0),(3.0,3.0))],
        'G' => vec![((4.0,1.0),(3.0,0.0)),((3.0,0.0),(1.0,0.0)),((1.0,0.0),(0.0,1.0)),((0.0,1.0),(0.0,5.0)),((0.0,5.0),(1.0,6.0)),((1.0,6.0),(4.0,6.0)),((4.0,6.0),(4.0,3.0)),((4.0,3.0),(2.0,3.0))],
        'H' => vec![((0.0,0.0),(0.0,6.0)),((4.0,0.0),(4.0,6.0)),((0.0,3.0),(4.0,3.0))],
        'I' => vec![((1.0,0.0),(3.0,0.0)),((2.0,0.0),(2.0,6.0)),((1.0,6.0),(3.0,6.0))],
        'J' => vec![((1.0,0.0),(4.0,0.0)),((3.0,0.0),(3.0,5.0)),((3.0,5.0),(2.0,6.0)),((2.0,6.0),(1.0,6.0)),((1.0,6.0),(0.0,5.0))],
        'K' => vec![((0.0,0.0),(0.0,6.0)),((4.0,0.0),(0.0,3.0)),((0.0,3.0),(4.0,6.0))],
        'L' => vec![((0.0,0.0),(0.0,6.0)),((0.0,6.0),(4.0,6.0))],
        'M' => vec![((0.0,6.0),(0.0,0.0)),((0.0,0.0),(2.0,3.0)),((2.0,3.0),(4.0,0.0)),((4.0,0.0),(4.0,6.0))],
        'N' => vec![((0.0,6.0),(0.0,0.0)),((0.0,0.0),(4.0,6.0)),((4.0,6.0),(4.0,0.0))],
        'O' => vec![((1.0,0.0),(3.0,0.0)),((3.0,0.0),(4.0,1.0)),((4.0,1.0),(4.0,5.0)),((4.0,5.0),(3.0,6.0)),((3.0,6.0),(1.0,6.0)),((1.0,6.0),(0.0,5.0)),((0.0,5.0),(0.0,1.0)),((0.0,1.0),(1.0,0.0))],
        'P' => vec![((0.0,6.0),(0.0,0.0)),((0.0,0.0),(3.0,0.0)),((3.0,0.0),(4.0,1.0)),((4.0,1.0),(4.0,2.0)),((4.0,2.0),(3.0,3.0)),((3.0,3.0),(0.0,3.0))],
        'Q' => vec![((1.0,0.0),(3.0,0.0)),((3.0,0.0),(4.0,1.0)),((4.0,1.0),(4.0,5.0)),((4.0,5.0),(3.0,6.0)),((3.0,6.0),(1.0,6.0)),((1.0,6.0),(0.0,5.0)),((0.0,5.0),(0.0,1.0)),((0.0,1.0),(1.0,0.0)),((3.0,5.0),(4.5,6.5))],
        'R' => vec![((0.0,6.0),(0.0,0.0)),((0.0,0.0),(3.0,0.0)),((3.0,0.0),(4.0,1.0)),((4.0,1.0),(4.0,2.0)),((4.0,2.0),(3.0,3.0)),((3.0,3.0),(0.0,3.0)),((2.0,3.0),(4.0,6.0))],
        'S' => vec![((4.0,1.0),(3.0,0.0)),((3.0,0.0),(1.0,0.0)),((1.0,0.0),(0.0,1.0)),((0.0,1.0),(0.0,2.0)),((0.0,2.0),(1.0,3.0)),((1.0,3.0),(3.0,3.0)),((3.0,3.0),(4.0,4.0)),((4.0,4.0),(4.0,5.0)),((4.0,5.0),(3.0,6.0)),((3.0,6.0),(1.0,6.0)),((1.0,6.0),(0.0,5.0))],
        'T' => vec![((0.0,0.0),(4.0,0.0)),((2.0,0.0),(2.0,6.0))],
        'U' => vec![((0.0,0.0),(0.0,5.0)),((0.0,5.0),(1.0,6.0)),((1.0,6.0),(3.0,6.0)),((3.0,6.0),(4.0,5.0)),((4.0,5.0),(4.0,0.0))],
        'V' => vec![((0.0,0.0),(2.0,6.0)),((2.0,6.0),(4.0,0.0))],
        'W' => vec![((0.0,0.0),(1.0,6.0)),((1.0,6.0),(2.0,3.0)),((2.0,3.0),(3.0,6.0)),((3.0,6.0),(4.0,0.0))],
        'X' => vec![((0.0,0.0),(4.0,6.0)),((4.0,0.0),(0.0,6.0))],
        'Y' => vec![((0.0,0.0),(2.0,3.0)),((4.0,0.0),(2.0,3.0)),((2.0,3.0),(2.0,6.0))],
        'Z' => vec![((0.0,0.0),(4.0,0.0)),((4.0,0.0),(0.0,6.0)),((0.0,6.0),(4.0,6.0))],
        '0' => vec![((1.0,0.0),(3.0,0.0)),((3.0,0.0),(4.0,1.0)),((4.0,1.0),(4.0,5.0)),((4.0,5.0),(3.0,6.0)),((3.0,6.0),(1.0,6.0)),((1.0,6.0),(0.0,5.0)),((0.0,5.0),(0.0,1.0)),((0.0,1.0),(1.0,0.0))],
        '1' => vec![((1.0,1.0),(2.0,0.0)),((2.0,0.0),(2.0,6.0)),((1.0,6.0),(3.0,6.0))],
        '2' => vec![((0.0,1.0),(1.0,0.0)),((1.0,0.0),(3.0,0.0)),((3.0,0.0),(4.0,1.0)),((4.0,1.0),(4.0,2.0)),((4.0,2.0),(0.0,6.0)),((0.0,6.0),(4.0,6.0))],
        '3' => vec![((0.0,1.0),(1.0,0.0)),((1.0,0.0),(3.0,0.0)),((3.0,0.0),(4.0,1.0)),((4.0,1.0),(4.0,2.0)),((4.0,2.0),(3.0,3.0)),((3.0,3.0),(2.0,3.0)),((3.0,3.0),(4.0,4.0)),((4.0,4.0),(4.0,5.0)),((4.0,5.0),(3.0,6.0)),((3.0,6.0),(1.0,6.0)),((1.0,6.0),(0.0,5.0))],
        '4' => vec![((0.0,0.0),(0.0,3.0)),((0.0,3.0),(4.0,3.0)),((4.0,0.0),(4.0,6.0))],
        '5' => vec![((4.0,0.0),(0.0,0.0)),((0.0,0.0),(0.0,3.0)),((0.0,3.0),(3.0,3.0)),((3.0,3.0),(4.0,4.0)),((4.0,4.0),(4.0,5.0)),((4.0,5.0),(3.0,6.0)),((3.0,6.0),(0.0,6.0))],
        '6' => vec![((3.0,0.0),(1.0,0.0)),((1.0,0.0),(0.0,1.0)),((0.0,1.0),(0.0,5.0)),((0.0,5.0),(1.0,6.0)),((1.0,6.0),(3.0,6.0)),((3.0,6.0),(4.0,5.0)),((4.0,5.0),(4.0,4.0)),((4.0,4.0),(3.0,3.0)),((3.0,3.0),(0.0,3.0))],
        '7' => vec![((0.0,0.0),(4.0,0.0)),((4.0,0.0),(2.0,6.0))],
        '8' => vec![((1.0,0.0),(3.0,0.0)),((3.0,0.0),(4.0,1.0)),((4.0,1.0),(4.0,2.0)),((4.0,2.0),(3.0,3.0)),((3.0,3.0),(4.0,4.0)),((4.0,4.0),(4.0,5.0)),((4.0,5.0),(3.0,6.0)),((3.0,6.0),(1.0,6.0)),((1.0,6.0),(0.0,5.0)),((0.0,5.0),(0.0,4.0)),((0.0,4.0),(1.0,3.0)),((1.0,3.0),(0.0,2.0)),((0.0,2.0),(0.0,1.0)),((0.0,1.0),(1.0,0.0))],
        '9' => vec![((4.0,3.0),(1.0,3.0)),((1.0,3.0),(0.0,2.0)),((0.0,2.0),(0.0,1.0)),((0.0,1.0),(1.0,0.0)),((1.0,0.0),(3.0,0.0)),((3.0,0.0),(4.0,1.0)),((4.0,1.0),(4.0,5.0)),((4.0,5.0),(3.0,6.0)),((3.0,6.0),(1.0,6.0))],
        _ => vec![],
    }
}

/// Calculate total width of rendered text.
fn text_total_width(text: &str, size: f64) -> f64 {
    let scale = size / 6.0;
    let char_width = 4.0 * scale;
    let spacing = 1.5 * scale;
    let n = text.len();
    if n == 0 {
        0.0
    } else {
        char_width * n as f64 + spacing * (n as f64 - 1.0)
    }
}

/// Render text as Gerber line commands using stroke font.
fn render_text_commands(commands: &mut Vec<Command>, text: &str, x: f64, y: f64, size: f64) {
    let scale = size / 6.0;
    let char_width = 4.0 * scale;
    let spacing = 1.5 * scale;

    let mut offset_x = 0.0;
    for ch in text.chars() {
        for ((sx, sy), (ex, ey)) in char_strokes(ch) {
            commands.push(move_to(x + offset_x + sx * scale, y + sy * scale));
            commands.push(linear_mode());
            commands.push(line_to(x + offset_x + ex * scale, y + ey * scale));
        }
        offset_x += char_width + spacing;
    }
}
