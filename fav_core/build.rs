fn main() -> Result<(), Box<dyn std::error::Error>> {
    protobuf_codegen::Codegen::new()
        .pure()
        .includes(["./proto"])
        .inputs(["./proto/data.proto"])
        .out_dir("./src/proto")
        .run()
        .unwrap();
    Ok(())
}
