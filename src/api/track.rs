use crate::{meta::meta, proto::data::Meta};
use tracing::info;

impl Meta {
    fn track(&mut self, id: i64) {
        let target = self
            .lists
            .iter_mut()
            .find(|l| l.id == id)
            .expect(&format!("id {} not found", id));
        target.is_tracked = true;
        info!("Tracking id:{} title:{}", target.id, target.title);
    }
}

pub(crate) fn track(id: i64) {
    let mut meta = meta().clone();
    meta.track(id);
    meta.persist();
}
