use std::net::IpAddr;

use rcgen::{
    Certificate, CertificateParams, CertificateSigningRequestParams, DistinguishedName, Issuer,
    KeyPair, SanType,
};

pub struct JobCerts {
    pub controller_cert: Certificate,
    pub controller_key: KeyPair,
    pub root_ca_cert: Certificate,
    pub root_ca_key: KeyPair,
    // pub perceived_controller_ip: IpAddr,
}

fn new_cert_params(is_ca: bool) -> CertificateParams {
    let mut params = CertificateParams::default();
    if is_ca {
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    }
    params
}

pub fn generate_job_certs(job_uuid: &str, ip: IpAddr) -> anyhow::Result<JobCerts> {
    let mut root_dn = DistinguishedName::new();
    root_dn.push(rcgen::DnType::CommonName, format!("Malbox-CA-{}", job_uuid));
    let mut root_ca_params = new_cert_params(true);
    root_ca_params.distinguished_name = root_dn;
    let root_ca_key = KeyPair::generate()?;
    let root_ca_cert = root_ca_params.self_signed(&root_ca_key)?;
    let ca_issuer = Issuer::from_params(&root_ca_params, &root_ca_key);
    let mut controller_dn = DistinguishedName::new();
    controller_dn.push(
        rcgen::DnType::CommonName,
        format!("Malbox-controller-{}", job_uuid),
    );
    let mut controller_params = new_cert_params(false);
    controller_params.distinguished_name = controller_dn;
    println!(
        "[special]: we set controller's certificate to use this ip: {}",
        ip
    );
    controller_params.subject_alt_names = vec![SanType::IpAddress(ip)];
    let controller_key = KeyPair::generate()?;
    let controller_cert = controller_params.signed_by(&controller_key, &ca_issuer)?;

    Ok(JobCerts {
        controller_cert,
        controller_key,
        root_ca_cert,
        root_ca_key,
        // perceived_controller_ip: ip,
    })
}

impl JobCerts {
    pub fn sign_agent_csr(&self, csr_pem: &str) -> anyhow::Result<Certificate> {
        let csr = CertificateSigningRequestParams::from_pem(csr_pem)?;
        let issuer = Issuer::from_ca_cert_der(self.root_ca_cert.der(), &self.root_ca_key)?;
        let agent_cert = csr.signed_by(&issuer)?;
        Ok(agent_cert)
    }
}
