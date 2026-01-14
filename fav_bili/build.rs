use anyhow::Result;
use vergen::{Emitter, RustcBuilder};
use vergen_git2::Git2Builder;

fn main() -> Result<()> {
    let rustc = RustcBuilder::default()
        .host_triple(true)
        .semver(true)
        .build()?;
    let git = Git2Builder::default().describe(true, true, None).build()?;
    Emitter::default()
        .add_instructions(&rustc)?
        .add_instructions(&git)?
        .emit()?;
    Ok(())
}
