use std::sync::Arc;

use malbox_agent::agent::AgentService;
use malbox_proto::pb::bootstrap_server::BootstrapServer;
pub mod bootstrap;
pub mod certs;
use tokio::sync::Mutex;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};

use crate::bootstrap::{AgentCerts, BootstrapService};

pub const LISTEN_ADDR: &str = "0.0.0.0:50055";

async fn start_mtls_server(state: Arc<Mutex<AgentCerts>>) -> anyhow::Result<()> {
    let final_state = state.lock().await;

    let agent_cert = &final_state.agent_cert_pem;
    let agent_key = &final_state.agent_key_pem;
    let ca_cert = &final_state.root_ca_pem;
    let controller_ip = &final_state.server_ip_addr;
    let server_tls = ServerTlsConfig::new()
        .identity(Identity::from_pem(agent_cert, agent_key))
        .client_ca_root(Certificate::from_pem(ca_cert))
        .client_auth_optional(false);
    // let client_tls = ClientTlsConfig::new()
    //     .identity(Identity::from_pem(agent_cert, agent_key))
    //     .ca_certificate(Certificate::from_pem(ca_cert));
    let addr = LISTEN_ADDR.parse()?;
    let server_future = Server::builder()
        .tls_config(server_tls)?
        .add_service(malbox_proto::pb::agent_server::AgentServer::new(
            AgentService,
        ))
        .serve(addr);
    let controller_url = format!("https://{}:50055", controller_ip);
    println!("Controller URL: {}", controller_url);
    // for sending the events back
    // let _channel = tonic::transport::Channel::from_shared(controller_url)
    //     .unwrap()
    //     .tls_config(client_tls)?
    //     .connect()
    //     .await?;
    // println!("mTLS Agent server listening on :50055");
    server_future.await?;
    println!("that sucks");
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let state = Arc::new(Mutex::new(AgentCerts::default()));
    let bootstrap_svc = BootstrapService {
        state: state.clone(),
        done_tx: std::sync::Mutex::new(Some(done_tx)),
    };
    let addr = LISTEN_ADDR.parse()?;
    let bootstrap_server = Server::builder()
        .add_service(BootstrapServer::new(bootstrap_svc))
        .serve(addr);
    println!("Malbox Agent started. Listening on {}", addr);
    tokio::select! {
        res = bootstrap_server => res?,
        _ = done_rx => {
            println!("Bootstrap complete. Insecure channel closed.");
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let final_state = state.clone();
            tokio::spawn(async move {
                if let Err(e) = start_mtls_server(final_state).await {
                    eprintln!("mTLS server failed: {}", e);
                    std::process::exit(1);
                }
            });
        }
    }

    tokio::signal::ctrl_c().await?;
    println!("Agent shutting down");
    Ok(())
}
