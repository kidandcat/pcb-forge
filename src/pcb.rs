use anyhow::Result;
use std::io::Write;
use std::path::Path;
use uuid::Uuid;

use crate::router::RoutedNet;
use crate::schema::{Board, Component, Layer, Options};

// Placement constraints
const MIN_COURTYARD_CLEARANCE: f64 = 0.5; // mm, minimum gap between component courtyards
// Stitching via parameters
const STITCHING_VIA_SPACING: f64 = 10.0; // mm, grid spacing
const STITCHING_VIA_SIZE: f64 = 0.6; // mm
const STITCHING_VIA_DRILL: f64 = 0.3; // mm
const STITCHING_VIA_MARGIN: f64 = 3.0; // mm, margin from board edge

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

/// Check if a component is a small passive-like device (≤2 pins, not a connector).
/// Includes resistors, capacitors, diodes, LEDs, buttons.
fn is_passive_like(comp: &Component) -> bool {
    comp.pins.len() <= 2 && !is_connector_component(comp)
}

/// Check if a net name represents a power rail (VCC, GND, VBUS, etc.)
fn is_power_net_name(name: &str) -> bool {
    let upper = name.to_uppercase();
    upper == "GND"
        || upper.starts_with("VCC")
        || upper.starts_with("VDD")
        || upper == "3V3"
        || upper == "5V"
        || upper.starts_with("V3V3")
        || upper == "VBUS"
        || upper == "VBAT"
}

/// Build passive-to-anchor clustering using signal-weighted adjacency.
/// Signal nets count 10x more than power nets for determining affinity.
/// Power-only passives (decoupling caps) are distributed evenly among anchors.
/// Returns a vector where entry[i] = Some(anchor_idx) for passives, None for non-passives.
fn build_passive_clusters(
    components: &[Component],
    adj: &[Vec<usize>],
    nets: &[crate::schema::Net],
) -> Vec<Option<usize>> {
    let n = components.len();

    // Build signal-weighted adjacency: signal nets = 10, power nets = 1
    let mut weighted_adj = vec![vec![0usize; n]; n];
    for net in nets {
        let weight = if is_power_net_name(&net.name) { 1 } else { 10 };
        let mut comp_indices: Vec<usize> = Vec::new();
        for pr in &net.pins {
            if let Some(idx) = components.iter().position(|c| c.name == pr.component) {
                if !comp_indices.contains(&idx) {
                    comp_indices.push(idx);
                }
            }
        }
        for i in 0..comp_indices.len() {
            for j in (i + 1)..comp_indices.len() {
                weighted_adj[comp_indices[i]][comp_indices[j]] += weight;
                weighted_adj[comp_indices[j]][comp_indices[i]] += weight;
            }
        }
    }

    let mut cluster: Vec<Option<usize>> = vec![None; n];

    // First pass: cluster each passive with its most-connected non-passive (signal-weighted)
    for i in 0..n {
        if !is_passive_like(&components[i]) {
            continue;
        }

        let mut best_anchor: Option<usize> = None;
        let mut best_score = 0usize;
        let mut best_pins = 0usize;

        for j in 0..n {
            if i == j || is_passive_like(&components[j]) {
                continue; // Only non-passives (ICs, modules, connectors) as anchors
            }

            let score = weighted_adj[i][j];
            if score == 0 {
                continue;
            }

            let pins = components[j].pins.len();
            if score > best_score || (score == best_score && pins > best_pins) {
                best_score = score;
                best_anchor = Some(j);
                best_pins = pins;
            }
        }

        cluster[i] = best_anchor;
    }

    // Second pass: redistribute power-only passives for even distribution.
    // If a passive has only power connections (weighted score ≤ 2, meaning no signal nets)
    // and its anchor is overloaded, move it to an equally-connected but less-loaded anchor.
    let mut anchor_counts: std::collections::HashMap<usize, usize> =
        std::collections::HashMap::new();
    for i in 0..n {
        if let Some(a) = cluster[i] {
            *anchor_counts.entry(a).or_insert(0) += 1;
        }
    }

    for i in 0..n {
        if !is_passive_like(&components[i]) {
            continue;
        }
        let current = match cluster[i] {
            Some(a) => a,
            None => continue,
        };

        // Check if this passive has signal connections to its anchor
        let has_signal = weighted_adj[i][current] > adj[i][current]; // signal weight > just power count
        if has_signal {
            continue; // Strong signal binding, don't redistribute
        }

        // This passive has only power connections — find less-loaded alternatives
        let current_adj = adj[i][current];
        let current_count = *anchor_counts.get(&current).unwrap_or(&0);

        let mut best_alt: Option<usize> = None;
        let mut best_alt_count = current_count;

        for j in 0..n {
            if i == j || j == current || is_passive_like(&components[j]) {
                continue;
            }
            if is_connector_component(&components[j]) {
                continue; // Don't redistribute to connectors
            }
            if adj[i][j] < current_adj {
                continue; // Must have at least as many shared nets
            }

            let alt_count = *anchor_counts.get(&j).unwrap_or(&0);
            if alt_count + 1 < best_alt_count {
                best_alt = Some(j);
                best_alt_count = alt_count;
            }
        }

        if let Some(alt) = best_alt {
            *anchor_counts.get_mut(&current).unwrap() -= 1;
            *anchor_counts.entry(alt).or_insert(0) += 1;
            cluster[i] = Some(alt);
        }
    }

    // Third pass: unclustered passives inherit anchor from connected clustered passives
    let snapshot = cluster.clone();
    for i in 0..n {
        if cluster[i].is_some() || !is_passive_like(&components[i]) {
            continue;
        }
        let mut best_anchor: Option<usize> = None;
        let mut best_score = 0usize;
        for j in 0..n {
            if i == j || adj[i][j] == 0 {
                continue;
            }
            if let Some(anchor) = snapshot[j] {
                if adj[i][j] > best_score {
                    best_score = adj[i][j];
                    best_anchor = Some(anchor);
                }
            }
        }
        cluster[i] = best_anchor;
    }

    cluster
}

/// Compute component sizes for placement with minimum courtyard clearance.
/// Uses placement_bounds (Fab layer / pad bounds) rather than courtyard_bounds
/// because courtyard can include antenna keep-out zones that are much larger
/// than the physical component body.
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
    pub fn compute(routed_nets: &[crate::router::RoutedNet], total_nets: usize, board: &Board) -> Self {
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

        let opts = &board.options;
        let board_area = board.width * board.height;
        let composite = (nets_routed as f64) * opts.net_reward
            - total_trace_length * opts.trace_penalty
            - (via_count as f64) * opts.via_penalty
            - (clearance_violations as f64) * 10000.0
            - board_area * opts.board_penalty;

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

/// Generate placement variant configs with different seeds and strategies.
/// The number of variants and base spacing come from the options.
pub fn generate_placement_configs(options: &Options) -> Vec<PlacementConfig> {
    let n = options.placement_variants.max(1);
    let base = options.spacing;
    (0..n as u64)
        .map(|i| PlacementConfig {
            seed: i * 7919 + 42, // distinct primes for variety
            spacing_mult: base + (i as f64 - (n as f64 - 1.0) / 2.0) * 0.08,
            center_angle: (i as f64) * std::f64::consts::PI * 2.0 / n as f64,
        })
        .collect()
}

/// Public wrapper to compute optimal board dimensions (for validation/display).
pub fn auto_size_board_pub(board: &mut Board) {
    auto_size_board(board);
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

/// Place components using connectivity-aware algorithm with functional clustering.
/// Passives are grouped with their parent IC/connector and placed adjacent to them.
fn place_components_with_config(board: &mut Board, config: &PlacementConfig) {
    let margin = 5.0;
    let n = board.components.len();
    if n == 0 {
        return;
    }

    auto_size_board(board);

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

    // 2. Build functional clusters: each passive maps to its best anchor
    let passive_cluster = build_passive_clusters(&board.components, &adj, &board.nets);
    let mut anchor_passives: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, anchor) in passive_cluster.iter().enumerate() {
        if let Some(a) = anchor {
            anchor_passives.entry(*a).or_default().push(i);
        }
    }

    // Log clusters for debugging
    for (&anchor_idx, passives) in &anchor_passives {
        let anchor_name = &board.components[anchor_idx].ref_des;
        let passive_names: Vec<&str> = passives
            .iter()
            .map(|&p| board.components[p].ref_des.as_str())
            .collect();
        eprintln!(
            "  Cluster {}: {:?}",
            anchor_name, passive_names
        );
    }

    // 3. Separate connectors
    let connector_indices: Vec<usize> = (0..n)
        .filter(|&i| is_connector_component(&board.components[i]))
        .collect();

    // 4. Place connectors on board edges
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

    // 5. Place connector-clustered passives near their connectors
    for &conn_idx in &connector_indices {
        if let Some(passives) = anchor_passives.get(&conn_idx) {
            let (cx, cy) = positions[conn_idx].unwrap();
            for &p in passives {
                if positions[p].is_some() {
                    continue;
                }
                let (px, py) = find_non_overlapping_with_step(
                    cx, cy, &sizes, &positions, p,
                    margin, board.width - margin, margin, board.height - margin,
                    1.0,
                );
                positions[p] = Some((px, py));
            }
        }
    }

    // 6. Identify anchor components (non-connector, >2 pins)
    let anchor_indices: Vec<usize> = (0..n)
        .filter(|&i| {
            !is_connector_component(&board.components[i])
                && !is_passive_like(&board.components[i])
        })
        .collect();

    if anchor_indices.is_empty() {
        // Only connectors and passives — apply and return
        for (i, comp) in board.components.iter_mut().enumerate() {
            if let Some((x, y)) = positions[i] {
                comp.x = x;
                comp.y = y;
            }
        }
        return;
    }

    let total_conn: Vec<usize> = (0..n).map(|i| adj[i].iter().sum()).collect();

    // 7. Place most-connected anchor at center
    let center_idx = *anchor_indices
        .iter()
        .max_by_key(|&&i| total_conn[i])
        .unwrap();

    let cx = board.width / 2.0 + config.center_angle.cos() * 2.0;
    let cy = board.height / 2.0 + config.center_angle.sin() * 2.0;
    positions[center_idx] = Some((
        safe_clamp(cx, margin + 5.0, board.width - margin - 5.0),
        safe_clamp(cy, margin + 5.0, board.height - margin - 5.0),
    ));

    // Place center anchor's passives immediately
    if let Some(passives) = anchor_passives.get(&center_idx) {
        let (acx, acy) = positions[center_idx].unwrap();
        for &p in passives {
            if positions[p].is_some() {
                continue;
            }
            let (px, py) = find_non_overlapping_with_step(
                acx, acy, &sizes, &positions, p,
                margin, board.width - margin, margin, board.height - margin,
                1.0,
            );
            positions[p] = Some((px, py));
        }
    }

    // 8. Place remaining anchors greedily, with their passives after each
    let mut remaining_anchors: Vec<usize> = anchor_indices
        .iter()
        .copied()
        .filter(|&i| i != center_idx)
        .collect();

    while !remaining_anchors.is_empty() {
        let mut best_ri = 0;
        let mut best_score = 0usize;
        for (ri, &comp_idx) in remaining_anchors.iter().enumerate() {
            let mut conn_score = 0usize;
            for placed_idx in 0..n {
                if positions[placed_idx].is_some() {
                    conn_score += adj[comp_idx][placed_idx];
                }
            }
            if conn_score > best_score
                || (conn_score == best_score
                    && total_conn[comp_idx] > total_conn[remaining_anchors[best_ri]])
            {
                best_score = conn_score;
                best_ri = ri;
            }
        }

        let comp_idx = remaining_anchors.remove(best_ri);

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

        // Place this anchor's passives immediately nearby
        if let Some(passives) = anchor_passives.get(&comp_idx) {
            for &p in passives {
                if positions[p].is_some() {
                    continue;
                }
                let (ppx, ppy) = find_non_overlapping_with_step(
                    px, py, &sizes, &positions, p,
                    margin, board.width - margin, margin, board.height - margin,
                    1.0, // tight step for passives near their anchor
                );
                positions[p] = Some((ppx, ppy));
            }
        }
    }

    // 9. Place any remaining unclustered components via greedy
    let mut remaining: Vec<usize> = (0..n).filter(|&i| positions[i].is_none()).collect();

    while !remaining.is_empty() {
        let mut best_ri = 0;
        let mut best_score = 0usize;
        for (ri, &comp_idx) in remaining.iter().enumerate() {
            let mut conn_score = 0usize;
            for placed_idx in 0..n {
                if positions[placed_idx].is_some() {
                    conn_score += adj[comp_idx][placed_idx];
                }
            }
            if conn_score > best_score
                || (conn_score == best_score
                    && total_conn[comp_idx] > total_conn[remaining[best_ri]])
            {
                best_score = conn_score;
                best_ri = ri;
            }
        }

        let comp_idx = remaining.remove(best_ri);

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

    // 10. Apply all positions
    for (i, comp) in board.components.iter_mut().enumerate() {
        if let Some((x, y)) = positions[i] {
            comp.x = x;
            comp.y = y;
        }
    }

    // 11. Force-directed relaxation with centering
    force_directed_relaxation(board, &adj, margin);

    // 12. Simulated annealing
    simulated_annealing(board, config.seed, margin);

    // 13. Post-placement: enforce passive proximity to anchors
    post_place_passives_near_anchors(board);

    // 14. Final overlap fix
    fix_remaining_overlaps(board);
}

/// Compute total wire length (sum of Manhattan distances between connected pads).
fn compute_wire_length(board: &Board) -> f64 {
    let mut total = 0.0;
    for net in &board.nets {
        let positions: Vec<(f64, f64)> = net
            .pins
            .iter()
            .filter_map(|pr| {
                let comp = board.components.iter().find(|c| c.name == pr.component)?;
                let pin = comp.pins.iter().find(|p| p.name == pr.pin)?;
                let (rx, ry) = rotate_point(pin.x, pin.y, comp.rotation);
                Some((comp.x + rx, comp.y + ry))
            })
            .collect();
        // MST-like: sum nearest-neighbor chain distances
        if positions.len() < 2 {
            continue;
        }
        let mut remaining: Vec<usize> = (1..positions.len()).collect();
        let mut current = 0;
        while !remaining.is_empty() {
            let (best_ri, best_dist) = remaining
                .iter()
                .enumerate()
                .map(|(ri, &idx)| {
                    let d = (positions[current].0 - positions[idx].0).abs()
                        + (positions[current].1 - positions[idx].1).abs();
                    (ri, d)
                })
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .unwrap();
            total += best_dist;
            current = remaining.remove(best_ri);
        }
    }
    total
}

/// Rotate a point (x, y) by angle degrees around origin.
fn rotate_point(x: f64, y: f64, angle_deg: f64) -> (f64, f64) {
    if angle_deg == 0.0 {
        return (x, y);
    }
    let rad = angle_deg.to_radians();
    let cos_a = rad.cos();
    let sin_a = rad.sin();
    (x * cos_a - y * sin_a, x * sin_a + y * cos_a)
}

/// Force-directed relaxation: gently pull strongly-connected components closer
/// while maintaining routing channels. Only attracts; relies on overlap fixing
/// for repulsion.
fn force_directed_relaxation(board: &mut Board, adj: &[Vec<usize>], margin: f64) {
    let n = board.components.len();
    if n < 2 {
        return;
    }

    let sizes = compute_placement_sizes(&board.components);
    let iterations = 40;
    let max_step: f64 = 0.5; // mm max displacement per iteration (conservative)

    // Don't move connectors (they're placed on edges intentionally)
    let movable: Vec<bool> = board
        .components
        .iter()
        .map(|c| !is_connector_component(c))
        .collect();

    for _ in 0..iterations {
        let mut forces: Vec<(f64, f64)> = vec![(0.0, 0.0); n];

        for i in 0..n {
            if !movable[i] {
                continue;
            }

            for j in 0..n {
                if i == j {
                    continue;
                }
                let connectivity = adj[i][j];
                if connectivity == 0 {
                    continue; // Only attract connected components
                }

                let dx = board.components[j].x - board.components[i].x;
                let dy = board.components[j].y - board.components[i].y;
                let dist = (dx * dx + dy * dy).sqrt().max(0.1);

                // Only attract if components are far apart relative to their sizes
                let ideal_dist = (sizes[i].0 + sizes[j].0) / 2.0 + 2.0; // leave 2mm routing channel
                if dist > ideal_dist * 1.5 {
                    // Gentle attraction proportional to connectivity and excess distance
                    let force = 0.15 * connectivity as f64 * (dist - ideal_dist) / dist;
                    forces[i].0 += force * dx;
                    forces[i].1 += force * dy;
                }
            }
        }

        // Centering force: gently push components toward board center for uniform distribution
        let board_cx = board.width / 2.0;
        let board_cy = board.height / 2.0;
        for i in 0..n {
            if !movable[i] {
                continue;
            }
            let dx = board_cx - board.components[i].x;
            let dy = board_cy - board.components[i].y;
            let dist_to_center = (dx * dx + dy * dy).sqrt().max(0.1);
            // Stronger centering when far from center
            let center_force = 0.03 * dist_to_center;
            forces[i].0 += center_force * dx / dist_to_center;
            forces[i].1 += center_force * dy / dist_to_center;
        }

        // Apply forces with clamping, checking for overlaps
        for i in 0..n {
            if !movable[i] {
                continue;
            }
            let (fx, fy) = forces[i];
            let mag = (fx * fx + fy * fy).sqrt();
            if mag < 0.01 {
                continue;
            }
            let scale = max_step.min(mag) / mag;
            let new_x = board.components[i].x + fx * scale;
            let new_y = board.components[i].y + fy * scale;

            let (hw, hh) = (sizes[i].0 / 2.0, sizes[i].1 / 2.0);
            let clamped_x = safe_clamp(new_x, margin + hw, board.width - margin - hw);
            let clamped_y = safe_clamp(new_y, margin + hh, board.height - margin - hh);

            // Only apply if it doesn't create overlaps
            let old_x = board.components[i].x;
            let old_y = board.components[i].y;
            board.components[i].x = clamped_x;
            board.components[i].y = clamped_y;

            if check_component_overlaps(&board.components, &sizes, i, 0.0) {
                board.components[i].x = old_x;
                board.components[i].y = old_y;
            }
        }
    }
}

/// Get effective placement size accounting for rotation.
fn effective_size(base_size: (f64, f64), rotation: f64) -> (f64, f64) {
    let rot = rotation % 180.0;
    if (rot - 90.0).abs() < 1.0 {
        (base_size.1, base_size.0) // swapped for 90° or 270°
    } else {
        base_size
    }
}

/// Check if a specific component overlaps with any other, using current rotation for sizing.
/// Uses a small tolerance to allow near-touching (the final overlap fix pass handles cleanup).
fn check_component_overlaps(
    components: &[Component],
    base_sizes: &[(f64, f64)],
    check_idx: usize,
    tolerance: f64,
) -> bool {
    let (wi, hi) = effective_size(base_sizes[check_idx], components[check_idx].rotation);
    for j in 0..components.len() {
        if j == check_idx {
            continue;
        }
        let (wj, hj) = effective_size(base_sizes[j], components[j].rotation);
        let dx = (components[check_idx].x - components[j].x).abs();
        let dy = (components[check_idx].y - components[j].y).abs();
        if dx < (wi + wj) / 2.0 - tolerance && dy < (hi + hj) / 2.0 - tolerance {
            return true;
        }
    }
    false
}

/// Simulated annealing: try random swaps, rotations, and nudges.
/// Accepts worse solutions with decreasing probability to escape local minima.
fn simulated_annealing(board: &mut Board, seed: u64, margin: f64) {
    let n = board.components.len();
    if n < 3 {
        return;
    }

    let base_sizes = compute_placement_sizes(&board.components);

    // Don't move connectors
    let movable_indices: Vec<usize> = (0..n)
        .filter(|&i| !is_connector_component(&board.components[i]))
        .collect();

    if movable_indices.len() < 2 {
        return;
    }

    let initial_wl = compute_wire_length(board);
    let mut best_wl = initial_wl;
    let mut current_wl = best_wl;
    let mut best_positions: Vec<(f64, f64, f64)> = board
        .components
        .iter()
        .map(|c| (c.x, c.y, c.rotation))
        .collect();

    let mut rng_state = seed ^ 0xDEADBEEF;
    let lcg_next = |state: &mut u64| -> u64 {
        *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *state >> 16
    };

    let initial_temp = best_wl * 0.25;
    let cooling = 0.999;
    let iterations = 5000;
    let mut temp = initial_temp;
    let mut accepted = 0u32;

    for _ in 0..iterations {
        let r = lcg_next(&mut rng_state);
        let move_type = r % 10; // 0-3: swap, 4-5: rotate, 6-9: nudge

        // Save state of affected components
        let saved: Vec<(f64, f64, f64)> = board
            .components
            .iter()
            .map(|c| (c.x, c.y, c.rotation))
            .collect();

        match move_type {
            0..=3 => {
                // Swap two non-connector components
                let ai = lcg_next(&mut rng_state) as usize % movable_indices.len();
                let bi = lcg_next(&mut rng_state) as usize % movable_indices.len();
                if ai == bi {
                    continue;
                }
                let a = movable_indices[ai];
                let b = movable_indices[bi];
                let (ax, ay) = (board.components[a].x, board.components[a].y);
                board.components[a].x = board.components[b].x;
                board.components[a].y = board.components[b].y;
                board.components[b].x = ax;
                board.components[b].y = ay;
            }
            4..=5 => {
                // Rotate a component by 90°
                let a = movable_indices[lcg_next(&mut rng_state) as usize % movable_indices.len()];
                board.components[a].rotation = (board.components[a].rotation + 90.0) % 360.0;
            }
            _ => {
                // Nudge a component
                let a = movable_indices[lcg_next(&mut rng_state) as usize % movable_indices.len()];
                let scale = 5.0 * (temp / initial_temp).max(0.05);
                let dx = ((lcg_next(&mut rng_state) % 200) as f64 - 100.0) * 0.01 * scale;
                let dy = ((lcg_next(&mut rng_state) % 200) as f64 - 100.0) * 0.01 * scale;
                let (ew, eh) = effective_size(base_sizes[a], board.components[a].rotation);
                board.components[a].x = safe_clamp(
                    board.components[a].x + dx,
                    margin + ew / 2.0,
                    board.width - margin - ew / 2.0,
                );
                board.components[a].y = safe_clamp(
                    board.components[a].y + dy,
                    margin + eh / 2.0,
                    board.height - margin - eh / 2.0,
                );
            }
        }

        // Check for overlaps only on moved components (with small tolerance)
        let tolerance = 0.3; // mm tolerance for near-touching
        let changed: Vec<usize> = (0..n)
            .filter(|&i| {
                (board.components[i].x - saved[i].0).abs() > 0.01
                    || (board.components[i].y - saved[i].1).abs() > 0.01
                    || (board.components[i].rotation - saved[i].2).abs() > 0.01
            })
            .collect();
        let moved_overlap = changed.iter().any(|&idx| {
            check_component_overlaps(&board.components, &base_sizes, idx, tolerance)
        });

        if moved_overlap {
            for (i, c) in board.components.iter_mut().enumerate() {
                c.x = saved[i].0;
                c.y = saved[i].1;
                c.rotation = saved[i].2;
            }
            continue;
        }

        let new_wl = compute_wire_length(board);
        let delta = new_wl - current_wl;

        // Metropolis criterion
        let accept = if delta <= 0.0 {
            true
        } else {
            let p = (-delta / temp.max(0.01)).exp();
            let r = (lcg_next(&mut rng_state) % 10000) as f64 / 10000.0;
            r < p
        };

        if accept {
            accepted += 1;
            current_wl = new_wl;
            if current_wl < best_wl {
                best_wl = current_wl;
                best_positions = board
                    .components
                    .iter()
                    .map(|c| (c.x, c.y, c.rotation))
                    .collect();
            }
        } else {
            for (i, c) in board.components.iter_mut().enumerate() {
                c.x = saved[i].0;
                c.y = saved[i].1;
                c.rotation = saved[i].2;
            }
        }

        temp *= cooling;
    }

    // Restore best found positions
    for (i, c) in board.components.iter_mut().enumerate() {
        c.x = best_positions[i].0;
        c.y = best_positions[i].1;
        c.rotation = best_positions[i].2;
    }

    eprintln!(
        "  SA optimization: wire length {:.1}mm → {:.1}mm ({} moves accepted)",
        initial_wl, best_wl, accepted
    );
}

/// Final pass: detect any remaining overlaps and fix them by moving offending components.
fn fix_remaining_overlaps(board: &mut Board) {
    let margin = 5.0;
    let sizes = compute_placement_sizes(&board.components);
    let n = board.components.len();

    // Check all pairs for overlap
    let mut has_overlap = true;
    let mut iterations = 0;
    while has_overlap && iterations < 20 {
        has_overlap = false;
        iterations += 1;

        for i in 0..n {
            for j in (i + 1)..n {
                let (wi, hi) = sizes[i];
                let (wj, hj) = sizes[j];
                let dx = (board.components[i].x - board.components[j].x).abs();
                let dy = (board.components[i].y - board.components[j].y).abs();
                let min_dx = (wi + wj) / 2.0;
                let min_dy = (hi + hj) / 2.0;

                if dx < min_dx && dy < min_dy {
                    has_overlap = true;
                    eprintln!(
                        "⚠ Overlap detected: {} and {} (dx={:.1}, dy={:.1}, need dx>{:.1} or dy>{:.1})",
                        board.components[i].ref_des, board.components[j].ref_des,
                        dx, dy, min_dx, min_dy
                    );

                    // Move the smaller component (fewer pins) away
                    let move_idx = if board.components[i].pins.len() <= board.components[j].pins.len() { i } else { j };
                    let anchor_idx = if move_idx == i { j } else { i };

                    let positions: Vec<Option<(f64, f64)>> = board.components.iter().enumerate()
                        .map(|(k, c)| if k == move_idx { None } else { Some((c.x, c.y)) })
                        .collect();

                    let (px, py) = find_non_overlapping(
                        board.components[anchor_idx].x,
                        board.components[anchor_idx].y,
                        &sizes, &positions, move_idx,
                        margin, board.width - margin, margin, board.height - margin,
                    );
                    board.components[move_idx].x = px;
                    board.components[move_idx].y = py;
                }
            }
        }
    }
}

/// Clamp that never panics: if min > max, returns midpoint.
fn safe_clamp(val: f64, min: f64, max: f64) -> f64 {
    if min > max {
        (min + max) / 2.0
    } else {
        val.clamp(min, max)
    }
}

/// Auto-calculate optimal board dimensions from component courtyard areas,
/// the density factor, and the desired aspect ratio.
///
/// Ensures all components fit by adjusting aspect_ratio and density if needed.
fn auto_size_board(board: &mut Board) {
    let sizes = compute_placement_sizes(&board.components);
    let total_courtyard_area: f64 = sizes.iter().map(|(w, h)| w * h).sum();

    let margin = 5.0;
    let min_margin = 2.0 * margin; // margins on both sides

    // Find the largest component dimensions
    let max_comp_width = sizes.iter().map(|(w, _)| *w).fold(0.0_f64, f64::max);
    let max_comp_height = sizes.iter().map(|(_, h)| *h).fold(0.0_f64, f64::max);

    let min_board_width = max_comp_width + min_margin;
    let min_board_height = max_comp_height + min_margin;

    let desired_ar = board.aspect_ratio.max(0.1);

    // Ensure total area is large enough that both min dimensions can be satisfied
    let min_area_for_components = min_board_width * min_board_height;
    let mut density = board.options.density;
    let mut board_area = (total_courtyard_area * density).max(400.0);

    if total_courtyard_area > 0.0 && board_area < min_area_for_components {
        let new_density = (min_area_for_components * 1.05) / total_courtyard_area;
        density = new_density.max(density);
        board_area = (total_courtyard_area * density).max(min_area_for_components);
        eprintln!(
            "⚠ Warning: density increased from {:.2} to {:.2} to fit all components",
            board.options.density, density
        );
        board.options.density = density;
    }
    board_area = board_area.max(min_area_for_components);

    // Calculate valid aspect_ratio range:
    //   width = sqrt(area * ar) >= min_board_width  →  ar >= min_board_width² / area
    //   height = sqrt(area / ar) >= min_board_height →  ar <= area / min_board_height²
    let ar_min = (min_board_width * min_board_width) / board_area;
    let ar_max = board_area / (min_board_height * min_board_height);

    let ar = safe_clamp(desired_ar, ar_min, ar_max);
    if (ar - desired_ar).abs() > 0.01 {
        eprintln!(
            "⚠ Warning: aspect_ratio adjusted from {:.2} to {:.2} to fit components (valid range: {:.2}–{:.2})",
            desired_ar, ar, ar_min, ar_max
        );
    }

    board.width = (board_area * ar).sqrt().max(min_board_width).max(20.0);
    board.height = (board_area / board.width).max(min_board_height).max(20.0);
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
    // Fallback: exhaustive grid search over entire board area
    let grid_step = 2.0;
    let mut best_pos = None;
    let mut best_dist = f64::MAX;
    let mut gy = min_y + mh / 2.0;
    while gy <= max_y - mh / 2.0 {
        let mut gx = min_x + mw / 2.0;
        while gx <= max_x - mw / 2.0 {
            let overlaps = positions.iter().enumerate().any(|(i, pos)| {
                if i == my_idx { return false; }
                if let Some((cx, cy)) = pos {
                    let (cw, ch) = sizes[i];
                    (gx - cx).abs() < (mw / 2.0 + cw / 2.0) && (gy - cy).abs() < (mh / 2.0 + ch / 2.0)
                } else { false }
            });
            if !overlaps {
                let dist = (gx - tx).powi(2) + (gy - ty).powi(2);
                if dist < best_dist {
                    best_dist = dist;
                    best_pos = Some((gx, gy));
                }
            }
            gx += grid_step;
        }
        gy += grid_step;
    }
    best_pos.unwrap_or((
        safe_clamp(tx, min_x + mw / 2.0, max_x - mw / 2.0),
        safe_clamp(ty, min_y + mh / 2.0, max_y - mh / 2.0),
    ))
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
    // Fallback: exhaustive grid search over entire board area
    let grid_step = 2.0;
    let mut best_pos = None;
    let mut best_dist = f64::MAX;
    let mut gy = min_y + mh / 2.0;
    while gy <= max_y - mh / 2.0 {
        let mut gx = min_x + mw / 2.0;
        while gx <= max_x - mw / 2.0 {
            let overlaps = positions.iter().enumerate().any(|(i, pos)| {
                if i == my_idx { return false; }
                if let Some((cx, cy)) = pos {
                    let (cw, ch) = sizes[i];
                    (gx - cx).abs() < (mw / 2.0 + cw / 2.0) && (gy - cy).abs() < (mh / 2.0 + ch / 2.0)
                } else { false }
            });
            if !overlaps {
                let dist = (gx - tx).powi(2) + (gy - ty).powi(2);
                if dist < best_dist {
                    best_dist = dist;
                    best_pos = Some((gx, gy));
                }
            }
            gx += grid_step;
        }
        gy += grid_step;
    }
    best_pos.unwrap_or((
        safe_clamp(tx, min_x + mw / 2.0, max_x - mw / 2.0),
        safe_clamp(ty, min_y + mh / 2.0, max_y - mh / 2.0),
    ))
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
        (min_x + hw, safe_clamp(near_y, min_y + hh, max_y - hh)),
        (max_x - hw, safe_clamp(near_y, min_y + hh, max_y - hh)),
        (safe_clamp(near_x, min_x + hw, max_x - hw), min_y + hh),
        (safe_clamp(near_x, min_x + hw, max_x - hw), max_y - hh),
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

/// Post-placement: move ALL passive components close to their functional anchor
/// (the IC, module, or connector they serve). This replaces the separate protection
/// and decoupling post-placement passes with one comprehensive pass.
fn post_place_passives_near_anchors(board: &mut Board) {
    let sizes = compute_placement_sizes(&board.components);
    let margin = 5.0;
    let n = board.components.len();

    // Rebuild adjacency matrix
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

    let clusters = build_passive_clusters(&board.components, &adj, &board.nets);
    let max_proximity = 3.0; // mm from anchor boundary

    // Sort passives by distance to anchor (farthest first) so we fix worst cases first
    let mut passive_moves: Vec<(usize, usize)> = Vec::new();
    for i in 0..n {
        if let Some(anchor_idx) = clusters[i] {
            passive_moves.push((i, anchor_idx));
        }
    }
    passive_moves.sort_by(|a, b| {
        let dist_a = {
            let (ax, ay) = (board.components[a.1].x, board.components[a.1].y);
            let (px, py) = (board.components[a.0].x, board.components[a.0].y);
            (px - ax).powi(2) + (py - ay).powi(2)
        };
        let dist_b = {
            let (ax, ay) = (board.components[b.1].x, board.components[b.1].y);
            let (px, py) = (board.components[b.0].x, board.components[b.0].y);
            (px - ax).powi(2) + (py - ay).powi(2)
        };
        dist_b
            .partial_cmp(&dist_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for (passive_idx, anchor_idx) in passive_moves {
        let (aw, ah) = sizes[anchor_idx];
        let ax = board.components[anchor_idx].x;
        let ay = board.components[anchor_idx].y;
        let px = board.components[passive_idx].x;
        let py = board.components[passive_idx].y;

        // Distance from passive center to anchor center
        let dist = ((px - ax).powi(2) + (py - ay).powi(2)).sqrt();
        // Approximate anchor boundary radius
        let anchor_radius = aw.max(ah) / 2.0;
        let (pw, ph) = sizes[passive_idx];
        let passive_radius = pw.max(ph) / 2.0;

        if dist > anchor_radius + passive_radius + max_proximity {
            // Move passive closer to anchor
            let positions: Vec<Option<(f64, f64)>> = board
                .components
                .iter()
                .enumerate()
                .map(|(k, c)| {
                    if k == passive_idx {
                        None
                    } else {
                        Some((c.x, c.y))
                    }
                })
                .collect();

            let (new_x, new_y) = find_non_overlapping(
                ax, ay, &sizes, &positions, passive_idx,
                margin, board.width - margin, margin, board.height - margin,
            );
            board.components[passive_idx].x = new_x;
            board.components[passive_idx].y = new_y;
        }
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
