use rand::RngExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct MonitorBinaries {
    pub capemon32: PathBuf,
    pub capemon64: PathBuf,
    pub loader32: PathBuf,
    pub loader64: PathBuf,
    pub work_dir: PathBuf,
    pub dll_dir: PathBuf,
    pub results_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct MonitorState {
    pub binaries: MonitorBinaries,
    pub shutdown_mutex: String,
    pub terminate_event_prefix: String,
    pub command_pipe: String,
    pub log_pipe_prefix: String,
}

fn random_name(len: usize, ext: &str) -> String {
    let mut rng = rand::rng();
    let name: String = (0..len)
        .map(|_| rng.sample(rand::distr::Alphanumeric) as char)
        .collect();
    format!("{}.{}", name.to_lowercase(), ext)
}

fn random_string(len: usize) -> String {
    let mut rng = rand::rng();
    (0..len)
        .map(|_| rng.sample(rand::distr::Alphanumeric) as char)
        .collect()
}

impl MonitorBinaries {
    pub fn setup(work_dir: &Path) -> anyhow::Result<Self> {
        let dll_dir = work_dir.join("dll");
        let results_dir = work_dir.join("results");
        std::fs::create_dir_all(&dll_dir)?;
        std::fs::create_dir_all(&results_dir)?;
        let capemon32_bytes = include_bytes!("../assets/capemon.dll");
        let capemon64_bytes = include_bytes!("../assets/capemon_x64.dll");
        let loader32_bytes = include_bytes!("../assets/loader.exe");
        let loader64_bytes = include_bytes!("../assets/loader_x64.exe");
        let capemon32 = dll_dir.join(random_name(8, "dll"));
        let capemon64 = dll_dir.join(random_name(8, "dll"));
        let loader32 = work_dir.join(random_name(8, "exe"));
        let loader64 = work_dir.join(random_name(8, "exe"));
        std::fs::write(&capemon32, capemon32_bytes)?;
        std::fs::write(&capemon64, capemon64_bytes)?;
        std::fs::write(&loader32, loader32_bytes)?;
        std::fs::write(&loader64, loader64_bytes)?;
        Ok(MonitorBinaries {
            capemon32,
            capemon64,
            loader32,
            loader64,
            work_dir: work_dir.to_path_buf(),
            dll_dir,
            results_dir,
        })
    }
    pub fn cleanup(&self) {
        let _ = std::fs::remove_file(&self.capemon32);
        let _ = std::fs::remove_file(&self.capemon64);
        let _ = std::fs::remove_file(&self.loader32);
        let _ = std::fs::remove_file(&self.loader64);
        let _ = std::fs::remove_dir_all(&self.dll_dir);
        let _ = std::fs::remove_dir_all(&self.results_dir);
    }
}

impl MonitorState {
    pub fn init(work_dir: &Path) -> anyhow::Result<Self> {
        let binaries = MonitorBinaries::setup(work_dir)?;
        Ok(MonitorState {
            binaries,
            shutdown_mutex: format!("Global\\{}", random_string(10)),
            terminate_event_prefix: format!(r"Global\{}", random_string(10)),
            command_pipe: format!(r"\\.\PIPE\{}", random_string(7)),
            log_pipe_prefix: format!(r"\\.\PIPE\{}", random_string(9)),
        })
    }
    pub fn write_config(
        &self,
        pid: u32,
        sample_path: &str,
        is_first_process: bool,
        startup_time: u64,
    ) -> anyhow::Result<()> {
        let config_path = self.binaries.dll_dir.join(format!("{}.ini", pid));
        let logserver = format!("{}{}", self.log_pipe_prefix, pid);
        let terminate_event = format!("{}{}", self.terminate_event_prefix, pid);
        let content = format!(
            "host-ip=127.0.0.1\n\
             host-port=8000\n\
             pipe={}\n\
             logserver={}\n\
             results={}\n\
             analyzer={}\n\
             first-process={}\n\
             startup-time={}\n\
             file-of-interest={}\n\
             shutdown-mutex={}\n\
             terminate-event={}\n",
            self.command_pipe,
            logserver,
            self.binaries.results_dir.display(),
            self.binaries.work_dir.display(),
            if is_first_process { 1 } else { 0 },
            startup_time,
            sample_path,
            self.shutdown_mutex,
            terminate_event,
        );
        std::fs::write(&config_path, content)?;
        Ok(())
    }
    pub fn cleanup(&self) {
        self.binaries.cleanup();
    }
}
