use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use crate::pcb::PlacementScore;
use crate::router::RoutedNet;
use crate::schema::Board;

use super::api;

/// A single placement variant with its routed traces and score.
#[derive(Clone)]
pub struct PlacementVariant {
    pub board: Board,
    pub routed_nets: Vec<RoutedNet>,
    pub score: PlacementScore,
}

/// Shared application state accessible from all handlers.
pub struct AppState {
    /// The original (unplaced) board template.
    pub board: Board,
    /// Top 3 placement variants after build (best first).
    pub variants: Vec<PlacementVariant>,
    /// Currently selected variant index (0-2).
    pub selected_variant: usize,
    pub input_path: PathBuf,
    pub output_dir: PathBuf,
    pub use_topola: bool,
    pub build_status: BuildStatus,
    pub build_log: Vec<String>,
}

#[derive(Clone, serde::Serialize)]
pub struct BuildStatus {
    pub running: bool,
    pub step: String,
    pub progress: u8, // 0-100
    pub error: Option<String>,
    pub completed: bool,
}

impl Default for BuildStatus {
    fn default() -> Self {
        Self {
            running: false,
            step: String::new(),
            progress: 0,
            error: None,
            completed: false,
        }
    }
}

pub type SharedState = Arc<RwLock<AppState>>;

pub async fn start_server(
    board: Board,
    input_path: PathBuf,
    output_dir: PathBuf,
    use_topola: bool,
    port: u16,
) -> Result<()> {
    let state: SharedState = Arc::new(RwLock::new(AppState {
        board,
        variants: Vec::new(),
        selected_variant: 0,
        input_path,
        output_dir,
        use_topola,
        build_status: BuildStatus::default(),
        build_log: Vec::new(),
    }));

    let app = axum::Router::new()
        .route("/", axum::routing::get(api::index))
        .route("/api/board", axum::routing::get(api::get_board))
        .route(
            "/api/component/{ref_des}",
            axum::routing::put(api::update_component),
        )
        .route("/api/board/size", axum::routing::put(api::update_board_size))
        .route("/api/build", axum::routing::post(api::trigger_build))
        .route("/api/build/status", axum::routing::get(api::build_status))
        .route(
            "/api/variant/{index}",
            axum::routing::get(api::get_variant),
        )
        .route(
            "/api/variant/{index}/select",
            axum::routing::post(api::select_variant),
        )
        .route("/api/export/toml", axum::routing::get(api::export_toml))
        .route("/api/export/zip", axum::routing::get(api::export_zip))
        .route("/api/viewer", axum::routing::get(api::get_viewer_svg))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    let url = format!("http://127.0.0.1:{}", port);
    println!("pcb-forge UI starting on {}", url);

    // Open browser
    let _ = open::that(&url);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("Server listening on {}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}
