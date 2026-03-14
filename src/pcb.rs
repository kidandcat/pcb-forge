use anyhow::Result;
use std::io::Write;
use std::path::Path;
use uuid::Uuid;

use crate::router::RoutedNet;
use crate::schema::{Board, Component, Layer};

// Placement constraints
const MIN_COURTYARD_CLEARANCE: f64 = 0.5; // mm, minimum gap between component courtyards
// Stitching via parameters
const STITCHING_VIA_SPACING: f64 = 10.0; // mm, grid spacing
const STITCHING_VIA_SIZE: f64 = 0.6; // mm
const STITCHING_VIA_DRILL: f64 = 0.3; // mm
const STITCHING_VIA_MARGIN: f64 = 3.0; // mm, margin from board edge

/// Check if a component is a protection device (TVS, ESD, Zener)
fn is_protection_component(comp: &Component) -> bool {
    let ref_lower = comp.ref_des.to_lowercase();
    let val_lower = comp.value.to_lowercase();
    let desc_lower = comp
        .description
        .as_deref()
        .unwrap_or("")
        .to_lowercase();

    let is_diode_ref = ref_lower.starts_with('d');
    let has_protection_keyword = val_lower.contains("tvs")
        || val_lower.contains("esd")
        || val_lower.contains("zener")
        || val_lower.contains("protection")
        || desc_lower.contains("tvs")
        || desc_lower.contains("esd")
        || desc_lower.contains("protection");

    (is_diode_ref && has_protection_keyword) || has_protection_keyword
}

/// Check if a component is a decoupling/bypass capacitor (100nF)
fn is_decoupling_cap(comp: &Component) -> bool {
    let ref_lower = comp.ref_des.to_lowercase();
    let val_lower = comp.value.to_lowercase().replace(' ', "");
    let fp_lower = comp.footprint.to_lowercase();

    let is_cap = ref_lower.starts_with('c') || fp_lower.contains("capacitor");
    let is_100nf = val_lower == "100nf"
        || val_lower == "0.1uf"
        || val_lower == "100n"
        || val_lower == "0.1μf";

    is_cap && is_100nf
}

/// Check if a component is a connector (expanded detection).
/// Matches ref_des J*, or footprint containing USB, connector, jack, header, SMA, barrel.
fn is_connector_component(comp: &Component) -> bool {
    let ref_lower = comp.ref_des.to_lowercase();
    let fp_lower = comp.footprint.to_lowercase();
    ref_lower.starts_with('j')
        || fp_lower.contains("connector")
        || fp_lower.contains("usb")
        || fp_lower.contains("jst")
        || fp_lower.contains("jack")
        || fp_lower.contains("header")
        || fp_lower.contains("sma")
        || fp_lower.contains("barrel")
}

/// Check if a component is an IC (multi-pin active device)
fn is_ic_component(comp: &Component) -> bool {
    let fp_lower = comp.footprint.to_lowercase();
    comp.pins.len() > 4
        && !is_connector_component(comp)
        && !fp_lower.contains("capacitor")
        && !fp_lower.contains("resistor")
        && !fp_lower.contains("inductor")
        && !fp_lower.contains("led")
        && !fp_lower.contains("button")
        && !fp_lower.contains("switch")
        && !fp_lower.contains("diode")
}

/// Check if a net name is a VCC/power supply net (excluding GND)
fn is_vcc_net(name: &str) -> bool {
    let upper = name.to_uppercase();
    upper.starts_with("VCC")
        || upper.starts_with("VDD")
        || upper.starts_with("V3V3")
        || upper == "3V3"
        || upper == "5V"
}

/// Compute component sizes for placement with minimum courtyard clearance
fn compute_placement_sizes(components: &[Component]) -> Vec<(f64, f64)> {
    components
        .iter()
        .map(|c| {
            if let Some(fp) = &c.footprint_data {
                let (min_x, min_y, max_x, max_y) = fp.placement_bounds();
                (
                    max_x - min_x + 2.0 * MIN_COURTYARD_CLEARANCE,
                    max_y - min_y + 2.0 * MIN_COURTYARD_CLEARANCE,
                )
            } else {
                (12.0, 8.0)
            }
        })
        .collect()
}

/// Configuration for a single placement variant.
#[derive(Debug, Clone)]
pub struct PlacementConfig {
    /// Seed for deterministic pseudo-random decisions.
    pub seed: u64,
    /// Spacing multiplier (1.0 = default, >1 = more spread, <1 = tighter).
    pub spacing_mult: f64,
    /// Center offset angle (radians) — rotates starting placement direction.
    pub center_angle: f64,
}

/// Score for a routed placement variant.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlacementScore {
    pub nets_routed: usize,
    pub total_nets: usize,
    pub total_trace_length: f64,
    pub via_count: usize,
    pub clearance_violations: usize,
    pub composite: f64,
}

impl PlacementScore {
    pub fn compute(routed_nets: &[crate::router::RoutedNet], total_nets: usize) -> Self {
        let nets_routed = routed_nets
            .iter()
            .filter(|rn| !rn.segments.is_empty())
            .count();
        let total_trace_length: f64 = routed_nets
            .iter()
            .flat_map(|rn| &rn.segments)
            .map(|s| {
                ((s.end.0 - s.start.0).powi(2) + (s.end.1 - s.start.1).powi(2)).sqrt()
            })
            .sum();
        let via_count: usize = routed_nets.iter().map(|rn| rn.vias.len()).sum();
        let clearance_violations = 0; // TODO: implement DRC check

        let composite = (nets_routed as f64) * 1000.0
            - total_trace_length * 0.1
            - (via_count as f64) * 50.0
            - (clearance_violations as f64) * 10000.0;

        Self {
            nets_routed,
            total_nets,
            total_trace_length,
            via_count,
            clearance_violations,
            composite,
        }
    }
}

const NUM_VARIANTS: usize = 10;

/// Generate 10 placement variant configs with different seeds and strategies.
pub fn generate_placement_configs() -> Vec<PlacementConfig> {
    (0..NUM_VARIANTS as u64)
        .map(|i| PlacementConfig {
            seed: i * 7919 + 42, // distinct primes for variety
            spacing_mult: 1.0 + (i as f64 - 4.5) * 0.08, // range ~0.64..1.36
            center_angle: (i as f64) * std::f64::consts::PI * 2.0 / NUM_VARIANTS as f64,
        })
        .collect()
}

/// Generate a single placement variant: returns a Board with components placed.
pub fn generate_placement(board: &Board, config: &PlacementConfig) -> Board {
    let mut variant = board.clone();
    place_components_with_config(&mut variant, config);
    variant
}

/// Write a KiCad PCB file from a board that already has placement applied.
pub fn write_pcb_file(board: &Board, output: &Path) -> Result<()> {
    let mut buf = String::new();
    write_pcb_header(&mut buf);
    write_layers(&mut buf);
    write_setup(&mut buf, board);
    write_nets(&mut buf, board);
    write_board_outline(&mut buf, board);
    write_components(&mut buf, board);
    write_zones(&mut buf, board);
    write_stitching_vias(&mut buf, board);
    buf.push_str(")\n");

    let mut file = std::fs::File::create(output)?;
    file.write_all(buf.as_bytes())?;
    Ok(())
}

pub fn generate_pcb(board: &mut Board, output: &Path) -> Result<()> {
    let config = PlacementConfig {
        seed: 42,
        spacing_mult: 1.0,
        center_angle: 0.0,
    };
    place_components_with_config(board, &config);
    write_pcb_file(board, output)
}

/// Simple LCG pseudo-random number generator (deterministic, no external deps).
struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self { state: seed.wrapping_add(1) }
    }
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.state
    }
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Shuffle a slice in place (Fisher-Yates).
    fn shuffle<T>(&mut self, slice: &mut [T]) {
        for i in (1..slice.len()).rev() {
            let j = (self.next_u64() as usize) % (i + 1);
            slice.swap(i, j);
        }
    }
}

/// Determine the preferred edge for a connector based on its type.
/// Returns (edge_side, rotation) where edge_side is 0=left, 1=right, 2=top, 3=bottom.
fn connector_preferred_edge(comp: &Component, seed_hint: u64) -> (usize, f64) {
    let fp_lower = comp.footprint.to_lowercase();
    let val_lower = comp.value.to_lowercase();

    if fp_lower.contains("usb") {
        // USB: top or left
        if seed_hint % 2 == 0 { (2, 0.0) } else { (0, 90.0) }
    } else if fp_lower.contains("header") || fp_lower.contains("sma") {
        // Headers/SMA: top
        (2, 0.0)
    } else if fp_lower.contains("barrel") || fp_lower.contains("jack")
        || val_lower.contains("power") || val_lower.contains("dc")
    {
        // Power jacks: bottom-left corner area
        (3, 0.0)
    } else if fp_lower.contains("jst") || fp_lower.contains("battery") || val_lower.contains("battery") {
        // Battery connectors: bottom
        (3, 0.0)
    } else {
        // Generic connectors: distribute across edges based on seed
        ((seed_hint as usize) % 4, 0.0)
    }
}

/// Place a connector on a specific board edge.
/// edge: 0=left, 1=right, 2=top, 3=bottom.
fn place_on_edge(
    edge: usize,
    sizes: &[(f64, f64)],
    positions: &[Option<(f64, f64)>],
    my_idx: usize,
    board_width: f64,
    board_height: f64,
    margin: f64,
    hint_pos: f64, // position along the edge (0.0-1.0)
) -> (f64, f64) {
    let (mw, mh) = sizes[my_idx];
    let hw = mw / 2.0;
    let hh = mh / 2.0;

    let (base_x, base_y) = match edge {
        0 => (margin + hw, margin + hh + hint_pos * (board_height - 2.0 * margin - mh)), // left
        1 => (board_width - margin - hw, margin + hh + hint_pos * (board_height - 2.0 * margin - mh)), // right
        2 => (margin + hw + hint_pos * (board_width - 2.0 * margin - mw), margin + hh), // top
        _ => (margin + hw + hint_pos * (board_width - 2.0 * margin - mw), board_height - margin - hh), // bottom
    };

    // Check for overlaps and slide along edge if needed
    let best = (base_x, base_y);
    let no_overlap = |px: f64, py: f64| -> bool {
        !positions.iter().enumerate().any(|(i, pos)| {
            if i == my_idx { return false; }
            if let Some((cx, cy)) = pos {
                let (cw, ch) = sizes[i];
                (px - cx).abs() < (hw + cw / 2.0) && (py - cy).abs() < (hh + ch / 2.0)
            } else { false }
        })
    };

    if no_overlap(best.0, best.1) {
        return best;
    }

    // Slide along the edge
    for offset in 1..40 {
        let s = offset as f64 * 2.0;
        let candidates: Vec<(f64, f64)> = match edge {
            0 | 1 => vec![(best.0, best.1 + s), (best.0, best.1 - s)],
            _ => vec![(best.0 + s, best.1), (best.0 - s, best.1)],
        };
        for (px, py) in candidates {
            if px - hw < margin || px + hw > board_width - margin
                || py - hh < margin || py + hh > board_height - margin
            {
                continue;
            }
            if no_overlap(px, py) {
                return (px, py);
            }
        }
    }

    // Fallback to general edge finder
    find_edge_position(
        sizes, positions, my_idx, base_x, base_y,
        margin, board_width - margin, margin, board_height - margin,
    )
}

/// Place components using connectivity-aware algorithm with configurable seed/strategy.
fn place_components_with_config(board: &mut Board, config: &PlacementConfig) {
    let margin = 5.0;
    let n = board.components.len();
    if n == 0 {
        return;
    }

    auto_size_board_if_needed(board, margin);

    let mut rng = SimpleRng::new(config.seed);

    // 1. Build connectivity adjacency matrix
    let mut adj = vec![vec![0usize; n]; n];
    for net in &board.nets {
        let mut comp_indices: Vec<usize> = Vec::new();
        for pr in &net.pins {
            if let Some(idx) = board.components.iter().position(|c| c.name == pr.component) {
                if !comp_indices.contains(&idx) {
                    comp_indices.push(idx);
                }
            }
        }
        for i in 0..comp_indices.len() {
            for j in (i + 1)..comp_indices.len() {
                adj[comp_indices[i]][comp_indices[j]] += 1;
                adj[comp_indices[j]][comp_indices[i]] += 1;
            }
        }
    }

    let sizes = compute_placement_sizes(&board.components);
    let mut positions: Vec<Option<(f64, f64)>> = vec![None; n];

    // 2. Separate connectors from non-connectors
    let connector_indices: Vec<usize> = (0..n)
        .filter(|&i| is_connector_component(&board.components[i]))
        .collect();
    let non_connector_indices: Vec<usize> = (0..n)
        .filter(|&i| !is_connector_component(&board.components[i]))
        .collect();

    // 3. Place connectors on board edges first
    for (ci, &comp_idx) in connector_indices.iter().enumerate() {
        let (edge, rotation) = connector_preferred_edge(
            &board.components[comp_idx],
            config.seed + ci as u64,
        );
        let hint = if connector_indices.len() > 1 {
            (ci as f64 + 0.5) / connector_indices.len() as f64
        } else {
            0.5
        };
        let (px, py) = place_on_edge(
            edge, &sizes, &positions, comp_idx,
            board.width, board.height, margin, hint,
        );
        positions[comp_idx] = Some((px, py));
        board.components[comp_idx].rotation = rotation;
    }

    // 4. Find most-connected non-connector component → place near center (with offset from config)
    if non_connector_indices.is_empty() {
        // Only connectors — apply and return
        for (i, comp) in board.components.iter_mut().enumerate() {
            if let Some((x, y)) = positions[i] {
                comp.x = x;
                comp.y = y;
            }
        }
        return;
    }

    let total_conn: Vec<usize> = (0..n).map(|i| adj[i].iter().sum()).collect();

    // Shuffle non-connector indices by seed to vary which component gets center priority
    let mut ordered_non_conn = non_connector_indices.clone();
    rng.shuffle(&mut ordered_non_conn);

    // Among top-connected, pick based on shuffled order
    ordered_non_conn.sort_by(|&a, &b| total_conn[b].cmp(&total_conn[a]));
    // Pick from top 3 most connected based on seed
    let top_pick = (config.seed as usize) % ordered_non_conn.len().min(3);
    let center_idx = ordered_non_conn[top_pick];

    let cx = board.width / 2.0 + config.center_angle.cos() * 3.0 * config.spacing_mult;
    let cy = board.height / 2.0 + config.center_angle.sin() * 3.0 * config.spacing_mult;
    positions[center_idx] = Some((cx.clamp(margin + 5.0, board.width - margin - 5.0),
                                   cy.clamp(margin + 5.0, board.height - margin - 5.0)));

    // 5. Place remaining non-connector components using connectivity-greedy with seed-varied order
    let mut remaining: Vec<usize> = non_connector_indices
        .iter()
        .copied()
        .filter(|&i| i != center_idx)
        .collect();
    rng.shuffle(&mut remaining);

    // Sort by connectivity to placed components (greedy), but with some randomization
    while !remaining.is_empty() {
        // Score each remaining component by connectivity to placed components
        let mut scored: Vec<(usize, usize, f64)> = remaining
            .iter()
            .enumerate()
            .map(|(ri, &comp_idx)| {
                let mut conn_score = 0usize;
                for placed_idx in 0..n {
                    if positions[placed_idx].is_some() {
                        conn_score += adj[comp_idx][placed_idx];
                    }
                }
                let noise = rng.next_f64() * 0.3; // small randomization
                (ri, conn_score, conn_score as f64 + noise)
            })
            .collect();
        scored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());

        let best_ri = scored[0].0;
        let comp_idx = remaining.remove(best_ri);

        // Find best placed neighbor
        let mut best_neighbor = center_idx;
        let mut best_conn = 0usize;
        for placed_idx in 0..n {
            if positions[placed_idx].is_some() && adj[comp_idx][placed_idx] > best_conn {
                best_conn = adj[comp_idx][placed_idx];
                best_neighbor = placed_idx;
            }
        }

        let (target_x, target_y) = positions[best_neighbor].unwrap_or((cx, cy));
        let step = 1.5 * config.spacing_mult;

        let (px, py) = find_non_overlapping_with_step(
            target_x, target_y, &sizes, &positions, comp_idx,
            margin, board.width - margin, margin, board.height - margin,
            step,
        );

        positions[comp_idx] = Some((px, py));
    }

    // 6. Apply all positions
    for (i, comp) in board.components.iter_mut().enumerate() {
        if let Some((x, y)) = positions[i] {
            comp.x = x;
            comp.y = y;
        }
    }

    // 7. Post-placement fixes
    post_place_protection_near_connectors(board);
    post_place_decoupling_near_ics(board);
}

/// Auto-calculate board dimensions if not specified (width or height == 0).
/// Estimates from total component area.
fn auto_size_board_if_needed(board: &mut Board, margin: f64) {
    if board.width > 0.0 && board.height > 0.0 {
        return;
    }

    let sizes = compute_placement_sizes(&board.components);
    let total_area: f64 = sizes.iter().map(|(w, h)| w * h).sum();
    let side = (total_area * 3.0).sqrt().max(30.0);
    if board.width <= 0.0 {
        board.width = side;
    }
    if board.height <= 0.0 {
        board.height = side * 0.75;
    }
    let _ = margin; // used by callers for placement margins
}

/// Find a non-overlapping position near (tx, ty) using spiral search.
fn find_non_overlapping(
    tx: f64, ty: f64,
    sizes: &[(f64, f64)],
    positions: &[Option<(f64, f64)>],
    my_idx: usize,
    min_x: f64, max_x: f64, min_y: f64, max_y: f64,
) -> (f64, f64) {
    let (mw, mh) = sizes[my_idx];
    let step = 1.5;

    for r in 0..80 {
        let radius = r as f64 * step;
        let points: Vec<(f64, f64)> = if r == 0 {
            vec![(tx, ty)]
        } else {
            let n_pts = (r * 8).max(8) as usize;
            (0..n_pts)
                .map(|i| {
                    let a = 2.0 * std::f64::consts::PI * i as f64 / n_pts as f64;
                    (tx + radius * a.cos(), ty + radius * a.sin())
                })
                .collect()
        };

        for (px, py) in points {
            let hw = mw / 2.0;
            let hh = mh / 2.0;
            if px - hw < min_x || px + hw > max_x || py - hh < min_y || py + hh > max_y {
                continue;
            }
            let overlaps = positions.iter().enumerate().any(|(i, pos)| {
                if i == my_idx {
                    return false;
                }
                if let Some((cx, cy)) = pos {
                    let (cw, ch) = sizes[i];
                    (px - cx).abs() < (hw + cw / 2.0) && (py - cy).abs() < (hh + ch / 2.0)
                } else {
                    false
                }
            });
            if !overlaps {
                return (px, py);
            }
        }
    }
    (
        tx.clamp(min_x + mw / 2.0, max_x - mw / 2.0),
        ty.clamp(min_y + mh / 2.0, max_y - mh / 2.0),
    )
}

/// Find a non-overlapping position near (tx, ty) using spiral search with configurable step.
fn find_non_overlapping_with_step(
    tx: f64, ty: f64,
    sizes: &[(f64, f64)],
    positions: &[Option<(f64, f64)>],
    my_idx: usize,
    min_x: f64, max_x: f64, min_y: f64, max_y: f64,
    step: f64,
) -> (f64, f64) {
    let (mw, mh) = sizes[my_idx];

    for r in 0..80 {
        let radius = r as f64 * step;
        let points: Vec<(f64, f64)> = if r == 0 {
            vec![(tx, ty)]
        } else {
            let n_pts = (r * 8).max(8) as usize;
            (0..n_pts)
                .map(|i| {
                    let a = 2.0 * std::f64::consts::PI * i as f64 / n_pts as f64;
                    (tx + radius * a.cos(), ty + radius * a.sin())
                })
                .collect()
        };

        for (px, py) in points {
            let hw = mw / 2.0;
            let hh = mh / 2.0;
            if px - hw < min_x || px + hw > max_x || py - hh < min_y || py + hh > max_y {
                continue;
            }
            let overlaps = positions.iter().enumerate().any(|(i, pos)| {
                if i == my_idx { return false; }
                if let Some((cx, cy)) = pos {
                    let (cw, ch) = sizes[i];
                    (px - cx).abs() < (hw + cw / 2.0) && (py - cy).abs() < (hh + ch / 2.0)
                } else { false }
            });
            if !overlaps {
                return (px, py);
            }
        }
    }
    (
        tx.clamp(min_x + mw / 2.0, max_x - mw / 2.0),
        ty.clamp(min_y + mh / 2.0, max_y - mh / 2.0),
    )
}

/// Find position near board edge, close to (near_x, near_y).
fn find_edge_position(
    sizes: &[(f64, f64)],
    positions: &[Option<(f64, f64)>],
    my_idx: usize,
    near_x: f64, near_y: f64,
    min_x: f64, max_x: f64, min_y: f64, max_y: f64,
) -> (f64, f64) {
    let (mw, mh) = sizes[my_idx];
    let hw = mw / 2.0;
    let hh = mh / 2.0;

    let mut candidates = vec![
        (min_x + hw, near_y.clamp(min_y + hh, max_y - hh)),
        (max_x - hw, near_y.clamp(min_y + hh, max_y - hh)),
        (near_x.clamp(min_x + hw, max_x - hw), min_y + hh),
        (near_x.clamp(min_x + hw, max_x - hw), max_y - hh),
    ];
    candidates.sort_by(|a, b| {
        let da = (a.0 - near_x).powi(2) + (a.1 - near_y).powi(2);
        let db = (b.0 - near_x).powi(2) + (b.1 - near_y).powi(2);
        da.partial_cmp(&db).unwrap()
    });

    for (px, py) in &candidates {
        let overlaps = positions.iter().enumerate().any(|(i, pos)| {
            if i == my_idx { return false; }
            if let Some((cx, cy)) = pos {
                let (cw, ch) = sizes[i];
                (px - cx).abs() < (hw + cw / 2.0) && (py - cy).abs() < (hh + ch / 2.0)
            } else { false }
        });
        if !overlaps {
            return (*px, *py);
        }
    }

    // Try offset positions along edges
    for (bx, by) in &candidates {
        for offset in 1..20 {
            let s = offset as f64 * 3.0;
            for &(dx, dy) in &[(s, 0.0), (-s, 0.0), (0.0, s), (0.0, -s)] {
                let px = bx + dx;
                let py = by + dy;
                if px - hw < min_x || px + hw > max_x || py - hh < min_y || py + hh > max_y {
                    continue;
                }
                let overlaps = positions.iter().enumerate().any(|(i, pos)| {
                    if i == my_idx { return false; }
                    if let Some((cx, cy)) = pos {
                        let (cw, ch) = sizes[i];
                        (px - cx).abs() < (hw + cw / 2.0) && (py - cy).abs() < (hh + ch / 2.0)
                    } else { false }
                });
                if !overlaps {
                    return (px, py);
                }
            }
        }
    }

    find_non_overlapping(near_x, near_y, sizes, positions, my_idx, min_x, max_x, min_y, max_y)
}

/// Post-placement: move protection components (TVS/ESD) adjacent to their connected connectors.
/// Ensures < 5mm distance and same layer (no via needed).
fn post_place_protection_near_connectors(board: &mut Board) {
    let sizes = compute_placement_sizes(&board.components);
    let margin = 5.0;
    let n = board.components.len();

    // Collect moves first to avoid borrow issues
    let mut moves: Vec<(usize, f64, f64)> = Vec::new();

    for idx in 0..n {
        if !is_protection_component(&board.components[idx]) {
            continue;
        }

        let comp_name = board.components[idx].name.clone();
        let mut best_connector_pos = None;
        let mut best_dist = f64::MAX;

        for net in &board.nets {
            if !net.pins.iter().any(|p| p.component == comp_name) {
                continue;
            }
            for pin_ref in &net.pins {
                if pin_ref.component == comp_name {
                    continue;
                }
                if let Some(conn) = board.components.iter().find(|c| {
                    c.name == pin_ref.component && is_connector_component(c)
                }) {
                    let dist = (conn.x - board.components[idx].x).powi(2)
                        + (conn.y - board.components[idx].y).powi(2);
                    if dist < best_dist {
                        best_dist = dist;
                        best_connector_pos = Some((conn.x, conn.y));
                    }
                }
            }
        }

        if let Some((tx, ty)) = best_connector_pos {
            moves.push((idx, tx, ty));
        }
    }

    for (comp_idx, target_x, target_y) in moves {
        let positions: Vec<Option<(f64, f64)>> = board
            .components
            .iter()
            .enumerate()
            .map(|(i, c)| {
                if i == comp_idx {
                    None
                } else {
                    Some((c.x, c.y))
                }
            })
            .collect();

        let (px, py) = find_non_overlapping(
            target_x,
            target_y,
            &sizes,
            &positions,
            comp_idx,
            margin,
            board.width - margin,
            margin,
            board.height - margin,
        );

        board.components[comp_idx].x = px;
        board.components[comp_idx].y = py;
    }
}

/// Post-placement: move decoupling capacitors (100nF) adjacent to their connected ICs.
/// Assigns each cap to the closest IC sharing a VCC net, places within 5mm.
fn post_place_decoupling_near_ics(board: &mut Board) {
    let sizes = compute_placement_sizes(&board.components);
    let margin = 5.0;
    let n = board.components.len();

    let cap_indices: Vec<usize> = (0..n)
        .filter(|&i| is_decoupling_cap(&board.components[i]))
        .collect();

    let mut assigned_ics: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut moves: Vec<(usize, f64, f64)> = Vec::new();

    for &cap_idx in &cap_indices {
        let cap_name = board.components[cap_idx].name.clone();

        // Find ICs sharing a VCC net with this cap
        let mut candidate_ics: Vec<usize> = Vec::new();
        for net in &board.nets {
            if !is_vcc_net(&net.name) {
                continue;
            }
            if !net.pins.iter().any(|p| p.component == cap_name) {
                continue;
            }
            for pin_ref in &net.pins {
                if pin_ref.component == cap_name {
                    continue;
                }
                if let Some(ic_idx) = board.components.iter().position(|c| {
                    c.name == pin_ref.component && is_ic_component(c)
                }) {
                    if !candidate_ics.contains(&ic_idx) && !assigned_ics.contains(&ic_idx) {
                        candidate_ics.push(ic_idx);
                    }
                }
            }
        }

        // Find closest unassigned IC
        let mut best_ic = None;
        let mut best_dist = f64::MAX;
        for &ic_idx in &candidate_ics {
            let ic = &board.components[ic_idx];
            let cap = &board.components[cap_idx];
            let dist = (ic.x - cap.x).powi(2) + (ic.y - cap.y).powi(2);
            if dist < best_dist {
                best_dist = dist;
                best_ic = Some(ic_idx);
            }
        }

        if let Some(ic_idx) = best_ic {
            assigned_ics.insert(ic_idx);
            let target_x = board.components[ic_idx].x;
            let target_y = board.components[ic_idx].y;
            moves.push((cap_idx, target_x, target_y));
        }
    }

    for (comp_idx, target_x, target_y) in moves {
        let positions: Vec<Option<(f64, f64)>> = board
            .components
            .iter()
            .enumerate()
            .map(|(i, c)| {
                if i == comp_idx {
                    None
                } else {
                    Some((c.x, c.y))
                }
            })
            .collect();

        let (px, py) = find_non_overlapping(
            target_x,
            target_y,
            &sizes,
            &positions,
            comp_idx,
            margin,
            board.width - margin,
            margin,
            board.height - margin,
        );

        board.components[comp_idx].x = px;
        board.components[comp_idx].y = py;
    }
}

/// Write GND stitching vias distributed across the board in a regular grid.
/// Connects F.Cu and B.Cu ground planes every ~10mm, avoiding component areas.
fn write_stitching_vias(buf: &mut String, board: &Board) {
    let gnd_idx = match board.nets.iter().position(|n| n.name == "GND") {
        Some(idx) => idx + 1, // KiCad nets are 1-indexed
        None => return,
    };

    let margin = STITCHING_VIA_MARGIN;
    let mut x = margin;
    while x < board.width - margin {
        let mut y = margin;
        while y < board.height - margin {
            // Check if position is far enough from all components
            let too_close = board.components.iter().any(|comp| {
                if let Some(fp) = &comp.footprint_data {
                    let (min_x, min_y, max_x, max_y) = fp.placement_bounds();
                    let abs_min_x = comp.x + min_x - 1.0;
                    let abs_min_y = comp.y + min_y - 1.0;
                    let abs_max_x = comp.x + max_x + 1.0;
                    let abs_max_y = comp.y + max_y + 1.0;
                    x >= abs_min_x && x <= abs_max_x && y >= abs_min_y && y <= abs_max_y
                } else {
                    let dist = ((comp.x - x).powi(2) + (comp.y - y).powi(2)).sqrt();
                    dist < 3.0
                }
            });

            if !too_close {
                buf.push_str(&format!(
                    "  (via (at {} {}) (size {}) (drill {}) (layers \"F.Cu\" \"B.Cu\") (net {}) (uuid \"{}\"))\n",
                    x, y, STITCHING_VIA_SIZE, STITCHING_VIA_DRILL, gnd_idx, Uuid::new_v4()
                ));
            }

            y += STITCHING_VIA_SPACING;
        }
        x += STITCHING_VIA_SPACING;
    }
}

/// Write copper pour zone definitions for power nets.
fn write_zones(buf: &mut String, board: &Board) {
    // GND zone on B.Cu — solid ground plane covering the entire board.
    // connect_pads ensures thermal relief connections; min_thickness prevents fragmentation.
    if let Some(gnd_idx) = board.nets.iter().position(|n| n.name == "GND") {
        buf.push_str(&format!(
            "  (zone (net {}) (net_name \"GND\") (layer \"B.Cu\") (uuid \"{}\")\n",
            gnd_idx + 1,
            Uuid::new_v4()
        ));
        buf.push_str("    (connect_pads (clearance 0.3))\n");
        buf.push_str(
            "    (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.5) (min_thickness 0.25) (island_removal_mode 2) (island_area_min 10.0))\n",
        );
        buf.push_str(&format!(
            "    (polygon (pts\n      (xy 0 0) (xy {} 0) (xy {} {}) (xy 0 {})\n    ))\n",
            board.width, board.width, board.height, board.height
        ));
        buf.push_str("  )\n\n");

        // Also add GND zone on F.Cu with lower priority, to help stitching vias
        buf.push_str(&format!(
            "  (zone (net {}) (net_name \"GND\") (layer \"F.Cu\") (uuid \"{}\")\n",
            gnd_idx + 1,
            Uuid::new_v4()
        ));
        buf.push_str("    (connect_pads (clearance 0.3))\n");
        buf.push_str(
            "    (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.5) (min_thickness 0.25) (island_removal_mode 2) (island_area_min 10.0))\n",
        );
        buf.push_str("    (priority 2)\n");
        buf.push_str(&format!(
            "    (polygon (pts\n      (xy 0 0) (xy {} 0) (xy {} {}) (xy 0 {})\n    ))\n",
            board.width, board.width, board.height, board.height
        ));
        buf.push_str("  )\n\n");
    }

    // VCC3V3 zone on F.Cu (lower priority, fills around signals)
    if let Some(vcc_idx) = board.nets.iter().position(|n| n.name == "VCC3V3") {
        buf.push_str(&format!(
            "  (zone (net {}) (net_name \"VCC3V3\") (layer \"F.Cu\") (uuid \"{}\")\n",
            vcc_idx + 1,
            Uuid::new_v4()
        ));
        buf.push_str("    (connect_pads (clearance 0.3))\n");
        buf.push_str(
            "    (fill yes (thermal_gap 0.5) (thermal_bridge_width 0.5) (min_thickness 0.25))\n",
        );
        buf.push_str("    (priority 1)\n");
        buf.push_str(&format!(
            "    (polygon (pts\n      (xy 0 0) (xy {} 0) (xy {} {}) (xy 0 {})\n    ))\n",
            board.width, board.width, board.height, board.height
        ));
        buf.push_str("  )\n\n");
    }
}

fn write_pcb_header(buf: &mut String) {
    buf.push_str("(kicad_pcb\n");
    buf.push_str("  (version 20240108)\n");
    buf.push_str("  (generator \"pcb-forge\")\n");
    buf.push_str("  (generator_version \"0.1.0\")\n");
    buf.push_str("  (general\n    (thickness 1.6)\n    (legacy_teardrops no)\n  )\n");
    buf.push_str("  (paper \"A4\")\n");
}

fn write_layers(buf: &mut String) {
    buf.push_str("  (layers\n");
    buf.push_str("    (0 \"F.Cu\" signal)\n");
    buf.push_str("    (31 \"B.Cu\" signal)\n");
    buf.push_str("    (32 \"B.Adhes\" user \"B.Adhesive\")\n");
    buf.push_str("    (33 \"F.Adhes\" user \"F.Adhesive\")\n");
    buf.push_str("    (34 \"B.Paste\" user)\n");
    buf.push_str("    (35 \"F.Paste\" user)\n");
    buf.push_str("    (36 \"B.SilkS\" user \"B.Silkscreen\")\n");
    buf.push_str("    (37 \"F.SilkS\" user \"F.Silkscreen\")\n");
    buf.push_str("    (38 \"B.Mask\" user)\n");
    buf.push_str("    (39 \"F.Mask\" user)\n");
    buf.push_str("    (40 \"Dwgs.User\" user \"User.Drawings\")\n");
    buf.push_str("    (41 \"Cmts.User\" user \"User.Comments\")\n");
    buf.push_str("    (42 \"Eco1.User\" user \"User.Eco1\")\n");
    buf.push_str("    (43 \"Eco2.User\" user \"User.Eco2\")\n");
    buf.push_str("    (44 \"Edge.Cuts\" user)\n");
    buf.push_str("    (45 \"Margin\" user)\n");
    buf.push_str("    (46 \"B.CrtYd\" user \"B.Courtyard\")\n");
    buf.push_str("    (47 \"F.CrtYd\" user \"F.Courtyard\")\n");
    buf.push_str("    (48 \"B.Fab\" user)\n");
    buf.push_str("    (49 \"F.Fab\" user)\n");
    buf.push_str("  )\n\n");
}

fn write_setup(buf: &mut String, board: &Board) {
    buf.push_str("  (setup\n");
    buf.push_str("    (pad_to_mask_clearance 0.05)\n");
    buf.push_str("    (pcbplotparams\n");
    buf.push_str("      (layerselection 0x00010fc_ffffffff)\n");
    buf.push_str("      (plot_on_all_layers_selection 0x0000000_00000000)\n");
    buf.push_str("      (disableapertmacros no)\n");
    buf.push_str("      (usegerberextensions yes)\n");
    buf.push_str("      (usegerberattributes yes)\n");
    buf.push_str("      (usegerberadvancedattributes yes)\n");
    buf.push_str("      (creategerberjobfile yes)\n");
    buf.push_str("      (dashed_line_dash_ratio 12.000000)\n");
    buf.push_str("      (dashed_line_gap_ratio 3.000000)\n");
    buf.push_str("      (svgprecision 4)\n");
    buf.push_str("      (plotframeref no)\n");
    buf.push_str("      (viasonmask no)\n");
    buf.push_str("      (mode 1)\n");
    buf.push_str("      (useauxorigin no)\n");
    buf.push_str("      (hpglpennumber 1)\n");
    buf.push_str("      (hpglpenspeed 20)\n");
    buf.push_str("      (hpglpendiameter 15.000000)\n");
    buf.push_str("      (pdf_front_fp_property_popups yes)\n");
    buf.push_str("      (pdf_back_fp_property_popups yes)\n");
    buf.push_str("      (pdf_metadata yes)\n");
    buf.push_str("      (excludeedgelayer yes)\n");
    buf.push_str(&format!(
        "      (linewidth {})\n",
        board.trace_width / 1000.0
    ));
    buf.push_str("      (plotinvisibletext no)\n");
    buf.push_str("      (sketchpadsonfab no)\n");
    buf.push_str("      (subtractmaskfromsilk no)\n");
    buf.push_str("      (outputformat 1)\n");
    buf.push_str("      (mirror no)\n");
    buf.push_str("      (drillshape 1)\n");
    buf.push_str("      (scaleselection 1)\n");
    buf.push_str("      (outputdirectory \"\")\n");
    buf.push_str("    )\n");
    buf.push_str("  )\n\n");
}

fn write_nets(buf: &mut String, board: &Board) {
    buf.push_str("  (net 0 \"\")\n");
    for (i, net) in board.nets.iter().enumerate() {
        buf.push_str(&format!("  (net {} \"{}\")\n", i + 1, net.name));
    }
    buf.push_str("\n");
}

fn write_board_outline(buf: &mut String, board: &Board) {
    let layer = Layer::EdgeCuts.name();
    buf.push_str(&format!(
        "  (gr_rect (start 0 0) (end {} {})\n    (stroke (width 0.05) (type default))\n    (fill none)\n    (layer \"{}\")\n    (uuid \"{}\")\n  )\n\n",
        board.width, board.height, layer, Uuid::new_v4()
    ));
}

fn write_components(buf: &mut String, board: &Board) {
    for comp in &board.components {
        write_footprint(buf, comp, board);
    }
}

fn write_footprint(buf: &mut String, comp: &Component, board: &Board) {
    let fp_uuid = Uuid::new_v4();

    buf.push_str(&format!("  (footprint \"{}\"\n", comp.footprint));
    buf.push_str("    (layer \"F.Cu\")\n");
    buf.push_str(&format!("    (uuid \"{}\")\n", fp_uuid));
    if comp.rotation != 0.0 {
        buf.push_str(&format!("    (at {} {} {})\n", comp.x, comp.y, comp.rotation));
    } else {
        buf.push_str(&format!("    (at {} {})\n", comp.x, comp.y));
    }

    // Properties
    buf.push_str(&format!(
        "    (property \"Reference\" \"{}\"\n      (at 0 -3 0)\n      (layer \"F.SilkS\")\n      (uuid \"{}\")\n      (effects (font (size 1 1) (thickness 0.15)))\n    )\n",
        comp.ref_des, Uuid::new_v4()
    ));
    buf.push_str(&format!(
        "    (property \"Value\" \"{}\"\n      (at 0 3 0)\n      (layer \"F.Fab\")\n      (uuid \"{}\")\n      (effects (font (size 1 1) (thickness 0.15)))\n    )\n",
        comp.value, Uuid::new_v4()
    ));
    buf.push_str(&format!(
        "    (property \"Footprint\" \"{}\"\n      (at 0 0 0)\n      (layer \"F.Fab\")\n      (uuid \"{}\")\n      (effects (font (size 1.27 1.27) (thickness 0.15)) hide)\n    )\n",
        comp.footprint, Uuid::new_v4()
    ));

    if let Some(fp) = &comp.footprint_data {
        // Write real footprint lines (courtyard, fab, silkscreen)
        for line in &fp.lines {
            buf.push_str(&format!(
                "    (fp_line (start {} {}) (end {} {})\n      (stroke (width {}) (type solid))\n      (layer \"{}\")\n      (uuid \"{}\")\n    )\n",
                line.start.0, line.start.1, line.end.0, line.end.1,
                line.width, line.layer, Uuid::new_v4()
            ));
        }

        // Write real pads with net assignments
        for pad in &fp.pads {
            if pad.number.is_empty() && !pad.layers.iter().any(|l| l.contains("Cu")) {
                continue; // skip paste-only helper pads
            }

            let net_str = find_net_for_pad(comp, &pad.number, board);

            let layers_str = pad.layers.join("\" \"");

            let drill_str = if let Some(d) = pad.drill {
                format!("\n      (drill {})", d)
            } else {
                String::new()
            };

            let net_line = if net_str.is_empty() {
                String::new()
            } else {
                format!("\n      {}", net_str)
            };

            buf.push_str(&format!(
                "    (pad \"{}\" {} {} (at {} {}) (size {} {}){}\n      (layers \"{}\"){}\n      (uuid \"{}\")\n    )\n",
                pad.number, pad.pad_type, pad.shape,
                pad.at_x, pad.at_y,
                pad.size_w, pad.size_h,
                drill_str,
                layers_str,
                net_line,
                Uuid::new_v4()
            ));
        }
    } else {
        // Fallback: generic pads (legacy behavior)
        write_generic_footprint(buf, comp);
    }

    buf.push_str("  )\n\n");
}

/// Find net assignment string for a given pad number on a component.
fn find_net_for_pad(comp: &Component, pad_number: &str, board: &Board) -> String {
    // Find which pin has this pad number
    let pin = comp.pins.iter().find(|p| p.number == pad_number);
    if let Some(pin) = pin {
        // Find which net this pin belongs to
        for (i, net) in board.nets.iter().enumerate() {
            for pin_ref in &net.pins {
                if pin_ref.component == comp.name && pin_ref.pin == pin.name {
                    return format!("(net {} \"{}\")", i + 1, net.name);
                }
            }
        }
    }
    String::new()
}

/// Append routed traces and vias to an existing .kicad_pcb file.
/// This must be called after generate_pcb() and after routing is complete.
pub fn append_routed_traces(
    pcb_path: &Path,
    board: &Board,
    routed_nets: &[RoutedNet],
) -> Result<()> {
    let mut content = std::fs::read_to_string(pcb_path)?;

    // Remove trailing ")\n" to insert traces before close
    let trimmed = content.trim_end();
    if trimmed.ends_with(')') {
        content = trimmed[..trimmed.len() - 1].to_string();
        content.push('\n');
    }

    // Build net name → index map (net 0 is empty/unassigned in KiCad)
    let net_indices: std::collections::HashMap<&str, usize> = board
        .nets
        .iter()
        .enumerate()
        .map(|(i, n)| (n.name.as_str(), i + 1))
        .collect();

    // Write trace segments
    for rn in routed_nets {
        let net_idx = net_indices.get(rn.name.as_str()).copied().unwrap_or(0);

        for seg in &rn.segments {
            let layer = if seg.layer == 0 { "F.Cu" } else { "B.Cu" };
            content.push_str(&format!(
                "  (segment (start {} {}) (end {} {}) (width {}) (layer \"{}\") (net {}) (uuid \"{}\"))\n",
                seg.start.0, seg.start.1, seg.end.0, seg.end.1,
                seg.width, layer, net_idx, Uuid::new_v4()
            ));
        }

        for via in &rn.vias {
            content.push_str(&format!(
                "  (via (at {} {}) (size {}) (drill {}) (layers \"F.Cu\" \"B.Cu\") (net {}) (uuid \"{}\"))\n",
                via.x, via.y, via.size, via.drill, net_idx, Uuid::new_v4()
            ));
        }
    }

    content.push_str(")\n");

    std::fs::write(pcb_path, &content)?;
    Ok(())
}

fn write_generic_footprint(buf: &mut String, comp: &Component) {
    let pin_count = comp.pins.len();
    let body_w = 8.0_f64.max(pin_count as f64 * 1.0);
    let body_h = 6.0_f64.max(pin_count as f64 * 0.8);

    // Courtyard
    buf.push_str(&format!(
        "    (fp_rect (start {} {}) (end {} {})\n      (stroke (width 0.05) (type default))\n      (fill none)\n      (layer \"F.CrtYd\")\n      (uuid \"{}\")\n    )\n",
        -body_w / 2.0, -body_h / 2.0,
        body_w / 2.0, body_h / 2.0,
        Uuid::new_v4()
    ));

    // Fab layer
    buf.push_str(&format!(
        "    (fp_rect (start {} {}) (end {} {})\n      (stroke (width 0.1) (type default))\n      (fill none)\n      (layer \"F.Fab\")\n      (uuid \"{}\")\n    )\n",
        -(body_w - 0.5) / 2.0, -(body_h - 0.5) / 2.0,
        (body_w - 0.5) / 2.0, (body_h - 0.5) / 2.0,
        Uuid::new_v4()
    ));

    // Generic pads
    let half = (pin_count + 1) / 2;
    for (i, pin) in comp.pins.iter().enumerate() {
        let (pad_x, pad_y) = if i < half {
            let py = -(half as f64 - 1.0) * 1.27 / 2.0 + i as f64 * 1.27;
            (-(body_w / 2.0 - 0.5), py)
        } else {
            let ri = i - half;
            let right_count = pin_count - half;
            let py = -(right_count as f64 - 1.0) * 1.27 / 2.0 + ri as f64 * 1.27;
            (body_w / 2.0 - 0.5, py)
        };

        buf.push_str(&format!(
            "    (pad \"{}\" smd rect (at {} {}) (size 1.5 0.6)\n      (layers \"F.Cu\" \"F.Paste\" \"F.Mask\")\n      (uuid \"{}\")\n    )\n",
            pin.number, pad_x, pad_y, Uuid::new_v4()
        ));
    }
}
