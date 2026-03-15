use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use crate::schema::Board;

// Grid resolution: 0.25mm (compromise between precision and A* performance)
const GRID_SIZE: f64 = 0.25;
const CLEARANCE_MM: f64 = 0.25;
const TRACE_WIDTH_SIGNAL: f64 = 0.25;
const TRACE_WIDTH_POWER: f64 = 0.5;
const VIA_DRILL: f64 = 0.3;
const VIA_SIZE: f64 = 0.6;

// Cost constants (inspired by FreeRouting maze router)
const COST_STRAIGHT: i32 = 10;
const COST_DIRECTION_PENALTY: i32 = 15;
const COST_VIA: i32 = 200;
const COST_BEND: i32 = 5;

const MAX_RIPUP_ITERATIONS: usize = 50;
const MAX_ASTAR_ITERATIONS: usize = 500_000;
const CONGESTION_MULTIPLIER: f64 = 40.0;
const MAX_BLOCKERS_TO_RIP: usize = 5;

/// Routing grid cell
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GridPoint {
    pub x: i32,
    pub y: i32,
    pub layer: u8, // 0 = F.Cu, 1 = B.Cu
}

/// A routed trace segment
#[derive(Debug, Clone, serde::Serialize)]
pub struct TraceSegment {
    pub start: (f64, f64),
    pub end: (f64, f64),
    pub layer: u8,
    pub width: f64,
}

/// A via between layers
#[derive(Debug, Clone, serde::Serialize)]
pub struct Via {
    pub x: f64,
    pub y: f64,
    pub drill: f64,
    pub size: f64,
}

/// Result of routing a single net
#[derive(Debug, Clone, serde::Serialize)]
pub struct RoutedNet {
    pub name: String,
    pub segments: Vec<TraceSegment>,
    pub vias: Vec<Via>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct AStarNode {
    point: GridPoint,
    g_cost: i32,
    f_cost: i32,
    direction: Option<(i32, i32)>,
}

impl Ord for AStarNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .f_cost
            .cmp(&self.f_cost)
            .then_with(|| other.g_cost.cmp(&self.g_cost))
    }
}

impl PartialOrd for AStarNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub struct Router {
    grid_width: i32,
    grid_height: i32,
    /// Cells blocked by pads/components
    pad_obstacles: HashSet<GridPoint>,
    /// Cells blocked by routed traces (ref-counted for correct rip-up)
    trace_obstacles: HashMap<GridPoint, u32>,
    /// Map from net name to set of grid points that are pad locations for that net
    net_pads: HashMap<String, HashSet<GridPoint>>,
    /// Exact path cells → owning net name (for blocker identification)
    trace_ownership: HashMap<GridPoint, String>,
    /// Cumulative congestion history (incremented each rip-up iteration)
    congestion_history: HashMap<(i32, i32, u8), f64>,
}

fn mm_to_grid(mm: f64) -> i32 {
    (mm / GRID_SIZE).round() as i32
}

fn grid_to_mm(g: i32) -> f64 {
    g as f64 * GRID_SIZE
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

fn is_power_net(name: &str) -> bool {
    let upper = name.to_uppercase();
    upper == "GND"
        || upper.starts_with("VCC")
        || upper.starts_with("VBAT")
        || upper.starts_with("VDD")
        || upper.starts_with("V3V3")
        || upper == "3V3"
        || upper == "5V"
        || upper.starts_with("VBUS")
}

/// Check if a net connects critical components that need short, direct traces.
/// TVS/ESD→connector and decoupling cap→IC connections are routed first.
fn is_critical_net(net_name: &str, board: &Board) -> bool {
    let net = match board.nets.iter().find(|n| n.name == net_name) {
        Some(n) => n,
        None => return false,
    };

    let mut has_protection = false;
    let mut has_connector = false;
    let mut has_decoupling = false;
    let mut has_ic = false;

    for pin_ref in &net.pins {
        if let Some(comp) = board.components.iter().find(|c| c.name == pin_ref.component) {
            let fp_lower = comp.footprint.to_lowercase();
            let val_lower = comp.value.to_lowercase().replace(' ', "");
            let desc_lower = comp.description.as_deref().unwrap_or("").to_lowercase();
            let ref_lower = comp.ref_des.to_lowercase();

            // Protection component
            if val_lower.contains("tvs")
                || val_lower.contains("esd")
                || val_lower.contains("zener")
                || desc_lower.contains("protection")
                || desc_lower.contains("tvs")
                || desc_lower.contains("esd")
            {
                has_protection = true;
            }

            // Connector
            if fp_lower.contains("connector")
                || fp_lower.contains("usb")
                || fp_lower.contains("jst")
            {
                has_connector = true;
            }

            // Decoupling cap (100nF)
            let is_100nf = val_lower == "100nf"
                || val_lower == "0.1uf"
                || val_lower == "100n";
            if (ref_lower.starts_with('c') || fp_lower.contains("capacitor")) && is_100nf {
                has_decoupling = true;
            }

            // IC (multi-pin active device)
            if comp.pins.len() > 4
                && !fp_lower.contains("connector")
                && !fp_lower.contains("usb")
                && !fp_lower.contains("jst")
                && !fp_lower.contains("capacitor")
                && !fp_lower.contains("resistor")
                && !fp_lower.contains("led")
                && !fp_lower.contains("button")
            {
                has_ic = true;
            }
        }
    }

    (has_protection && has_connector) || (has_decoupling && has_ic)
}

fn trace_width_for_net(name: &str) -> f64 {
    if is_power_net(name) {
        TRACE_WIDTH_POWER
    } else {
        TRACE_WIDTH_SIGNAL
    }
}

/// Clearance in grid cells for a given trace width
fn clearance_cells(trace_width: f64) -> i32 {
    let total = trace_width / 2.0 + CLEARANCE_MM;
    (total / GRID_SIZE).ceil() as i32
}

/// Difficulty score for net ordering: harder nets (longer span, more pins) first
fn net_difficulty(pin_positions: &[(f64, f64)]) -> f64 {
    if pin_positions.len() < 2 {
        return 0.0;
    }
    let min_x = pin_positions.iter().map(|p| p.0).fold(f64::MAX, f64::min);
    let max_x = pin_positions.iter().map(|p| p.0).fold(f64::MIN, f64::max);
    let min_y = pin_positions.iter().map(|p| p.1).fold(f64::MAX, f64::min);
    let max_y = pin_positions.iter().map(|p| p.1).fold(f64::MIN, f64::max);
    let span = (max_x - min_x) + (max_y - min_y);
    span * pin_positions.len() as f64
}

impl Router {
    pub fn new(board_width: f64, board_height: f64, _grid_size: f64) -> Self {
        let grid_width = (board_width / GRID_SIZE).ceil() as i32 + 1;
        let grid_height = (board_height / GRID_SIZE).ceil() as i32 + 1;

        Router {
            grid_width,
            grid_height,
            pad_obstacles: HashSet::new(),
            trace_obstacles: HashMap::new(),
            net_pads: HashMap::new(),
            trace_ownership: HashMap::new(),
            congestion_history: HashMap::new(),
        }
    }

    /// Mark a rectangular area as pad obstacle on the grid (with clearance expansion).
    /// The clearance accounts for the widest possible trace width to ensure that
    /// any trace center placed at the edge of the obstacle zone still maintains
    /// CLEARANCE_MM edge-to-edge distance from the pad.
    fn mark_pad_obstacle(
        &mut self,
        cx: f64,
        cy: f64,
        half_w: f64,
        half_h: f64,
        layer: u8,
        net_name: &str,
    ) {
        // Clearance from pad edge must account for the widest trace that could
        // be routed nearby. Without this, a power trace (0.5mm) placed at the
        // obstacle boundary would overlap the pad.
        let cl = CLEARANCE_MM + TRACE_WIDTH_POWER / 2.0;
        let gx_start = mm_to_grid(cx - half_w - cl);
        let gx_end = mm_to_grid(cx + half_w + cl);
        let gy_start = mm_to_grid(cy - half_h - cl);
        let gy_end = mm_to_grid(cy + half_h + cl);

        let layers: Vec<u8> = if layer == 2 {
            vec![0, 1]
        } else {
            vec![layer]
        };

        for gx in gx_start..=gx_end {
            for gy in gy_start..=gy_end {
                for &l in &layers {
                    self.pad_obstacles.insert(GridPoint {
                        x: gx,
                        y: gy,
                        layer: l,
                    });
                }
            }
        }

        // Record exact pad cells (no clearance) as passable for this net
        let exact_gx_start = mm_to_grid(cx - half_w);
        let exact_gx_end = mm_to_grid(cx + half_w);
        let exact_gy_start = mm_to_grid(cy - half_h);
        let exact_gy_end = mm_to_grid(cy + half_h);

        let net_set = self.net_pads.entry(net_name.to_string()).or_default();
        for gx in exact_gx_start..=exact_gx_end {
            for gy in exact_gy_start..=exact_gy_end {
                for &l in &layers {
                    net_set.insert(GridPoint {
                        x: gx,
                        y: gy,
                        layer: l,
                    });
                }
            }
        }
    }

    /// Mark trace cells as occupied with clearance expansion (ref-counted)
    fn mark_trace_cells(&mut self, path: &[GridPoint], trace_width: f64, net_name: &str) {
        let cl = clearance_cells(trace_width);
        for &pt in path {
            self.trace_ownership.insert(pt, net_name.to_string());
            for dx in -cl..=cl {
                for dy in -cl..=cl {
                    *self.trace_obstacles.entry(GridPoint {
                        x: pt.x + dx,
                        y: pt.y + dy,
                        layer: pt.layer,
                    }).or_default() += 1;
                }
            }
        }
    }

    /// Remove trace cells for a previously routed path (ref-counted for correct rip-up)
    fn unmark_trace_cells(&mut self, path: &[GridPoint], trace_width: f64) {
        let cl = clearance_cells(trace_width);
        for &pt in path {
            self.trace_ownership.remove(&pt);
            for dx in -cl..=cl {
                for dy in -cl..=cl {
                    let cell = GridPoint {
                        x: pt.x + dx,
                        y: pt.y + dy,
                        layer: pt.layer,
                    };
                    if let Some(count) = self.trace_obstacles.get_mut(&cell) {
                        if *count <= 1 {
                            self.trace_obstacles.remove(&cell);
                        } else {
                            *count -= 1;
                        }
                    }
                }
            }
        }
    }

    /// Check if a grid point is valid for routing a specific net.
    /// Own pad cells are ALWAYS passable (trace clearance from nearby nets
    /// doesn't prevent connecting to our own pads).
    fn is_valid_for_net(&self, point: GridPoint, net_name: &str) -> bool {
        if point.x < 0 || point.x >= self.grid_width || point.y < 0 || point.y >= self.grid_height
        {
            return false;
        }

        // Own pad cells are always reachable — even if covered by another
        // net's trace clearance zone. The pad physically exists regardless;
        // routing to it doesn't worsen any clearance situation.
        if let Some(net_set) = self.net_pads.get(net_name) {
            if net_set.contains(&point) {
                return true;
            }
        }

        if self.trace_obstacles.contains_key(&point) {
            return false;
        }

        if self.pad_obstacles.contains(&point) {
            return false;
        }

        true
    }

    /// Identify which routed nets block a failed net by checking trace ownership
    /// within the bounding box of the failed net's pins.
    fn find_blocking_nets(&self, pin_positions: &[(f64, f64)], net_name: &str) -> Vec<String> {
        let (mut min_x, mut min_y) = (i32::MAX, i32::MAX);
        let (mut max_x, mut max_y) = (i32::MIN, i32::MIN);
        for &(px, py) in pin_positions {
            let gx = mm_to_grid(px);
            let gy = mm_to_grid(py);
            min_x = min_x.min(gx);
            max_x = max_x.max(gx);
            min_y = min_y.min(gy);
            max_y = max_y.max(gy);
        }
        // Expand bounding box by margin (~5mm)
        let margin = 20;
        min_x -= margin;
        max_x += margin;
        min_y -= margin;
        max_y += margin;

        let mut counts: HashMap<String, usize> = HashMap::new();
        for (&pt, owner) in &self.trace_ownership {
            if owner != net_name
                && pt.x >= min_x
                && pt.x <= max_x
                && pt.y >= min_y
                && pt.y <= max_y
            {
                *counts.entry(owner.clone()).or_default() += 1;
            }
        }

        let mut result: Vec<_> = counts.into_iter().collect();
        result.sort_by(|a, b| b.1.cmp(&a.1)); // Most blocking first
        result.into_iter().map(|(name, _)| name).collect()
    }

    /// Additional A* cost for congested cells
    fn congestion_cost(&self, point: GridPoint) -> i32 {
        self.congestion_history
            .get(&(point.x, point.y, point.layer))
            .map(|&h| (h * CONGESTION_MULTIPLIER) as i32)
            .unwrap_or(0)
    }

    /// Route all nets using real pad positions from footprint data.
    pub fn route_all(&mut self, board: &Board) -> Vec<RoutedNet> {
        // Step 1: Build net membership map
        let mut pin_to_net: HashMap<(String, String), String> = HashMap::new();
        for net in &board.nets {
            for pin_ref in &net.pins {
                pin_to_net.insert(
                    (pin_ref.component.clone(), pin_ref.pin.clone()),
                    net.name.clone(),
                );
            }
        }

        // Step 2: Mark ALL pads of ALL components as obstacles with clearance
        for comp in &board.components {
            if let Some(fp) = &comp.footprint_data {
                for pad in fp.signal_pads() {
                    // Apply component rotation to pad position
                    let (rot_x, rot_y) = rotate_point(pad.at_x, pad.at_y, comp.rotation);
                    let abs_x = comp.x + rot_x;
                    let abs_y = comp.y + rot_y;
                    let half_w = pad.size_w / 2.0;
                    let half_h = pad.size_h / 2.0;

                    let layer = if pad.pad_type == "thru_hole" {
                        2 // both layers
                    } else if pad.layers.iter().any(|l| l.contains("B.Cu")) {
                        1
                    } else {
                        0
                    };

                    let net_name = comp
                        .pins
                        .iter()
                        .find(|p| p.number == pad.number)
                        .and_then(|p| pin_to_net.get(&(comp.name.clone(), p.name.clone())))
                        .cloned()
                        .unwrap_or_default();

                    self.mark_pad_obstacle(abs_x, abs_y, half_w, half_h, layer, &net_name);
                }
            }
        }

        // Note: Silkscreen text labels are NOT routing obstacles.
        // Silkscreen is printed on the solder mask layer, not on copper.
        // Traces on F.Cu/B.Cu pass underneath silkscreen without issue.

        // Step 3: Collect nets with pin positions
        struct NetInfo {
            name: String,
            pin_positions: Vec<(f64, f64)>,
            trace_width: f64,
        }

        let mut net_infos: Vec<NetInfo> = Vec::new();
        for net in &board.nets {
            if net.pins.len() < 2 {
                continue;
            }

            let pin_positions: Vec<(f64, f64)> = net
                .pins
                .iter()
                .filter_map(|pin_ref| {
                    let comp = board
                        .components
                        .iter()
                        .find(|c| c.name == pin_ref.component)?;
                    let pin = comp.pins.iter().find(|p| p.name == pin_ref.pin)?;
                    // Apply component rotation to pin position
                    let (rot_x, rot_y) = rotate_point(pin.x, pin.y, comp.rotation);
                    Some((comp.x + rot_x, comp.y + rot_y))
                })
                .collect();

            if pin_positions.len() < 2 {
                continue;
            }

            net_infos.push(NetInfo {
                name: net.name.clone(),
                pin_positions,
                trace_width: trace_width_for_net(&net.name),
            });
        }

        // Step 4: Sort nets — critical first, then signal by difficulty (harder first),
        // then power nets last
        net_infos.sort_by(|a, b| {
            let a_critical = is_critical_net(&a.name, board);
            let b_critical = is_critical_net(&b.name, board);
            let a_power = is_power_net(&a.name);
            let b_power = is_power_net(&b.name);
            b_critical
                .cmp(&a_critical) // critical nets first (true > false)
                .then_with(|| a_power.cmp(&b_power)) // power nets last
                .then_with(|| {
                    // Within same tier, harder nets first (longer span, more pins)
                    let a_diff = net_difficulty(&a.pin_positions);
                    let b_diff = net_difficulty(&b.pin_positions);
                    b_diff
                        .partial_cmp(&a_diff)
                        .unwrap_or(Ordering::Equal)
                })
        });

        let total = net_infos.len();

        // Step 5: Initial routing pass
        let mut routed: Vec<RoutedNet> = Vec::new();
        let mut routed_paths: HashMap<String, (Vec<Vec<GridPoint>>, f64)> = HashMap::new();
        let mut unrouted: Vec<String> = Vec::new();

        for net_info in &net_infos {
            match self.route_net(
                &net_info.name,
                &net_info.pin_positions,
                net_info.trace_width,
            ) {
                Some((routed_net, paths)) => {
                    routed_paths
                        .insert(net_info.name.clone(), (paths, net_info.trace_width));
                    routed.push(routed_net);
                }
                None => {
                    unrouted.push(net_info.name.clone());
                }
            }
        }

        eprintln!("  Initial: {}/{} nets routed", routed.len(), total);

        if unrouted.is_empty() {
            return routed;
        }

        // Step 6: Rip-up & retry with congestion escalation
        let mut best_routed: Vec<RoutedNet> = routed.clone();
        let mut best_count = routed.len();
        let mut stagnation_count = 0u32;

        for iteration in 0..MAX_RIPUP_ITERATIONS {
            if unrouted.is_empty() {
                break;
            }

            // Update congestion map from current routing
            for (_, (paths, _)) in &routed_paths {
                for path in paths {
                    for &pt in path {
                        *self
                            .congestion_history
                            .entry((pt.x, pt.y, pt.layer))
                            .or_default() += 1.0;
                    }
                }
            }

            let failed = std::mem::take(&mut unrouted);

            for net_name in &failed {
                let net_info = net_infos.iter().find(|n| &n.name == net_name).unwrap();
                let mut succeeded = false;

                // Try without rip-up (congestion costs may have shifted things)
                if let Some((rn, paths)) = self.route_net(
                    &net_info.name,
                    &net_info.pin_positions,
                    net_info.trace_width,
                ) {
                    routed_paths
                        .insert(net_info.name.clone(), (paths, net_info.trace_width));
                    routed.push(rn);
                    succeeded = true;
                }

                if !succeeded {
                    // Identify blocking nets and rip up the worst offenders
                    let blockers =
                        self.find_blocking_nets(&net_info.pin_positions, &net_info.name);
                    let to_rip: Vec<String> =
                        blockers.into_iter().take(MAX_BLOCKERS_TO_RIP).collect();

                    if to_rip.is_empty() {
                        unrouted.push(net_name.clone());
                        continue;
                    }

                    // Rip up blocking nets (save for rollback)
                    let mut ripped: Vec<(String, Vec<Vec<GridPoint>>, f64, RoutedNet)> =
                        Vec::new();
                    for rip_name in &to_rip {
                        if let Some((paths, tw)) = routed_paths.remove(rip_name) {
                            for path in &paths {
                                self.unmark_trace_cells(path, tw);
                            }
                            if let Some(idx) =
                                routed.iter().position(|r| r.name == *rip_name)
                            {
                                let rn = routed.remove(idx);
                                ripped.push((rip_name.clone(), paths, tw, rn));
                            }
                        }
                    }

                    // Try routing the failed net in the freed space
                    if let Some((rn, paths)) = self.route_net(
                        &net_info.name,
                        &net_info.pin_positions,
                        net_info.trace_width,
                    ) {
                        routed_paths.insert(
                            net_info.name.clone(),
                            (paths, net_info.trace_width),
                        );
                        routed.push(rn);

                        // Re-route the ripped nets (they should find alternative paths)
                        for (rip_name, _old_paths, rip_tw, _old_rn) in ripped {
                            let rip_info =
                                net_infos.iter().find(|n| n.name == rip_name).unwrap();
                            if let Some((re_rn, re_paths)) = self.route_net(
                                &rip_info.name,
                                &rip_info.pin_positions,
                                rip_info.trace_width,
                            ) {
                                routed_paths
                                    .insert(rip_name, (re_paths, rip_tw));
                                routed.push(re_rn);
                            } else {
                                unrouted.push(rip_name);
                            }
                        }
                    } else {
                        // Rollback: restore all ripped nets
                        for (rip_name, old_paths, rip_tw, old_rn) in ripped {
                            for path in &old_paths {
                                self.mark_trace_cells(path, rip_tw, &rip_name);
                            }
                            routed_paths
                                .insert(rip_name, (old_paths, rip_tw));
                            routed.push(old_rn);
                        }
                        unrouted.push(net_name.clone());
                    }
                }
            }

            // Track best result
            if routed.len() > best_count {
                best_routed = routed.clone();
                best_count = routed.len();
                stagnation_count = 0;
            } else {
                stagnation_count += 1;
            }

            eprintln!(
                "  Rip-up iteration {}/{}: {}/{} nets routed (best: {}/{})",
                iteration + 1,
                MAX_RIPUP_ITERATIONS,
                routed.len(),
                total,
                best_count,
                total
            );

            if stagnation_count >= 8 {
                break; // No more progress possible
            }
        }

        // Step 7: Use best result, report unrouted
        let routed_names: HashSet<String> =
            best_routed.iter().map(|r| r.name.clone()).collect();
        for net_info in &net_infos {
            if !routed_names.contains(&net_info.name) {
                eprintln!("WARNING: Net '{}' could not be routed!", net_info.name);
                best_routed.push(RoutedNet {
                    name: net_info.name.clone(),
                    segments: Vec::new(),
                    vias: Vec::new(),
                });
            }
        }

        eprintln!(
            "  Routing complete: {}/{} nets routed ({} rip-up iterations used)",
            best_count,
            total,
            self.congestion_history.len().min(MAX_RIPUP_ITERATIONS)
        );

        best_routed
    }

    /// Route a single net. For multi-pin signal nets, tries different MST start pins.
    /// Power nets allow partial routing (copper pour handles the rest).
    fn route_net(
        &mut self,
        net_name: &str,
        pin_positions: &[(f64, f64)],
        trace_width: f64,
    ) -> Option<(RoutedNet, Vec<Vec<GridPoint>>)> {
        let is_power = is_power_net(net_name);

        // For multi-pin signal nets, try different MST start pins
        let max_starts = if !is_power && pin_positions.len() > 2 {
            pin_positions.len().min(4)
        } else {
            1
        };

        for start_idx in 0..max_starts {
            if let Some(result) =
                self.route_net_from(net_name, pin_positions, trace_width, start_idx, is_power)
            {
                return Some(result);
            }
        }
        None
    }

    /// Try routing a net using Prim's MST starting from a specific pin.
    fn route_net_from(
        &mut self,
        net_name: &str,
        pin_positions: &[(f64, f64)],
        trace_width: f64,
        start_idx: usize,
        allow_partial: bool,
    ) -> Option<(RoutedNet, Vec<Vec<GridPoint>>)> {
        let mut segments = Vec::new();
        let mut vias = Vec::new();
        let mut all_paths: Vec<Vec<GridPoint>> = Vec::new();

        // Prim's MST ordering from start_idx
        let mut connected: Vec<(f64, f64)> = vec![pin_positions[start_idx]];
        let mut remaining: Vec<(f64, f64)> = pin_positions
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != start_idx)
            .map(|(_, &p)| p)
            .collect();

        while !remaining.is_empty() {
            let mut best_dist = f64::MAX;
            let mut best_ri = 0;
            let mut best_start = connected[0];

            for (ri, rpos) in remaining.iter().enumerate() {
                for cpos in &connected {
                    let dist = (rpos.0 - cpos.0).abs() + (rpos.1 - cpos.1).abs();
                    if dist < best_dist {
                        best_dist = dist;
                        best_ri = ri;
                        best_start = *cpos;
                    }
                }
            }

            let target = remaining.remove(best_ri);

            match self.find_path(best_start, target, net_name, trace_width) {
                Some(path) => {
                    for window in path.windows(2) {
                        let from = window[0];
                        let to = window[1];

                        if from.layer != to.layer {
                            vias.push(Via {
                                x: grid_to_mm(from.x),
                                y: grid_to_mm(from.y),
                                drill: VIA_DRILL,
                                size: VIA_SIZE,
                            });
                        } else {
                            segments.push(TraceSegment {
                                start: (grid_to_mm(from.x), grid_to_mm(from.y)),
                                end: (grid_to_mm(to.x), grid_to_mm(to.y)),
                                layer: to.layer,
                                width: trace_width,
                            });
                        }
                    }

                    self.mark_trace_cells(&path, trace_width, net_name);
                    all_paths.push(path);
                    connected.push(target);
                }
                None => {
                    if allow_partial {
                        // Power nets: skip failed connection (copper pour handles it)
                        continue;
                    }
                    // Signal nets: undo ALL traces (no partial routing)
                    for path in &all_paths {
                        self.unmark_trace_cells(path, trace_width);
                    }
                    return None;
                }
            }
        }

        if all_paths.is_empty() {
            return None;
        }

        let segments = merge_collinear_segments(segments);

        Some((
            RoutedNet {
                name: net_name.to_string(),
                segments,
                vias,
            },
            all_paths,
        ))
    }

    fn find_path(
        &self,
        start: (f64, f64),
        end: (f64, f64),
        net_name: &str,
        _trace_width: f64,
    ) -> Option<Vec<GridPoint>> {
        let start_gx = mm_to_grid(start.0);
        let start_gy = mm_to_grid(start.1);
        let end_gx = mm_to_grid(end.0);
        let end_gy = mm_to_grid(end.1);

        if start_gx == end_gx && start_gy == end_gy {
            return Some(vec![GridPoint {
                x: start_gx,
                y: start_gy,
                layer: 0,
            }]);
        }

        let end_ref = GridPoint {
            x: end_gx,
            y: end_gy,
            layer: 0,
        };

        let mut open_set = BinaryHeap::new();
        let mut came_from: HashMap<GridPoint, GridPoint> = HashMap::new();
        let mut g_score: HashMap<GridPoint, i32> = HashMap::new();
        let mut direction_at: HashMap<GridPoint, (i32, i32)> = HashMap::new();

        // Try starting on both layers
        for start_layer in 0..=1u8 {
            let sp = GridPoint {
                x: start_gx,
                y: start_gy,
                layer: start_layer,
            };
            if self.is_valid_for_net(sp, net_name) {
                g_score.insert(sp, 0);
                open_set.push(AStarNode {
                    point: sp,
                    g_cost: 0,
                    f_cost: Self::heuristic(sp, end_ref),
                    direction: None,
                });
            }
        }

        let directions: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
        let mut iterations: usize = 0;

        while let Some(current) = open_set.pop() {
            iterations += 1;
            if iterations > MAX_ASTAR_ITERATIONS {
                eprintln!(
                    "WARNING: A* exceeded {} iterations for net '{}' ({:.2},{:.2})->({:.2},{:.2}), leaving unrouted",
                    MAX_ASTAR_ITERATIONS, net_name,
                    start.0, start.1, end.0, end.1
                );
                return None;
            }

            if current.point.x == end_gx && current.point.y == end_gy {
                let mut path = vec![current.point];
                let mut curr = current.point;
                while let Some(&prev) = came_from.get(&curr) {
                    path.push(prev);
                    curr = prev;
                }
                path.reverse();
                eprintln!(
                    "  A* routed net '{}': explored {} cells, path length {} segments",
                    net_name, iterations, path.len()
                );
                return Some(path);
            }

            let current_g = match g_score.get(&current.point) {
                Some(&g) if g == current.g_cost => g,
                _ => continue,
            };

            let current_dir = direction_at.get(&current.point).copied();

            for &(dx, dy) in &directions {
                let neighbor = GridPoint {
                    x: current.point.x + dx,
                    y: current.point.y + dy,
                    layer: current.point.layer,
                };

                if !self.is_valid_for_net(neighbor, net_name) {
                    continue;
                }

                let mut move_cost = COST_STRAIGHT;

                // Layer direction preference: F.Cu horizontal, B.Cu vertical
                let is_horizontal = dy == 0;
                let preferred = if current.point.layer == 0 {
                    is_horizontal
                } else {
                    !is_horizontal
                };
                if !preferred {
                    move_cost += COST_DIRECTION_PENALTY;
                }

                if let Some(prev_dir) = current_dir {
                    if prev_dir != (dx, dy) {
                        move_cost += COST_BEND;
                    }
                }

                // Congestion cost: penalize cells used in previous iterations
                move_cost += self.congestion_cost(neighbor);

                let tentative_g = current_g + move_cost;
                if tentative_g < *g_score.get(&neighbor).unwrap_or(&i32::MAX) {
                    came_from.insert(neighbor, current.point);
                    g_score.insert(neighbor, tentative_g);
                    direction_at.insert(neighbor, (dx, dy));
                    open_set.push(AStarNode {
                        point: neighbor,
                        g_cost: tentative_g,
                        f_cost: tentative_g + Self::heuristic(neighbor, end_ref),
                        direction: Some((dx, dy)),
                    });
                }
            }

            // Via (layer change)
            let other_layer = 1 - current.point.layer;
            let via_point = GridPoint {
                x: current.point.x,
                y: current.point.y,
                layer: other_layer,
            };
            if self.is_valid_for_net(via_point, net_name) {
                let tentative_g = current_g + COST_VIA + self.congestion_cost(via_point);
                if tentative_g < *g_score.get(&via_point).unwrap_or(&i32::MAX) {
                    came_from.insert(via_point, current.point);
                    g_score.insert(via_point, tentative_g);
                    direction_at.remove(&via_point);
                    open_set.push(AStarNode {
                        point: via_point,
                        g_cost: tentative_g,
                        f_cost: tentative_g + Self::heuristic(via_point, end_ref),
                        direction: None,
                    });
                }
            }
        }

        eprintln!(
            "WARNING: A* exhausted search space for net '{}' after {} iterations ({:.2},{:.2})->({:.2},{:.2}), no route found",
            net_name, iterations, start.0, start.1, end.0, end.1
        );
        None
    }

    fn heuristic(a: GridPoint, b: GridPoint) -> i32 {
        let manhattan = (a.x - b.x).abs() + (a.y - b.y).abs();
        manhattan * COST_STRAIGHT
    }
}

/// Merge collinear consecutive segments on the same layer
fn merge_collinear_segments(segments: Vec<TraceSegment>) -> Vec<TraceSegment> {
    if segments.is_empty() {
        return segments;
    }

    let mut merged: Vec<TraceSegment> = Vec::new();
    let mut current = segments[0].clone();

    for seg in segments.iter().skip(1) {
        if seg.layer == current.layer && (seg.width - current.width).abs() < 0.001 {
            let cur_dx = current.end.0 - current.start.0;
            let cur_dy = current.end.1 - current.start.1;
            let seg_dx = seg.end.0 - seg.start.0;
            let seg_dy = seg.end.1 - seg.start.1;

            let same_dir = (cur_dx == 0.0 && seg_dx == 0.0 && cur_dy.signum() == seg_dy.signum())
                || (cur_dy == 0.0 && seg_dy == 0.0 && cur_dx.signum() == seg_dx.signum());

            let contiguous = (current.end.0 - seg.start.0).abs() < 0.001
                && (current.end.1 - seg.start.1).abs() < 0.001;

            if same_dir && contiguous {
                current.end = seg.end;
                continue;
            }
        }

        merged.push(current);
        current = seg.clone();
    }
    merged.push(current);
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::footprint::{FootprintData, PadData};
    use crate::schema::{Board, Component, Net, Options, Pin, PinRef, PinType};

    const MIN_CLEARANCE: f64 = 0.2;

    /// Verify that clearance_cells for signal traces guarantees minimum edge-to-edge gap.
    #[test]
    fn test_clearance_cells_signal_to_signal() {
        let cl = clearance_cells(TRACE_WIDTH_SIGNAL);
        let min_center_to_center = (cl + 1) as f64 * GRID_SIZE;
        let edge_to_edge = min_center_to_center - TRACE_WIDTH_SIGNAL;
        assert!(
            edge_to_edge >= MIN_CLEARANCE,
            "Signal-to-signal edge clearance {:.3}mm < {:.3}mm minimum",
            edge_to_edge,
            MIN_CLEARANCE
        );
    }

    /// Verify that clearance_cells for power traces guarantees minimum edge-to-edge gap.
    #[test]
    fn test_clearance_cells_power_to_power() {
        let cl = clearance_cells(TRACE_WIDTH_POWER);
        let min_center_to_center = (cl + 1) as f64 * GRID_SIZE;
        let edge_to_edge = min_center_to_center - TRACE_WIDTH_POWER;
        assert!(
            edge_to_edge >= MIN_CLEARANCE,
            "Power-to-power edge clearance {:.3}mm < {:.3}mm minimum",
            edge_to_edge,
            MIN_CLEARANCE
        );
    }

    /// Verify clearance between a signal trace obstacle and an adjacent power trace center.
    #[test]
    fn test_clearance_cells_signal_near_power() {
        let cl_signal = clearance_cells(TRACE_WIDTH_SIGNAL);
        let min_center_to_center = (cl_signal + 1) as f64 * GRID_SIZE;
        let edge_to_edge =
            min_center_to_center - TRACE_WIDTH_SIGNAL / 2.0 - TRACE_WIDTH_POWER / 2.0;
        assert!(
            edge_to_edge >= MIN_CLEARANCE,
            "Signal-to-power edge clearance {:.3}mm < {:.3}mm minimum",
            edge_to_edge,
            MIN_CLEARANCE
        );
    }

    /// Verify mark/unmark trace cells are symmetric (no orphan obstacles).
    #[test]
    fn test_trace_mark_unmark_symmetry() {
        let mut router = Router::new(10.0, 10.0, GRID_SIZE);
        let path = vec![
            GridPoint { x: 5, y: 5, layer: 0 },
            GridPoint { x: 6, y: 5, layer: 0 },
            GridPoint { x: 7, y: 5, layer: 0 },
        ];

        assert!(router.trace_obstacles.is_empty());
        router.mark_trace_cells(&path, TRACE_WIDTH_SIGNAL, "TEST_NET");
        assert!(!router.trace_obstacles.is_empty());
        router.unmark_trace_cells(&path, TRACE_WIDTH_SIGNAL);
        assert!(
            router.trace_obstacles.is_empty(),
            "Trace obstacles not fully cleared after unmark"
        );
    }

    /// Verify that merge_collinear_segments joins aligned contiguous segments.
    #[test]
    fn test_merge_collinear_segments_basic() {
        let segments = vec![
            TraceSegment {
                start: (0.0, 0.0),
                end: (1.0, 0.0),
                layer: 0,
                width: 0.25,
            },
            TraceSegment {
                start: (1.0, 0.0),
                end: (2.0, 0.0),
                layer: 0,
                width: 0.25,
            },
            TraceSegment {
                start: (2.0, 0.0),
                end: (3.0, 0.0),
                layer: 0,
                width: 0.25,
            },
        ];
        let merged = merge_collinear_segments(segments);
        assert_eq!(merged.len(), 1, "Three collinear segments should merge into one");
        assert!((merged[0].start.0 - 0.0).abs() < 0.001);
        assert!((merged[0].end.0 - 3.0).abs() < 0.001);
    }

    /// Verify that non-collinear segments are not merged.
    #[test]
    fn test_merge_collinear_segments_bend() {
        let segments = vec![
            TraceSegment {
                start: (0.0, 0.0),
                end: (1.0, 0.0),
                layer: 0,
                width: 0.25,
            },
            TraceSegment {
                start: (1.0, 0.0),
                end: (1.0, 1.0),
                layer: 0,
                width: 0.25,
            },
        ];
        let merged = merge_collinear_segments(segments);
        assert_eq!(merged.len(), 2, "Non-collinear segments should not merge");
    }

    /// Verify that pad obstacle clearance accounts for trace width.
    #[test]
    fn test_pad_obstacle_accounts_for_trace_width() {
        let mut router = Router::new(20.0, 20.0, GRID_SIZE);

        // Place a pad at (10, 10) with 1mm x 1mm size
        router.mark_pad_obstacle(10.0, 10.0, 0.5, 0.5, 0, "NET1");

        // Check that cells near the pad are blocked
        let pad_edge_x = 10.0 + 0.5; // = 10.5mm (pad right edge)

        // A power trace center placed right at clearance boundary should still
        // maintain CLEARANCE_MM from the pad edge. The obstacle zone should extend
        // CLEARANCE_MM + TRACE_WIDTH_POWER/2 from pad edge.
        let too_close_x = pad_edge_x + CLEARANCE_MM; // Not enough clearance for power trace
        let too_close_gx = mm_to_grid(too_close_x);
        let point = GridPoint {
            x: too_close_gx,
            y: mm_to_grid(10.0),
            layer: 0,
        };

        // This point should be blocked because a power trace here would be too
        // close to the pad (its edge would only be CLEARANCE_MM - POWER/2 from pad)
        assert!(
            !router.is_valid_for_net(point, "OTHER_NET"),
            "Point at {:.2}mm from pad edge should be blocked (too close for power trace)",
            too_close_x - pad_edge_x
        );
    }

    /// Helper to create a simple test board with two components and one net.
    fn make_simple_board() -> Board {
        let pad_a = PadData {
            number: "1".to_string(),
            pad_type: "smd".to_string(),
            shape: "rect".to_string(),
            at_x: 0.0,
            at_y: 0.0,
            size_w: 1.0,
            size_h: 0.6,
            layers: vec!["F.Cu".to_string()],
            drill: None,
        };
        let pad_b = PadData {
            number: "1".to_string(),
            pad_type: "smd".to_string(),
            shape: "rect".to_string(),
            at_x: 0.0,
            at_y: 0.0,
            size_w: 1.0,
            size_h: 0.6,
            layers: vec!["F.Cu".to_string()],
            drill: None,
        };

        Board {
            width: 20.0,
            height: 20.0,
            aspect_ratio: 1.0,
            layers: 2,
            trace_width: 0.25,
            clearance: 0.25,
            options: Options::default(),
            components: vec![
                Component {
                    ref_des: "R1".to_string(),
                    name: "comp_a".to_string(),
                    footprint: "R_0402".to_string(),
                    value: "10K".to_string(),
                    lcsc: None,
                    pins: vec![Pin {
                        name: "P1".to_string(),
                        number: "1".to_string(),
                        pin_type: PinType::Passive,
                        x: 0.0,
                        y: 0.0,
                    }],
                    description: None,
                    footprint_data: Some(FootprintData {
                        name: "R_0402".to_string(),
                        pads: vec![pad_a],
                        lines: vec![],
                    }),
                    x: 5.0,
                    y: 10.0,
                    rotation: 0.0,
                },
                Component {
                    ref_des: "R2".to_string(),
                    name: "comp_b".to_string(),
                    footprint: "R_0402".to_string(),
                    value: "10K".to_string(),
                    lcsc: None,
                    pins: vec![Pin {
                        name: "P1".to_string(),
                        number: "1".to_string(),
                        pin_type: PinType::Passive,
                        x: 0.0,
                        y: 0.0,
                    }],
                    description: None,
                    footprint_data: Some(FootprintData {
                        name: "R_0402".to_string(),
                        pads: vec![pad_b],
                        lines: vec![],
                    }),
                    x: 15.0,
                    y: 10.0,
                    rotation: 0.0,
                },
            ],
            nets: vec![Net {
                name: "NET1".to_string(),
                pins: vec![
                    PinRef {
                        component: "comp_a".to_string(),
                        pin: "P1".to_string(),
                    },
                    PinRef {
                        component: "comp_b".to_string(),
                        pin: "P1".to_string(),
                    },
                ],
            }],
        }
    }

    /// Route a simple 2-pin net and verify trace segments respect minimum clearance
    /// with pads of other nets.
    #[test]
    fn test_routed_traces_respect_clearance() {
        let board = make_simple_board();
        let mut router = Router::new(board.width, board.height, GRID_SIZE);
        let routed = router.route_all(&board);

        for net in &routed {
            for seg in &net.segments {
                // Verify segments are within board bounds
                assert!(
                    seg.start.0 >= 0.0 && seg.start.0 <= board.width,
                    "Segment start X {:.2} out of board bounds",
                    seg.start.0
                );
                assert!(
                    seg.start.1 >= 0.0 && seg.start.1 <= board.height,
                    "Segment start Y {:.2} out of board bounds",
                    seg.start.1
                );
                assert!(
                    seg.end.0 >= 0.0 && seg.end.0 <= board.width,
                    "Segment end X {:.2} out of board bounds",
                    seg.end.0
                );
                assert!(
                    seg.end.1 >= 0.0 && seg.end.1 <= board.height,
                    "Segment end Y {:.2} out of board bounds",
                    seg.end.1
                );
            }
        }
    }

    /// Verify that two parallel routed traces maintain minimum clearance.
    #[test]
    fn test_parallel_traces_clearance() {
        // Create a board with two parallel nets that must be routed side by side
        let make_pad = || PadData {
            number: "1".to_string(),
            pad_type: "smd".to_string(),
            shape: "rect".to_string(),
            at_x: 0.0,
            at_y: 0.0,
            size_w: 0.6,
            size_h: 0.6,
            layers: vec!["F.Cu".to_string()],
            drill: None,
        };

        let board = Board {
            width: 20.0,
            height: 20.0,
            aspect_ratio: 1.0,
            layers: 2,
            trace_width: 0.25,
            clearance: 0.25,
            options: Options::default(),
            components: vec![
                Component {
                    ref_des: "R1".to_string(),
                    name: "c1".to_string(),
                    footprint: "R_0402".to_string(),
                    value: "10K".to_string(),
                    lcsc: None,
                    pins: vec![Pin {
                        name: "P1".to_string(),
                        number: "1".to_string(),
                        pin_type: PinType::Passive,
                        x: 0.0,
                        y: 0.0,
                    }],
                    description: None,
                    footprint_data: Some(FootprintData {
                        name: "R_0402".to_string(),
                        pads: vec![make_pad()],
                        lines: vec![],
                    }),
                    x: 3.0,
                    y: 10.0,
                    rotation: 0.0,
                },
                Component {
                    ref_des: "R2".to_string(),
                    name: "c2".to_string(),
                    footprint: "R_0402".to_string(),
                    value: "10K".to_string(),
                    lcsc: None,
                    pins: vec![Pin {
                        name: "P1".to_string(),
                        number: "1".to_string(),
                        pin_type: PinType::Passive,
                        x: 0.0,
                        y: 0.0,
                    }],
                    description: None,
                    footprint_data: Some(FootprintData {
                        name: "R_0402".to_string(),
                        pads: vec![make_pad()],
                        lines: vec![],
                    }),
                    x: 17.0,
                    y: 10.0,
                    rotation: 0.0,
                },
                Component {
                    ref_des: "R3".to_string(),
                    name: "c3".to_string(),
                    footprint: "R_0402".to_string(),
                    value: "10K".to_string(),
                    lcsc: None,
                    pins: vec![Pin {
                        name: "P1".to_string(),
                        number: "1".to_string(),
                        pin_type: PinType::Passive,
                        x: 0.0,
                        y: 0.0,
                    }],
                    description: None,
                    footprint_data: Some(FootprintData {
                        name: "R_0402".to_string(),
                        pads: vec![make_pad()],
                        lines: vec![],
                    }),
                    x: 3.0,
                    y: 10.75,
                    rotation: 0.0,
                },
                Component {
                    ref_des: "R4".to_string(),
                    name: "c4".to_string(),
                    footprint: "R_0402".to_string(),
                    value: "10K".to_string(),
                    lcsc: None,
                    pins: vec![Pin {
                        name: "P1".to_string(),
                        number: "1".to_string(),
                        pin_type: PinType::Passive,
                        x: 0.0,
                        y: 0.0,
                    }],
                    description: None,
                    footprint_data: Some(FootprintData {
                        name: "R_0402".to_string(),
                        pads: vec![make_pad()],
                        lines: vec![],
                    }),
                    x: 17.0,
                    y: 10.75,
                    rotation: 0.0,
                },
            ],
            nets: vec![
                Net {
                    name: "NET_A".to_string(),
                    pins: vec![
                        PinRef { component: "c1".to_string(), pin: "P1".to_string() },
                        PinRef { component: "c2".to_string(), pin: "P1".to_string() },
                    ],
                },
                Net {
                    name: "NET_B".to_string(),
                    pins: vec![
                        PinRef { component: "c3".to_string(), pin: "P1".to_string() },
                        PinRef { component: "c4".to_string(), pin: "P1".to_string() },
                    ],
                },
            ],
        };

        let mut router = Router::new(board.width, board.height, GRID_SIZE);
        let routed = router.route_all(&board);

        // Collect all segments from different nets
        let net_a_segs: Vec<&TraceSegment> = routed
            .iter()
            .filter(|r| r.name == "NET_A")
            .flat_map(|r| &r.segments)
            .collect();
        let net_b_segs: Vec<&TraceSegment> = routed
            .iter()
            .filter(|r| r.name == "NET_B")
            .flat_map(|r| &r.segments)
            .collect();

        // Check minimum distance between segments of different nets
        for seg_a in &net_a_segs {
            for seg_b in &net_b_segs {
                if seg_a.layer != seg_b.layer {
                    continue;
                }
                let dist = min_segment_distance(seg_a, seg_b);
                let edge_dist = dist - seg_a.width / 2.0 - seg_b.width / 2.0;
                assert!(
                    edge_dist >= MIN_CLEARANCE - 0.01, // small tolerance for grid quantization
                    "Traces too close: NET_A seg ({:.2},{:.2})-({:.2},{:.2}) to NET_B seg ({:.2},{:.2})-({:.2},{:.2}): edge distance {:.3}mm < {:.3}mm",
                    seg_a.start.0, seg_a.start.1, seg_a.end.0, seg_a.end.1,
                    seg_b.start.0, seg_b.start.1, seg_b.end.0, seg_b.end.1,
                    edge_dist, MIN_CLEARANCE
                );
            }
        }
    }

    /// Minimum distance between two line segments (center-to-center).
    fn min_segment_distance(a: &TraceSegment, b: &TraceSegment) -> f64 {
        // Sample points along each segment and find minimum distance
        let steps = 20;
        let mut min_dist = f64::MAX;
        for i in 0..=steps {
            let t = i as f64 / steps as f64;
            let ax = a.start.0 + t * (a.end.0 - a.start.0);
            let ay = a.start.1 + t * (a.end.1 - a.start.1);
            for j in 0..=steps {
                let s = j as f64 / steps as f64;
                let bx = b.start.0 + s * (b.end.0 - b.start.0);
                let by = b.start.1 + s * (b.end.1 - b.start.1);
                let dist = ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt();
                if dist < min_dist {
                    min_dist = dist;
                }
            }
        }
        min_dist
    }

    /// Verify that no routed trace segment passes through a pad of another net.
    #[test]
    fn test_no_trace_through_foreign_pad() {
        let board = make_simple_board();
        let mut router = Router::new(board.width, board.height, GRID_SIZE);
        let routed = router.route_all(&board);

        // Build map of pad locations per net
        let mut pad_locations: Vec<(String, f64, f64, f64, f64, u8)> = Vec::new(); // (net, cx, cy, hw, hh, layer)
        let mut pin_to_net: HashMap<(String, String), String> = HashMap::new();
        for net in &board.nets {
            for pin_ref in &net.pins {
                pin_to_net.insert(
                    (pin_ref.component.clone(), pin_ref.pin.clone()),
                    net.name.clone(),
                );
            }
        }
        for comp in &board.components {
            if let Some(fp) = &comp.footprint_data {
                for pad in fp.signal_pads() {
                    let (rx, ry) = rotate_point(pad.at_x, pad.at_y, comp.rotation);
                    let cx = comp.x + rx;
                    let cy = comp.y + ry;
                    let net = comp
                        .pins
                        .iter()
                        .find(|p| p.number == pad.number)
                        .and_then(|p| pin_to_net.get(&(comp.name.clone(), p.name.clone())))
                        .cloned()
                        .unwrap_or_default();
                    let layer: u8 = if pad.pad_type == "thru_hole" {
                        2
                    } else if pad.layers.iter().any(|l| l.contains("B.Cu")) {
                        1
                    } else {
                        0
                    };
                    pad_locations.push((net, cx, cy, pad.size_w / 2.0, pad.size_h / 2.0, layer));
                }
            }
        }

        // Check each trace segment against foreign pads
        for routed_net in &routed {
            for seg in &routed_net.segments {
                for (pad_net, pcx, pcy, phw, phh, pad_layer) in &pad_locations {
                    if pad_net == &routed_net.name {
                        continue; // Same net, OK
                    }
                    if *pad_layer != 2 && *pad_layer != seg.layer {
                        continue; // Different layer
                    }

                    // Check if segment passes through pad rectangle
                    let steps = 20;
                    for i in 0..=steps {
                        let t = i as f64 / steps as f64;
                        let sx = seg.start.0 + t * (seg.end.0 - seg.start.0);
                        let sy = seg.start.1 + t * (seg.end.1 - seg.start.1);

                        let in_pad = sx >= pcx - phw
                            && sx <= pcx + phw
                            && sy >= pcy - phh
                            && sy <= pcy + phh;
                        assert!(
                            !in_pad,
                            "Trace '{}' passes through pad of net '{}' at ({:.2}, {:.2})",
                            routed_net.name, pad_net, sx, sy
                        );
                    }
                }
            }
        }
    }
}
