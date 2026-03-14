use anyhow::Result;
use std::io::Write;
use std::path::Path;
use uuid::Uuid;

use crate::schema::Board;

pub fn generate_schematic(board: &Board, output: &Path) -> Result<()> {
    let mut buf = String::new();

    let sheet_uuid = Uuid::new_v4();

    // KiCad 7 schematic header
    buf.push_str("(kicad_sch\n");
    buf.push_str("  (version 20231120)\n");
    buf.push_str("  (generator \"pcb-forge\")\n");
    buf.push_str(&format!("  (generator_version \"0.1.0\")\n"));
    buf.push_str(&format!("  (uuid \"{}\")\n", sheet_uuid));
    buf.push_str("  (paper \"A3\")\n\n");

    // Library symbols section
    buf.push_str("  (lib_symbols\n");
    for comp in &board.components {
        write_lib_symbol(&mut buf, comp);
    }
    buf.push_str("  )\n\n");

    // Power symbols
    write_power_symbols(&mut buf);

    // Place component instances
    let cols = (board.components.len() as f64).sqrt().ceil() as usize;
    let spacing_x = 40.0; // mm in schematic space (actually mils but KiCad uses mm)
    let spacing_y = 50.0;
    let start_x = 50.0;
    let start_y = 50.0;

    for (i, comp) in board.components.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let x = start_x + col as f64 * spacing_x;
        let y = start_y + row as f64 * spacing_y;

        write_symbol_instance(&mut buf, comp, x, y);
    }

    // Wires for nets
    // For the schematic, we add net labels on the pins instead of drawing wires
    // This is cleaner and KiCad handles it
    for net in &board.nets {
        for pin_ref in &net.pins {
            if let Some((comp_idx, _comp)) = board
                .components
                .iter()
                .enumerate()
                .find(|(_, c)| c.name == pin_ref.component)
            {
                let col = comp_idx % cols;
                let row = comp_idx / cols;
                let x = start_x + col as f64 * spacing_x;
                let y = start_y + row as f64 * spacing_y;

                // Find pin index for offset
                let pin_idx = _comp
                    .pins
                    .iter()
                    .position(|p| p.name == pin_ref.pin)
                    .unwrap_or(0);

                let pin_y = y + 5.0 + pin_idx as f64 * 2.54;
                let label_x = x + 20.0;

                write_net_label(&mut buf, &net.name, label_x, pin_y);
            }
        }
    }

    buf.push_str(")\n");

    let mut file = std::fs::File::create(output)?;
    file.write_all(buf.as_bytes())?;

    Ok(())
}

fn write_lib_symbol(buf: &mut String, comp: &crate::schema::Component) {
    let lib_name = format!("pcb-forge:{}", comp.name);
    buf.push_str(&format!("    (symbol \"{}\"\n", lib_name));
    buf.push_str(&format!(
        "      (pin_names (offset 1.016))\n"
    ));
    buf.push_str("      (exclude_from_sim no)\n");
    buf.push_str("      (in_bom yes)\n");
    buf.push_str("      (on_board yes)\n");

    // Properties
    buf.push_str(&format!(
        "      (property \"Reference\" \"{}\" (at 0 1.27 0) (effects (font (size 1.27 1.27))))\n",
        comp.ref_des
    ));
    buf.push_str(&format!(
        "      (property \"Value\" \"{}\" (at 0 -1.27 0) (effects (font (size 1.27 1.27))))\n",
        comp.value
    ));
    buf.push_str(&format!(
        "      (property \"Footprint\" \"{}\" (at 0 -3.81 0) (effects (font (size 1.27 1.27)) hide))\n",
        comp.footprint
    ));

    if let Some(lcsc) = &comp.lcsc {
        buf.push_str(&format!(
            "      (property \"LCSC\" \"{}\" (at 0 -6.35 0) (effects (font (size 1.27 1.27)) hide))\n",
            lcsc
        ));
    }

    // Symbol body - unit 0 (shared)
    buf.push_str(&format!("      (symbol \"{}_{}_0\"\n", comp.name, 0));

    // Draw rectangle
    let pin_count = comp.pins.len();
    let body_height = (pin_count as f64 * 2.54).max(5.08);
    let body_width = 15.24;

    buf.push_str(&format!(
        "        (rectangle (start -{:.2} -{:.2}) (end {:.2} {:.2})\n",
        body_width / 2.0,
        body_height / 2.0,
        body_width / 2.0,
        body_height / 2.0
    ));
    buf.push_str("          (stroke (width 0.254) (type default))\n");
    buf.push_str("          (fill (type background))\n");
    buf.push_str("        )\n");
    buf.push_str("      )\n");

    // Symbol body - unit 1 (pins)
    buf.push_str(&format!("      (symbol \"{}_{}_1\"\n", comp.name, 1));

    // Place pins - left side for inputs/bidirectional, right side for outputs
    let left_pins: Vec<_> = comp
        .pins
        .iter()
        .filter(|p| !matches!(p.pin_type, crate::schema::PinType::Output))
        .collect();
    let right_pins: Vec<_> = comp
        .pins
        .iter()
        .filter(|p| matches!(p.pin_type, crate::schema::PinType::Output))
        .collect();

    let total_left = left_pins.len().max(1);
    let total_right = right_pins.len().max(1);
    let max_pins = total_left.max(total_right);
    let _pin_area_height = max_pins as f64 * 2.54;

    for (i, pin) in left_pins.iter().enumerate() {
        let py = (body_height / 2.0) - 1.27 - i as f64 * 2.54;
        let px = -(body_width / 2.0) - 2.54;
        buf.push_str(&format!(
            "        (pin {} line (at {:.2} {:.2} 0) (length 2.54)\n",
            pin.pin_type.to_kicad_str(),
            px,
            py
        ));
        buf.push_str(&format!(
            "          (name \"{}\" (effects (font (size 1.27 1.27))))\n",
            pin.name
        ));
        buf.push_str(&format!(
            "          (number \"{}\" (effects (font (size 1.27 1.27))))\n",
            pin.number
        ));
        buf.push_str("        )\n");
    }

    for (i, pin) in right_pins.iter().enumerate() {
        let py = (body_height / 2.0) - 1.27 - i as f64 * 2.54;
        let px = (body_width / 2.0) + 2.54;
        buf.push_str(&format!(
            "        (pin {} line (at {:.2} {:.2} 180) (length 2.54)\n",
            pin.pin_type.to_kicad_str(),
            px,
            py
        ));
        buf.push_str(&format!(
            "          (name \"{}\" (effects (font (size 1.27 1.27))))\n",
            pin.name
        ));
        buf.push_str(&format!(
            "          (number \"{}\" (effects (font (size 1.27 1.27))))\n",
            pin.number
        ));
        buf.push_str("        )\n");
    }

    buf.push_str("      )\n"); // close unit 1
    buf.push_str("    )\n"); // close symbol
}

fn write_power_symbols(buf: &mut String) {
    // VCC3V3 power symbol
    buf.push_str("  (power_port \"VCC3V3\"\n");
    buf.push_str(&format!("    (uuid \"{}\")\n", Uuid::new_v4()));
    buf.push_str("  )\n");

    // GND power symbol
    buf.push_str("  (power_port \"GND\"\n");
    buf.push_str(&format!("    (uuid \"{}\")\n", Uuid::new_v4()));
    buf.push_str("  )\n");
}

fn write_symbol_instance(
    buf: &mut String,
    comp: &crate::schema::Component,
    x: f64,
    y: f64,
) {
    let inst_uuid = Uuid::new_v4();
    let lib_name = format!("pcb-forge:{}", comp.name);

    buf.push_str(&format!("  (symbol\n"));
    buf.push_str(&format!("    (lib_id \"{}\")\n", lib_name));
    buf.push_str(&format!("    (at {:.2} {:.2} 0)\n", x, y));
    buf.push_str("    (unit 1)\n");
    buf.push_str(&format!("    (uuid \"{}\")\n", inst_uuid));

    // Instance properties
    buf.push_str(&format!(
        "    (property \"Reference\" \"{}\" (at {:.2} {:.2} 0)\n      (effects (font (size 1.27 1.27)))\n    )\n",
        comp.ref_des,
        x,
        y - 3.0
    ));
    buf.push_str(&format!(
        "    (property \"Value\" \"{}\" (at {:.2} {:.2} 0)\n      (effects (font (size 1.27 1.27)))\n    )\n",
        comp.value,
        x,
        y - 5.0
    ));
    buf.push_str(&format!(
        "    (property \"Footprint\" \"{}\" (at {:.2} {:.2} 0)\n      (effects (font (size 1.27 1.27)) hide)\n    )\n",
        comp.footprint,
        x,
        y - 7.0
    ));

    // Pin instances (all default)
    for pin in &comp.pins {
        buf.push_str(&format!(
            "    (pin \"{}\" (uuid \"{}\"))\n",
            pin.number,
            Uuid::new_v4()
        ));
    }

    buf.push_str("  )\n\n");
}

fn write_net_label(buf: &mut String, net_name: &str, x: f64, y: f64) {
    buf.push_str(&format!("  (net_label \"{}\"\n", net_name));
    buf.push_str(&format!("    (at {:.2} {:.2} 0)\n", x, y));
    buf.push_str(&format!("    (effects (font (size 1.27 1.27)))\n"));
    buf.push_str(&format!("    (uuid \"{}\")\n", Uuid::new_v4()));
    buf.push_str("  )\n");
}
