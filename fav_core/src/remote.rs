use crate::FavCoreResult;

pub trait Remote {
    fn login(&self) -> FavCoreResult<()>;
    fn logout(&self) -> FavCoreResult<()>;
}
