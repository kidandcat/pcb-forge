# pcb-forge — Autonomous PCB Design Tool

pcb-forge generates production-ready PCB designs from a declarative TOML circuit definition. It auto-places components using connectivity-aware algorithms, routes traces with A* or topological routers, and outputs KiCad, Gerber, BOM, and JLCPCB-ready files.

## TOML Input Format

The input file has 4 sections:

### [board] — Board configuration
```toml
[board]
aspect_ratio = 1.3    # Board shape: 1.0=square, >1=landscape, <1=portrait (default: 1.0)
layers = 2            # Copper layers (default: 2)
trace_width = 0.25    # Signal trace width in mm (default: 0.25)
clearance = 0.2       # Trace-to-trace clearance in mm (default: 0.25)
footprint_lib = "~/.local/share/kicad/9.0/footprints"  # Optional KiCad library path
```

Board width/height are auto-calculated from component areas. Do NOT specify them manually.

### [components.*] — One section per component
```toml
[components.esp32]
footprint = "RF_Module.pretty/ESP32-S3-WROOM-1.kicad_mod"  # KiCad footprint path
value = "ESP32-S3-WROOM-1"                                  # Component value
lcsc = "C2913202"                                           # Optional LCSC part number
description = "Main MCU"                                    # Optional description

[components.esp32.pins]
VCC = 2          # pin_name = pad_number
GND = 1
TX0 = 37
```

### [[nets]] — Connections between component pins
```toml
[[nets]]
name = "SPI_MOSI"
pins = ["esp32.IO11", "lora.MOSI"]   # Format: "component_name.pin_name"
```

### [power] — Power rail consolidation (optional)
```toml
[power]
vcc = ["vreg.VOUT", "esp32.VCC", "c1.P1"]
gnd = ["vreg.GND", "esp32.GND", "c1.P2"]
```

### [options] — AI-tunable parameters
```toml
[options]
density = 2.0             # Courtyard area multiplier for board sizing (1.5=tight, 2.5=spacious)
spacing = 1.0             # Base component spacing multiplier (0.5=tight, 2.0=spread)
placement_variants = 10   # Number of placement variants to generate and compare
board_penalty = 0.5       # Score penalty per mm² of board area
trace_penalty = 0.1       # Score penalty per mm of trace length
via_penalty = 50.0        # Score penalty per via
net_reward = 1000.0       # Score reward per successfully routed net
```

## How to Run

```bash
# Full build — generates placement variants, routes, and outputs manufacturing files
cargo run -- build input.toml -o output/

# Build with topological router (better quality, slower)
cargo run -- build input.toml -o output/ --topola

# Validate TOML without building
cargo run -- validate input.toml

# Launch interactive web UI
cargo run -- ui input.toml
```

## Interpreting Results

After a build, the output directory contains:
- `placement-1/`, `placement-2/`, `placement-3/` — Top 3 placement variants ranked by score
- Each variant contains:
  - `pcb-preview.png` — Visual preview (READ THIS to evaluate the design)
  - `pcb-forge.kicad_pcb` — KiCad PCB file
  - `gerbers/` — Manufacturing files
  - `BOM.csv`, `PickAndPlace.csv` — Assembly files
  - `jlcpcb.zip` — Ready-to-order JLCPCB package
  - `viewer.html` — Interactive HTML viewer

### Score breakdown (printed during build)
```
  │ Placement   │ Nets     │ Trace length │ Vias │ Score     │
  │ #1          │  18/20   │     320.5mm  │   12 │   16368.0 │
```
- **Nets routed**: Higher is better — unrouted nets mean the design is incomplete
- **Trace length**: Lower is better — shorter traces = less noise, faster signals
- **Vias**: Lower is better — each via adds impedance and manufacturing cost
- **Score**: Composite metric (higher = better), computed from the [options] weights

## Tuning Parameters (Iterative Workflow)

After reviewing the PNG preview images, adjust `[options]` to improve the design:

### If components are too spread out / board is too large:
- Decrease `density` (e.g., 1.5-1.8)
- Decrease `spacing` (e.g., 0.7-0.8)

### If routing fails (many unrouted nets):
- Increase `density` (e.g., 2.5-3.0)
- Increase `spacing` (e.g., 1.2-1.5)
- Increase `placement_variants` (e.g., 20-30) for more attempts

### If traces are too long:
- Increase `trace_penalty` (e.g., 0.5) to favor shorter traces

### If too many vias:
- Increase `via_penalty` (e.g., 100-200)

### If board is larger than needed:
- Increase `board_penalty` (e.g., 1.0-2.0)

### Recommended iterative flow:
1. Start with defaults, run `cargo run -- build input.toml -o output/`
2. Review `output/placement-1/pcb-preview.png`
3. Adjust [options] based on observations above
4. Re-run build
5. Compare scores and previews
6. Repeat until satisfied
