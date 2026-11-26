#![allow(clippy::enum_variant_names)]

use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Secondary indexes on join-table reverse lookups and state filter columns.
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_set_account_account_id")
                    .table(SetAccount::Table)
                    .col(SetAccount::AccountId)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_up_account_account_id")
                    .table(UpAccount::Table)
                    .col(UpAccount::AccountId)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_media_set_set_id")
                    .table(MediaSet::Table)
                    .col(MediaSet::SetId)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_media_up_up_id")
                    .table(MediaUp::Table)
                    .col(MediaUp::UpId)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_media_state")
                    .table(Media::Table)
                    .col(Media::State)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_set_state")
                    .table(Set::Table)
                    .col(Set::State)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_up_state")
                    .table(Up::Table)
                    .col(Up::State)
                    .to_owned(),
            )
            .await?;

        // Additive columns (SQLite requires one ADD COLUMN per ALTER TABLE statement).
        manager
            .alter_table(
                Table::alter()
                    .table(Account::Table)
                    .add_column(string_null(Account::RefreshToken))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Media::Table)
                    .add_column(integer(Media::RetryCount).default(0))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Media::Table)
                    .add_column(string_null(Media::LastError))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Media::Table)
                    .add_column(big_integer_null(Media::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        // Per-page (cid) archival tracking table for multi-page videos and DB-vs-disk reconciliation.
        manager
            .create_table(
                Table::create()
                    .table(MediaPage::Table)
                    .if_not_exists()
                    .col(big_unsigned(MediaPage::MediaId))
                    .col(big_unsigned(MediaPage::Cid))
                    .col(integer(MediaPage::Page))
                    .col(string(MediaPage::Part))
                    .col(string(MediaPage::State).default("Pending"))
                    .col(string_null(MediaPage::LocalPath))
                    .col(big_integer_null(MediaPage::DownloadedAt))
                    .primary_key(Index::create().col(MediaPage::MediaId).col(MediaPage::Cid))
                    .foreign_key(
                        ForeignKey::create()
                            .name("mediapage_media_fk")
                            .from(MediaPage::Table, MediaPage::MediaId)
                            .to(Media::Table, Media::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_media_page_state")
                    .table(MediaPage::Table)
                    .col(MediaPage::State)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .if_exists()
                    .name("idx_media_page_state")
                    .table(MediaPage::Table)
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(MediaPage::Table).if_exists().to_owned())
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Media::Table)
                    .drop_column(Media::UpdatedAt)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Media::Table)
                    .drop_column(Media::LastError)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Media::Table)
                    .drop_column(Media::RetryCount)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(Account::Table)
                    .drop_column(Account::RefreshToken)
                    .to_owned(),
            )
            .await?;

        for (name, table_drop) in [
            (
                "idx_up_state",
                Index::drop()
                    .if_exists()
                    .name("idx_up_state")
                    .table(Up::Table)
                    .to_owned(),
            ),
            (
                "idx_set_state",
                Index::drop()
                    .if_exists()
                    .name("idx_set_state")
                    .table(Set::Table)
                    .to_owned(),
            ),
            (
                "idx_media_state",
                Index::drop()
                    .if_exists()
                    .name("idx_media_state")
                    .table(Media::Table)
                    .to_owned(),
            ),
            (
                "idx_media_up_up_id",
                Index::drop()
                    .if_exists()
                    .name("idx_media_up_up_id")
                    .table(MediaUp::Table)
                    .to_owned(),
            ),
            (
                "idx_media_set_set_id",
                Index::drop()
                    .if_exists()
                    .name("idx_media_set_set_id")
                    .table(MediaSet::Table)
                    .to_owned(),
            ),
            (
                "idx_up_account_account_id",
                Index::drop()
                    .if_exists()
                    .name("idx_up_account_account_id")
                    .table(UpAccount::Table)
                    .to_owned(),
            ),
            (
                "idx_set_account_account_id",
                Index::drop()
                    .if_exists()
                    .name("idx_set_account_account_id")
                    .table(SetAccount::Table)
                    .to_owned(),
            ),
        ] {
            let _ = name;
            manager.drop_index(table_drop).await?;
        }

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Account {
    Table,
    RefreshToken,
}

#[derive(DeriveIden)]
enum Up {
    Table,
    State,
}

#[derive(DeriveIden)]
enum Set {
    Table,
    State,
}

#[derive(DeriveIden)]
enum Media {
    Table,
    Id,
    State,
    RetryCount,
    LastError,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum MediaPage {
    Table,
    MediaId,
    Cid,
    Page,
    Part,
    State,
    LocalPath,
    DownloadedAt,
}

#[derive(DeriveIden)]
enum MediaUp {
    Table,
    UpId,
}

#[derive(DeriveIden)]
enum MediaSet {
    Table,
    SetId,
}

#[derive(DeriveIden)]
enum SetAccount {
    Table,
    AccountId,
}

#[derive(DeriveIden)]
enum UpAccount {
    Table,
    AccountId,
}
