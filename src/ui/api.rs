use std::io::Write;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::router::RoutedNet;
use crate::{bom, gerber, pcb, router, schematic, topola_router, viewer};

use super::frontend;
use super::server::{BuildStatus, SharedState};

// GET / — serve HTML frontend
pub async fn index() -> Html<&'static str> {
    Html(frontend::HTML)
}

// GET /api/board — return board state as JSON
pub async fn get_board(State(state): State<SharedState>) -> impl IntoResponse {
    let st = state.read().await;

    #[derive(Serialize)]
    struct BoardResponse {
        board: BoardJson,
        routed_nets: Option<Vec<RoutedNet>>,
        has_build: bool,
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
        manually_placed: bool,
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

    let board_json = BoardJson {
        width: st.board.width,
        height: st.board.height,
        layers: st.board.layers,
        trace_width: st.board.trace_width,
        clearance: st.board.clearance,
        components: st
            .board
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
                manually_placed: c.manually_placed,
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
        nets: st
            .board
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

    let resp = BoardResponse {
        has_build: st.routed_nets.is_some(),
        routed_nets: st.routed_nets.clone(),
        board: board_json,
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
        // Invalidate routes since positions changed
        st.routed_nets = None;
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
    // Invalidate routes
    st.routed_nets = None;
    Json(serde_json::json!({"ok": true}))
}

// POST /api/build — run full build pipeline, return SSE stream
pub async fn trigger_build(State(state): State<SharedState>) -> Response {
    // Check if already building
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

    // Set building state
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

    // Run build in background, streaming SSE events
    let state_clone = state.clone();
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(32);

    tokio::spawn(async move {
        run_build(state_clone, tx).await;
    });

    // Convert mpsc receiver to SSE stream
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
        let msg = format!(
            "data: {}\n\n",
            serde_json::json!({"step": step, "progress": progress})
        );
        msg
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

    // Clone what we need
    let (mut board, output_dir, use_topola, _input_path) = {
        let st = state.read().await;
        (
            st.board.clone(),
            st.output_dir.clone(),
            st.use_topola,
            st.input_path.clone(),
        )
    };

    // Step 1: create output dir
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

    // Step 2: Generate schematic
    update_status!("Generating schematic...", 10);
    let sch_path = output_dir.join("pcb-forge.kicad_sch");
    if let Err(e) = schematic::generate_schematic(&board, &sch_path) {
        finish_with_error(&state, &tx, &format!("Schematic: {}", e)).await;
        return;
    }

    // Step 3: Generate PCB layout
    update_status!("Generating PCB layout...", 25);
    let pcb_path = output_dir.join("pcb-forge.kicad_pcb");
    if let Err(e) = pcb::generate_pcb(&mut board, &pcb_path) {
        finish_with_error(&state, &tx, &format!("PCB layout: {}", e)).await;
        return;
    }

    // Update board in state (placement may have been adjusted)
    {
        let mut st = state.write().await;
        st.board = board.clone();
    }

    // Step 4: Route traces
    update_status!("Routing traces...", 40);
    let routed_nets = if use_topola {
        match topola_router::route_with_topola(&board) {
            Ok(nets) => nets,
            Err(e) => {
                finish_with_error(&state, &tx, &format!("Routing: {}", e)).await;
                return;
            }
        }
    } else {
        let mut r = router::Router::new(board.width, board.height, 0.1);
        r.route_all(&board)
    };

    // Append routed traces
    update_status!("Writing routed traces...", 55);
    if let Err(e) = pcb::append_routed_traces(&pcb_path, &board, &routed_nets) {
        finish_with_error(&state, &tx, &format!("Trace append: {}", e)).await;
        return;
    }

    // Step 5: Generate Gerbers
    update_status!("Generating Gerber files...", 65);
    let gerber_dir = output_dir.join("gerbers");
    if let Err(e) = gerber::generate_gerbers(&board, &routed_nets, &gerber_dir) {
        finish_with_error(&state, &tx, &format!("Gerbers: {}", e)).await;
        return;
    }

    // Step 6: Generate BOM
    update_status!("Generating BOM...", 75);
    if let Err(e) = bom::generate_bom(&board, &output_dir) {
        finish_with_error(&state, &tx, &format!("BOM: {}", e)).await;
        return;
    }

    // Step 7: Create JLCPCB ZIP
    update_status!("Creating JLCPCB ZIP...", 82);
    let zip_path = output_dir.join("jlcpcb.zip");
    if let Err(e) = create_jlcpcb_zip(&gerber_dir, &output_dir, &zip_path) {
        finish_with_error(&state, &tx, &format!("ZIP: {}", e)).await;
        return;
    }

    // Step 8: Generate viewer
    update_status!("Generating viewer...", 90);
    let viewer_path = output_dir.join("viewer.html");
    if let Err(e) = viewer::generate_viewer(&board, &routed_nets, &viewer_path) {
        finish_with_error(&state, &tx, &format!("Viewer: {}", e)).await;
        return;
    }

    // Step 9: Generate PNG
    update_status!("Generating PNG preview...", 95);
    let png_path = output_dir.join("pcb-preview.png");
    if let Err(e) = viewer::generate_png(&board, &routed_nets, &png_path) {
        finish_with_error(&state, &tx, &format!("PNG: {}", e)).await;
        return;
    }

    // Done!
    {
        let mut st = state.write().await;
        st.routed_nets = Some(routed_nets);
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

    // Read original TOML and update positions
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

    // Parse original to get structure
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

    // Update component positions
    if let Some(components) = doc.get_mut("components").and_then(|c| c.as_table_mut()) {
        for comp in &st.board.components {
            if let Some(comp_table) = components.get_mut(&comp.name).and_then(|c| c.as_table_mut())
            {
                comp_table.insert("x".to_string(), toml::Value::Float(comp.x));
                comp_table.insert("y".to_string(), toml::Value::Float(comp.y));
                if comp.rotation.abs() > 0.01 {
                    comp_table
                        .insert("rotation".to_string(), toml::Value::Float(comp.rotation));
                }
            }
        }
    }

    // Update board size
    if let Some(board) = doc.get_mut("board").and_then(|b| b.as_table_mut()) {
        board.insert("width".to_string(), toml::Value::Float(st.board.width));
        board.insert("height".to_string(), toml::Value::Float(st.board.height));
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

// GET /api/export/zip — download jlcpcb.zip
pub async fn export_zip(State(state): State<SharedState>) -> impl IntoResponse {
    let st = state.read().await;
    let zip_path = st.output_dir.join("jlcpcb.zip");

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

// GET /api/viewer — return SVG of current PCB
pub async fn get_viewer_svg(State(state): State<SharedState>) -> impl IntoResponse {
    let st = state.read().await;
    let empty = vec![];
    let routed = st.routed_nets.as_deref().unwrap_or(&empty);
    let svg = viewer::render_standalone_svg(&st.board, routed);

    (StatusCode::OK, [(header::CONTENT_TYPE, "image/svg+xml")], svg)
}
