use anyhow::Result;

use std::path::Path;

use crate::schema::Board;

pub fn generate_bom(board: &Board, output_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(output_dir)?;

    generate_bom_csv(board, output_dir)?;
    generate_pick_and_place(board, output_dir)?;

    Ok(())
}

fn generate_bom_csv(board: &Board, output_dir: &Path) -> Result<()> {
    let output_path = output_dir.join("BOM.csv");
    let mut writer = csv::Writer::from_path(&output_path)?;

    // JLCPCB BOM format
    writer.write_record(["Comment", "Designator", "Footprint", "LCSC Part Number"])?;

    for comp in &board.components {
        let lcsc = comp.lcsc.as_deref().unwrap_or("");
        writer.write_record([
            comp.value.as_str(),
            comp.ref_des.as_str(),
            comp.footprint.as_str(),
            lcsc,
        ])?;
    }

    writer.flush()?;
    Ok(())
}

fn generate_pick_and_place(board: &Board, output_dir: &Path) -> Result<()> {
    let output_path = output_dir.join("PickAndPlace.csv");
    let mut writer = csv::Writer::from_path(&output_path)?;

    // JLCPCB pick-and-place format
    writer.write_record([
        "Designator",
        "Val",
        "Package",
        "Mid X",
        "Mid Y",
        "Rotation",
        "Layer",
    ])?;

    for comp in &board.components {
        writer.write_record([
            comp.ref_des.as_str(),
            comp.value.as_str(),
            comp.footprint.as_str(),
            &format!("{:.4}mm", comp.x),
            &format!("{:.4}mm", comp.y),
            &format!("{:.1}", comp.rotation),
            "top",
        ])?;
    }

    writer.flush()?;
    Ok(())
}
