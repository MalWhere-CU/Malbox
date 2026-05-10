use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        env::set_var("OUT_DIR", "gen");
    }
    // tonic_prost_build::compile_protos("proto/malbox.proto")?;
    tonic_prost_build::configure().compile_protos(&["proto/malbox.proto"], &["proto"])?;
    Ok(())
}
