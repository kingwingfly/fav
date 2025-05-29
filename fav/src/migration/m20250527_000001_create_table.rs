#![allow(clippy::enum_variant_names)]

use sea_orm_migration::{prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Account::Table)
                    .if_not_exists()
                    .col(big_unsigned_uniq(Account::AccountId))
                    .col(string(Account::Name))
                    .col(string(Account::Cookies))
                    .col(
                        enumeration(Account::State, "state", ["Active", "Inactive", "Expired"])
                            .default("Active"),
                    )
                    .primary_key(Index::create().col(Account::AccountId))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Up::Table)
                    .if_not_exists()
                    .col(big_unsigned_uniq(Up::UpId))
                    .col(string(Up::Name))
                    .primary_key(Index::create().col(Up::UpId))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Set::Table)
                    .if_not_exists()
                    .col(big_unsigned_uniq(Set::SetId))
                    .col(string(Set::Name))
                    .col(
                        enumeration(Set::State, "state", ["Active", "Inactive", "Unreachable"])
                            .default("Inactive"),
                    )
                    .primary_key(Index::create().col(Set::SetId))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(Video::Table)
                    .if_not_exists()
                    .col(string_uniq(Video::BvId))
                    .col(string(Video::Title))
                    .col(
                        enumeration(
                            Video::State,
                            "state",
                            [
                                "Pending",
                                "Downloading",
                                "Completed",
                                "Failed",
                                "Expired",
                                "PermissionDenied",
                            ],
                        )
                        .default("Pending"),
                    )
                    .primary_key(Index::create().col(Video::BvId))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(VideoSet::Table)
                    .if_not_exists()
                    .col(string(VideoSet::BvId))
                    .col(big_unsigned(VideoSet::SetId))
                    .primary_key(Index::create().col(VideoSet::BvId).col(VideoSet::SetId))
                    .foreign_key(
                        ForeignKey::create()
                            .name("bvset_bv_fk")
                            .from(VideoSet::Table, VideoSet::BvId)
                            .to(Video::Table, Video::BvId)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("bvset_set_fk")
                            .from(VideoSet::Table, VideoSet::SetId)
                            .to(Set::Table, Set::SetId)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(VideoUp::Table)
                    .if_not_exists()
                    .col(string(VideoUp::BvId))
                    .col(big_unsigned(VideoUp::UpId))
                    .primary_key(Index::create().col(VideoUp::BvId).col(VideoUp::UpId))
                    .foreign_key(
                        ForeignKey::create()
                            .name("bvup_bv_fk")
                            .from(VideoUp::Table, VideoUp::BvId)
                            .to(Video::Table, Video::BvId)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("bvup_up_fk")
                            .from(VideoUp::Table, VideoUp::UpId)
                            .to(Up::Table, Up::UpId)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(SetAccount::Table)
                    .if_not_exists()
                    .col(big_unsigned(SetAccount::SetId))
                    .col(big_unsigned(SetAccount::AccountId))
                    .primary_key(
                        Index::create()
                            .col(SetAccount::SetId)
                            .col(SetAccount::AccountId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("setaccount_set_fk")
                            .from(SetAccount::Table, SetAccount::SetId)
                            .to(Set::Table, Set::SetId)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("setaccount_account_fk")
                            .from(SetAccount::Table, SetAccount::AccountId)
                            .to(Account::Table, Account::AccountId)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(VideoUp::Table).to_owned())
            .await
            .unwrap();
        manager
            .drop_table(Table::drop().table(VideoSet::Table).to_owned())
            .await
            .unwrap();
        manager
            .drop_table(Table::drop().table(SetAccount::Table).to_owned())
            .await
            .unwrap();
        manager
            .drop_table(Table::drop().table(Account::Table).to_owned())
            .await
            .unwrap();
        manager
            .drop_table(Table::drop().table(Up::Table).to_owned())
            .await
            .unwrap();
        manager
            .drop_table(Table::drop().table(Set::Table).to_owned())
            .await
            .unwrap();
        manager
            .drop_table(Table::drop().table(Video::Table).to_owned())
            .await
            .unwrap();
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Account {
    Table,
    AccountId,
    Name,
    Cookies,
    State,
}

#[derive(DeriveIden)]
enum Up {
    Table,
    UpId,
    Name,
}

#[derive(DeriveIden)]
enum Set {
    Table,
    SetId,
    Name,
    State,
}

#[derive(DeriveIden)]
enum Video {
    Table,
    BvId,
    Title,
    State,
}

#[derive(DeriveIden)]
enum VideoUp {
    Table,
    BvId,
    UpId,
}

#[derive(DeriveIden)]
enum VideoSet {
    Table,
    BvId,
    SetId,
}

#[derive(DeriveIden)]
enum SetAccount {
    Table,
    SetId,
    AccountId,
}
