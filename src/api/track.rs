use super::error::Result;
use crate::meta::meta;
use tracing::info;

pub(crate) fn track(id: i64) -> Result<()> {
    let mut meta = meta().clone();
    let target = meta
        .lists
        .iter_mut()
        .find(|l| l.id == id)
        .expect(&format!("id {} not found", id));
    target.is_tracked = true;
    info!("Tracking id:{} title:{}", target.id, target.title);
    meta.persist();
    Ok(())
}
