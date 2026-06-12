use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use malbox_proto::pb::{Ack, AnalysisCompleted, AnalysisRequest, Event, FileChunk, Hello, event};
use tokio::{fs::File, io::AsyncWriteExt, process::Command, sync::mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

pub struct AgentService;

fn prepare_execution_command(sample_path: &Path) -> Command {
    let ext = sample_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "exe" | "com" | "scr" | "bat" | "cmd" => Command::new(sample_path),
        "ps1" => {
            let mut cmd = Command::new("powershell.exe");
            cmd.arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-File")
                .arg(sample_path);
            cmd
        }
        "js" | "jse" => {
            let mut cmd = Command::new("wscript.exe");
            cmd.arg(sample_path);
            cmd
        }
        "vbs" | "vbe" => {
            let mut cmd = Command::new("wscript.exe");
            cmd.arg(sample_path);
            cmd
        }
        "dll" => {
            let mut cmd = Command::new("rundll32.exe");
            cmd.arg(sample_path).arg("DllMain");
            cmd
        }
        _ => Command::new(sample_path),
    }
}

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
        let sample_path = PathBuf::from(format!(
            "C:\\Users\\mohamed\\Desktop\\sample\\{}",
            safe_filename
        ));
        let (tx, rx) = mpsc::channel(128);
        tokio::spawn(async move {
            let start = std::time::Instant::now();
            let mut cmd = prepare_execution_command(&sample_path);
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    let error_msg = if e.raw_os_error() == Some(193) {
                        format!(
                            "OS Error 193: '{}' is not a valid Win32 executable.",
                            req.sample_name
                        )
                    } else {
                        e.to_string()
                    };
                    let _ = tx.send(Err(Status::internal(error_msg))).await;
                    return;
                }
            };
            let result =
                tokio::time::timeout(Duration::from_secs(req.timeout_secs as u64), child.wait())
                    .await;
            let (exit_code, runtime_ms) = match result {
                Ok(Ok(status)) => (
                    status.code().unwrap_or(-1),
                    start.elapsed().as_millis() as u64,
                ),
                _ => (-1, start.elapsed().as_millis() as u64),
            };
            let _ = child.kill().await;
            let _ = tx
                .send(Ok(Event {
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                    kind: Some(event::Kind::Done(AnalysisCompleted {
                        exit_code,
                        runtime_ms,
                    })),
                }))
                .await;
        });
        let stream = ReceiverStream::new(rx);
        Ok(Response::new(stream))
    }
}
