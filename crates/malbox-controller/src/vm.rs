use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result};
use tracing::info;
use uuid::Uuid;
use virt::{domain::Domain, sys::VIR_DOMAIN_UNDEFINE_KEEP_NVRAM};

use crate::config::Config;

const DOMAIN_TEMPLATE: &str = include_str!("../../../templates/domain.xml");
const VM_NAME_PREFIX: &str = "malbox-job-";
pub struct VmManager {
    config: Config,
    pub conn: virt::connect::Connect,
}
pub struct JobDomain {
    pub domain: virt::domain::Domain,
    pub job_uuid: String,
    pub vm_name: String,
    pub overlay_path: PathBuf,
}

impl VmManager {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let conn = virt::connect::Connect::open(Some(&config.libvirt.uri))?;
        Ok(Self { config, conn })
    }
    fn create_overlay(&self, overlay_path: &Path) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.config.paths.master_image.exists(),
            "master image not found at {}",
            self.config.paths.master_image.display()
        );
        info!(overlay = %overlay_path.display(), "Creating qcow2");
        anyhow::ensure!(
            !overlay_path.exists(),
            "overlay already exists at {}",
            overlay_path.display()
        );
        let output = Command::new("qemu-img")
            .args([
                "create",
                "-f",
                "qcow2",
                "-F",
                "qcow2",
                "-b",
                &self.config.paths.master_image.to_string_lossy(),
                &overlay_path.to_string_lossy(),
            ])
            .output()
            .context("invoking qemu-img")?;
        anyhow::ensure!(
            output.status.success(),
            "Failed to create overlay from golden image: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(())
    }
    fn render_domain(&self, name: &str, uuid: &str, overlay_path: &Path) -> String {
        DOMAIN_TEMPLATE
            .replace("{{NAME}}", name)
            .replace("{{UUID}}", uuid)
            .replace("{{OVERLAY_PATH}}", &overlay_path.to_string_lossy())
    }
    pub fn create_job_domain(&self) -> anyhow::Result<JobDomain> {
        let job_uuid = Uuid::new_v4().to_string();
        let vm_name = format!("{}{}", VM_NAME_PREFIX, job_uuid);
        let overlay_path = self
            .config
            .paths
            .overlay_dir
            .join(format!("{}.qcow2", job_uuid));
        self.create_overlay(&overlay_path)
            .context("creating qcow2 overlay")?;
        let xml = self.render_domain(&vm_name, &job_uuid, &overlay_path);
        let domain = match self.define_and_start(&xml) {
            Ok(d) => d,
            Err(e) => {
                let _ = std::fs::remove_file(&overlay_path);
                return Err(e);
            }
        };
        Ok(JobDomain {
            domain,
            job_uuid,
            vm_name,
            overlay_path,
        })
    }
    fn define_and_start(&self, xml: &str) -> Result<Domain> {
        let domain = Domain::define_xml(&self.conn, xml).context("defining libvirt domain")?;
        if let Err(e) = domain.create() {
            if let Err(e) = domain.undefine_flags(VIR_DOMAIN_UNDEFINE_KEEP_NVRAM) {
                eprintln!("Failed to undefine with NVRAM: {}", e);
                return Err(anyhow::anyhow!("starting domain: {}", e));
            }
            domain.undefine()?;
            return Err(anyhow::anyhow!("starting domain: {}", e));
        }
        Ok(domain)
    }
}
