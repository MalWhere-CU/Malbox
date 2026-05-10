use std::{net::IpAddr, path::Path, time::Duration};

use anyhow::Context;
use config::Config;
use malbox_proto::pb::{
    FileChunk, Hello, RequestCsrEnvelope, SignedCertEnvelope, Who, agent_client::AgentClient,
    bootstrap_client::BootstrapClient,
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, BufReader},
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{
    Request,
    transport::{Certificate, ClientTlsConfig, Identity},
};
use vm::VmManager;

use crate::cert::JobCerts;

mod cert;
mod config;
mod vm;

async fn test_bootstrap(ip: &str) -> anyhow::Result<JobCerts> {
    let url = format!("http://{}:50055", ip);
    println!("Waiting for Agent Bootstrap service at {}...", url);
    let mut client = loop {
        match BootstrapClient::connect(url.clone()).await {
            Ok(c) => break c,
            Err(_) => {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    };
    let controller_ip = client
        .who_am_i(tonic::Request::new(Who {}))
        .await
        .unwrap()
        .into_inner()
        .ip_addr;
    println!("Connected to Agent Bootstrap service!");
    let certs = cert::generate_job_certs("x012", controller_ip.parse::<IpAddr>().unwrap())?;
    let req = tonic::Request::new(RequestCsrEnvelope {
        ip_addr: ip.to_owned(),
        root_ca_pem: certs.root_ca_cert.pem(),
        job_uuid: "x012".to_owned(),
    });
    let csr: String = match client.request_csr(req).await {
        Ok(res) => {
            let r = res.into_inner();
            r.csr_pem
        }
        Err(e) => {
            println!("Failed to send certs to agent: {}", e);
            return Err(anyhow::anyhow!("failed to request csr from agent"));
        }
    };
    let agent_cert = certs.sign_agent_csr(&csr)?;
    let req = tonic::Request::new(SignedCertEnvelope {
        signed_cert_pem: agent_cert.pem(),
    });
    match client.switch_connection(req).await {
        Ok(res) => {
            let res = res.into_inner();
            if res.success {
                println!("Key Exchange complete");
            } else {
                println!("Key exchange failed: {}", res.error_message);
            }
        }
        Err(e) => {
            println!("Failed to Switch connection to mTLS: {}", e);
            return Err(anyhow::anyhow!("failed to switch connection to mTLS"));
        }
    }
    Ok(certs)
}

async fn test_mtls_hello(
    agent_ip: &str,
    controller_certs: &cert::JobCerts,
    local_file_path: &str,
) -> anyhow::Result<()> {
    let ca_cert_pem = controller_certs.root_ca_cert.pem();
    let controller_cert_pem = controller_certs.controller_cert.pem();
    let controller_key_pem = controller_certs.controller_key.serialize_pem();
    let client_tls = ClientTlsConfig::new()
        .identity(Identity::from_pem(
            &controller_cert_pem,
            &controller_key_pem,
        ))
        .ca_certificate(Certificate::from_pem(ca_cert_pem));

    let agent_url = format!("https://{}:50055", agent_ip);
    println!("{}", agent_url);
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
    let mut agent_client = AgentClient::new(channel);
    let hello_req = tonic::Request::new(Hello {
        msg: "controller".to_owned(),
    });
    let response = agent_client.hello_svc(hello_req).await?;
    println!("Hi {}, I'm controller", response.into_inner().msg);
    let filename = std::path::Path::new(local_file_path)
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid file path"))?
        .to_string_lossy()
        .to_string();
    let file = File::open(local_file_path).await?;
    let mut reader = BufReader::with_capacity(32 * 1024, file);
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let filename_clone = filename.clone();
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
                            filename_clone.clone()
                        } else {
                            String::new()
                        },
                    };
                    if tx.send(chunk).await.is_err() {
                        break;
                    }
                    is_first = false;
                }
                Err(e) => {
                    eprintln!("Read error: {}", e);
                    break;
                }
            }
        }
    });
    let stream = ReceiverStream::new(rx);

    let response = agent_client
        .upload_sample(Request::new(stream))
        .await?
        .into_inner();
    if response.success {
        println!("File uploaded successfully: {}", filename);
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "Upload rejected: {}",
            response.error_message
        ))
    }
}

#[tokio::main]
async fn main() {
    let path = Path::new("config.EXAMPLE.toml");
    let conf = Config::from_file(path)
        .with_context(|| format!("Failed to load configuration from {}", path.display()))
        .unwrap();
    let vm_mgr = VmManager::new(conf.to_owned())
        .with_context(|| format!("Failed to connect to libvirt at {}", conf.libvirt.uri))
        .unwrap();
    println!(
        "Connected successfully to libvirt at {}",
        vm_mgr.conn.get_uri().unwrap()
    );
    let jd = vm_mgr.create_job_domain().unwrap();
    println!("Started VM: {}", jd.vm_name);
    println!("MAC ADDRESS: {}", jd.mac);
    loop {
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .expect("Expected a line");
        match input.trim() {
            "destroy" => {
                jd.teardown();
                break;
            }
            "state" => {
                println!("{:?}", jd.get_state());
            }
            "bootstrap" => {
                let certs = test_bootstrap(&jd.ip_addr).await.unwrap();
                if let Err(e) = test_mtls_hello(&jd.ip_addr, &certs, "test.ps1").await {
                    println!("mTLS hello test failed: {}", e);
                }
            }
            _ => {}
        }
    }
}
