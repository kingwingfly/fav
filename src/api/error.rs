use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(super)), context(suffix(false)))]
pub enum ApiError {
    #[snafu(display("Reqwest error: {}", source), context(false))]
    NetWorkError { source: reqwest::Error },
    #[snafu(display("{}", msg))]
    LogoutFail { source: reqwest::Error, msg: String },
    #[snafu(display("{}", msg))]
    PullFail { msg: String },
}

pub type Result<T, E = ApiError> = std::result::Result<T, E>;
