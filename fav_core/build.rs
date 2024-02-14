use rustc_version::{version_meta, Channel};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set cfg flags depending on release channel
    let channel = match version_meta().unwrap().channel {
        Channel::Stable => "CHANNEL_STABLE",
        Channel::Beta => "CHANNEL_BETA",
        Channel::Nightly => "CHANNEL_NIGHTLY",
        Channel::Dev => "CHANNEL_DEV",
    };
    println!("cargo:rustc-cfg={}", channel);
    #[cfg(not(doc))]
    protobuf_codegen::Codegen::new()
        .pure()
        .includes(["./proto"])
        .inputs(["./proto/data.proto"])
        .out_dir("./src/test_utils/proto")
        .run()
        .unwrap();
    Ok(())
}
