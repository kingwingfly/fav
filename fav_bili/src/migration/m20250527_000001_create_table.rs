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
                    .col(
                        enumeration(Up::State, "state", ["Active", "Inactive", "Deactivated"])
                            .default("Inactive"),
                    )
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
                    .col(big_unsigned(Set::Count))
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
                    .table(Media::Table)
                    .if_not_exists()
                    .col(big_unsigned_uniq(Media::Id))
                    .col(string_uniq(Media::BvId))
                    .col(string(Media::Title))
                    .col(
                        enumeration(Media::Type, "type", ["Video", "Audio", "Collection"])
                            .default("Video"),
                    )
                    .col(
                        enumeration(
                            Media::State,
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
                    .primary_key(Index::create().col(Media::Id))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(MediaSet::Table)
                    .if_not_exists()
                    .col(big_unsigned(MediaSet::Id))
                    .col(big_unsigned(MediaSet::SetId))
                    .primary_key(Index::create().col(MediaSet::Id).col(MediaSet::SetId))
                    .foreign_key(
                        ForeignKey::create()
                            .name("mediaset_media_fk")
                            .from(MediaSet::Table, MediaSet::Id)
                            .to(Media::Table, Media::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("mediaset_set_fk")
                            .from(MediaSet::Table, MediaSet::SetId)
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
                    .table(MediaUp::Table)
                    .if_not_exists()
                    .col(big_unsigned(MediaUp::Id))
                    .col(big_unsigned(MediaUp::UpId))
                    .primary_key(Index::create().col(MediaUp::Id).col(MediaUp::UpId))
                    .foreign_key(
                        ForeignKey::create()
                            .name("mediaup_media_fk")
                            .from(MediaUp::Table, MediaUp::Id)
                            .to(Media::Table, Media::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("mediaup_up_fk")
                            .from(MediaUp::Table, MediaUp::UpId)
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
        manager
            .create_table(
                Table::create()
                    .table(UpAccount::Table)
                    .if_not_exists()
                    .col(big_unsigned(UpAccount::UpId))
                    .col(big_unsigned(UpAccount::AccountId))
                    .primary_key(
                        Index::create()
                            .col(UpAccount::UpId)
                            .col(UpAccount::AccountId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("upaccount_up_fk")
                            .from(UpAccount::Table, UpAccount::UpId)
                            .to(Up::Table, Up::UpId)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("upaccount_account_fk")
                            .from(UpAccount::Table, UpAccount::AccountId)
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
            .drop_table(Table::drop().table(MediaUp::Table).to_owned())
            .await
            .unwrap();
        manager
            .drop_table(Table::drop().table(MediaSet::Table).to_owned())
            .await
            .unwrap();
        manager
            .drop_table(Table::drop().table(SetAccount::Table).to_owned())
            .await
            .unwrap();
        manager
            .drop_table(Table::drop().table(UpAccount::Table).to_owned())
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
            .drop_table(Table::drop().table(Media::Table).to_owned())
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
    State,
}

#[derive(DeriveIden)]
enum Set {
    Table,
    SetId,
    Name,
    Count,
    State,
}

#[derive(DeriveIden)]
enum Media {
    Table,
    Id,
    BvId,
    Title,
    Type,
    State,
}

#[derive(DeriveIden)]
enum MediaUp {
    Table,
    Id,
    UpId,
}

#[derive(DeriveIden)]
enum MediaSet {
    Table,
    Id,
    SetId,
}

#[derive(DeriveIden)]
enum SetAccount {
    Table,
    SetId,
    AccountId,
}

#[derive(DeriveIden)]
enum UpAccount {
    Table,
    UpId,
    AccountId,
}
