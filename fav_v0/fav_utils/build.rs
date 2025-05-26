use protobuf::reflect::MessageDescriptor;
use protobuf_codegen::{Codegen, Customize, CustomizeCallback};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Codegen::new()
        .pure()
        .includes(["proto"])
        .inputs(["proto/bili.proto"])
        .cargo_out_dir("proto")
        .customize_callback(MyCustomizeCallback)
        .run()
        .unwrap();
    let path = std::path::PathBuf::from(std::env::var("OUT_DIR")?).join("proto/bili.rs");
    let gen = std::fs::read_to_string(&path)?;
    let processed = gen.replace("#!", "//").replace("//!", "//");
    std::fs::write(path, processed)?;
    println!("cargo:return-if-changed=proto/bili.proto");
    println!("cargo:return-if-changed=build.rs");
    Ok(())
}

struct MyCustomizeCallback;

impl CustomizeCallback for MyCustomizeCallback {
    fn message(&self, message: &MessageDescriptor) -> Customize {
        let name = message.name();
        let c = Customize::default();
        match name {
            "Upper" => c.before(DERIVE_UPPER),
            "BiliRes" => c.before(DERIVE_BILIRES),
            "BiliSet" => c.before(DERIVE_BILISET),
            _ => c,
        }
    }
}

const DERIVE_UPPER: &str = "#[derive(fav_core::attr::Attr)]\n#[attr(id(mid), title(name))]";
const DERIVE_BILIRES: &str =
    "#[derive(fav_core::attr::Attr, fav_core::status::Status, fav_core::attr::Owner)]\n#[attr(id(bvid))]";
const DERIVE_BILISET: &str = "#[derive(fav_core::attr::Attr, fav_core::status::Status)]";
