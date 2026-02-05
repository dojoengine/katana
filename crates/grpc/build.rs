use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir =
        PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR environment variable not set"));

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .file_descriptor_set_path(out_dir.join("starknet_descriptor.bin"))
        // Allow clippy lints on generated code for enum variant naming and size patterns
        .type_attribute(".", "#[allow(clippy::enum_variant_names, clippy::large_enum_variant)]")
        .compile(&["proto/starknet.proto"], &["proto"])?;

    println!("cargo:rerun-if-changed=proto");

    Ok(())
}
