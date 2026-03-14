use anyhow::{Context, Result};
use std::path::Path;

use crate::router::RoutedNet;
use crate::schema::Board;

/// Generate an interactive HTML/SVG viewer for the PCB and open it in the browser.
pub fn generate_viewer(board: &Board, routed_nets: &[RoutedNet], output_path: &Path) -> Result<()> {
    let html = render_html(board, routed_nets);
    std::fs::write(output_path, &html)?;
    println!("  → {}", output_path.display());
    Ok(())
}

/// Open the viewer HTML file in the default browser (macOS).
pub fn open_viewer(path: &Path) {
    let _ = std::process::Command::new("open").arg(path).spawn();
}

fn render_html(board: &Board, routed_nets: &[RoutedNet]) -> String {
    let mut svg_elements = String::new();

    // Board dimensions for viewBox
    let margin = 5.0;
    let vb_x = -margin;
    let vb_y = -margin;
    let vb_w = board.width + margin * 2.0;
    let vb_h = board.height + margin * 2.0;

    // === Layer groups ===

    // 1. Copper zones (semi-transparent fills)
    svg_elements.push_str("  <g id=\"layer-zones\" class=\"layer-group\">\n");
    render_zones(board, &mut svg_elements);
    svg_elements.push_str("  </g>\n");

    // 2. B.Cu traces (blue, rendered first so F.Cu is on top)
    svg_elements.push_str("  <g id=\"layer-bcu\" class=\"layer-group\">\n");
    render_traces(routed_nets, 1, "#4444ff", &mut svg_elements);
    svg_elements.push_str("  </g>\n");

    // 3. F.Cu traces (red)
    svg_elements.push_str("  <g id=\"layer-fcu\" class=\"layer-group\">\n");
    render_traces(routed_nets, 0, "#ff3333", &mut svg_elements);
    svg_elements.push_str("  </g>\n");

    // 4. Vias
    svg_elements.push_str("  <g id=\"layer-vias\" class=\"layer-group\">\n");
    render_vias(routed_nets, &mut svg_elements);
    svg_elements.push_str("  </g>\n");

    // 5. Component courtyards (grey outlines)
    svg_elements.push_str("  <g id=\"layer-courtyard\" class=\"layer-group\">\n");
    render_courtyards(board, &mut svg_elements);
    svg_elements.push_str("  </g>\n");

    // 6. Pads
    svg_elements.push_str("  <g id=\"layer-pads\" class=\"layer-group\">\n");
    render_pads(board, &mut svg_elements);
    svg_elements.push_str("  </g>\n");

    // 7. Silkscreen
    svg_elements.push_str("  <g id=\"layer-silkscreen\" class=\"layer-group\">\n");
    render_silkscreen(board, &mut svg_elements);
    svg_elements.push_str("  </g>\n");

    // 8. Board outline (Edge.Cuts)
    svg_elements.push_str("  <g id=\"layer-edge-cuts\" class=\"layer-group\">\n");
    svg_elements.push_str(&format!(
        "    <rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"#cccc00\" stroke-width=\"0.15\" />\n",
        board.width, board.height
    ));
    svg_elements.push_str("  </g>\n");

    // Build net info JSON for hover tooltips
    let net_info_json = build_net_info_json(board, routed_nets);

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>pcb-forge Viewer</title>
<style>
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{ background: #1a1a2e; color: #e0e0e0; font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace; overflow: hidden; height: 100vh; }}
#toolbar {{
  position: fixed; top: 0; left: 0; right: 0; z-index: 10;
  background: #16213e; border-bottom: 1px solid #0f3460;
  padding: 8px 16px; display: flex; align-items: center; gap: 16px;
  font-size: 13px;
}}
#toolbar h1 {{ font-size: 14px; color: #e94560; font-weight: 600; white-space: nowrap; }}
.layer-toggle {{ display: flex; align-items: center; gap: 4px; cursor: pointer; user-select: none; }}
.layer-toggle input {{ cursor: pointer; }}
.layer-toggle .swatch {{ width: 12px; height: 12px; border-radius: 2px; display: inline-block; }}
#info-bar {{
  position: fixed; bottom: 0; left: 0; right: 0; z-index: 10;
  background: #16213e; border-top: 1px solid #0f3460;
  padding: 6px 16px; font-size: 12px; display: flex; gap: 24px;
}}
#info-bar .coord {{ color: #a0a0a0; }}
#info-bar .hover-info {{ color: #e94560; flex: 1; }}
#svg-container {{ position: absolute; top: 38px; bottom: 28px; left: 0; right: 0; cursor: grab; }}
#svg-container.dragging {{ cursor: grabbing; }}
svg {{ width: 100%; height: 100%; }}
.pad-hover:hover {{ filter: brightness(1.5); }}
.trace-hover:hover {{ filter: brightness(1.5); stroke-width: inherit; }}
.scale-bar {{ fill: none; stroke: #888; stroke-width: 0.1; }}
.scale-text {{ font-size: 1.2px; fill: #888; font-family: sans-serif; }}
</style>
</head>
<body>
<div id="toolbar">
  <h1>pcb-forge</h1>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="layer-edge-cuts"><span class="swatch" style="background:#cccc00"></span>Edge.Cuts</label>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="layer-fcu"><span class="swatch" style="background:#ff3333"></span>F.Cu</label>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="layer-bcu"><span class="swatch" style="background:#4444ff"></span>B.Cu</label>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="layer-pads"><span class="swatch" style="background:#c8a84e"></span>Pads</label>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="layer-vias"><span class="swatch" style="background:#888"></span>Vias</label>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="layer-zones"><span class="swatch" style="background:rgba(100,100,255,0.3)"></span>Zones</label>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="layer-silkscreen"><span class="swatch" style="background:#fff"></span>Silkscreen</label>
  <label class="layer-toggle"><input type="checkbox" checked data-layer="layer-courtyard"><span class="swatch" style="background:#666"></span>Courtyard</label>
</div>
<div id="svg-container">
<svg id="pcb-svg" xmlns="http://www.w3.org/2000/svg" viewBox="{vb_x} {vb_y} {vb_w} {vb_h}">
  <defs>
    <pattern id="grid-mm" width="1" height="1" patternUnits="userSpaceOnUse">
      <path d="M 1 0 L 0 0 0 1" fill="none" stroke="rgba(255,255,255,0.04)" stroke-width="0.02"/>
    </pattern>
    <pattern id="grid-5mm" width="5" height="5" patternUnits="userSpaceOnUse">
      <rect width="5" height="5" fill="url(#grid-mm)"/>
      <path d="M 5 0 L 0 0 0 5" fill="none" stroke="rgba(255,255,255,0.08)" stroke-width="0.03"/>
    </pattern>
  </defs>
  <!-- Background -->
  <rect x="{vb_x}" y="{vb_y}" width="{vb_w}" height="{vb_h}" fill="#0d3320"/>
  <!-- PCB body -->
  <rect x="0" y="0" width="{bw}" height="{bh}" fill="#1a5c36" rx="0.5" ry="0.5"/>
  <!-- Grid overlay -->
  <rect x="0" y="0" width="{bw}" height="{bh}" fill="url(#grid-5mm)"/>
  <!-- Scale bar -->
  {scale_bar}
  <!-- Layers -->
{svg_elements}
</svg>
</div>
<div id="info-bar">
  <span class="coord" id="coords">X: — Y: —</span>
  <span class="hover-info" id="hover-info">Hover over a pad or trace for details</span>
  <span class="coord">Board: {bw}mm × {bh}mm</span>
</div>

<script>
const NET_INFO = {net_info_json};

// --- Layer toggles ---
document.querySelectorAll('.layer-toggle input').forEach(cb => {{
  cb.addEventListener('change', () => {{
    const g = document.getElementById(cb.dataset.layer);
    if (g) g.style.display = cb.checked ? '' : 'none';
  }});
}});

// --- Pan & Zoom ---
const svgEl = document.getElementById('pcb-svg');
const container = document.getElementById('svg-container');
let viewBox = {{ x: {vb_x}, y: {vb_y}, w: {vb_w}, h: {vb_h} }};
let isPanning = false;
let panStart = {{ x: 0, y: 0 }};

function updateViewBox() {{
  svgEl.setAttribute('viewBox', `${{viewBox.x}} ${{viewBox.y}} ${{viewBox.w}} ${{viewBox.h}}`);
}}

container.addEventListener('wheel', (e) => {{
  e.preventDefault();
  const rect = svgEl.getBoundingClientRect();
  const mx = (e.clientX - rect.left) / rect.width;
  const my = (e.clientY - rect.top) / rect.height;
  const factor = e.deltaY > 0 ? 1.15 : 1 / 1.15;
  const newW = viewBox.w * factor;
  const newH = viewBox.h * factor;
  viewBox.x += (viewBox.w - newW) * mx;
  viewBox.y += (viewBox.h - newH) * my;
  viewBox.w = newW;
  viewBox.h = newH;
  updateViewBox();
}}, {{ passive: false }});

container.addEventListener('mousedown', (e) => {{
  if (e.button === 0) {{
    isPanning = true;
    panStart = {{ x: e.clientX, y: e.clientY }};
    container.classList.add('dragging');
  }}
}});

window.addEventListener('mousemove', (e) => {{
  // Update coordinates
  const rect = svgEl.getBoundingClientRect();
  const svgX = viewBox.x + (e.clientX - rect.left) / rect.width * viewBox.w;
  const svgY = viewBox.y + (e.clientY - rect.top) / rect.height * viewBox.h;
  document.getElementById('coords').textContent = `X: ${{svgX.toFixed(2)}}mm  Y: ${{svgY.toFixed(2)}}mm`;

  if (isPanning) {{
    const dx = (e.clientX - panStart.x) / rect.width * viewBox.w;
    const dy = (e.clientY - panStart.y) / rect.height * viewBox.h;
    viewBox.x -= dx;
    viewBox.y -= dy;
    panStart = {{ x: e.clientX, y: e.clientY }};
    updateViewBox();
  }}
}});

window.addEventListener('mouseup', () => {{
  isPanning = false;
  container.classList.remove('dragging');
}});

// --- Hover info ---
svgEl.addEventListener('mouseover', (e) => {{
  const el = e.target.closest('[data-info]');
  if (el) {{
    document.getElementById('hover-info').textContent = el.dataset.info;
  }}
}});
svgEl.addEventListener('mouseout', (e) => {{
  const el = e.target.closest('[data-info]');
  if (el) {{
    document.getElementById('hover-info').textContent = 'Hover over a pad or trace for details';
  }}
}});

// --- Keyboard shortcuts ---
document.addEventListener('keydown', (e) => {{
  if (e.key === 'f' || e.key === 'F') {{
    // Fit to view
    viewBox = {{ x: {vb_x}, y: {vb_y}, w: {vb_w}, h: {vb_h} }};
    updateViewBox();
  }}
}});
</script>
</body>
</html>"##,
        vb_x = vb_x,
        vb_y = vb_y,
        vb_w = vb_w,
        vb_h = vb_h,
        bw = board.width,
        bh = board.height,
        scale_bar = render_scale_bar(board),
        svg_elements = svg_elements,
        net_info_json = net_info_json,
    )
}

fn render_scale_bar(board: &Board) -> String {
    // Place a scale bar at bottom-left outside the board
    let bar_y = board.height + 2.5;
    let bar_len = 10.0; // 10mm
    format!(
        concat!(
            "<line x1=\"0\" y1=\"{y}\" x2=\"{len}\" y2=\"{y}\" class=\"scale-bar\"/>",
            "<line x1=\"0\" y1=\"{y1}\" x2=\"0\" y2=\"{y2}\" class=\"scale-bar\"/>",
            "<line x1=\"{len}\" y1=\"{y1}\" x2=\"{len}\" y2=\"{y2}\" class=\"scale-bar\"/>",
            "<text x=\"{tx}\" y=\"{ty}\" class=\"scale-text\" text-anchor=\"middle\" font-size=\"1.2\" font-family=\"sans-serif\" fill=\"#888\">10mm</text>",
        ),
        y = bar_y,
        y1 = bar_y - 0.5,
        y2 = bar_y + 0.5,
        len = bar_len,
        tx = bar_len / 2.0,
        ty = bar_y + 1.8,
    )
}

fn render_zones(board: &Board, out: &mut String) {
    // GND zone on B.Cu
    let has_gnd = board.nets.iter().any(|n| n.name == "GND");
    if has_gnd {
        out.push_str(&format!(
            "    <rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"rgba(68,68,255,0.12)\" data-info=\"Zone: GND (B.Cu)\" />\n",
            board.width, board.height
        ));
    }
    // VCC3V3 zone on F.Cu
    let has_vcc = board.nets.iter().any(|n| n.name == "VCC3V3");
    if has_vcc {
        out.push_str(&format!(
            "    <rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"rgba(255,68,68,0.08)\" data-info=\"Zone: VCC3V3 (F.Cu)\" />\n",
            board.width, board.height
        ));
    }
}

fn render_traces(routed_nets: &[RoutedNet], layer: u8, color: &str, out: &mut String) {
    for rn in routed_nets {
        for seg in &rn.segments {
            if seg.layer != layer {
                continue;
            }
            out.push_str(&format!(
                "    <line x1=\"{:.3}\" y1=\"{:.3}\" x2=\"{:.3}\" y2=\"{:.3}\" stroke=\"{}\" stroke-width=\"{:.3}\" stroke-linecap=\"round\" class=\"trace-hover\" data-info=\"Trace: {} | Layer: {} | Width: {:.2}mm\" />\n",
                seg.start.0, seg.start.1, seg.end.0, seg.end.1,
                color, seg.width,
                rn.name,
                if layer == 0 { "F.Cu" } else { "B.Cu" },
                seg.width,
            ));
        }
    }
}

fn render_vias(routed_nets: &[RoutedNet], out: &mut String) {
    for rn in routed_nets {
        for via in &rn.vias {
            // Outer annular ring
            out.push_str(&format!(
                "    <circle cx=\"{:.3}\" cy=\"{:.3}\" r=\"{:.3}\" fill=\"#888\" stroke=\"#aaa\" stroke-width=\"0.05\" data-info=\"Via: {} | Drill: {:.2}mm | Size: {:.2}mm\" class=\"pad-hover\" />\n",
                via.x, via.y, via.size / 2.0,
                rn.name, via.drill, via.size,
            ));
            // Drill hole
            out.push_str(&format!(
                "    <circle cx=\"{:.3}\" cy=\"{:.3}\" r=\"{:.3}\" fill=\"#1a5c36\" />\n",
                via.x, via.y, via.drill / 2.0,
            ));
        }
    }
}

fn render_courtyards(board: &Board, out: &mut String) {
    for comp in &board.components {
        if let Some(ref fp) = comp.footprint_data {
            // Draw courtyard/fab lines
            let crtyd_lines: Vec<_> = fp
                .lines
                .iter()
                .filter(|l| l.layer.contains("CrtYd") || l.layer.contains("Fab"))
                .collect();

            if crtyd_lines.is_empty() {
                // Fallback: draw bounding box from courtyard_bounds
                let (min_x, min_y, max_x, max_y) = fp.courtyard_bounds();
                out.push_str(&format!(
                    "    <rect x=\"{:.3}\" y=\"{:.3}\" width=\"{:.3}\" height=\"{:.3}\" fill=\"none\" stroke=\"#666\" stroke-width=\"0.08\" stroke-dasharray=\"0.3,0.15\" data-info=\"{} ({})\" />\n",
                    comp.x + min_x, comp.y + min_y,
                    max_x - min_x, max_y - min_y,
                    comp.ref_des, comp.value,
                ));
            } else {
                for line in &crtyd_lines {
                    let (sx, sy) = rotate_point(line.start.0, line.start.1, comp.rotation);
                    let (ex, ey) = rotate_point(line.end.0, line.end.1, comp.rotation);
                    out.push_str(&format!(
                        "    <line x1=\"{:.3}\" y1=\"{:.3}\" x2=\"{:.3}\" y2=\"{:.3}\" stroke=\"#666\" stroke-width=\"0.08\" data-info=\"{} ({})\" />\n",
                        comp.x + sx, comp.y + sy,
                        comp.x + ex, comp.y + ey,
                        comp.ref_des, comp.value,
                    ));
                }
            }
        }
    }
}

fn render_pads(board: &Board, out: &mut String) {
    // Build a lookup: (component_name, pin_number) -> net_name
    let mut pin_net_map: std::collections::HashMap<(String, String), String> = std::collections::HashMap::new();
    for net in &board.nets {
        for pref in &net.pins {
            // Find the component to get the pin number from pin name
            if let Some(comp) = board.components.iter().find(|c| c.name == pref.component) {
                if let Some(pin) = comp.pins.iter().find(|p| p.name == pref.pin) {
                    pin_net_map.insert((comp.name.clone(), pin.number.clone()), net.name.clone());
                }
            }
        }
    }

    for comp in &board.components {
        if let Some(ref fp) = comp.footprint_data {
            for pad in &fp.pads {
                // Skip paste-only helper pads (no Cu layers)
                if !pad.layers.iter().any(|l| l.contains("Cu")) {
                    continue;
                }

                let (rx, ry) = rotate_point(pad.at_x, pad.at_y, comp.rotation);
                let px = comp.x + rx;
                let py = comp.y + ry;

                let net_name = pin_net_map
                    .get(&(comp.name.clone(), pad.number.clone()))
                    .map(|s| s.as_str())
                    .unwrap_or("unconnected");

                // Find pin name for this pad number
                let pin_name = comp
                    .pins
                    .iter()
                    .find(|p| p.number == pad.number)
                    .map(|p| p.name.as_str())
                    .unwrap_or(&pad.number);

                let info = format!(
                    "{}.{} (pad {}) | Net: {} | {:.2}×{:.2}mm",
                    comp.ref_des, pin_name, pad.number, net_name, pad.size_w, pad.size_h
                );

                // Determine color based on pad type/layers
                // Handle wildcard layers: "*.Cu" means all copper layers (through-hole)
                let has_wildcard_cu = pad.layers.iter().any(|l| l == "*.Cu");
                let is_front = has_wildcard_cu || pad.layers.iter().any(|l| l == "F.Cu");
                let is_back = has_wildcard_cu || pad.layers.iter().any(|l| l == "B.Cu");
                let color = if is_front && is_back {
                    "#c8a84e" // through-hole: gold
                } else if is_front {
                    "#d4564e" // front: reddish
                } else if is_back {
                    "#5e6ed4" // back: bluish
                } else {
                    "#c8a84e"
                };

                let (sw, sh) = rotate_size(pad.size_w, pad.size_h, comp.rotation);

                match pad.shape.as_str() {
                    "circle" => {
                        let r = sw.max(sh) / 2.0;
                        out.push_str(&format!(
                            "    <circle cx=\"{:.3}\" cy=\"{:.3}\" r=\"{:.3}\" fill=\"{}\" opacity=\"0.85\" class=\"pad-hover\" data-info=\"{}\" />\n",
                            px, py, r, color, info,
                        ));
                    }
                    _ => {
                        // rect, roundrect, oval → all as rounded rects
                        let rx = if pad.shape == "roundrect" || pad.shape == "oval" {
                            (sw.min(sh) * 0.25).min(0.3)
                        } else {
                            0.05
                        };
                        out.push_str(&format!(
                            "    <rect x=\"{:.3}\" y=\"{:.3}\" width=\"{:.3}\" height=\"{:.3}\" rx=\"{:.3}\" fill=\"{}\" opacity=\"0.85\" class=\"pad-hover\" data-info=\"{}\" />\n",
                            px - sw / 2.0, py - sh / 2.0,
                            sw, sh, rx, color, info,
                        ));
                    }
                }

                // Draw drill hole for through-hole pads
                if let Some(drill) = pad.drill {
                    out.push_str(&format!(
                        "    <circle cx=\"{:.3}\" cy=\"{:.3}\" r=\"{:.3}\" fill=\"#1a5c36\" />\n",
                        px, py, drill / 2.0,
                    ));
                }
            }
        }
    }
}

fn render_silkscreen(board: &Board, out: &mut String) {
    for comp in &board.components {
        if let Some(ref fp) = comp.footprint_data {
            // Draw silkscreen lines from footprint
            for line in &fp.lines {
                if !line.layer.contains("SilkS") {
                    continue;
                }
                let (sx, sy) = rotate_point(line.start.0, line.start.1, comp.rotation);
                let (ex, ey) = rotate_point(line.end.0, line.end.1, comp.rotation);
                out.push_str(&format!(
                    "    <line x1=\"{:.3}\" y1=\"{:.3}\" x2=\"{:.3}\" y2=\"{:.3}\" stroke=\"white\" stroke-width=\"{:.3}\" stroke-linecap=\"round\" opacity=\"0.9\" />\n",
                    comp.x + sx, comp.y + sy,
                    comp.x + ex, comp.y + ey,
                    line.width.max(0.1),
                ));
            }
        }

        // Reference designator label - centered on component with proportional size
        let font_size = if let Some(ref fp) = comp.footprint_data {
            let (min_x, min_y, max_x, max_y) = fp.courtyard_bounds();
            let comp_w = max_x - min_x;
            let comp_h = max_y - min_y;
            (comp_w.min(comp_h) * 0.25).clamp(0.6, 2.0)
        } else {
            0.8
        };

        // Offset y by ~0.35 * font_size for visual vertical centering (resvg compatible)
        out.push_str(&format!(
            "    <text x=\"{:.3}\" y=\"{:.3}\" fill=\"#ffdd00\" font-size=\"{:.2}\" font-family=\"sans-serif\" text-anchor=\"middle\" data-info=\"{} ({})\">{}</text>\n",
            comp.x, comp.y + font_size * 0.35,
            font_size,
            comp.ref_des, comp.value,
            comp.value,
        ));
    }
}

fn build_net_info_json(board: &Board, routed_nets: &[RoutedNet]) -> String {
    let mut entries = Vec::new();
    for net in &board.nets {
        let routed = routed_nets.iter().find(|r| r.name == net.name);
        let seg_count = routed.map_or(0, |r| r.segments.len());
        let via_count = routed.map_or(0, |r| r.vias.len());
        let pins: Vec<String> = net
            .pins
            .iter()
            .map(|p| format!("{}.{}", p.component, p.pin))
            .collect();
        entries.push(format!(
            "\"{}\":{{\"pins\":[{}],\"segments\":{},\"vias\":{}}}",
            net.name,
            pins.iter()
                .map(|p| format!("\"{}\"", p))
                .collect::<Vec<_>>()
                .join(","),
            seg_count,
            via_count,
        ));
    }
    format!("{{{}}}", entries.join(","))
}

/// Rotate a point (x, y) around origin by `degrees`.
fn rotate_point(x: f64, y: f64, degrees: f64) -> (f64, f64) {
    if degrees.abs() < 0.01 {
        return (x, y);
    }
    let rad = degrees.to_radians();
    let cos = rad.cos();
    let sin = rad.sin();
    (x * cos - y * sin, x * sin + y * cos)
}

/// Get the effective size of a pad after rotation (swap w/h for 90/270 deg).
fn rotate_size(w: f64, h: f64, degrees: f64) -> (f64, f64) {
    let d = ((degrees % 360.0) + 360.0) % 360.0;
    if (d - 90.0).abs() < 1.0 || (d - 270.0).abs() < 1.0 {
        (h, w)
    } else {
        (w, h)
    }
}

/// Generate a PNG preview image of the PCB.
pub fn generate_png(board: &Board, routed_nets: &[RoutedNet], output_path: &Path) -> Result<()> {
    let svg_str = render_standalone_svg(board, routed_nets);

    // Load system fonts so text labels render correctly
    let mut fontdb = resvg::usvg::fontdb::Database::new();
    fontdb.load_system_fonts();
    let options = resvg::usvg::Options {
        fontdb: std::sync::Arc::new(fontdb),
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_str(&svg_str, &options)
        .context("Failed to parse SVG for PNG rendering")?;

    let svg_size = tree.size();

    // Calculate pixel dimensions: at least 1920px wide, proportional height
    let min_width = 1920u32;
    let scale = (min_width as f32) / svg_size.width();
    let px_w = min_width;
    let px_h = (svg_size.height() * scale).ceil() as u32;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(px_w, px_h)
        .context("Failed to create pixmap")?;

    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    pixmap
        .save_png(output_path)
        .context("Failed to save PNG")?;

    println!("  → {}", output_path.display());
    Ok(())
}

/// Render a standalone SVG document (no HTML/JS) suitable for rasterization.
pub fn render_standalone_svg(board: &Board, routed_nets: &[RoutedNet]) -> String {
    let mut svg_elements = String::new();

    let margin = 5.0;
    let vb_x = -margin;
    let vb_y = -margin;
    let vb_w = board.width + margin * 2.0;
    let vb_h = board.height + margin * 2.0;

    // Zones
    svg_elements.push_str("  <g>\n");
    render_zones(board, &mut svg_elements);
    svg_elements.push_str("  </g>\n");

    // B.Cu traces
    svg_elements.push_str("  <g>\n");
    render_traces(routed_nets, 1, "#4444ff", &mut svg_elements);
    svg_elements.push_str("  </g>\n");

    // F.Cu traces
    svg_elements.push_str("  <g>\n");
    render_traces(routed_nets, 0, "#ff3333", &mut svg_elements);
    svg_elements.push_str("  </g>\n");

    // Vias
    svg_elements.push_str("  <g>\n");
    render_vias(routed_nets, &mut svg_elements);
    svg_elements.push_str("  </g>\n");

    // Courtyards
    svg_elements.push_str("  <g>\n");
    render_courtyards(board, &mut svg_elements);
    svg_elements.push_str("  </g>\n");

    // Pads
    svg_elements.push_str("  <g>\n");
    render_pads(board, &mut svg_elements);
    svg_elements.push_str("  </g>\n");

    // Silkscreen
    svg_elements.push_str("  <g>\n");
    render_silkscreen(board, &mut svg_elements);
    svg_elements.push_str("  </g>\n");

    // Board outline
    svg_elements.push_str(&format!(
        "  <rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"#cccc00\" stroke-width=\"0.15\" />\n",
        board.width, board.height
    ));

    // Scale bar
    let scale_bar = render_scale_bar(board);

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vb_x} {vb_y} {vb_w} {vb_h}" width="{vb_w}" height="{vb_h}">
  <!-- Background -->
  <rect x="{vb_x}" y="{vb_y}" width="{vb_w}" height="{vb_h}" fill="#0d3320"/>
  <!-- PCB body -->
  <rect x="0" y="0" width="{bw}" height="{bh}" fill="#1a5c36" rx="0.5" ry="0.5"/>
  <!-- Scale bar -->
  {scale_bar}
  <!-- Layers -->
{svg_elements}
</svg>"##,
        vb_x = vb_x,
        vb_y = vb_y,
        vb_w = vb_w,
        vb_h = vb_h,
        bw = board.width,
        bh = board.height,
        scale_bar = scale_bar,
        svg_elements = svg_elements,
    )
}
