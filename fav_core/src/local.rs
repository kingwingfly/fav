use crate::meta::Meta;

trait Local<M>
where
    M: Meta,
{
    fn path(&self) -> &str;
    fn persist(meta: M);
}
