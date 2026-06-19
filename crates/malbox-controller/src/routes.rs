use std::convert::Infallible;

use axum::{
    Json,
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
};
use futures::{Stream, StreamExt};
use serde::Serialize;
use tokio::fs;
use tokio_stream::wrappers::BroadcastStream;

use crate::{AppState, job::JobEvent};

#[derive(Serialize)]
pub struct SubmitResponse {
    pub job_id: String,
    pub status: String,
}

pub async fn submit_job(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<SubmitResponse>), (StatusCode, String)> {
    let sample_dir = state.config.paths.report_dir.join("samples");
    fs::create_dir_all(&sample_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    {
        let raw_filename = field
            .file_name()
            .unwrap_or("unknown")
            .to_string();

        let safe_filename = std::path::Path::new(&raw_filename)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let data = field
            .bytes()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

        let disk_name = format!("{}_{}", uuid::Uuid::new_v4(), safe_filename);
        let sample_path = sample_dir.join(&disk_name);

        fs::write(&sample_path, &data)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let job_id = state.tracker.submit(
            sample_path,
            safe_filename,
            300,
            state.vm_mgr.clone(),
            state.config.clone(),
        );

        return Ok((
            StatusCode::CREATED,
            Json(SubmitResponse {
                job_id,
                status: "queued".into(),
            }),
        ));
    }

    Err((StatusCode::BAD_REQUEST, "no file uploaded".into()))
}

pub async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<crate::job::JobInfo>, StatusCode> {
    state
        .tracker
        .get(&id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

pub async fn job_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    let rx = state
        .tracker
        .subscribe(&id)
        .ok_or(StatusCode::NOT_FOUND)?;

    let stream = BroadcastStream::new(rx).filter_map(|result| async {
        match result {
            Ok(job_event) => {
                let data = serde_json::to_string(&job_event).ok()?;
                let event_name = match &job_event {
                    JobEvent::StatusChanged(_) => "status",
                    JobEvent::Error(_) => "error",
                    JobEvent::Completed { .. } => "completed",
                };
                Some(Ok(Event::default().event(event_name).data(data)))
            }
            Err(_) => None,
        }
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

pub async fn cancel_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> StatusCode {
    if state.tracker.cancel(&id) {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}
