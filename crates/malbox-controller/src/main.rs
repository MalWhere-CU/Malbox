use std::{path::Path, sync::Arc};

use anyhow::Context;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use config::Config;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use vm::VmManager;

use crate::job::JobTracker;

mod cert;
mod config;
mod job;
mod routes;
mod vm;
mod volatility;

#[derive(Clone)]
pub struct AppState {
    pub tracker: JobTracker,
    pub vm_mgr: Arc<VmManager>,
    pub config: Config,
}

async fn serve_frontend() -> impl axum::response::IntoResponse {
    let html = include_str!("../../../frontend.html");
    axum::response::Html(html)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let path = Path::new("config.EXAMPLE.toml");
    let config = Config::from_file(path)
        .with_context(|| format!("Failed to load config from {}", path.display()))
        .unwrap();

    let vm_mgr = Arc::new(
        VmManager::new(config.clone())
            .with_context(|| format!("Failed to connect to libvirt at {}", config.libvirt.uri))
            .unwrap(),
    );
    println!("Connected to libvirt at {}", vm_mgr.conn.get_uri().unwrap());

    let tracker = JobTracker::new();
    let state = AppState {
        tracker,
        vm_mgr,
        config: config.clone(),
    };

    let app = Router::new()
        .route("/", get(serve_frontend))
        .route("/jobs", post(routes::submit_job))
        .route(
            "/jobs/{id}",
            get(routes::get_job).delete(routes::cancel_job),
        )
        .route("/jobs/{id}/events", get(routes::job_events))
        .layer(TraceLayer::new_for_http())
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024))
        .with_state(state);

    let listener = TcpListener::bind(&config.listen_addr).await.unwrap();
    println!("Listening on {}", config.listen_addr);
    axum::serve(listener, app).await.unwrap();
}
