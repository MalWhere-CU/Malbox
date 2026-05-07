use std::path::Path;

use anyhow::Context;
use config::Config;
use vm::VmManager;

mod config;
mod vm;

fn main() {
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
            _ => {}
        }
    }
}
