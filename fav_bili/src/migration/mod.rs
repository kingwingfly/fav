mod m20250527_000001_create_table;
mod m20261126_000002_enhance_schema;
mod m20261127_000003_archive_cursor_and_file_meta;

pub use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20250527_000001_create_table::Migration),
            Box::new(m20261126_000002_enhance_schema::Migration),
            Box::new(m20261127_000003_archive_cursor_and_file_meta::Migration),
        ]
    }
}
