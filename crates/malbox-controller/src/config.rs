use std::path::PathBuf;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct LibvirtConfig {
    pub uri: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct VmConfig {
    pub vcpus: u32,
    pub memory_mb: u32,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PathsConfig {
    pub master_image: PathBuf,
    pub overlay_dir: PathBuf,
    pub report_dir: PathBuf,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Config {
    pub libvirt: LibvirtConfig,
    pub vm: VmConfig,
    pub paths: PathsConfig,
}

impl Config {
    pub fn from_file(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config = toml::from_str(&content)?;
        Ok(config)
    }
}
