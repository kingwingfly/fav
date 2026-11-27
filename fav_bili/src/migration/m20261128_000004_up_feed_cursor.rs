use sea_orm_migration::{prelude::*, schema::*, sea_orm::Statement};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(MediaUp::Table)
                    .add_column(boolean(MediaUp::FromUpFeed).default(false))
                    .to_owned(),
            )
            .await?;

        let backend = manager.get_database_backend();
        let db = manager.get_connection();
        // Preserve fast 1-page incremental sync for already-Active UPs that have synced or started backfilling.
        db.execute(Statement::from_string(
            backend,
            r#"UPDATE media_up SET from_up_feed = 1 WHERE up_id IN (SELECT up_id FROM "up" WHERE state = 'Active' AND (sync_complete = 1 OR resume_page IS NOT NULL));"#,
        ))
        .await?;
        // Reset non-Active UPs so favorite-set creators previously marked sync_complete = 1 by
        // Migration #3 perform a full channel backfill if activated later.
        db.execute(Statement::from_string(
            backend,
            r#"UPDATE "up" SET sync_complete = 0, resume_page = NULL WHERE state != 'Active';"#,
        ))
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(MediaUp::Table)
                    .drop_column(MediaUp::FromUpFeed)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum MediaUp {
    Table,
    FromUpFeed,
}
