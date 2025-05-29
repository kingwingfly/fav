#[allow(unused)]
mod entity_inner;

pub use entity_inner::*;

pub trait ToTableRecord<const N: usize> {
    fn to_record(self) -> [String; N];
}

impl ToTableRecord<3> for account::Model {
    fn to_record(self) -> [String; 3] {
        [self.account_id.to_string(), self.name, self.state]
    }
}
