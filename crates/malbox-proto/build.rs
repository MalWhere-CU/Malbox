fn main() -> Result<(), Box<dyn std::error::Error>> {
    // tonic_prost_build::compile_protos("proto/malbox.proto")?;
    tonic_prost_build::configure()
        .out_dir("gen")
        .compile_protos(&["proto/malbox.proto"], &["proto"])?;
    Ok(())
}
