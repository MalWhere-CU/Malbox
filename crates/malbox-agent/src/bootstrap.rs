use malbox_proto::pb::{
    Ack, CsrEnvelope, RequestCsrEnvelope, SignedCertEnvelope, bootstrap_server::Bootstrap,
};
use malbox_proto::pb::{IpMsg, Who};
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::{Mutex, oneshot};
use tonic::{Request, Response, Status};

use crate::certs::generate_csr;
pub struct AgentCerts {
    pub root_ca_pem: String,
    pub agent_key_pem: String,
    pub agent_cert_pem: String,
    pub server_ip_addr: IpAddr,
}

impl Default for AgentCerts {
    fn default() -> Self {
        let e = "";
        AgentCerts {
            root_ca_pem: e.to_owned(),
            agent_key_pem: e.to_owned(),
            agent_cert_pem: e.to_owned(),
            server_ip_addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        }
    }
}

pub struct BootstrapService {
    pub state: Arc<Mutex<AgentCerts>>,
    pub done_tx: StdMutex<Option<oneshot::Sender<()>>>,
}

#[tonic::async_trait]
impl Bootstrap for BootstrapService {
    async fn who_am_i(&self, request: Request<Who>) -> Result<Response<IpMsg>, Status> {
        let mut state = self.state.lock().await;
        let ip = request.remote_addr().unwrap().ip();
        state.server_ip_addr = ip;
        Ok(Response::new(IpMsg {
            ip_addr: ip.to_string(),
        }))
    }
    async fn request_csr(
        &self,
        request: Request<RequestCsrEnvelope>,
    ) -> Result<Response<CsrEnvelope>, Status> {
        let req = request.into_inner();
        let mut state = self.state.lock().await;
        state.root_ca_pem = req.root_ca_pem.clone();

        let ip = req.ip_addr.parse::<std::net::IpAddr>().unwrap();
        let csr_output = generate_csr(&req.job_uuid, ip).unwrap();
        state.agent_key_pem = csr_output.0.serialize_pem();
        Ok(Response::new(CsrEnvelope {
            csr_pem: csr_output.1,
        }))
    }

    async fn switch_connection(
        &self,
        request: Request<SignedCertEnvelope>,
    ) -> Result<Response<Ack>, Status> {
        let req = request.into_inner();
        let mut state = self.state.lock().await;
        if req.signed_cert_pem.is_empty() {
            return Err(Status::invalid_argument("Empty Certificate PEM"));
        }
        state.agent_cert_pem = req.signed_cert_pem;
        if let Some(tx) = self.done_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
        Ok(Response::new(Ack {
            success: true,
            error_message: "".to_owned(),
        }))
    }
}
