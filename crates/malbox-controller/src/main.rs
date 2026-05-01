use std::path::Path;

use config::Config;

mod config;
fn main() {
    let path = Path::new("config.EXAMPLE.toml");
    let conf = Config::from_file(path).unwrap();
    println!("{conf:?}");
}
