use std::{fmt, process::ExitCode};

/// Semantic process exit codes for CLI automation and scripting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitKind {
    Ok = 0,
    Runtime = 1,
    Usage = 2,
    Auth = 3,
    PartialFailure = 4,
}

impl ExitKind {
    pub fn exit_code(self) -> ExitCode {
        ExitCode::from(self as u8)
    }
}

impl From<ExitKind> for ExitCode {
    fn from(kind: ExitKind) -> Self {
        kind.exit_code()
    }
}

/// Application error carrying a semantic [`ExitKind`] alongside an [`anyhow::Error`] chain.
#[derive(Debug)]
pub struct AppError {
    pub kind: ExitKind,
    pub source: anyhow::Error,
}

pub type AppResult<T> = std::result::Result<T, AppError>;

impl AppError {
    pub fn new(kind: ExitKind, err: impl Into<anyhow::Error>) -> Self {
        Self {
            kind,
            source: err.into(),
        }
    }

    pub fn runtime(err: impl Into<anyhow::Error>) -> Self {
        Self::new(ExitKind::Runtime, err)
    }

    pub fn usage(err: impl Into<anyhow::Error>) -> Self {
        Self::new(ExitKind::Usage, err)
    }

    pub fn auth(err: impl Into<anyhow::Error>) -> Self {
        Self::new(ExitKind::Auth, err)
    }

    pub fn partial(err: impl Into<anyhow::Error>) -> Self {
        Self::new(ExitKind::PartialFailure, err)
    }

    pub fn kind(&self) -> ExitKind {
        self.kind
    }

    pub fn exit_code(&self) -> ExitCode {
        self.kind.exit_code()
    }

    pub fn into_inner(self) -> anyhow::Error {
        self.source
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            write!(f, "{:#}", self.source)
        } else {
            write!(f, "{}", self.source)
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.source()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        let kind = classify_anyhow(&err);
        Self { kind, source: err }
    }
}

/// Extract the [`ExitKind`] from any [`anyhow::Error`] chain, defaulting to [`ExitKind::Runtime`].
pub fn classify_anyhow(err: &anyhow::Error) -> ExitKind {
    for cause in err.chain() {
        if let Some(app_err) = cause.downcast_ref::<AppError>() {
            return app_err.kind;
        }
    }
    ExitKind::Runtime
}

/// Wrapper for sensitive credential strings (cookies, CSRF tokens, refresh tokens)
/// that always redacts its value in [`fmt::Debug`] and [`fmt::Display`].
#[derive(Clone, PartialEq, Eq, Default)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::*;

    #[test]
    fn exit_kind_codes_match_contract() {
        assert_eq!(ExitKind::Ok as u8, 0);
        assert_eq!(ExitKind::Runtime as u8, 1);
        assert_eq!(ExitKind::Usage as u8, 2);
        assert_eq!(ExitKind::Auth as u8, 3);
        assert_eq!(ExitKind::PartialFailure as u8, 4);
    }

    #[test]
    fn app_error_preserves_kind_through_anyhow_context() {
        let base: anyhow::Error = AppError::auth(anyhow!("cookie expired")).into();
        let wrapped = base.context("failed checking account");
        assert_eq!(classify_anyhow(&wrapped), ExitKind::Auth);
        let converted = AppError::from(wrapped);
        assert_eq!(converted.kind(), ExitKind::Auth);
        let rendered = format!("{converted:#}");
        assert!(rendered.contains("failed checking account"));
        assert!(rendered.contains("cookie expired"));
    }

    #[test]
    fn app_error_direct_downcast_from_anyhow() {
        let err: anyhow::Error = AppError::usage(anyhow!("missing id")).into();
        let converted = AppError::from(err);
        assert_eq!(converted.kind(), ExitKind::Usage);
        assert_eq!(converted.to_string(), "missing id");
    }

    #[test]
    fn secret_string_redacts_debug_and_display() {
        let secret = SecretString::new("SESSDATA=super_secret_token");
        assert_eq!(format!("{secret}"), "[REDACTED]");
        assert_eq!(format!("{secret:?}"), "[REDACTED]");
        assert_eq!(secret.expose(), "SESSDATA=super_secret_token");
        assert!(!secret.is_empty());
        assert_eq!(secret.len(), "SESSDATA=super_secret_token".len());
    }
}
