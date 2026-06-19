use std::{
    collections::HashMap,
    net::IpAddr,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use malbox_proto::pb::{
    AnalysisRequest, FileChunk, Hello, RequestCsrEnvelope, SignedCertEnvelope, Who,
    agent_client::AgentClient, bootstrap_client::BootstrapClient,
};
use tokio::{
    fs::File,
    io::AsyncReadExt,
    sync::broadcast::{self},
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{
    Request,
    transport::{Certificate, Channel, ClientTlsConfig, Identity},
};

use crate::{
    cert::{self, JobCerts},
    config::Config,
    vm::VmManager,
};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    SpawningVm,
    Bootstrapping,
    Uploading,
    Analyzing,
    Completed,
    Failed,
}
#[derive(Clone, serde::Serialize)]
#[serde(tag = "type", content = "data")]
pub enum JobEvent {
    #[serde(rename = "status")]
    StatusChanged(String),
    #[serde(rename = "error")]
    Error(String),
    #[serde(rename = "completed")]
    Completed { exit_code: i32, runtime_ms: u64 },
}
#[derive(serde::Serialize)]
pub struct JobInfo {
    pub id: String,
    pub status: JobStatus,
    pub sample_name: String,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<AnalysisResult>,
}
#[derive(Clone, serde::Serialize)]
pub struct AnalysisResult {
    pub exit_code: i32,
    pub runtime_ms: u64,
    pub features_json: String,
}
pub struct Job {
    pub id: String,
    pub status: JobStatus,
    pub sample_name: String,
    pub sample_path: PathBuf,
    pub timeout_secs: u32,
    pub created_at: u64,
    pub result: Option<AnalysisResult>,
    pub tx: broadcast::Sender<JobEvent>,
}

pub struct JobTracker {
    jobs: Arc<RwLock<HashMap<String, Job>>>,
}

impl JobTracker {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    pub fn submit(
        &self,
        sample_path: PathBuf,
        sample_name: String,
        timeout_secs: u32,
        vm_mgr: Arc<VmManager>,
        config: Config,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, _) = broadcast::channel(64);
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let job = Job {
            id: id.clone(),
            status: JobStatus::Queued,
            sample_name: sample_name.clone(),
            sample_path: sample_path.clone(),
            timeout_secs,
            created_at,
            result: None,
            tx: tx.clone(),
        };

        self.jobs.write().unwrap().insert(id.clone(), job);
        let job_id = id.clone();
        tokio::spawn(run_job(
            job_id,
            sample_path,
            sample_name,
            timeout_secs,
            self.clone(),
            vm_mgr,
            config,
        ));

        id
    }
    pub fn get(&self, id: &str) -> Option<JobInfo> {
        let jobs = self.jobs.read().unwrap();
        jobs.get(id).map(|j| JobInfo {
            id: j.id.clone(),
            status: j.status.clone(),
            sample_name: j.sample_name.clone(),
            created_at: j.created_at,
            result: j.result.clone(),
        })
    }
    pub fn subscribe(&self, id: &str) -> Option<broadcast::Receiver<JobEvent>> {
        let jobs = self.jobs.read().unwrap();
        jobs.get(id).map(|j| j.tx.subscribe())
    }
    pub fn cancel(&self, id: &str) -> bool {
        self.jobs.write().unwrap().remove(id).is_some()
    }
    pub fn has(&self, id: &str) -> bool {
        self.jobs.read().unwrap().contains_key(id)
    }
    fn set_status(&self, id: &str, status: JobStatus) {
        let mut jobs = self.jobs.write().unwrap();
        if let Some(job) = jobs.get_mut(id) {
            let _ = job.tx.send(JobEvent::StatusChanged(
                format!("{:?}", status).to_lowercase(),
            ));
            job.status = status;
        }
    }
    fn set_result(&self, id: &str, result: AnalysisResult) {
        let mut jobs = self.jobs.write().unwrap();
        if let Some(job) = jobs.get_mut(id) {
            let _ = job.tx.send(JobEvent::Completed {
                exit_code: result.exit_code,
                runtime_ms: result.runtime_ms,
            });
            job.result = Some(result);
            job.status = JobStatus::Completed;
        }
    }
    fn emit_error(&self, id: &str, msg: String) {
        let jobs = self.jobs.read().unwrap();
        if let Some(job) = jobs.get(id) {
            let _ = job.tx.send(JobEvent::Error(msg));
        }
    }
}

impl Clone for JobTracker {
    fn clone(&self) -> Self {
        Self {
            jobs: Arc::clone(&self.jobs),
        }
    }
}

async fn run_job(
    job_id: String,
    sample_path: PathBuf,
    sample_name: String,
    timeout_secs: u32,
    tracker: JobTracker,
    vm_mgr: Arc<VmManager>,
    config: Config,
) {
    match run_job_inner(
        &job_id,
        &sample_path,
        &sample_name,
        timeout_secs,
        &tracker,
        &vm_mgr,
        &config,
    )
    .await
    {
        Ok(result) => {
            tracker.set_result(&job_id, result);
        }
        Err(e) => {
            let msg = e.to_string();
            eprintln!("[job {}] failed: {}", job_id, msg);
            tracker.emit_error(&job_id, msg);
            tracker.set_status(&job_id, JobStatus::Failed);
        }
    }
}
async fn run_job_inner(
    job_id: &str,
    sample_path: &PathBuf,
    sample_name: &str,
    timeout_secs: u32,
    tracker: &JobTracker,
    vm_mgr: &VmManager,
    _config: &Config,
) -> anyhow::Result<AnalysisResult> {
    tracker.set_status(job_id, JobStatus::SpawningVm);
    let jd = vm_mgr.create_job_domain().context("failed to create VM")?;
    if !tracker.has(job_id) {
        jd.teardown();
        return Err(anyhow::anyhow!("job cancelled"));
    }
    tracker.set_status(job_id, JobStatus::Bootstrapping);
    let certs = bootstrap_mtls(job_id, &jd.ip_addr)
        .await
        .context("mTLS bootstrap failed")?;
    if !tracker.has(job_id) {
        jd.teardown();
        return Err(anyhow::anyhow!("job cancelled"));
    }
    tracker.set_status(job_id, JobStatus::Uploading);
    let mut client = connect_mtls(&jd.ip_addr, &certs)
        .await
        .context("mTLS connect failed")?;
    upload_sample(&mut client, sample_path, sample_name)
        .await
        .context("sample upload failed")?;
    if !tracker.has(job_id) {
        jd.teardown();
        return Err(anyhow::anyhow!("job cancelled"));
    }
    tracker.set_status(job_id, JobStatus::Analyzing);
    let result = match run_analysis(&mut client, timeout_secs, sample_name)
        .await
        .context("analysis failed")
    {
        Ok(r) => r,
        Err(e) => {
            jd.teardown();
            return Err(anyhow::anyhow!(
                "{{\"status\": \"error\", \"message\": \"{}\"}}",
                e
            ));
        }
    };
    jd.teardown();
    Ok(result)
}

async fn bootstrap_mtls(job_id: &str, ip: &str) -> anyhow::Result<JobCerts> {
    let url = format!("http://{}:50055", ip);
    let mut client = loop {
        match BootstrapClient::connect(url.clone()).await {
            Ok(c) => break c,
            Err(_) => tokio::time::sleep(Duration::from_secs(2)).await,
        }
    };
    let controller_ip = client
        .who_am_i(Request::new(Who {}))
        .await?
        .into_inner()
        .ip_addr;
    let certs = cert::generate_job_certs(job_id, controller_ip.parse::<IpAddr>()?)?;
    let csr: String = client
        .request_csr(Request::new(RequestCsrEnvelope {
            ip_addr: ip.to_owned(),
            root_ca_pem: certs.root_ca_cert.pem(),
            job_uuid: job_id.to_owned(),
        }))
        .await?
        .into_inner()
        .csr_pem;
    let agent_cert = certs.sign_agent_csr(&csr)?;
    let res = client
        .switch_connection(Request::new(SignedCertEnvelope {
            signed_cert_pem: agent_cert.pem(),
        }))
        .await?
        .into_inner();
    if !res.success {
        return Err(anyhow::anyhow!(
            "key exchange failed: {}",
            res.error_message
        ));
    }
    Ok(certs)
}

async fn connect_mtls(
    agent_ip: &str,
    controller_certs: &JobCerts,
) -> anyhow::Result<AgentClient<Channel>> {
    let client_tls = ClientTlsConfig::new()
        .identity(Identity::from_pem(
            controller_certs.controller_cert.pem(),
            controller_certs.controller_key.serialize_pem(),
        ))
        .ca_certificate(Certificate::from_pem(controller_certs.root_ca_cert.pem()));
    let agent_url = format!("https://{}:50055", agent_ip);
    println!("awaiting connection? pray it works!");
    let channel = loop {
        match tonic::transport::Channel::from_shared(agent_url.clone())?
            .tls_config(client_tls.clone())?
            .connect()
            .await
        {
            Ok(c) => break c,
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    };
    println!("It worked!!");
    let mut client = AgentClient::new(channel);
    let resp = client
        .hello_svc(Request::new(Hello {
            msg: "controller".to_owned(),
        }))
        .await?;
    println!("Agent says: {}", resp.into_inner().msg);
    Ok(client)
}

async fn upload_sample(
    client: &mut AgentClient<Channel>,
    sample_path: &PathBuf,
    sample_name: &str,
) -> anyhow::Result<()> {
    let file = File::open(sample_path).await?;
    let mut reader = tokio::io::BufReader::with_capacity(32 * 1024, file);
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let name = sample_name.to_string();
    tokio::spawn(async move {
        let mut is_first = true;
        let mut buffer = vec![0u8; 32 * 1024];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = FileChunk {
                        data: buffer[..n].to_vec(),
                        is_first,
                        filename: if is_first {
                            name.clone()
                        } else {
                            String::new()
                        },
                    };
                    if tx.send(chunk).await.is_err() {
                        break;
                    }
                    is_first = false;
                }
                Err(_) => break,
            }
        }
    });
    let stream = ReceiverStream::new(rx);
    let resp = client
        .upload_sample(Request::new(stream))
        .await?
        .into_inner();
    if resp.success {
        Ok(())
    } else {
        Err(anyhow::anyhow!("upload rejected: {}", resp.error_message))
    }
}

async fn run_analysis(
    client: &mut AgentClient<Channel>,
    timeout_secs: u32,
    sample_name: &str,
) -> anyhow::Result<AnalysisResult> {
    let mut stream = client
        .analyze(AnalysisRequest {
            timeout_secs,
            sample_name: sample_name.to_string(),
        })
        .await?
        .into_inner();
    while let Some(ev) = stream.message().await? {
        if let Some(malbox_proto::pb::event::Kind::Done(done)) = ev.kind {
            return Ok(AnalysisResult {
                exit_code: done.exit_code,
                runtime_ms: done.runtime_ms,
                features_json: done.features_json,
            });
        }
    }
    Err(anyhow::anyhow!("analysis stream ended without completion"))
}
