#![allow(clippy::enum_variant_names)]

use sea_orm_migration::{prelude::*, schema::*, sea_orm::Statement};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Additive columns (SQLite requires one ADD COLUMN per ALTER TABLE statement).
        manager
            .alter_table(
                Table::alter()
                    .table(Set::Table)
                    .add_column(string(Set::Kind).default("Created"))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Set::Table)
                    .add_column(big_integer_null(Set::OwnerMid))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Set::Table)
                    .add_column(boolean(Set::SyncComplete).default(false))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Set::Table)
                    .add_column(integer_null(Set::ResumePage))
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Up::Table)
                    .add_column(boolean(Up::SyncComplete).default(false))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Up::Table)
                    .add_column(integer_null(Up::ResumePage))
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(MediaPage::Table)
                    .add_column(big_integer_null(MediaPage::FileSize))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(MediaPage::Table)
                    .add_column(big_integer_null(MediaPage::FileInode))
                    .to_owned(),
            )
            .await?;

        // One-time backfill: existing sets/UPs that already have fetched media rows are treated as
        // initially synced so upgrading users immediately get fast 1-page incremental sync.
        let backend = manager.get_database_backend();
        let db = manager.get_connection();
        db.execute(Statement::from_string(
            backend,
            r#"UPDATE "set" SET sync_complete = 1 WHERE EXISTS (SELECT 1 FROM media_set ms WHERE ms.set_id = "set".set_id);"#,
        ))
        .await?;
        db.execute(Statement::from_string(
            backend,
            r#"UPDATE "up" SET sync_complete = 1 WHERE EXISTS (SELECT 1 FROM media_up mu WHERE mu.up_id = "up".up_id);"#,
        ))
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(MediaPage::Table)
                    .drop_column(MediaPage::FileInode)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(MediaPage::Table)
                    .drop_column(MediaPage::FileSize)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Up::Table)
                    .drop_column(Up::ResumePage)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Up::Table)
                    .drop_column(Up::SyncComplete)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Set::Table)
                    .drop_column(Set::ResumePage)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Set::Table)
                    .drop_column(Set::SyncComplete)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Set::Table)
                    .drop_column(Set::OwnerMid)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Set::Table)
                    .drop_column(Set::Kind)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Set {
    Table,
    Kind,
    OwnerMid,
    SyncComplete,
    ResumePage,
}

#[derive(DeriveIden)]
enum Up {
    Table,
    SyncComplete,
    ResumePage,
}

#[derive(DeriveIden)]
enum MediaPage {
    Table,
    FileSize,
    FileInode,
}
