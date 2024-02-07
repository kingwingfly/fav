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
    #[snafu(display("Ctrl-C Cancelled"))]
    Canclled,
    #[snafu(display("Create temp file fail"), context(false))]
    CreateTempFileFail { source: std::io::Error },
    #[snafu(display("Persist temp file fail"), context(false))]
    PersistFail { source: tempfile::PersistError },
    #[snafu(display("{}", msg))]
    MergeFail { msg: String },
}

pub type Result<T, E = ApiError> = std::result::Result<T, E>;
