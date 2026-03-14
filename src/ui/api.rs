use std::io::Write;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::pcb::PlacementScore;
use crate::router::RoutedNet;
use crate::{bom, gerber, pcb, router, schematic, topola_router, viewer};

use super::frontend;
use super::server::{BuildStatus, PlacementVariant, SharedState};

// GET / — serve HTML frontend
pub async fn index() -> Html<&'static str> {
    Html(frontend::HTML)
}

// GET /api/board — return board state as JSON (uses selected variant if available)
pub async fn get_board(State(state): State<SharedState>) -> impl IntoResponse {
    let st = state.read().await;

    #[derive(Serialize)]
    struct BoardResponse {
        board: BoardJson,
        routed_nets: Option<Vec<RoutedNet>>,
        has_build: bool,
        variants: Vec<VariantSummary>,
        selected_variant: usize,
    }

    #[derive(Serialize)]
    struct VariantSummary {
        index: usize,
        score: PlacementScore,
    }

    #[derive(Serialize)]
    struct BoardJson {
        width: f64,
        height: f64,
        layers: u32,
        trace_width: f64,
        clearance: f64,
        components: Vec<ComponentJson>,
        nets: Vec<NetJson>,
    }

    #[derive(Serialize)]
    struct ComponentJson {
        ref_des: String,
        name: String,
        footprint: String,
        value: String,
        x: f64,
        y: f64,
        rotation: f64,
        pins: Vec<PinJson>,
        footprint_data: Option<FootprintDataJson>,
    }

    #[derive(Serialize)]
    struct PinJson {
        name: String,
        number: String,
        x: f64,
        y: f64,
    }

    #[derive(Serialize)]
    struct FootprintDataJson {
        name: String,
        pads: Vec<PadJson>,
        lines: Vec<LineJson>,
    }

    #[derive(Serialize)]
    struct PadJson {
        number: String,
        pad_type: String,
        shape: String,
        at_x: f64,
        at_y: f64,
        size_w: f64,
        size_h: f64,
        layers: Vec<String>,
        drill: Option<f64>,
    }

    #[derive(Serialize)]
    struct LineJson {
        start: (f64, f64),
        end: (f64, f64),
        layer: String,
        width: f64,
    }

    #[derive(Serialize)]
    struct NetJson {
        name: String,
        pins: Vec<PinRefJson>,
    }

    #[derive(Serialize)]
    struct PinRefJson {
        component: String,
        pin: String,
    }

    // Use selected variant's board if available, otherwise the template board
    let (active_board, routed_nets) = if !st.variants.is_empty() {
        let idx = st.selected_variant.min(st.variants.len() - 1);
        let v = &st.variants[idx];
        (&v.board, Some(v.routed_nets.clone()))
    } else {
        (&st.board, None)
    };

    let board_json = BoardJson {
        width: active_board.width,
        height: active_board.height,
        layers: active_board.layers,
        trace_width: active_board.trace_width,
        clearance: active_board.clearance,
        components: active_board
            .components
            .iter()
            .map(|c| ComponentJson {
                ref_des: c.ref_des.clone(),
                name: c.name.clone(),
                footprint: c.footprint.clone(),
                value: c.value.clone(),
                x: c.x,
                y: c.y,
                rotation: c.rotation,
                pins: c
                    .pins
                    .iter()
                    .map(|p| PinJson {
                        name: p.name.clone(),
                        number: p.number.clone(),
                        x: p.x,
                        y: p.y,
                    })
                    .collect(),
                footprint_data: c.footprint_data.as_ref().map(|fp| FootprintDataJson {
                    name: fp.name.clone(),
                    pads: fp
                        .pads
                        .iter()
                        .map(|pad| PadJson {
                            number: pad.number.clone(),
                            pad_type: pad.pad_type.clone(),
                            shape: pad.shape.clone(),
                            at_x: pad.at_x,
                            at_y: pad.at_y,
                            size_w: pad.size_w,
                            size_h: pad.size_h,
                            layers: pad.layers.clone(),
                            drill: pad.drill,
                        })
                        .collect(),
                    lines: fp
                        .lines
                        .iter()
                        .map(|l| LineJson {
                            start: l.start,
                            end: l.end,
                            layer: l.layer.clone(),
                            width: l.width,
                        })
                        .collect(),
                }),
            })
            .collect(),
        nets: active_board
            .nets
            .iter()
            .map(|n| NetJson {
                name: n.name.clone(),
                pins: n
                    .pins
                    .iter()
                    .map(|p| PinRefJson {
                        component: p.component.clone(),
                        pin: p.pin.clone(),
                    })
                    .collect(),
            })
            .collect(),
    };

    let variant_summaries: Vec<VariantSummary> = st
        .variants
        .iter()
        .enumerate()
        .map(|(i, v)| VariantSummary {
            index: i,
            score: v.score.clone(),
        })
        .collect();

    let resp = BoardResponse {
        has_build: !st.variants.is_empty(),
        routed_nets,
        board: board_json,
        variants: variant_summaries,
        selected_variant: st.selected_variant,
    };

    Json(resp)
}

// PUT /api/component/:ref_des — update component position/rotation
#[derive(Deserialize)]
pub struct UpdateComponent {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub rotation: Option<f64>,
}

pub async fn update_component(
    State(state): State<SharedState>,
    Path(ref_des): Path<String>,
    Json(update): Json<UpdateComponent>,
) -> impl IntoResponse {
    let mut st = state.write().await;

    if let Some(comp) = st
        .board
        .components
        .iter_mut()
        .find(|c| c.ref_des == ref_des)
    {
        if let Some(x) = update.x {
            comp.x = x;
        }
        if let Some(y) = update.y {
            comp.y = y;
        }
        if let Some(r) = update.rotation {
            comp.rotation = r;
        }
        // Invalidate build since positions changed
        st.variants.clear();
        (StatusCode::OK, Json(serde_json::json!({"ok": true})))
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Component {} not found", ref_des)})),
        )
    }
}

// PUT /api/board/size — update board dimensions
#[derive(Deserialize)]
pub struct UpdateBoardSize {
    pub width: Option<f64>,
    pub height: Option<f64>,
}

pub async fn update_board_size(
    State(state): State<SharedState>,
    Json(update): Json<UpdateBoardSize>,
) -> impl IntoResponse {
    let mut st = state.write().await;
    if let Some(w) = update.width {
        st.board.width = w;
    }
    if let Some(h) = update.height {
        st.board.height = h;
    }
    st.variants.clear();
    Json(serde_json::json!({"ok": true}))
}

// GET /api/variant/:index — get a specific variant's board data
pub async fn get_variant(
    State(state): State<SharedState>,
    Path(index): Path<usize>,
) -> impl IntoResponse {
    let st = state.read().await;
    if index >= st.variants.len() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Variant not found"})),
        )
            .into_response();
    }

    let v = &st.variants[index];
    Json(serde_json::json!({
        "index": index,
        "score": v.score,
        "routed_nets": v.routed_nets,
    }))
    .into_response()
}

// POST /api/variant/:index/select — select a variant as active
pub async fn select_variant(
    State(state): State<SharedState>,
    Path(index): Path<usize>,
) -> impl IntoResponse {
    let mut st = state.write().await;
    if index >= st.variants.len() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Variant not found"})),
        );
    }
    st.selected_variant = index;
    (StatusCode::OK, Json(serde_json::json!({"ok": true, "selected": index})))
}

// POST /api/build — run full build pipeline with 10 placement variants, return SSE stream
pub async fn trigger_build(State(state): State<SharedState>) -> Response {
    {
        let st = state.read().await;
        if st.build_status.running {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "Build already in progress"})),
            )
                .into_response();
        }
    }

    {
        let mut st = state.write().await;
        st.build_status = BuildStatus {
            running: true,
            step: "Starting...".into(),
            progress: 0,
            error: None,
            completed: false,
        };
        st.build_log.clear();
    }

    let state_clone = state.clone();
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(32);

    tokio::spawn(async move {
        run_build(state_clone, tx).await;
    });

    let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let stream = tokio_stream::StreamExt::map(rx_stream, Ok::<_, std::convert::Infallible>);

    Response::builder()
        .status(200)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(Body::from_stream(stream))
        .unwrap()
}

async fn run_build(state: SharedState, tx: tokio::sync::mpsc::Sender<String>) {
    let send_event = |step: &str, progress: u8| {
        format!(
            "data: {}\n\n",
            serde_json::json!({"step": step, "progress": progress})
        )
    };

    macro_rules! update_status {
        ($step:expr, $progress:expr) => {{
            let msg = send_event($step, $progress);
            let _ = tx.send(msg).await;
            let mut st = state.write().await;
            st.build_status.step = $step.to_string();
            st.build_status.progress = $progress;
            st.build_log.push($step.to_string());
        }};
    }

    let (board, output_dir, use_topola) = {
        let st = state.read().await;
        (st.board.clone(), st.output_dir.clone(), st.use_topola)
    };

    update_status!("Creating output directory...", 5);
    if let Err(e) = std::fs::create_dir_all(&output_dir) {
        let mut st = state.write().await;
        st.build_status.error = Some(format!("Failed to create output dir: {}", e));
        st.build_status.running = false;
        let _ = tx
            .send(format!(
                "data: {}\n\n",
                serde_json::json!({"error": e.to_string()})
            ))
            .await;
        return;
    }

    // Generate schematic
    update_status!("Generating schematic...", 8);
    let sch_path = output_dir.join("pcb-forge.kicad_sch");
    if let Err(e) = schematic::generate_schematic(&board, &sch_path) {
        finish_with_error(&state, &tx, &format!("Schematic: {}", e)).await;
        return;
    }

    // Generate 10 placement variants and route each
    update_status!("Generating placement variants...", 10);
    let configs = pcb::generate_placement_configs();
    let mut all_variants: Vec<PlacementVariant> = Vec::new();

    for (i, config) in configs.iter().enumerate() {
        let progress = 10 + (i as u8) * 7; // 10-80%
        update_status!(
            &format!("Placing & routing variant {}/10...", i + 1),
            progress
        );

        let placed_board = pcb::generate_placement(&board, config);
        let routed_nets = if use_topola {
            match topola_router::route_with_topola(&placed_board) {
                Ok(nets) => nets,
                Err(_) => continue,
            }
        } else {
            let mut r = router::Router::new(placed_board.width, placed_board.height, 0.1);
            r.route_all(&placed_board)
        };

        let score = pcb::PlacementScore::compute(&routed_nets, board.nets.len());
        all_variants.push(PlacementVariant {
            board: placed_board,
            routed_nets,
            score,
        });
    }

    // Sort by score and take top 3
    all_variants.sort_by(|a, b| b.score.composite.partial_cmp(&a.score.composite).unwrap());
    let top3: Vec<PlacementVariant> = all_variants.into_iter().take(3).collect();

    // Generate output files for top 3
    update_status!("Generating outputs for top 3...", 82);
    for (rank, variant) in top3.iter().enumerate() {
        let variant_dir = output_dir.join(format!("placement-{}", rank + 1));
        let _ = std::fs::create_dir_all(&variant_dir);

        let pcb_path = variant_dir.join("pcb-forge.kicad_pcb");
        let _ = pcb::write_pcb_file(&variant.board, &pcb_path);
        let _ = pcb::append_routed_traces(&pcb_path, &variant.board, &variant.routed_nets);

        let gerber_dir = variant_dir.join("gerbers");
        let _ = gerber::generate_gerbers(&variant.board, &variant.routed_nets, &gerber_dir);
        let _ = bom::generate_bom(&variant.board, &variant_dir);

        let zip_path = variant_dir.join("jlcpcb.zip");
        let _ = create_jlcpcb_zip(&gerber_dir, &variant_dir, &zip_path);

        let viewer_path = variant_dir.join("viewer.html");
        let _ = viewer::generate_viewer(&variant.board, &variant.routed_nets, &viewer_path);

        let png_path = variant_dir.join("pcb-preview.png");
        let _ = viewer::generate_png(&variant.board, &variant.routed_nets, &png_path);
    }

    // Store variants in state
    {
        let mut st = state.write().await;
        st.variants = top3;
        st.selected_variant = 0;
        st.build_status = BuildStatus {
            running: false,
            step: "Build complete!".into(),
            progress: 100,
            error: None,
            completed: true,
        };
    }

    let _ = tx
        .send(format!(
            "data: {}\n\n",
            serde_json::json!({"step": "Build complete!", "progress": 100, "done": true})
        ))
        .await;
}

async fn finish_with_error(
    state: &SharedState,
    tx: &tokio::sync::mpsc::Sender<String>,
    error: &str,
) {
    let mut st = state.write().await;
    st.build_status.error = Some(error.to_string());
    st.build_status.running = false;
    let _ = tx
        .send(format!(
            "data: {}\n\n",
            serde_json::json!({"error": error})
        ))
        .await;
}

fn create_jlcpcb_zip(
    gerber_dir: &std::path::Path,
    output_dir: &std::path::Path,
    zip_path: &std::path::Path,
) -> anyhow::Result<()> {
    let file = std::fs::File::create(zip_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

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

// GET /api/build/status
pub async fn build_status(State(state): State<SharedState>) -> impl IntoResponse {
    let st = state.read().await;
    Json(st.build_status.clone())
}

// GET /api/export/toml — generate TOML from current board state
pub async fn export_toml(State(state): State<SharedState>) -> impl IntoResponse {
    let st = state.read().await;

    let original = match std::fs::read_to_string(&st.input_path) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "text/plain")],
                format!("Failed to read input file: {}", e),
            )
                .into_response();
        }
    };

    let mut doc: toml::Value = match toml::from_str(&original) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "text/plain")],
                format!("Failed to parse TOML: {}", e),
            )
                .into_response();
        }
    };

    // Update board size from selected variant
    let active_board = if !st.variants.is_empty() {
        let idx = st.selected_variant.min(st.variants.len() - 1);
        &st.variants[idx].board
    } else {
        &st.board
    };

    if let Some(board) = doc.get_mut("board").and_then(|b| b.as_table_mut()) {
        board.insert("width".to_string(), toml::Value::Float(active_board.width));
        board.insert(
            "height".to_string(),
            toml::Value::Float(active_board.height),
        );
    }

    let toml_str = toml::to_string_pretty(&doc).unwrap_or_else(|_| "Error serializing".into());

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/toml"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"circuit.toml\"",
            ),
        ],
        toml_str,
    )
        .into_response()
}

// GET /api/export/zip — download jlcpcb.zip for selected variant
pub async fn export_zip(State(state): State<SharedState>) -> impl IntoResponse {
    let st = state.read().await;
    let variant_idx = st.selected_variant + 1;
    let zip_path = st
        .output_dir
        .join(format!("placement-{}", variant_idx))
        .join("jlcpcb.zip");

    match std::fs::read(&zip_path) {
        Ok(data) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/zip"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"jlcpcb.zip\"",
                ),
            ],
            data,
        )
            .into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::json!({"error": "No ZIP file found. Run a build first."}),
            ),
        )
            .into_response(),
    }
}

// GET /api/viewer — return SVG of current selected variant
pub async fn get_viewer_svg(State(state): State<SharedState>) -> impl IntoResponse {
    let st = state.read().await;

    let (board_ref, routed_ref);
    let empty = vec![];

    if !st.variants.is_empty() {
        let idx = st.selected_variant.min(st.variants.len() - 1);
        board_ref = &st.variants[idx].board;
        routed_ref = &st.variants[idx].routed_nets;
    } else {
        board_ref = &st.board;
        routed_ref = &empty;
    };

    let svg = viewer::render_standalone_svg(board_ref, routed_ref);
    (StatusCode::OK, [(header::CONTENT_TYPE, "image/svg+xml")], svg)
}
