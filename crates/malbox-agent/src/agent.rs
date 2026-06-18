use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use malbox_proto::pb::{Ack, AnalysisCompleted, AnalysisRequest, Event, FileChunk, Hello, event};
use tokio::{fs::File, io::AsyncWriteExt, process::Command, sync::mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

use crate::{
    analysis::{AnalysisConfig, AnalysisResult, run_analysis},
    process::Process,
};

pub struct AgentService;
#[tonic::async_trait]
impl malbox_proto::pb::agent_server::Agent for AgentService {
    type AnalyzeStream = ReceiverStream<Result<Event, Status>>;
    async fn hello_svc(&self, request: Request<Hello>) -> Result<Response<Hello>, Status> {
        println!("hi {}, I'm agent", request.into_inner().msg);
        Ok(Response::new(Hello {
            msg: "agent".to_owned(),
        }))
    }
    async fn upload_sample(
        &self,
        request: Request<Streaming<FileChunk>>,
    ) -> Result<Response<Ack>, Status> {
        let mut stream = request.into_inner();
        let first_chunk = match stream
            .message()
            .await
            .map_err(|e| Status::internal(format!("Failed to read stream: {}", e)))?
        {
            Some(chunk) => chunk,
            None => return Err(Status::invalid_argument("Empty file stream")),
        };
        let filename = first_chunk.filename.trim();
        if filename.is_empty() {
            return Err(Status::invalid_argument("Filename cannot be empty"));
        }
        let upload_dir = PathBuf::from("C:\\Users\\mohamed\\Desktop\\sample");
        tokio::fs::create_dir_all(&upload_dir)
            .await
            .map_err(|e| Status::internal(format!("Failed to create upload dir: {}", e)))?;
        let file_path = upload_dir.join(filename);
        if !file_path.starts_with(&upload_dir) {
            return Err(Status::invalid_argument("Invalid filename"));
        }
        let mut file = File::create(&file_path)
            .await
            .map_err(|e| Status::internal(format!("Failed to write chunk: {}", e)))?;

        file.write_all(&first_chunk.data)
            .await
            .map_err(|e| Status::internal(format!("Failed to write chunk: {}", e)))?;

        let mut total_bytes = first_chunk.data.len() as u64;
        while let Some(chunk) = stream
            .message()
            .await
            .map_err(|e| Status::internal(format!("Stream error: {}", e)))?
        {
            file.write_all(&chunk.data)
                .await
                .map_err(|e| Status::internal(format!("Failed to write chunk: {}", e)))?;
            total_bytes += chunk.data.len() as u64;
        }
        file.flush()
            .await
            .map_err(|e| Status::internal(format!("Failed to flush file: {}", e)))?;
        drop(file);
        println!("Received file: {} ({} bytes)", filename, total_bytes);
        Ok(Response::new(Ack {
            success: true,
            error_message: "".to_owned(),
        }))
    }
    async fn analyze(
        &self,
        request: Request<AnalysisRequest>,
    ) -> Result<Response<Self::AnalyzeStream>, Status> {
        let req = request.into_inner();
        let safe_filename = PathBuf::from(&req.sample_name)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| Status::invalid_argument("Invalid sample name"))?
            .to_string();
        let work_dir = PathBuf::from("C:\\Users\\mohamed\\Desktop\\sample");
        let sample_path = PathBuf::from(format!(
            "C:\\Users\\mohamed\\Desktop\\sample\\{}",
            safe_filename
        ));
        let (tx, rx) = mpsc::channel(128);
        tokio::spawn(async move {
            let analysis_config = AnalysisConfig {
                sample_path,
                timeout_secs: 300,
                work_dir,
            };
            let analysis_result = run_analysis(analysis_config).await;
            let result = match analysis_result {
                Ok(a) => a,
                Err(_) => AnalysisResult {
                    exit_code: 1,
                    features_json: "".to_owned(),
                    runtime_ms: 0,
                },
            };
            let _ = tx
                .send(Ok(Event {
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                    kind: Some(event::Kind::Done(AnalysisCompleted {
                        exit_code: result.exit_code,
                        features_json: result.features_json,
                        runtime_ms: result.runtime_ms,
                    })),
                }))
                .await;
        });
        println!("finished execution");
        let stream = ReceiverStream::new(rx);
        Ok(Response::new(stream))
    }
}
