mod active;
mod login;
mod logout;

pub(crate) use login::*;
pub(crate) use logout::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn logout_test() {
        assert!(logout().await.is_ok());
    }
}
