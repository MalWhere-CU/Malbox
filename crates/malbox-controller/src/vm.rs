use std::{path::PathBuf, process::Command};

use anyhow::Context;
use tracing::info;

use crate::config::Config;

const DOMAIN_TEMPLATE: &str = include_str!("../../../templates/domain.xml");

pub struct VmManager {
    config: Config,
    pub conn: virt::connect::Connect,
}

pub struct JobDomain {
    pub domain: virt::domain::Domain,
    pub job_uuid: String,
}

impl VmManager {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let conn = virt::connect::Connect::open(Some(&config.libvirt.uri))?;
        Ok(Self { config, conn })
    }
    pub fn create_overlay(&self, job_uuid: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.config.paths.master_image.exists(),
            "master image not found at {}",
            self.config.paths.master_image.display()
        );
        let overlay_path = self
            .config
            .paths
            .overlay_dir
            .join(format!("{job_uuid}.qcow2"));
        info!(%job_uuid, overlay = %overlay_path.display(), "Creating qcow2");
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
}
