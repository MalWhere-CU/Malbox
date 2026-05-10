use std::net::IpAddr;

use rcgen::{Certificate, CertificateParams, DistinguishedName, KeyPair, SanType};

pub struct AgentCert {
    pub agent_cert: Certificate,
    pub agent_key: KeyPair,
    pub root_ca: Certificate,
}

pub fn generate_csr(job_uuid: &str, private_ip: IpAddr) -> anyhow::Result<(KeyPair, String)> {
    let mut dn = DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, format!("agent-{}", job_uuid));
    let mut params = CertificateParams::default();
    println!(
        "[special]: we set agent's certificate to use this ip: {}",
        private_ip
    );
    params.distinguished_name = dn;
    params.subject_alt_names = vec![SanType::IpAddress(private_ip)];
    let key_pair = KeyPair::generate()?;
    let csr = params.serialize_request(&key_pair)?;
    let csr_pem = csr.pem()?;
    Ok((key_pair, csr_pem))
}
