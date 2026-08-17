use anyhow::Result;
use vergen::{Emitter, Rustc};
use vergen_git2::Git2;

fn main() -> Result<()> {
    let rustc = Rustc::builder().host_triple(true).semver(true).build();
    let git = Git2::builder().describe(true, true, None).build();
    Emitter::default()
        .add_instructions(&rustc)?
        .add_instructions(&git)?
        .emit()?;
    Ok(())
}
