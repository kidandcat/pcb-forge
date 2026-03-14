use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use crate::schema::Board;

// Grid resolution: 0.25mm (compromise between precision and A* performance)
const GRID_SIZE: f64 = 0.25;
const CLEARANCE_MM: f64 = 0.2;
const TRACE_WIDTH_SIGNAL: f64 = 0.25;
const TRACE_WIDTH_POWER: f64 = 0.5;
const VIA_DRILL: f64 = 0.3;
const VIA_SIZE: f64 = 0.6;

// Cost constants (inspired by FreeRouting maze router)
const COST_STRAIGHT: i32 = 10;
const COST_DIRECTION_PENALTY: i32 = 15;
const COST_VIA: i32 = 200;
const COST_BEND: i32 = 5;

const MAX_REROUTE_ATTEMPTS: usize = 3;
const MAX_ASTAR_ITERATIONS: usize = 500_000;

/// Routing grid cell
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GridPoint {
    pub x: i32,
    pub y: i32,
    pub layer: u8, // 0 = F.Cu, 1 = B.Cu
}

/// A routed trace segment
#[derive(Debug, Clone)]
pub struct TraceSegment {
    pub start: (f64, f64),
    pub end: (f64, f64),
    pub layer: u8,
    pub width: f64,
}

/// A via between layers
#[derive(Debug, Clone)]
pub struct Via {
    pub x: f64,
    pub y: f64,
    pub drill: f64,
    pub size: f64,
}

/// Result of routing a single net
#[derive(Debug, Clone)]
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
    /// Cells blocked by routed traces (with clearance)
    trace_obstacles: HashSet<GridPoint>,
    /// Map from net name to set of grid points that are pad locations for that net
    net_pads: HashMap<String, HashSet<GridPoint>>,
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

impl Router {
    pub fn new(board_width: f64, board_height: f64, _grid_size: f64) -> Self {
        let grid_width = (board_width / GRID_SIZE).ceil() as i32 + 1;
        let grid_height = (board_height / GRID_SIZE).ceil() as i32 + 1;

        Router {
            grid_width,
            grid_height,
            pad_obstacles: HashSet::new(),
            trace_obstacles: HashSet::new(),
            net_pads: HashMap::new(),
        }
    }

    /// Mark a rectangular area as pad obstacle on the grid (with clearance expansion)
    fn mark_pad_obstacle(
        &mut self,
        cx: f64,
        cy: f64,
        half_w: f64,
        half_h: f64,
        layer: u8,
        net_name: &str,
    ) {
        let cl = CLEARANCE_MM;
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

    /// Mark trace cells as occupied with clearance expansion
    fn mark_trace_cells(&mut self, path: &[GridPoint], trace_width: f64) {
        let cl = clearance_cells(trace_width);
        for &pt in path {
            for dx in -cl..=cl {
                for dy in -cl..=cl {
                    self.trace_obstacles.insert(GridPoint {
                        x: pt.x + dx,
                        y: pt.y + dy,
                        layer: pt.layer,
                    });
                }
            }
        }
    }

    /// Remove trace cells for a previously routed path (for rip-up)
    fn unmark_trace_cells(&mut self, path: &[GridPoint], trace_width: f64) {
        let cl = clearance_cells(trace_width);
        for &pt in path {
            for dx in -cl..=cl {
                for dy in -cl..=cl {
                    self.trace_obstacles.remove(&GridPoint {
                        x: pt.x + dx,
                        y: pt.y + dy,
                        layer: pt.layer,
                    });
                }
            }
        }
    }

    /// Check if a grid point is valid for routing a specific net
    fn is_valid_for_net(&self, point: GridPoint, net_name: &str) -> bool {
        if point.x < 0 || point.x >= self.grid_width || point.y < 0 || point.y >= self.grid_height
        {
            return false;
        }

        if self.trace_obstacles.contains(&point) {
            return false;
        }

        if self.pad_obstacles.contains(&point) {
            if let Some(net_set) = self.net_pads.get(net_name) {
                if net_set.contains(&point) {
                    return true;
                }
            }
            return false;
        }

        true
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

        // Step 4: Sort nets — critical first (TVS→connector, decoupling→IC),
        // then signal nets, then power nets
        net_infos.sort_by(|a, b| {
            let a_critical = is_critical_net(&a.name, board);
            let b_critical = is_critical_net(&b.name, board);
            let a_power = is_power_net(&a.name);
            let b_power = is_power_net(&b.name);
            b_critical
                .cmp(&a_critical) // critical nets first (true > false)
                .then_with(|| a_power.cmp(&b_power)) // power nets last
                .then_with(|| a.pin_positions.len().cmp(&b.pin_positions.len()))
        });

        // Step 5: Route each net
        let mut routed: Vec<RoutedNet> = Vec::new();
        let mut routed_paths: Vec<(String, Vec<Vec<GridPoint>>, f64)> = Vec::new();
        let mut unrouted: Vec<String> = Vec::new();

        for net_info in &net_infos {
            let result = self.route_net(
                &net_info.name,
                &net_info.pin_positions,
                net_info.trace_width,
            );

            match result {
                Some((routed_net, paths)) => {
                    routed_paths.push((
                        net_info.name.clone(),
                        paths,
                        net_info.trace_width,
                    ));
                    routed.push(routed_net);
                }
                None => {
                    unrouted.push(net_info.name.clone());
                }
            }
        }

        // Step 6: Rip-up and reroute for failed nets
        for attempt in 0..MAX_REROUTE_ATTEMPTS {
            if unrouted.is_empty() {
                break;
            }

            eprintln!(
                "  Rip-up attempt {}/{}: {} unrouted nets",
                attempt + 1,
                MAX_REROUTE_ATTEMPTS,
                unrouted.len()
            );

            let failed = std::mem::take(&mut unrouted);

            for net_name in &failed {
                let net_info = net_infos.iter().find(|n| &n.name == net_name).unwrap();
                let mut succeeded = false;

                // Try without rip-up first (maybe space freed up)
                if let Some((routed_net, paths)) = self.route_net(
                    &net_info.name,
                    &net_info.pin_positions,
                    net_info.trace_width,
                ) {
                    routed_paths.push((net_info.name.clone(), paths, net_info.trace_width));
                    routed.push(routed_net);
                    succeeded = true;
                }

                if !succeeded {
                    // Rip up each previously routed net, try routing this one,
                    // then re-route the ripped net
                    let mut rip_candidates: Vec<usize> = (0..routed_paths.len()).collect();
                    rip_candidates.sort_by(|&a, &b| {
                        let len_a: usize = routed_paths[a].1.iter().map(|p| p.len()).sum();
                        let len_b: usize = routed_paths[b].1.iter().map(|p| p.len()).sum();
                        len_b.cmp(&len_a)
                    });

                    for &rip_idx in &rip_candidates {
                        let rip_name = routed_paths[rip_idx].0.clone();
                        let rip_width = routed_paths[rip_idx].2;

                        // Unmark ripped net's traces
                        for path in &routed_paths[rip_idx].1 {
                            self.unmark_trace_cells(path, rip_width);
                        }

                        // Try routing our net
                        if let Some((new_routed, new_paths)) = self.route_net(
                            &net_info.name,
                            &net_info.pin_positions,
                            net_info.trace_width,
                        ) {
                            // Try re-routing the ripped net
                            let rip_info =
                                net_infos.iter().find(|n| n.name == rip_name).unwrap();
                            if let Some((rip_routed, rip_new_paths)) = self.route_net(
                                &rip_info.name,
                                &rip_info.pin_positions,
                                rip_info.trace_width,
                            ) {
                                // Both succeeded
                                routed_paths[rip_idx] =
                                    (rip_name.clone(), rip_new_paths, rip_width);
                                if let Some(r) =
                                    routed.iter_mut().find(|r| r.name == rip_name)
                                {
                                    *r = rip_routed;
                                }
                                routed_paths.push((
                                    net_info.name.clone(),
                                    new_paths,
                                    net_info.trace_width,
                                ));
                                routed.push(new_routed);
                                succeeded = true;
                                break;
                            } else {
                                // Undo - remove our traces, re-mark ripped
                                for path in &new_paths {
                                    self.unmark_trace_cells(path, net_info.trace_width);
                                }
                                for path in &routed_paths[rip_idx].1 {
                                    self.mark_trace_cells(path, rip_width);
                                }
                            }
                        } else {
                            // Re-mark ripped net
                            for path in &routed_paths[rip_idx].1 {
                                self.mark_trace_cells(path, rip_width);
                            }
                        }
                    }
                }

                if !succeeded {
                    unrouted.push(net_name.clone());
                }
            }
        }

        // Step 7: Report unrouted nets
        for net_name in &unrouted {
            eprintln!("WARNING: Net '{}' could not be routed!", net_name);
        }

        // Push empty RoutedNet for unrouted (NO partial traces)
        for net_name in &unrouted {
            routed.push(RoutedNet {
                name: net_name.clone(),
                segments: Vec::new(),
                vias: Vec::new(),
            });
        }

        routed
    }

    /// Route a single net using MST ordering. Returns None if any connection fails.
    fn route_net(
        &mut self,
        net_name: &str,
        pin_positions: &[(f64, f64)],
        trace_width: f64,
    ) -> Option<(RoutedNet, Vec<Vec<GridPoint>>)> {
        let mut segments = Vec::new();
        let mut vias = Vec::new();
        let mut all_paths: Vec<Vec<GridPoint>> = Vec::new();

        // Prim's MST ordering
        let mut connected: Vec<(f64, f64)> = vec![pin_positions[0]];
        let mut remaining: Vec<(f64, f64)> = pin_positions[1..].to_vec();

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

                    self.mark_trace_cells(&path, trace_width);
                    all_paths.push(path);
                    connected.push(target);
                }
                None => {
                    // Failed - undo ALL traces for this net (no partial traces)
                    for path in &all_paths {
                        self.unmark_trace_cells(path, trace_width);
                    }
                    return None;
                }
            }
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
                let tentative_g = current_g + COST_VIA;
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
