#[allow(unused)]
mod entity_inner;

pub use entity_inner::*;

use crate::table::head;

pub trait ToTableRecord<const N: usize> {
    fn to_record(self) -> [String; N];
}

impl ToTableRecord<3> for account::Model {
    fn to_record(self) -> [String; 3] {
        [self.account_id.to_string(), self.name, self.state]
    }
}

impl ToTableRecord<4> for set::Model {
    fn to_record(self) -> [String; 4] {
        [
            self.set_id.to_string(),
            head(self.name, 20),
            self.count.to_string(),
            self.state,
        ]
    }
}

impl ToTableRecord<5> for media::Model {
    fn to_record(self) -> [String; 5] {
        [
            self.id.to_string(),
            self.bv_id,
            head(self.title, 20),
            self.r#type.to_string(),
            self.state.to_string(),
        ]
    }
}

impl ToTableRecord<2> for up::Model {
    fn to_record(self) -> [String; 2] {
        [self.up_id.to_string(), head(self.name, 20)]
    }
}
