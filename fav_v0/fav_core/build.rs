fn main() -> Result<(), Box<dyn std::error::Error>> {
    protobuf_codegen::Codegen::new()
        .pure()
        .includes(["proto"])
        .inputs(["proto/data.proto"])
        .cargo_out_dir("proto")
        .run()
        .unwrap();
    let path = std::path::PathBuf::from(std::env::var("OUT_DIR")?).join("proto/data.rs");
    let gen = std::fs::read_to_string(&path)?;
    let processed = gen.replace("#!", "//").replace("//!", "//");
    std::fs::write(path, processed)?;
    println!("cargo:return-if-changed=proto");
    println!("cargo:return-if-changed=build.rs");
    Ok(())
}
