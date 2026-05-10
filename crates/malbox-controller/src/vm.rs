use std::{
    ffi::CString,
    fs,
    os::unix::thread,
    path::{Path, PathBuf},
    process::Command,
    ptr,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use rand::{Rng, distributions::Alphanumeric};
use tracing::{info, warn};
use uuid::Uuid;
use virt::{
    domain::Domain,
    network::Network,
    sys::{self, VIR_DOMAIN_UNDEFINE_KEEP_NVRAM, VIR_DOMAIN_UNDEFINE_NVRAM},
};

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
    pub nvram_path: PathBuf,
    pub mac: String,
    pub ip_addr: String,
}

// is this way ideal to keep track of states?
// We are probably going to need this later when we can keep track of multiple jobs at the same time
#[derive(Debug)]
pub enum DomainState {
    NoState,
    Running,
    Blocked,
    Paused,
    Shutdown,
    Shutoff,
    Crashed,
    PMSuspended,
    Unknown,
}

impl VmManager {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let conn = virt::connect::Connect::open(Some(&config.libvirt.uri))?;
        Ok(Self { config, conn })
    }
    fn generate_alphanumeric(&self, len: usize) -> String {
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(len)
            .map(char::from)
            .collect::<String>()
            .to_uppercase()
    }
    fn generate_wwn(&self) -> String {
        let mut rng = rand::thread_rng();
        let part1: u32 = rng.r#gen_range(0x0000000..0xFFFFFFF);
        let part2: u32 = rng.r#gen_range(0x00000000..0xFFFFFFFF);
        format!("0x5{:07x}{:08x}", part1, part2)
    }
    fn generate_disk_serial(&self) -> String {
        format!("WD-WCC6Y{}", self.generate_alphanumeric(7))
    }
    fn create_nvram(&self, nvram_path: &Path) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.config.paths.master_nvram.exists(),
            "master nvram not found at {}",
            self.config.paths.master_nvram.display()
        );
        info!(nvram = %nvram_path.display(), "Creating NVRAM");
        anyhow::ensure!(
            !nvram_path.exists(),
            "nvram already exists at {}",
            nvram_path.display()
        );
        fs::copy(&self.config.paths.master_nvram, nvram_path)?;
        Ok(())
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
    fn generate_mac(&self) -> String {
        let mut rng = rand::thread_rng();
        let prefix = [0x00, 0x1B, 0x21];
        let suffix: [u8; 3] = rng.r#gen();
        format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            prefix[0], prefix[1], prefix[2], suffix[0], suffix[1], suffix[2]
        )
    }
    fn render_domain(
        &self,
        name: &str,
        uuid: &str,
        overlay_path: &Path,
        nvram_path: &Path,
        mac: &String,
    ) -> String {
        let vm_serial = self.generate_alphanumeric(7);
        let board_serial = format!("/{}/CN123456789012", vm_serial);
        let chassis_serial = self.generate_alphanumeric(7);
        let disk_serial = self.generate_disk_serial();
        let disk_wwn = self.generate_wwn();
        DOMAIN_TEMPLATE
            .replace("{{NAME}}", name)
            .replace("{{UUID}}", uuid)
            .replace("{{OVERLAY_PATH}}", &overlay_path.to_string_lossy())
            .replace("{{MAC_ADDRESS}}", &mac)
            .replace("{{NVRAM_PATH}}", &nvram_path.to_string_lossy())
            .replace("{{VM_SERIAL}}", &vm_serial)
            .replace("{{BOARD_SERIAL}}", &board_serial)
            .replace("{{CHASSIS_SERIAL}}", &chassis_serial)
            .replace("{{DISK_SERIAL}}", &disk_serial)
            .replace("{{DISK_WWN}}", &disk_wwn)
    }
    fn get_ip_from_mac(network: &Network, mac: &str) -> Option<String> {
        let mac_cstr = CString::new(mac).ok()?;
        let mut ip_addr: Option<String> = None;
        let raw_net_ptr = unsafe { network.as_ptr() };
        unsafe {
            let mut leases: *mut *mut virt_sys::_virNetworkDHCPLease = ptr::null_mut();
            let n_leases =
                virt_sys::virNetworkGetDHCPLeases(raw_net_ptr, mac_cstr.as_ptr(), &mut leases, 0);
            if n_leases > 0 && !leases.is_null() {
                let c_ip = unsafe {
                    let first_lease = *leases;
                    let c_ip = (*first_lease).ipaddr;
                    if !c_ip.is_null() {
                        Some(
                            std::ffi::CStr::from_ptr(c_ip)
                                .to_string_lossy()
                                .into_owned(),
                        )
                    } else {
                        None
                    }
                };
                ip_addr = c_ip;
                for i in 0..n_leases as usize {
                    let lease_ptr = *leases.add(i);
                    if !lease_ptr.is_null() {
                        virt_sys::virNetworkDHCPLeaseFree(lease_ptr);
                    }
                }
                libc::free(leases as *mut libc::c_void);
            }
        }
        ip_addr
    }

    fn wait_for_ip_from_mac(
        network: &Network,
        mac: &str,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Option<String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(ip) = VmManager::get_ip_from_mac(network, mac) {
                return Some(ip);
            }
            // change this later to tokio
            std::thread::sleep(poll_interval);
        }
        None
    }
    pub fn create_job_domain(&self) -> anyhow::Result<JobDomain> {
        let job_uuid = Uuid::new_v4().to_string();
        let vm_name = format!("{}{}", VM_NAME_PREFIX, job_uuid);
        let overlay_path = self
            .config
            .paths
            .overlay_dir
            .join(format!("{}.qcow2", job_uuid));
        let nvram_path = self
            .config
            .paths
            .nvram_dir
            .join(format!("{}_vars.fd", job_uuid));
        self.create_overlay(&overlay_path)
            .context("creating qcow2 overlay")?;
        if let Err(e) = self.create_nvram(&nvram_path) {
            if let Err(e) = std::fs::remove_file(&overlay_path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(
                    path = %overlay_path.display(),
                    error = %e,
                    "overlay removal failed"
                    );
                }
            }
            return Err(anyhow::anyhow!(e));
        }
        let mac = self.generate_mac();
        let xml = self.render_domain(&vm_name, &job_uuid, &overlay_path, &nvram_path, &mac);
        let domain = match self.define_and_start(&xml) {
            Ok(d) => d,
            Err(e) => {
                let _ = std::fs::remove_file(&overlay_path);
                let _ = std::fs::remove_file(&nvram_path);
                return Err(e);
            }
        };
        let mut network: Option<virt::network::Network> = None;
        let networks = match self.conn.list_all_networks(63) {
            Ok(nets) => nets,
            Err(e) => {
                let _ = std::fs::remove_file(&overlay_path);
                let _ = std::fs::remove_file(&nvram_path);
                return Err(anyhow::anyhow!(e));
            }
        };
        for net in networks {
            //TODO: change this later when we create a special isolated network
            if net.get_name().expect("getting network name") == "default" {
                network = Some(net);
            }
        }
        let mut ip_addr: String = String::new();
        info!("Waiting till machine gets an IP");
        match network {
            Some(net) => {
                if let Some(ip) = VmManager::wait_for_ip_from_mac(
                    &net,
                    &mac,
                    Duration::from_secs(60),
                    Duration::from_secs(5),
                ) {
                    ip_addr = ip;
                } else {
                    let _ = std::fs::remove_file(&overlay_path);
                    let _ = std::fs::remove_file(&nvram_path);
                    //TODO: cleanup machine
                    return Err(anyhow::anyhow!("Obtaining ip address timed out."));
                }
            }
            None => {
                let _ = std::fs::remove_file(&overlay_path);
                let _ = std::fs::remove_file(&nvram_path);

                return Err(anyhow::anyhow!(
                    "could not find associated network (maybe the network was deleted after creation)"
                ));
            }
        };
        info!("Machine got an IP: {}", ip_addr);
        Ok(JobDomain {
            domain,
            job_uuid,
            vm_name,
            overlay_path,
            nvram_path,
            mac,
            ip_addr,
        })
    }
    fn define_and_start(&self, xml: &str) -> Result<Domain> {
        let domain = Domain::define_xml(&self.conn, xml).context("defining libvirt domain")?;
        if let Err(e) = domain.create() {
            if let Err(e) = domain.undefine_flags(VIR_DOMAIN_UNDEFINE_NVRAM) {
                eprintln!("Failed to undefine with NVRAM: {}", e);
                domain.undefine()?;
                return Err(anyhow::anyhow!("starting domain: {}", e));
            }
            return Err(anyhow::anyhow!("starting domain: {}", e));
        }
        Ok(domain)
    }
}

impl JobDomain {
    pub fn teardown(self) {
        if let Err(e) = self.domain.destroy() {
            tracing::warn!(vm = %self.vm_name, error = %e, "domain destroy failed");
        }
        if let Err(e) = self.domain.undefine_flags(VIR_DOMAIN_UNDEFINE_NVRAM) {
            tracing::warn!(vm = %self.vm_name, error = %e, "domain undefine failed");
        }
        if let Err(e) = std::fs::remove_file(&self.overlay_path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
            path = %self.overlay_path.display(),
            error = %e,
            "overlay removal failed"
            );
        }
        if let Err(e) = std::fs::remove_file(&self.nvram_path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
            path= %self.nvram_path.display(),
            error = %e,
            "nvram removal failed"
            );
        }
    }

    pub fn get_state(&self) -> anyhow::Result<DomainState> {
        let (state, _): (sys::virDomainState, i32) = self
            .domain
            .get_state()
            .with_context(|| "retrieving domain state")?;
        match state {
            0 => Ok(DomainState::NoState),
            1 => Ok(DomainState::Running),
            2 => Ok(DomainState::Blocked),
            3 => Ok(DomainState::Paused),
            4 => Ok(DomainState::Shutdown),
            5 => Ok(DomainState::Shutoff),
            6 => Ok(DomainState::Crashed),
            7 => Ok(DomainState::PMSuspended),
            _ => Ok(DomainState::Unknown),
        }
    }
}
