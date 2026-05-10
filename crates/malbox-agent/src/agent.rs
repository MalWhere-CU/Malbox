use std::path::PathBuf;

use malbox_proto::pb::{Ack, FileChunk, Hello};
use tokio::{fs::File, io::AsyncWriteExt};
use tonic::{Request, Response, Status, Streaming};

pub struct AgentService;

#[tonic::async_trait]
impl malbox_proto::pb::agent_server::Agent for AgentService {
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
}
