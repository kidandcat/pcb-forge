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
    /// Launch interactive web UI
    Ui {
        /// Path to the circuit definition TOML file
        input: PathBuf,

        /// Output directory (default: ./output)
        #[arg(short, long, default_value = "output")]
        output: PathBuf,

        /// Use Topola topological router instead of A* grid router
        #[arg(long)]
        topola: bool,

        /// Port to listen on (default: 8080)
        #[arg(short, long, default_value = "8080")]
        port: u16,
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
        Commands::Ui {
            input,
            output,
            topola,
            port,
        } => launch_ui(&input, &output, topola, port),
    }
}

fn build(input: &PathBuf, output: &PathBuf, use_topola: bool, open_viewer: bool) -> Result<()> {
    println!("pcb-forge v{}", env!("CARGO_PKG_VERSION"));
    println!("Building from: {}", input.display());

    // Step 1: Parse
    println!("[1/7] Parsing circuit definition...");
    let board = parser::parse_circuit(input).context("Failed to parse circuit")?;
    println!(
        "  → {} components, {} nets",
        board.components.len(),
        board.nets.len()
    );

    std::fs::create_dir_all(output)?;

    // Step 2: Generate schematic (shared across all variants)
    println!("[2/7] Generating schematic...");
    let sch_path = output.join("pcb-forge.kicad_sch");
    schematic::generate_schematic(&board, &sch_path)?;
    println!("  → {}", sch_path.display());

    // Step 3: Generate placement variants
    let num_variants = board.options.placement_variants;
    println!("[3/7] Generating {} placement variants...", num_variants);
    let configs = pcb::generate_placement_configs(&board.options);
    let mut variants: Vec<(
        pcb::PlacementConfig,
        pcb_forge::schema::Board,
        Vec<pcb_forge::router::RoutedNet>,
        pcb::PlacementScore,
    )> = Vec::new();

    for (i, config) in configs.iter().enumerate() {
        let placed_board = pcb::generate_placement(&board, config);
        print!("  variant {}/{}: placing...", i + 1, num_variants);

        // Step 4: Route each variant
        let routed_nets = if use_topola {
            print!(" routing (Topola)...");
            match topola_router::route_with_topola(&placed_board) {
                Ok(nets) => nets,
                Err(e) => {
                    println!(" FAILED: {}", e);
                    continue;
                }
            }
        } else {
            print!(" routing (A*)...");
            let mut router = Router::new(placed_board.width, placed_board.height, 0.1);
            router.route_all(&placed_board)
        };

        let score = pcb::PlacementScore::compute(&routed_nets, board.nets.len(), &placed_board);
        println!(
            " score={:.0} (nets={}/{}, length={:.1}mm, vias={})",
            score.composite, score.nets_routed, score.total_nets,
            score.total_trace_length, score.via_count
        );

        variants.push((config.clone(), placed_board, routed_nets, score));
    }

    // Step 5: Sort by score and select top 3
    println!("[4/7] Selecting top 3 placements...");
    variants.sort_by(|a, b| b.3.composite.partial_cmp(&a.3.composite).unwrap());
    let top3: Vec<_> = variants.into_iter().take(3).collect();

    // Print comparison table
    println!("\n  ┌─────────────┬──────────┬──────────────┬──────┬───────────┐");
    println!("  │ Placement   │ Nets     │ Trace length │ Vias │ Score     │");
    println!("  ├─────────────┼──────────┼──────────────┼──────┼───────────┤");
    for (rank, (_, _, _, score)) in top3.iter().enumerate() {
        println!(
            "  │ #{:<10} │ {:>3}/{:<3} │ {:>9.1}mm │ {:>4} │ {:>9.1} │",
            rank + 1,
            score.nets_routed,
            score.total_nets,
            score.total_trace_length,
            score.via_count,
            score.composite
        );
    }
    println!("  └─────────────┴──────────┴──────────────┴──────┴───────────┘\n");

    // Step 6: Generate outputs for top 3
    println!("[5/7] Generating outputs for top 3 placements...");
    for (rank, (_, ref placed_board, ref routed_nets, _)) in top3.iter().enumerate() {
        let variant_dir = output.join(format!("placement-{}", rank + 1));
        std::fs::create_dir_all(&variant_dir)?;

        // PCB file
        let pcb_path = variant_dir.join("pcb-forge.kicad_pcb");
        pcb::write_pcb_file(placed_board, &pcb_path)?;
        pcb::append_routed_traces(&pcb_path, placed_board, routed_nets)?;

        // Gerbers
        let gerber_dir = variant_dir.join("gerbers");
        gerber::generate_gerbers(placed_board, routed_nets, &gerber_dir)?;

        // BOM
        bom::generate_bom(placed_board, &variant_dir)?;

        // JLCPCB ZIP
        let zip_path = variant_dir.join("jlcpcb.zip");
        create_jlcpcb_zip(&gerber_dir, &variant_dir, &zip_path)?;

        // Viewer
        let viewer_path = variant_dir.join("viewer.html");
        viewer::generate_viewer(placed_board, routed_nets, &viewer_path)?;

        // PNG
        let png_path = variant_dir.join("pcb-preview.png");
        viewer::generate_png(placed_board, routed_nets, &png_path)?;

        println!("  → placement-{}: {}", rank + 1, variant_dir.display());
    }

    // Step 7: Generate shared BOM (from best placement)
    println!("[6/7] Generating shared BOM...");
    if let Some((_, ref best_board, _, _)) = top3.first() {
        bom::generate_bom(best_board, output)?;
        println!("  → BOM.csv, PickAndPlace.csv");
    }

    println!("[7/7] Done!");

    if open_viewer {
        if let Some((_, _, _, _)) = top3.first() {
            let best_viewer = output.join("placement-1/viewer.html");
            viewer::open_viewer(&best_viewer);
            println!("  → Opened best placement in browser");
        }
    }

    println!("\nBuild complete! Output: {}", output.display());
    println!("  Best 3 placements in: placement-1/, placement-2/, placement-3/");

    Ok(())
}

fn create_jlcpcb_zip(
    gerber_dir: &std::path::Path,
    output_dir: &std::path::Path,
    zip_path: &std::path::Path,
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

fn launch_ui(input: &PathBuf, output: &PathBuf, use_topola: bool, port: u16) -> Result<()> {
    println!("pcb-forge v{}", env!("CARGO_PKG_VERSION"));
    println!("Loading: {}", input.display());

    let board = parser::parse_circuit(input).context("Failed to parse circuit")?;
    println!(
        "  → {} components, {} nets",
        board.components.len(),
        board.nets.len()
    );

    std::fs::create_dir_all(output)?;

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(pcb_forge::ui::start_server(
        board,
        input.to_path_buf(),
        output.to_path_buf(),
        use_topola,
        port,
    ))?;

    Ok(())
}

fn validate(input: &PathBuf) -> Result<()> {
    println!("Validating: {}", input.display());

    let mut board = parser::parse_circuit(input).context("Failed to parse circuit")?;

    // Compute auto-sized dimensions for display
    pcb::auto_size_board_pub(&mut board);

    println!("Valid circuit definition:");
    println!("  Board: {:.1}mm x {:.1}mm (auto-sized, aspect_ratio={:.1}), {} layers",
        board.width, board.height, board.aspect_ratio, board.layers);
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
