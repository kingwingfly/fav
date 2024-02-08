pub trait Status {
    fn saved(&self) -> bool;
    fn fav(&self) -> bool;
    fn expired(&self) -> bool;
}
