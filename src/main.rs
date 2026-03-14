use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::PathBuf;

use pcb_forge::bom;
use pcb_forge::gerber;
use pcb_forge::parser;
use pcb_forge::pcb;
use pcb_forge::router::Router;
use pcb_forge::schematic;
use pcb_forge::topola_router;
use pcb_forge::viewer;

#[derive(Parser)]
#[command(name = "pcb-forge", version, about = "Autonomous PCB design tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build PCB from circuit definition
    Build {
        /// Path to the circuit definition TOML file
        input: PathBuf,

        /// Output directory (default: ./output)
        #[arg(short, long, default_value = "output")]
        output: PathBuf,

        /// Use Topola topological router instead of A* grid router
        #[arg(long)]
        topola: bool,

        /// Open 2D viewer after build
        #[arg(long)]
        view: bool,
    },
    /// Open 2D PCB viewer (builds first if needed)
    View {
        /// Path to the circuit definition TOML file
        input: PathBuf,

        /// Output directory (default: ./output)
        #[arg(short, long, default_value = "output")]
        output: PathBuf,

        /// Use Topola topological router instead of A* grid router
        #[arg(long)]
        topola: bool,
    },
    /// Validate a circuit definition without building
    Validate {
        /// Path to the circuit definition TOML file
        input: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Build {
            input,
            output,
            topola,
            view,
        } => build(&input, &output, topola, view),
        Commands::View {
            input,
            output,
            topola,
        } => build(&input, &output, topola, true),
        Commands::Validate { input } => validate(&input),
    }
}

fn build(input: &PathBuf, output: &PathBuf, use_topola: bool, open_viewer: bool) -> Result<()> {
    println!("pcb-forge v{}", env!("CARGO_PKG_VERSION"));
    println!("Building from: {}", input.display());

    // Step 1: Parse
    println!("[1/9] Parsing circuit definition...");
    let mut board = parser::parse_circuit(input).context("Failed to parse circuit")?;
    println!(
        "  → {} components, {} nets",
        board.components.len(),
        board.nets.len()
    );

    // Step 2: Create output directory
    std::fs::create_dir_all(output)?;

    // Step 3: Generate schematic
    println!("[2/9] Generating schematic...");
    let sch_path = output.join("pcb-forge.kicad_sch");
    schematic::generate_schematic(&board, &sch_path)?;
    println!("  → {}", sch_path.display());

    // Step 4: Generate PCB with placement
    println!("[3/9] Generating PCB layout...");
    let pcb_path = output.join("pcb-forge.kicad_pcb");
    pcb::generate_pcb(&mut board, &pcb_path)?;
    println!("  → {}", pcb_path.display());

    // Step 5: Route traces
    let routed_nets = if use_topola {
        println!("[4/9] Routing traces (Topola topological router)...");
        topola_router::route_with_topola(&board)?
    } else {
        println!("[4/9] Routing traces (A* router)...");
        let mut router = Router::new(board.width, board.height, 0.1);
        router.route_all(&board)
    };
    let total_segments: usize = routed_nets.iter().map(|r| r.segments.len()).sum();
    let total_vias: usize = routed_nets.iter().map(|r| r.vias.len()).sum();
    println!("  → {} segments, {} vias", total_segments, total_vias);

    // Step 5b: Append routed traces to the .kicad_pcb file
    pcb::append_routed_traces(&pcb_path, &board, &routed_nets)?;
    println!("  → Traces written to {}", pcb_path.display());

    // Step 6: Generate Gerbers
    println!("[5/9] Generating Gerber files...");
    let gerber_dir = output.join("gerbers");
    gerber::generate_gerbers(&board, &routed_nets, &gerber_dir)?;
    println!("  → {}", gerber_dir.display());

    // Step 7: Generate BOM
    println!("[6/9] Generating BOM and pick-and-place...");
    bom::generate_bom(&board, output)?;
    println!("  → BOM.csv, PickAndPlace.csv");

    // Step 8: Create JLCPCB ZIP
    println!("[7/9] Creating JLCPCB ZIP...");
    let zip_path = output.join("jlcpcb.zip");
    create_jlcpcb_zip(&gerber_dir, output, &zip_path)?;
    println!("  → {}", zip_path.display());

    // Step 9: Generate viewer
    println!("[8/9] Generating 2D viewer...");
    let viewer_path = output.join("viewer.html");
    viewer::generate_viewer(&board, &routed_nets, &viewer_path)?;

    // Step 10: Generate PNG preview
    println!("[9/9] Generating PNG preview...");
    let png_path = output.join("pcb-preview.png");
    viewer::generate_png(&board, &routed_nets, &png_path)?;

    if open_viewer {
        viewer::open_viewer(&viewer_path);
        println!("  → Opened in browser");
    }

    println!("\nBuild complete! Output: {}", output.display());

    Ok(())
}

fn create_jlcpcb_zip(
    gerber_dir: &PathBuf,
    output_dir: &PathBuf,
    zip_path: &PathBuf,
) -> Result<()> {
    let file = std::fs::File::create(zip_path).context("Failed to create jlcpcb.zip")?;
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // Add all gerber files
    if gerber_dir.exists() {
        for entry in std::fs::read_dir(gerber_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                zip.start_file(&name, options)?;
                let data = std::fs::read(&path)?;
                zip.write_all(&data)?;
            }
        }
    }

    // Add BOM.csv and PickAndPlace.csv
    for filename in &["BOM.csv", "PickAndPlace.csv"] {
        let path = output_dir.join(filename);
        if path.exists() {
            zip.start_file(*filename, options)?;
            let data = std::fs::read(&path)?;
            zip.write_all(&data)?;
        }
    }

    zip.finish()?;
    Ok(())
}

fn validate(input: &PathBuf) -> Result<()> {
    println!("Validating: {}", input.display());

    let board = parser::parse_circuit(input).context("Failed to parse circuit")?;

    println!("Valid circuit definition:");
    println!("  Board: {}mm x {}mm, {} layers", board.width, board.height, board.layers);
    println!("  Components: {}", board.components.len());
    for comp in &board.components {
        println!(
            "    {} ({}) - {} pins{}",
            comp.ref_des,
            comp.value,
            comp.pins.len(),
            comp.lcsc.as_ref().map_or(String::new(), |l| format!(" [LCSC: {}]", l))
        );
    }
    println!("  Nets: {}", board.nets.len());
    for net in &board.nets {
        println!("    {} → {} pins", net.name, net.pins.len());
    }

    Ok(())
}
