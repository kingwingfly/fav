use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)), context(suffix(None)))]
pub enum ApiError {
    #[snafu(display("reqwest error: {}", source), context(false))]
    Reqwest { source: reqwest::Error },
}

pub type Result<T, E = ApiError> = std::result::Result<T, E>;
