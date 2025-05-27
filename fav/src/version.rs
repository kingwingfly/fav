use const_format::formatc;

pub const VERSION: &str = formatc!(
    "VERSION: {fav_version}\tRUSTC: {rustc_version}",
    fav_version = env!("VERGEN_GIT_DESCRIBE"),
    rustc_version = env!("VERGEN_RUSTC_SEMVER")
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "human check needed"]
    fn print_version() {
        println!("{}", VERSION);
    }
}
