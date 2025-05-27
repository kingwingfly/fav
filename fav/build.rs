use vergen::RustcBuilder;
use vergen_gitcl::{Emitter, GitclBuilder};

fn main() {
    Emitter::default()
        .add_instructions(
            &GitclBuilder::default()
                .describe(true, true, Some("v*"))
                .build()
                .unwrap(),
        )
        .unwrap()
        .add_instructions(&RustcBuilder::default().semver(true).build().unwrap())
        .unwrap()
        .emit()
        .unwrap();
}
