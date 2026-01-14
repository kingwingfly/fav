use const_format::formatc;

pub const VERSION: &str = formatc!(
    "version: {} tag: {}\nrustc: {} {}",
    env!("CARGO_PKG_VERSION"),
    match option_env!("VERGEN_GIT_DESCRIBE") {
        Some(git_desc) => git_desc,
        None => "crates.io",
    },
    env!("VERGEN_RUSTC_SEMVER"),
    env!("VERGEN_RUSTC_HOST_TRIPLE"),
);
