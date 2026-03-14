use anyhow::{Context, Result};
use std::path::Path;

// ── Parsed footprint data ──────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct FootprintData {
    pub name: String,
    pub pads: Vec<PadData>,
    pub lines: Vec<FpLine>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PadData {
    pub number: String,
    pub pad_type: String, // "smd", "thru_hole"
    pub shape: String,    // "rect", "roundrect", "circle", "oval"
    pub at_x: f64,
    pub at_y: f64,
    pub size_w: f64,
    pub size_h: f64,
    pub layers: Vec<String>,
    pub drill: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FpLine {
    pub start: (f64, f64),
    pub end: (f64, f64),
    pub layer: String,
    pub width: f64,
}

impl FootprintData {
    /// Bounding box of the courtyard, or fab layer fallback, or pads fallback.
    /// Returns (min_x, min_y, max_x, max_y) relative to footprint origin.
    pub fn courtyard_bounds(&self) -> (f64, f64, f64, f64) {
        let crtyd: Vec<_> = self
            .lines
            .iter()
            .filter(|l| l.layer.contains("CrtYd"))
            .collect();
        let lines = if crtyd.is_empty() {
            let fab: Vec<_> = self.lines.iter().filter(|l| l.layer.contains("Fab")).collect();
            if fab.is_empty() {
                &self.lines
            } else {
                // use a temp vec
                return bounds_from_lines(&fab);
            }
        } else {
            return bounds_from_lines(&crtyd);
        };
        if lines.is_empty() {
            // fallback: compute from pads
            return self.pad_bounds();
        }
        bounds_from_lines(&lines.iter().collect::<Vec<_>>())
    }

    /// Bounds suitable for placement: use fab layer (body outline), then pads.
    /// Avoids courtyard which may include antenna keep-out zones.
    pub fn placement_bounds(&self) -> (f64, f64, f64, f64) {
        let fab: Vec<_> = self.lines.iter().filter(|l| l.layer.contains("Fab")).collect();
        if !fab.is_empty() {
            return bounds_from_lines(&fab);
        }
        self.pad_bounds()
    }

    fn pad_bounds(&self) -> (f64, f64, f64, f64) {
        if self.pads.is_empty() {
            return (-5.0, -5.0, 5.0, 5.0);
        }
        let mut min_x = f64::MAX;
        let mut min_y = f64::MAX;
        let mut max_x = f64::MIN;
        let mut max_y = f64::MIN;
        for p in &self.pads {
            let hw = p.size_w / 2.0;
            let hh = p.size_h / 2.0;
            min_x = min_x.min(p.at_x - hw);
            min_y = min_y.min(p.at_y - hh);
            max_x = max_x.max(p.at_x + hw);
            max_y = max_y.max(p.at_y + hh);
        }
        (min_x - 0.25, min_y - 0.25, max_x + 0.25, max_y + 0.25)
    }

    /// Get unique pads (skip duplicates with same number, keep first occurrence).
    /// This filters out thermal vias and paste-only pads.
    pub fn signal_pads(&self) -> Vec<&PadData> {
        let mut seen = std::collections::HashSet::new();
        self.pads
            .iter()
            .filter(|p| {
                if p.number.is_empty() {
                    return false; // skip unnamed pads (paste helpers)
                }
                // only keep pads on copper layers
                if !p.layers.iter().any(|l| l.contains("Cu")) {
                    return false;
                }
                seen.insert(p.number.clone())
            })
            .collect()
    }
}

fn bounds_from_lines(lines: &[&FpLine]) -> (f64, f64, f64, f64) {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    for l in lines {
        min_x = min_x.min(l.start.0).min(l.end.0);
        min_y = min_y.min(l.start.1).min(l.end.1);
        max_x = max_x.max(l.start.0).max(l.end.0);
        max_y = max_y.max(l.start.1).max(l.end.1);
    }
    if min_x == f64::MAX {
        (-5.0, -5.0, 5.0, 5.0)
    } else {
        (min_x, min_y, max_x, max_y)
    }
}

// ── S-expression tokenizer + parser ────────────────────────────────

#[derive(Debug)]
enum Token {
    LParen,
    RParen,
    Atom(String),
}

fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            b')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            b'"' => {
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                let s = String::from_utf8_lossy(&bytes[start..i]).to_string();
                tokens.push(Token::Atom(s));
                if i < bytes.len() {
                    i += 1;
                }
            }
            b if b.is_ascii_whitespace() => {
                i += 1;
            }
            _ => {
                let start = i;
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && bytes[i] != b'('
                    && bytes[i] != b')'
                {
                    i += 1;
                }
                let s = String::from_utf8_lossy(&bytes[start..i]).to_string();
                tokens.push(Token::Atom(s));
            }
        }
    }
    tokens
}

#[derive(Debug, Clone)]
enum SExpr {
    Atom(String),
    List(Vec<SExpr>),
}

impl SExpr {
    fn as_atom(&self) -> Option<&str> {
        match self {
            SExpr::Atom(s) => Some(s),
            _ => None,
        }
    }

    fn as_list(&self) -> Option<&[SExpr]> {
        match self {
            SExpr::List(v) => Some(v),
            _ => None,
        }
    }

    fn tag(&self) -> Option<&str> {
        self.as_list()
            .and_then(|items| items.first())
            .and_then(|s| s.as_atom())
    }

    /// Find first child list with given tag
    fn find(&self, tag: &str) -> Option<&SExpr> {
        self.as_list().and_then(|items| {
            items
                .iter()
                .find(|item| item.tag() == Some(tag))
        })
    }

    /// Get the nth element (0-indexed) if it's an atom
    fn nth_atom(&self, n: usize) -> Option<&str> {
        self.as_list()
            .and_then(|items| items.get(n))
            .and_then(|s| s.as_atom())
    }

    fn nth_f64(&self, n: usize) -> Option<f64> {
        self.nth_atom(n).and_then(|s| s.parse::<f64>().ok())
    }
}

fn parse_sexpr(tokens: &[Token], pos: &mut usize) -> Option<SExpr> {
    if *pos >= tokens.len() {
        return None;
    }
    match &tokens[*pos] {
        Token::LParen => {
            *pos += 1;
            let mut items = Vec::new();
            while *pos < tokens.len() {
                if matches!(tokens[*pos], Token::RParen) {
                    *pos += 1;
                    break;
                }
                if let Some(expr) = parse_sexpr(tokens, pos) {
                    items.push(expr);
                }
            }
            Some(SExpr::List(items))
        }
        Token::RParen => {
            *pos += 1;
            None
        }
        Token::Atom(s) => {
            let atom = SExpr::Atom(s.clone());
            *pos += 1;
            Some(atom)
        }
    }
}

// ── .kicad_mod file loading ────────────────────────────────────────

pub fn load_footprint(path: &Path) -> Result<FootprintData> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read footprint: {}", path.display()))?;
    parse_kicad_mod(&content)
}

fn parse_kicad_mod(content: &str) -> Result<FootprintData> {
    let tokens = tokenize(content);
    let mut pos = 0;
    let root = parse_sexpr(&tokens, &mut pos)
        .context("Failed to parse S-expression")?;

    let name = root
        .nth_atom(1)
        .unwrap_or("unknown")
        .to_string();

    let items = root.as_list().unwrap_or(&[]);

    let mut pads = Vec::new();
    let mut lines = Vec::new();

    for item in items {
        match item.tag() {
            Some("pad") => {
                if let Some(pad) = extract_pad(item) {
                    pads.push(pad);
                }
            }
            Some("fp_line") => {
                if let Some(line) = extract_fp_line(item) {
                    lines.push(line);
                }
            }
            Some("fp_rect") => {
                // Convert rectangle to 4 lines
                if let Some(rect_lines) = extract_fp_rect(item) {
                    lines.extend(rect_lines);
                }
            }
            _ => {}
        }
    }

    Ok(FootprintData { name, pads, lines })
}

fn extract_pad(expr: &SExpr) -> Option<PadData> {
    let items = expr.as_list()?;
    // (pad "1" smd rect (at x y) (size w h) (layers ...) ...)
    if items.len() < 5 {
        return None;
    }

    let number = items.get(1)?.as_atom()?.to_string();
    let pad_type = items.get(2)?.as_atom()?.to_string();
    let shape = items.get(3)?.as_atom()?.to_string();

    let at_node = expr.find("at")?;
    let at_x = at_node.nth_f64(1).unwrap_or(0.0);
    let at_y = at_node.nth_f64(2).unwrap_or(0.0);

    let size_node = expr.find("size")?;
    let size_w = size_node.nth_f64(1).unwrap_or(0.0);
    let size_h = size_node.nth_f64(2).unwrap_or(0.0);

    let layers = if let Some(layers_node) = expr.find("layers") {
        layers_node
            .as_list()?
            .iter()
            .skip(1) // skip "layers" tag
            .filter_map(|s| s.as_atom().map(|a| a.to_string()))
            .collect()
    } else {
        vec![]
    };

    let drill = expr.find("drill").and_then(|d| {
        let items = d.as_list()?;
        // Handle both (drill 0.75) and (drill oval 0.6 1.2)
        for item in items.iter().skip(1) {
            if let Some(v) = item.as_atom().and_then(|s| s.parse::<f64>().ok()) {
                return Some(v);
            }
        }
        None
    });

    Some(PadData {
        number,
        pad_type,
        shape,
        at_x,
        at_y,
        size_w,
        size_h,
        layers,
        drill,
    })
}

fn extract_fp_line(expr: &SExpr) -> Option<FpLine> {
    // (fp_line (start x y) (end x y) (stroke (width w) ...) (layer "F.CrtYd") ...)
    let start_node = expr.find("start")?;
    let end_node = expr.find("end")?;

    let sx = start_node.nth_f64(1)?;
    let sy = start_node.nth_f64(2)?;
    let ex = end_node.nth_f64(1)?;
    let ey = end_node.nth_f64(2)?;

    let layer = expr
        .find("layer")
        .and_then(|l| l.nth_atom(1))
        .unwrap_or("F.Fab")
        .to_string();

    let width = expr
        .find("stroke")
        .and_then(|s| s.find("width"))
        .and_then(|w| w.nth_f64(1))
        .unwrap_or(0.1);

    Some(FpLine {
        start: (sx, sy),
        end: (ex, ey),
        layer,
        width,
    })
}

fn extract_fp_rect(expr: &SExpr) -> Option<Vec<FpLine>> {
    // (fp_rect (start x1 y1) (end x2 y2) (layer "...") ...)
    let start_node = expr.find("start")?;
    let end_node = expr.find("end")?;

    let x1 = start_node.nth_f64(1)?;
    let y1 = start_node.nth_f64(2)?;
    let x2 = end_node.nth_f64(1)?;
    let y2 = end_node.nth_f64(2)?;

    let layer = expr
        .find("layer")
        .and_then(|l| l.nth_atom(1))
        .unwrap_or("F.Fab")
        .to_string();

    let width = expr
        .find("stroke")
        .and_then(|s| s.find("width"))
        .and_then(|w| w.nth_f64(1))
        .unwrap_or(0.1);

    Some(vec![
        FpLine { start: (x1, y1), end: (x2, y1), layer: layer.clone(), width },
        FpLine { start: (x2, y1), end: (x2, y2), layer: layer.clone(), width },
        FpLine { start: (x2, y2), end: (x1, y2), layer: layer.clone(), width },
        FpLine { start: (x1, y2), end: (x1, y1), layer, width },
    ])
}

// ── Fallback footprint generation ──────────────────────────────────

/// Generate a fallback footprint when .kicad_mod file is not found.
/// Creates a simple dual-row SMD package based on pin count.
pub fn generate_fallback(name: &str, pin_count: usize) -> FootprintData {
    let mut pads = Vec::new();

    if pin_count <= 2 {
        // Simple 2-pad component
        for i in 0..pin_count {
            let x = if i == 0 { -1.0 } else { 1.0 };
            pads.push(PadData {
                number: (i + 1).to_string(),
                pad_type: "smd".to_string(),
                shape: "rect".to_string(),
                at_x: x,
                at_y: 0.0,
                size_w: 1.0,
                size_h: 0.6,
                layers: vec!["F.Cu".into(), "F.Mask".into(), "F.Paste".into()],
                drill: None,
            });
        }
    } else {
        // Dual-row package
        let half = (pin_count + 1) / 2;
        let pitch = 1.27;
        let body_w = 8.0_f64.max(pin_count as f64 * 0.5);

        for i in 0..pin_count {
            let (x, y) = if i < half {
                let y = -(half as f64 - 1.0) * pitch / 2.0 + i as f64 * pitch;
                (-body_w / 2.0, y)
            } else {
                let ri = i - half;
                let right_count = pin_count - half;
                let y = -(right_count as f64 - 1.0) * pitch / 2.0 + ri as f64 * pitch;
                (body_w / 2.0, y)
            };

            pads.push(PadData {
                number: (i + 1).to_string(),
                pad_type: "smd".to_string(),
                shape: "rect".to_string(),
                at_x: x,
                at_y: y,
                size_w: 1.5,
                size_h: 0.6,
                layers: vec!["F.Cu".into(), "F.Mask".into(), "F.Paste".into()],
                drill: None,
            });
        }
    }

    // Generate courtyard lines
    let margin = 0.5;
    let (min_x, min_y, max_x, max_y) = if pads.is_empty() {
        (-5.0, -5.0, 5.0, 5.0)
    } else {
        let mut mnx = f64::MAX;
        let mut mny = f64::MAX;
        let mut mxx = f64::MIN;
        let mut mxy = f64::MIN;
        for p in &pads {
            mnx = mnx.min(p.at_x - p.size_w / 2.0);
            mny = mny.min(p.at_y - p.size_h / 2.0);
            mxx = mxx.max(p.at_x + p.size_w / 2.0);
            mxy = mxy.max(p.at_y + p.size_h / 2.0);
        }
        (mnx - margin, mny - margin, mxx + margin, mxy + margin)
    };

    let lines = vec![
        FpLine { start: (min_x, min_y), end: (max_x, min_y), layer: "F.CrtYd".into(), width: 0.05 },
        FpLine { start: (max_x, min_y), end: (max_x, max_y), layer: "F.CrtYd".into(), width: 0.05 },
        FpLine { start: (max_x, max_y), end: (min_x, max_y), layer: "F.CrtYd".into(), width: 0.05 },
        FpLine { start: (min_x, max_y), end: (min_x, min_y), layer: "F.CrtYd".into(), width: 0.05 },
    ];

    FootprintData {
        name: name.to_string(),
        pads,
        lines,
    }
}

// ── Resolve footprint path ─────────────────────────────────────────

/// Resolve a footprint reference to a filesystem path.
/// `footprint_ref` is like "RF_Module.pretty/ESP32-S3-WROOM-1.kicad_mod"
/// `lib_path` is the KiCad footprints base directory
pub fn resolve_footprint_path(footprint_ref: &str, lib_path: &Path) -> Option<std::path::PathBuf> {
    let full = lib_path.join(footprint_ref);
    if full.exists() {
        Some(full)
    } else {
        None
    }
}
