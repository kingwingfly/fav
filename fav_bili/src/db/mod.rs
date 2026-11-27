pub mod account;
pub mod media;
pub mod media_page;
pub mod media_set;
pub mod media_up;
pub mod set;
pub mod set_account;
pub mod up;
pub mod up_account;

use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, anyhow};
use sea_orm::{
    ConnectionTrait as _, DatabaseBackend, DatabaseConnection, SqlxSqliteConnector, Statement,
    sqlx::{
        ConnectOptions as _,
        sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    },
};
use sea_orm_migration::MigratorTrait as _;

use crate::{
    config::{enforce_db_permissions, open_private_file, set_dir_permissions_private},
    error::AppError,
    migration::Migrator,
};

pub use account::AccountFilter;
pub use media::{
    MediaFilter, MediaResetFilter, MediaSort, PruneReport, PullFilter, StatusSummary, TopUpStat,
};
pub use set::SetFilter;
pub use up::UpFilter;

/// Maximum rows per bulk `INSERT` / `IN (...)` batch to stay well below SQLite's variable limit.
pub const CHUNK_SIZE: usize = 500;

const BUSY_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct Db {
    pub(crate) conn: DatabaseConnection,
}

impl Db {
    /// Open or create the SQLite database at `db_path`, apply private `0700`/`0600` permissions,
    /// configure WAL/busy_timeout/foreign_keys, and run pending migrations.
    pub async fn open_or_create(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory '{}'", parent.display()))?;
            set_dir_permissions_private(parent)?;
        }

        if !db_path.exists() {
            // Pre-create with 0600 on Unix so credentials are never briefly world-readable.
            let _ = open_private_file(db_path)?;
        }

        let mut opts = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT);
        opts = opts.disable_statement_logging();

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .with_context(|| format!("Failed to open database '{}'", db_path.display()))?;

        let conn = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);
        Migrator::up(&conn, None)
            .await
            .with_context(|| format!("Failed to run migrations on '{}'", db_path.display()))?;
        enforce_db_permissions(db_path)?;
        Ok(Self { conn })
    }

    /// Open an existing SQLite database at `db_path`.
    /// When `read_only` is `true`, opens read-only without mutating journal mode or running migrations.
    pub async fn open_existing(db_path: &Path, read_only: bool) -> Result<Self> {
        if !db_path.exists() {
            return Err(AppError::usage(anyhow!(
                "Database not found at '{}'. Run `fav init` or `fav auth login` first.",
                db_path.display(),
            ))
            .into());
        }

        let mut opts = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(false)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT);
        opts = if read_only {
            opts.read_only(true)
        } else {
            opts.journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Normal)
        };
        opts = opts.disable_statement_logging();

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .with_context(|| format!("Failed to open database '{}'", db_path.display()))?;

        let conn = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);
        if !read_only {
            Migrator::up(&conn, None)
                .await
                .with_context(|| format!("Failed to run migrations on '{}'", db_path.display()))?;
            enforce_db_permissions(db_path)?;
        }
        Ok(Self { conn })
    }

    /// Open a fresh in-memory SQLite database with all migrations applied (for tests).
    pub async fn open_in_memory() -> Result<Self> {
        let mut opts = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT);
        opts = opts.disable_statement_logging();

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .context("Failed to open in-memory SQLite database")?;

        let conn = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);
        Migrator::up(&conn, None)
            .await
            .context("Failed to run migrations on in-memory SQLite database")?;
        Ok(Self { conn })
    }

    pub fn conn(&self) -> &DatabaseConnection {
        &self.conn
    }

    /// Verify SQLite structural integrity (`PRAGMA quick_check`).
    pub async fn check_integrity(&self) -> Result<()> {
        let row = self
            .conn
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA quick_check;",
            ))
            .await
            .context("Failed to run PRAGMA quick_check")?;
        let status: String = row
            .and_then(|r| r.try_get_by_index(0).ok())
            .unwrap_or_else(|| "unknown".to_owned());
        if status.eq_ignore_ascii_case("ok") {
            Ok(())
        } else {
            Err(anyhow!("SQLite integrity check failed: {status}"))
        }
    }
}

pub(crate) fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use sea_orm::{DatabaseBackend, Statement};

    use super::*;
    use crate::{
        entity::{account, media, media_set, media_up, set, set_account, up, up_account},
        error::{ExitKind, classify_anyhow},
        state::{AccountState, MediaPageState, MediaState, MediaType, SetKind, SetState, UpState},
    };

    fn sample_set(id: i64, name: &str, count: i64, state: SetState) -> set::Model {
        set::Model {
            set_id: id,
            name: name.to_owned(),
            count,
            state: state.to_string(),
            kind: SetKind::Created.to_string(),
            owner_mid: None,
            sync_complete: false,
            resume_page: None,
        }
    }

    fn sample_up(id: i64, name: &str, state: UpState) -> up::Model {
        up::Model {
            up_id: id,
            name: name.to_owned(),
            state: state.to_string(),
            sync_complete: false,
            resume_page: None,
        }
    }

    fn sample_account(id: i64, state: AccountState) -> account::Model {
        account::Model {
            account_id: id,
            name: format!("user_{id}"),
            cookies: format!("SESSDATA=tok_{id}; bili_jct=csrf_{id}"),
            state: state.to_string(),
            refresh_token: Some(format!("ref_{id}")),
        }
    }

    fn sample_media(id: i64, state: MediaState) -> media::Model {
        media::Model {
            id,
            bv_id: format!("BV1test{id:06}"),
            title: format!("Title {id}"),
            r#type: MediaType::Video.to_string(),
            state: state.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        }
    }

    #[tokio::test]
    async fn migration_up_down_roundtrip() {
        let db = Db::open_in_memory().await.unwrap();
        db.check_integrity().await.unwrap();
        Migrator::down(db.conn(), None).await.unwrap();
        Migrator::up(db.conn(), None).await.unwrap();
        db.check_integrity().await.unwrap();
    }

    #[tokio::test]
    async fn open_or_create_and_read_only_file_db() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join(".fav/fav.db");

        // Missing DB yields ExitKind::Usage
        let missing = Db::open_existing(&db_path, true).await.unwrap_err();
        assert_eq!(classify_anyhow(&missing), ExitKind::Usage);

        let db = Db::open_or_create(&db_path).await.unwrap();
        db.upsert_account(sample_account(1, AccountState::Active))
            .await
            .unwrap();

        // Check WAL mode and foreign_keys pragma
        let wal: String = db
            .conn()
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA journal_mode;",
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get_by_index(0)
            .unwrap();
        assert_eq!(wal.to_ascii_lowercase(), "wal");

        let fk: i64 = db
            .conn()
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA foreign_keys;",
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get_by_index(0)
            .unwrap();
        assert_eq!(fk, 1);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&db_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        // Open read-only and read account
        let ro = Db::open_existing(&db_path, true).await.unwrap();
        let acc = ro.get_account(1).await.unwrap();
        assert_eq!(acc.name, "user_1");
        // Write on read-only connection must fail
        assert!(ro.activate_account(1).await.is_err());
    }

    #[tokio::test]
    async fn upsert_account_reactivates_and_updates_refresh_token() {
        let db = Db::open_in_memory().await.unwrap();
        db.upsert_account(sample_account(10, AccountState::Inactive))
            .await
            .unwrap();
        db.set_account_state(10, AccountState::Expired)
            .await
            .unwrap();
        assert_eq!(
            db.get_account(10).await.unwrap().state,
            AccountState::Expired.as_str()
        );

        let mut refreshed = sample_account(10, AccountState::Active);
        refreshed.name = "renamed".to_owned();
        refreshed.refresh_token = Some("new_refresh".to_owned());
        db.upsert_account(refreshed).await.unwrap();

        let loaded = db.get_account(10).await.unwrap();
        assert_eq!(loaded.name, "renamed");
        assert_eq!(loaded.state, AccountState::Active.as_str());
        assert_eq!(loaded.refresh_token.as_deref(), Some("new_refresh"));
    }

    #[tokio::test]
    async fn upsert_medias_conflicts_on_id_and_preserves_completed() {
        let db = Db::open_in_memory().await.unwrap();
        db.upsert_medias([sample_media(100, MediaState::Pending)])
            .await
            .unwrap();
        db.set_media_state(100, MediaState::Completed)
            .await
            .unwrap();

        // Update same PK id with a new title and new bvid, passing Pending state:
        // must update title/bvid while preserving Completed state!
        let mut updated = sample_media(100, MediaState::Pending);
        updated.title = "Updated Title".to_owned();
        updated.bv_id = "BV1updated0001".to_owned();
        db.upsert_medias([updated]).await.unwrap();

        let loaded = db.get_media(100).await.unwrap();
        assert_eq!(loaded.title, "Updated Title");
        assert_eq!(loaded.bv_id, "BV1updated0001");
        assert_eq!(loaded.state, MediaState::Completed.as_str());

        // Empty bv_id on multiple invalidated videos must not collide on UNIQUE(bv_id)
        let mut inv1 = sample_media(101, MediaState::Pending);
        inv1.bv_id = String::new();
        let mut inv2 = sample_media(102, MediaState::Pending);
        inv2.bv_id = String::new();
        db.upsert_medias([inv1, inv2]).await.unwrap();
        assert_eq!(db.get_media(101).await.unwrap().bv_id, "av101");
        assert_eq!(db.get_media(102).await.unwrap().bv_id, "av102");
    }

    #[tokio::test]
    async fn chunked_bulk_upserts_exceeding_chunk_size_and_empty_guards() {
        let db = Db::open_in_memory().await.unwrap();

        // Empty iterators are safe no-ops
        db.upsert_medias(Vec::new()).await.unwrap();
        db.upsert_sets(Vec::new()).await.unwrap();
        db.upsert_ups(Vec::new()).await.unwrap();
        db.upsert_media_sets(Vec::new()).await.unwrap();
        db.upsert_media_ups(Vec::new()).await.unwrap();
        db.upsert_up_feed_media_ups(Vec::new()).await.unwrap();
        db.upsert_set_accounts(Vec::new()).await.unwrap();
        db.upsert_up_accounts(Vec::new()).await.unwrap();

        // > 500 rows (e.g. 650) across chunks
        let count = 650;
        let medias: Vec<_> = (1..=count)
            .map(|i| sample_media(i, MediaState::Pending))
            .collect();
        db.upsert_medias(medias).await.unwrap();
        assert_eq!(db.all_medias().await.unwrap().len(), count as usize);

        db.upsert_sets([sample_set(1, "Huge Set", count, SetState::Active)])
            .await
            .unwrap();

        let links: Vec<_> = (1..=count)
            .map(|id| media_set::Model { id, set_id: 1 })
            .collect();
        db.upsert_media_sets(links).await.unwrap();
        assert_eq!(
            db.get_media_ids_of_set(1).await.unwrap().len(),
            count as usize
        );
    }

    #[tokio::test]
    async fn prune_all_order_dry_run_and_completed_preservation() {
        let db = Db::open_in_memory().await.unwrap();
        db.upsert_account(sample_account(1, AccountState::Active))
            .await
            .unwrap();

        // Set 10: active & linked to account 1
        // Set 20: orphan (no account link) -> should be pruned
        // Set 30: inactive & linked to account 1 -> medias only in set 30 are inactive
        db.upsert_sets([
            sample_set(10, "ActiveSet", 1, SetState::Active),
            sample_set(20, "OrphanSet", 1, SetState::Active),
            sample_set(30, "InactiveSet", 2, SetState::Inactive),
        ])
        .await
        .unwrap();
        db.sync_account_sets(
            1,
            vec![
                sample_set(10, "ActiveSet", 1, SetState::Active),
                sample_set(30, "InactiveSet", 2, SetState::Inactive),
            ],
            true,
        )
        .await
        .unwrap();

        // Up 50: unfollowed, linked ONLY to media 200 (which will be pruned!)
        // Because prune runs sets -> medias -> ups, up 50 MUST be pruned in ONE pass!
        db.upsert_ups([sample_up(50, "OrphanUp", UpState::Inactive)])
            .await
            .unwrap();

        // Media 100: in ActiveSet -> keep
        // Media 200: Pending, only in InactiveSet + OrphanUp -> prune!
        // Media 300: Completed, only in InactiveSet -> preserve unless purge_completed=true!
        db.upsert_medias([
            sample_media(100, MediaState::Pending),
            sample_media(200, MediaState::Pending),
            sample_media(300, MediaState::Completed),
        ])
        .await
        .unwrap();
        db.upsert_media_sets([
            media_set::Model {
                id: 100,
                set_id: 10,
            },
            media_set::Model {
                id: 200,
                set_id: 30,
            },
            media_set::Model {
                id: 300,
                set_id: 30,
            },
        ])
        .await
        .unwrap();
        db.upsert_media_ups([media_up::Model {
            id: 200,
            up_id: 50,
            from_up_feed: false,
        }])
        .await
        .unwrap();

        // Dry run must predict exact counts without mutating DB
        let dry = db.prune_all(false, true).await.unwrap();
        assert!(dry.dry_run);
        assert_eq!(dry.sets_pruned, 1); // Set 20
        assert_eq!(dry.medias_pruned, 1); // Media 200 (Media 300 preserved!)
        assert_eq!(dry.ups_pruned, 1); // Up 50 pruned in the SAME pass!
        assert_eq!(db.all_sets().await.unwrap().len(), 3);
        assert_eq!(db.all_medias().await.unwrap().len(), 3);
        assert_eq!(db.all_ups().await.unwrap().len(), 1);

        // Real prune preserving Completed
        let real = db.prune_all(false, false).await.unwrap();
        assert_eq!(real.sets_pruned, 1);
        assert_eq!(real.medias_pruned, 1);
        assert_eq!(real.ups_pruned, 1);
        assert!(db.get_media(100).await.is_ok());
        assert!(db.get_media(200).await.is_err());
        assert!(db.get_media(300).await.is_ok());
        assert!(db.get_up(50).await.is_err());

        // Real prune with purge_completed = true deletes Media 300
        let purged = db.prune_all(true, false).await.unwrap();
        assert_eq!(purged.medias_pruned, 1);
        assert!(db.get_media(300).await.is_err());
    }

    #[tokio::test]
    async fn sync_account_ups_and_unlink_stale_media_sets() {
        let db = Db::open_in_memory().await.unwrap();
        db.upsert_account(sample_account(1, AccountState::Active))
            .await
            .unwrap();

        let u1 = sample_up(1, "u1", UpState::Inactive);
        let u2 = sample_up(2, "u2", UpState::Inactive);
        db.sync_account_ups(1, vec![u1.clone(), u2], true)
            .await
            .unwrap();
        assert_eq!(db.get_up_ids_of_account(1).await.unwrap().len(), 2);

        // Partial failure (unlink_stale = false): must NOT drop u2
        let unlinked = db.sync_account_ups(1, vec![u1], false).await.unwrap();
        assert!(unlinked.is_empty());
        assert_eq!(db.get_up_ids_of_account(1).await.unwrap().len(), 2);

        // Following == 0 with unlink_stale = true: must unlink ALL
        let unlinked = db.sync_account_ups(1, vec![], true).await.unwrap();
        assert_eq!(unlinked.len(), 2);
        assert!(db.get_up_ids_of_account(1).await.unwrap().is_empty());

        // Unlink stale media_set entries
        db.upsert_sets([sample_set(7, "S", 2, SetState::Active)])
            .await
            .unwrap();
        db.upsert_medias([
            sample_media(1, MediaState::Pending),
            sample_media(2, MediaState::Pending),
        ])
        .await
        .unwrap();
        db.upsert_media_sets([
            media_set::Model { id: 1, set_id: 7 },
            media_set::Model { id: 2, set_id: 7 },
        ])
        .await
        .unwrap();
        let removed = db.unlink_stale_media_sets(7, &[1]).await.unwrap();
        assert_eq!(removed, 1);
        assert_eq!(db.get_media_ids_of_set(7).await.unwrap(), vec![1]);
    }

    #[tokio::test]
    async fn media_page_failure_reset_skip_filters_and_status() {
        let db = Db::open_in_memory().await.unwrap();
        db.upsert_account(sample_account(1, AccountState::Active))
            .await
            .unwrap();
        db.upsert_ups([sample_up(9, "CoolUp", UpState::Active)])
            .await
            .unwrap();
        db.upsert_up_accounts([up_account::Model {
            up_id: 9,
            account_id: 1,
        }])
        .await
        .unwrap();
        db.upsert_medias([
            sample_media(1, MediaState::Pending),
            sample_media(2, MediaState::Pending),
            sample_media(3, MediaState::Completed),
        ])
        .await
        .unwrap();
        db.upsert_media_ups([
            media_up::Model {
                id: 1,
                up_id: 9,
                from_up_feed: false,
            },
            media_up::Model {
                id: 2,
                up_id: 9,
                from_up_feed: false,
            },
        ])
        .await
        .unwrap();

        // MediaPage lifecycle
        db.mark_page_completed(
            1,
            1001,
            1,
            "P1".to_owned(),
            "1-Title 1(1)-P1.mp4".to_owned(),
            Some(4096),
            Some(999_001),
        )
        .await
        .unwrap();
        db.mark_page_failed(1, 1002, 2, "P2".to_owned())
            .await
            .unwrap();
        let pages = db.pages_for_media(1).await.unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].state, MediaPageState::Completed.as_str());
        assert_eq!(pages[0].file_size, Some(4096));
        assert_eq!(pages[0].file_inode, Some(999_001));
        assert_eq!(pages[1].state, MediaPageState::Failed.as_str());
        assert_eq!(pages[1].file_size, None);
        assert_eq!(pages[1].file_inode, None);
        assert_eq!(db.all_completed_pages().await.unwrap().len(), 1);

        // Failure tracking + reset + skip
        db.record_media_failure(1, "HTTP 503").await.unwrap();
        db.record_media_failure(1, "HTTP 502").await.unwrap();
        let m1 = db.get_media(1).await.unwrap();
        assert_eq!(m1.state, MediaState::Failed.as_str());
        assert_eq!(m1.retry_count, 2);
        assert_eq!(m1.last_error.as_deref(), Some("HTTP 502"));

        let skipped = db.skip_medias(&[2]).await.unwrap();
        assert_eq!(skipped, 1);
        assert_eq!(
            db.get_media(2).await.unwrap().state,
            MediaState::Skipped.as_str()
        );

        let reset = db
            .reset_media_states(&MediaResetFilter {
                failed: true,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(reset, 1);
        let m1 = db.get_media(1).await.unwrap();
        assert_eq!(m1.state, MediaState::Pending.as_str());
        assert_eq!(m1.retry_count, 0);
        assert!(m1.last_error.is_none());

        // Clear NotFound errors instead of RecordNotUpdated
        let err = db.activate_set(99999).await.unwrap_err();
        assert!(err.to_string().contains("Unknown set<99999>"));

        // Query filter & status summary
        let filtered = db
            .query_medias(&MediaFilter {
                up_id: Some(9),
                search: Some("Title 1".to_owned()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, 1);

        let summary = db.status_summary(5).await.unwrap();
        assert_eq!(summary.accounts.active, 1);
        assert_eq!(summary.medias.total, 3);
        assert_eq!(summary.medias.pending, 1);
        assert_eq!(summary.medias.skipped, 1);
        assert_eq!(summary.medias.completed, 1);
        assert_eq!(summary.top_ups.len(), 1);
        assert_eq!(summary.top_ups[0].up_id, 9);
        assert_eq!(summary.top_ups[0].media_count, 2);
    }

    #[tokio::test]
    async fn migration_3_backfills_sync_complete_for_existing_populated_sets_and_ups() {
        let mut opts = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT);
        opts = opts.disable_statement_logging();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        let conn = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);

        // Apply only migrations #1 and #2 to simulate an existing v1.3.0 user database.
        Migrator::up(&conn, Some(2)).await.unwrap();

        for sql in [
            r#"INSERT INTO "set" (set_id, name, count, state) VALUES (1, 'PopulatedSet', 1, 'Active');"#,
            r#"INSERT INTO "set" (set_id, name, count, state) VALUES (2, 'EmptySet', 0, 'Inactive');"#,
            r#"INSERT INTO "up" (up_id, name, state) VALUES (10, 'PopulatedUp', 'Active');"#,
            r#"INSERT INTO "up" (up_id, name, state) VALUES (20, 'EmptyUp', 'Inactive');"#,
            r#"INSERT INTO media (id, bv_id, title, type, state, retry_count) VALUES (100, 'BV1backfill001', 'M', 'Video', 'Pending', 0);"#,
            r#"INSERT INTO media_set (id, set_id) VALUES (100, 1);"#,
            r#"INSERT INTO media_up (id, up_id) VALUES (100, 10);"#,
        ] {
            conn.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
                .await
                .unwrap();
        }

        // Now apply migration #3.
        Migrator::up(&conn, None).await.unwrap();
        let db = Db { conn };

        let s1 = db.get_set(1).await.unwrap();
        assert_eq!(s1.kind, SetKind::Created.as_str());
        assert_eq!(s1.owner_mid, None);
        assert!(s1.sync_complete);
        assert_eq!(s1.resume_page, None);

        let s2 = db.get_set(2).await.unwrap();
        assert_eq!(s2.kind, SetKind::Created.as_str());
        assert!(!s2.sync_complete);
        assert_eq!(s2.resume_page, None);

        let u10 = db.get_up(10).await.unwrap();
        assert!(u10.sync_complete);
        assert_eq!(u10.resume_page, None);

        let u20 = db.get_up(20).await.unwrap();
        assert!(!u20.sync_complete);
        assert_eq!(u20.resume_page, None);
    }

    #[tokio::test]
    async fn active_media_condition_and_pull_candidates_require_active_account_and_resource() {
        let db = Db::open_in_memory().await.unwrap();

        db.upsert_account(sample_account(1, AccountState::Active))
            .await
            .unwrap();
        db.upsert_account(sample_account(2, AccountState::Inactive))
            .await
            .unwrap();

        // Sets: 10 (Active, acct 1), 11 (Inactive, acct 1), 20 (Active, acct 2), 30 (Active, orphan)
        db.upsert_sets([
            sample_set(10, "ActiveSetActiveAcct", 1, SetState::Active),
            sample_set(11, "InactiveSetActiveAcct", 1, SetState::Inactive),
            sample_set(20, "ActiveSetInactiveAcct", 1, SetState::Active),
            sample_set(30, "ActiveSetOrphan", 1, SetState::Active),
        ])
        .await
        .unwrap();
        db.upsert_set_accounts([
            set_account::Model {
                set_id: 10,
                account_id: 1,
            },
            set_account::Model {
                set_id: 11,
                account_id: 1,
            },
            set_account::Model {
                set_id: 20,
                account_id: 2,
            },
        ])
        .await
        .unwrap();

        // UPs: 100 (Active, acct 1), 101 (Inactive, acct 1), 200 (Active, acct 2), 300 (Active, orphan)
        db.upsert_ups([
            sample_up(100, "ActiveUpActiveAcct", UpState::Active),
            sample_up(101, "InactiveUpActiveAcct", UpState::Inactive),
            sample_up(200, "ActiveUpInactiveAcct", UpState::Active),
            sample_up(300, "ActiveUpOrphan", UpState::Active),
        ])
        .await
        .unwrap();
        db.upsert_up_accounts([
            up_account::Model {
                up_id: 100,
                account_id: 1,
            },
            up_account::Model {
                up_id: 101,
                account_id: 1,
            },
            up_account::Model {
                up_id: 200,
                account_id: 2,
            },
        ])
        .await
        .unwrap();

        // Active-account ownership helpers
        assert!(db.is_set_owned_by_active_account(10).await.unwrap());
        assert!(db.is_set_owned_by_active_account(11).await.unwrap());
        assert!(!db.is_set_owned_by_active_account(20).await.unwrap());
        assert!(!db.is_set_owned_by_active_account(30).await.unwrap());
        assert_eq!(
            db.active_account_set_ids().await.unwrap(),
            HashSet::from([10, 11])
        );

        assert!(db.is_up_followed_by_active_account(100).await.unwrap());
        assert!(db.is_up_followed_by_active_account(101).await.unwrap());
        assert!(!db.is_up_followed_by_active_account(200).await.unwrap());
        assert!(!db.is_up_followed_by_active_account(300).await.unwrap());
        assert_eq!(
            db.active_account_up_ids().await.unwrap(),
            HashSet::from([100, 101])
        );

        // Medias across all combinations + two Completed medias (active vs inactive)
        db.upsert_medias([
            sample_media(1, MediaState::Pending),
            sample_media(2, MediaState::Pending),
            sample_media(3, MediaState::Pending),
            sample_media(4, MediaState::Pending),
            sample_media(5, MediaState::Pending),
            sample_media(6, MediaState::Pending),
            sample_media(7, MediaState::Pending),
            sample_media(8, MediaState::Pending),
            sample_media(9, MediaState::Completed),
            sample_media(10, MediaState::Completed),
        ])
        .await
        .unwrap();
        db.upsert_media_sets([
            media_set::Model { id: 1, set_id: 10 },
            media_set::Model { id: 2, set_id: 11 },
            media_set::Model { id: 5, set_id: 20 },
            media_set::Model { id: 7, set_id: 30 },
            media_set::Model { id: 9, set_id: 10 },
            media_set::Model { id: 10, set_id: 20 },
        ])
        .await
        .unwrap();
        db.upsert_media_ups([
            media_up::Model {
                id: 3,
                up_id: 100,
                from_up_feed: false,
            },
            media_up::Model {
                id: 4,
                up_id: 101,
                from_up_feed: false,
            },
            media_up::Model {
                id: 6,
                up_id: 200,
                from_up_feed: false,
            },
            media_up::Model {
                id: 8,
                up_id: 300,
                from_up_feed: false,
            },
        ])
        .await
        .unwrap();

        // Only medias 1, 3, and 9 are active!
        let active_ids: Vec<i64> = db
            .all_active_medias()
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(active_ids, vec![1, 3, 9]);
        assert_eq!(
            db.active_media_ids().await.unwrap(),
            HashSet::from([1, 3, 9])
        );
        assert_eq!(db.count_active_pending_medias().await.unwrap(), 2);
        assert_eq!(
            db.count_active_medias_in_state(MediaState::Completed)
                .await
                .unwrap(),
            1
        );
        assert_eq!(db.count_active_media_ids(&[]).await.unwrap(), 0);
        assert_eq!(
            db.count_active_media_ids(&[1, 1, 2, 3, 4, 9, 10])
                .await
                .unwrap(),
            3
        );
        assert!(db.is_media_active(1).await.unwrap());
        assert!(db.is_media_active(3).await.unwrap());
        assert!(db.is_media_active(9).await.unwrap());
        for inactive_id in [2, 4, 5, 6, 7, 8, 10] {
            assert!(
                !db.is_media_active(inactive_id).await.unwrap(),
                "media {inactive_id} must not be active"
            );
        }

        // completed_medias respects active_only
        let completed_active: Vec<i64> = db
            .completed_medias(true)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(completed_active, vec![9]);
        let completed_all: Vec<i64> = db
            .completed_medias(false)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(completed_all, vec![9, 10]);

        // pull_candidates ALWAYS enforces active_media_condition even with explicit ids / set_ids / up_ids!
        let all_pull: Vec<i64> = db
            .pull_candidates(&PullFilter::default())
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(all_pull, vec![1, 3]);

        let by_ids: Vec<i64> = db
            .pull_candidates(&PullFilter {
                ids: vec![1, 2, 5, 6, 7, 8],
                ..Default::default()
            })
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(by_ids, vec![1]);

        assert!(
            db.pull_candidates(&PullFilter {
                set_ids: vec![20],
                ..Default::default()
            })
            .await
            .unwrap()
            .is_empty(),
            "active set owned by inactive account must return no pull candidates"
        );
        assert!(
            db.pull_candidates(&PullFilter {
                set_ids: vec![11],
                ..Default::default()
            })
            .await
            .unwrap()
            .is_empty(),
            "inactive set must return no pull candidates"
        );
        assert!(
            db.pull_candidates(&PullFilter {
                up_ids: vec![200],
                ..Default::default()
            })
            .await
            .unwrap()
            .is_empty(),
            "active UP followed by inactive account must return no pull candidates"
        );
        let active_up_pull: Vec<i64> = db
            .pull_candidates(&PullFilter {
                up_ids: vec![100],
                ..Default::default()
            })
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(active_up_pull, vec![3]);

        // Deactivating account 1 immediately hides medias 1, 3, and 9!
        db.deactivate_account(1).await.unwrap();
        assert!(db.all_active_medias().await.unwrap().is_empty());
        assert!(
            db.pull_candidates(&PullFilter {
                ids: vec![1, 3],
                force: true,
                ..Default::default()
            })
            .await
            .unwrap()
            .is_empty()
        );
        assert!(!db.is_media_active(1).await.unwrap());
        assert!(!db.is_set_owned_by_active_account(10).await.unwrap());
        assert!(!db.is_up_followed_by_active_account(100).await.unwrap());
    }

    #[tokio::test]
    async fn set_kind_filter_sync_progress_cursors_and_unlink_stale() {
        let db = Db::open_in_memory().await.unwrap();
        db.upsert_account(sample_account(1, AccountState::Active))
            .await
            .unwrap();

        let s1 = set::Model {
            set_id: 1,
            name: "MyFav".to_owned(),
            count: 10,
            state: SetState::Inactive.to_string(),
            kind: SetKind::Created.to_string(),
            owner_mid: Some(1),
            sync_complete: false,
            resume_page: None,
        };
        let s2 = set::Model {
            set_id: 2,
            name: "CollectedFolder".to_owned(),
            count: 5,
            state: SetState::Inactive.to_string(),
            kind: SetKind::Collected.to_string(),
            owner_mid: Some(999),
            sync_complete: false,
            resume_page: None,
        };
        let s3 = set::Model {
            set_id: 3,
            name: "VideoSeason".to_owned(),
            count: 12,
            state: SetState::Inactive.to_string(),
            kind: SetKind::Season.to_string(),
            owner_mid: Some(888),
            sync_complete: false,
            resume_page: None,
        };

        db.sync_account_sets(1, vec![s1.clone(), s2.clone(), s3.clone()], true)
            .await
            .unwrap();

        // Filter by SetKind
        let seasons = db
            .query_sets(&SetFilter {
                kind: Some(SetKind::Season),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(seasons.len(), 1);
        assert_eq!(seasons[0].set_id, 3);
        assert_eq!(seasons[0].owner_mid, Some(888));

        let collected = db
            .query_sets(&SetFilter {
                kind: Some(SetKind::Collected),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].set_id, 2);

        // Activate set 1 and advance sync cursor, then re-upsert via sync_account_sets:
        // state, sync_complete, and resume_page MUST be preserved while name/count/kind/owner_mid update!
        db.activate_set(1).await.unwrap();
        db.set_set_sync_progress(1, false, Some(3)).await.unwrap();
        let after_cursor = db.get_set(1).await.unwrap();
        assert!(!after_cursor.sync_complete);
        assert_eq!(after_cursor.resume_page, Some(3));

        let mut updated_s1 = s1.clone();
        updated_s1.name = "RenamedFav".to_owned();
        updated_s1.count = 15;
        updated_s1.owner_mid = Some(42);
        // unlink_stale = false must NOT drop s2 or s3
        let unlinked = db
            .sync_account_sets(1, vec![updated_s1], false)
            .await
            .unwrap();
        assert!(unlinked.is_empty());
        assert_eq!(db.get_set_ids_of_account(1).await.unwrap().len(), 3);

        let reloaded = db.get_set(1).await.unwrap();
        assert_eq!(reloaded.name, "RenamedFav");
        assert_eq!(reloaded.count, 15);
        assert_eq!(reloaded.owner_mid, Some(42));
        assert_eq!(reloaded.state, SetState::Active.as_str());
        assert!(!reloaded.sync_complete);
        assert_eq!(reloaded.resume_page, Some(3));

        // Mark sync complete and unlink stale sets when unlink_stale = true
        db.set_set_sync_progress(1, true, None).await.unwrap();
        let completed_set = db.get_set(1).await.unwrap();
        assert!(completed_set.sync_complete);
        assert_eq!(completed_set.resume_page, None);

        let unlinked = db.sync_account_sets(1, vec![s1], true).await.unwrap();
        assert_eq!(unlinked.len(), 2);
        assert_eq!(db.get_set_ids_of_account(1).await.unwrap(), vec![1]);

        // UP sync progress cursor + preservation on upsert_ups
        db.upsert_ups([sample_up(50, "Creator", UpState::Inactive)])
            .await
            .unwrap();
        db.activate_up(50).await.unwrap();
        db.set_up_sync_progress(50, false, Some(4)).await.unwrap();
        db.upsert_ups([sample_up(50, "CreatorRenamed", UpState::Inactive)])
            .await
            .unwrap();
        let reloaded_up = db.get_up(50).await.unwrap();
        assert_eq!(reloaded_up.name, "CreatorRenamed");
        assert_eq!(reloaded_up.state, UpState::Active.as_str());
        assert!(!reloaded_up.sync_complete);
        assert_eq!(reloaded_up.resume_page, Some(4));

        db.set_up_sync_progress(50, true, None).await.unwrap();
        let done_up = db.get_up(50).await.unwrap();
        assert!(done_up.sync_complete);
        assert_eq!(done_up.resume_page, None);

        // Unknown IDs return clear errors
        assert!(db.set_set_sync_progress(99999, true, None).await.is_err());
        assert!(db.set_up_sync_progress(99999, true, None).await.is_err());
    }

    #[tokio::test]
    async fn media_page_relink_surgical_reset_and_batch_pages_query() {
        let db = Db::open_in_memory().await.unwrap();
        db.upsert_medias([
            sample_media(1, MediaState::Completed),
            sample_media(2, MediaState::Completed),
        ])
        .await
        .unwrap();

        db.mark_page_completed(
            1,
            101,
            1,
            "P1".to_owned(),
            "1-Title 1(1)-P1.mp4".to_owned(),
            Some(1000),
            Some(111),
        )
        .await
        .unwrap();
        db.mark_page_completed(
            1,
            102,
            2,
            "P2".to_owned(),
            "1-Title 1(2)-P2.mp4".to_owned(),
            Some(2000),
            Some(222),
        )
        .await
        .unwrap();
        db.mark_page_completed(
            2,
            201,
            1,
            "Only".to_owned(),
            "2-Title 2.mp4".to_owned(),
            Some(3000),
            Some(333),
        )
        .await
        .unwrap();

        // Batch query across multiple media IDs is deterministically ordered
        assert!(db.pages_for_media_ids(&[]).await.unwrap().is_empty());
        let batched = db.pages_for_media_ids(&[2, 1]).await.unwrap();
        let keys: Vec<(i64, i64)> = batched.iter().map(|p| (p.media_id, p.cid)).collect();
        assert_eq!(keys, vec![(1, 101), (1, 102), (2, 201)]);

        // Relink relocated file updates local_path, file_size, and file_inode
        db.update_page_local_path(
            1,
            101,
            "renamed/my_video.mp4".to_owned(),
            Some(1024),
            Some(444),
        )
        .await
        .unwrap();
        let p1 = db
            .pages_for_media(1)
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.cid == 101)
            .unwrap();
        assert_eq!(p1.local_path.as_deref(), Some("renamed/my_video.mp4"));
        assert_eq!(p1.file_size, Some(1024));
        assert_eq!(p1.file_inode, Some(444));
        assert_eq!(p1.state, MediaPageState::Completed.as_str());

        assert!(
            db.update_page_local_path(999, 999, "nope.mp4".to_owned(), None, None)
                .await
                .is_err()
        );

        // Surgical per-page reset leaves P1 of media 1 and P1 of media 2 Completed!
        assert_eq!(db.reset_specific_pages(&[]).await.unwrap(), 0);
        let reset_count = db
            .reset_specific_pages(&[(1, 102), (1, 102)])
            .await
            .unwrap();
        assert_eq!(reset_count, 1);

        let m1_pages = db.pages_for_media(1).await.unwrap();
        assert_eq!(m1_pages[0].cid, 101);
        assert_eq!(m1_pages[0].state, MediaPageState::Completed.as_str());
        assert_eq!(
            m1_pages[0].local_path.as_deref(),
            Some("renamed/my_video.mp4")
        );
        assert_eq!(m1_pages[0].file_size, Some(1024));
        assert_eq!(m1_pages[0].file_inode, Some(444));

        assert_eq!(m1_pages[1].cid, 102);
        assert_eq!(m1_pages[1].state, MediaPageState::Pending.as_str());
        assert_eq!(m1_pages[1].local_path, None);
        assert_eq!(m1_pages[1].downloaded_at, None);
        assert_eq!(m1_pages[1].file_size, None);
        assert_eq!(m1_pages[1].file_inode, None);

        let m2_pages = db.pages_for_media(2).await.unwrap();
        assert_eq!(m2_pages[0].state, MediaPageState::Completed.as_str());
        assert_eq!(m2_pages[0].file_inode, Some(333));
    }

    #[tokio::test]
    async fn migration_4_backfill_roundtrip_and_up_feed_monotonicity() {
        let mut opts = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT);
        opts = opts.disable_statement_logging();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        let conn = SqlxSqliteConnector::from_sqlx_sqlite_pool(pool);

        // Apply migrations #1 and #2, seed Active, Inactive, and Deactivated UPs with media_up links.
        Migrator::up(&conn, Some(2)).await.unwrap();
        for sql in [
            r#"INSERT INTO "up" (up_id, name, state) VALUES (10, 'ActiveUp', 'Active');"#,
            r#"INSERT INTO "up" (up_id, name, state) VALUES (11, 'ActiveUnsyncedUp', 'Active');"#,
            r#"INSERT INTO "up" (up_id, name, state) VALUES (12, 'ActiveResumingUp', 'Active');"#,
            r#"INSERT INTO "up" (up_id, name, state) VALUES (20, 'InactiveFavCreator', 'Inactive');"#,
            r#"INSERT INTO "up" (up_id, name, state) VALUES (30, 'DeactivatedUp', 'Deactivated');"#,
            r#"INSERT INTO media (id, bv_id, title, type, state, retry_count) VALUES (100, 'BV1m4test00100', 'M100', 'Video', 'Pending', 0);"#,
            r#"INSERT INTO media (id, bv_id, title, type, state, retry_count) VALUES (101, 'BV1m4test00101', 'M101', 'Video', 'Pending', 0);"#,
            r#"INSERT INTO media (id, bv_id, title, type, state, retry_count) VALUES (102, 'BV1m4test00102', 'M102', 'Video', 'Pending', 0);"#,
            r#"INSERT INTO media (id, bv_id, title, type, state, retry_count) VALUES (103, 'BV1m4test00103', 'M103', 'Video', 'Pending', 0);"#,
            r#"INSERT INTO media (id, bv_id, title, type, state, retry_count) VALUES (104, 'BV1m4test00104', 'M104', 'Video', 'Pending', 0);"#,
            r#"INSERT INTO media_up (id, up_id) VALUES (100, 10);"#,
            r#"INSERT INTO media_up (id, up_id) VALUES (101, 20);"#,
            r#"INSERT INTO media_up (id, up_id) VALUES (102, 30);"#,
        ] {
            conn.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
                .await
                .unwrap();
        }

        // Apply Migration #3 only: it marks UPs with pre-existing media_up rows (10, 20, 30)
        // sync_complete = 1, while UPs 11 and 12 stay sync_complete = 0.
        Migrator::up(&conn, Some(1)).await.unwrap();
        for sql in [
            // Active UP 11 has sync_complete = 0, resume_page = NULL, with a favorite-set media_up link.
            r#"INSERT INTO media_up (id, up_id) VALUES (103, 11);"#,
            // Active UP 12 has sync_complete = 0, resume_page = 2 (in-progress UP feed backfill).
            r#"INSERT INTO media_up (id, up_id) VALUES (104, 12);"#,
            r#"UPDATE "up" SET resume_page = 2 WHERE up_id = 12;"#,
            r#"UPDATE "up" SET resume_page = 3 WHERE up_id IN (20, 30);"#,
        ] {
            conn.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
                .await
                .unwrap();
        }

        // Apply Migration #4: backfills from_up_feed = 1 only for Active UPs with
        // (sync_complete = 1 OR resume_page IS NOT NULL), and resets non-Active UPs.
        Migrator::up(&conn, None).await.unwrap();
        let db = Db { conn };
        db.check_integrity().await.unwrap();

        let u10 = db.get_up(10).await.unwrap();
        assert!(u10.sync_complete);
        assert_eq!(u10.resume_page, None);
        assert_eq!(db.get_up_feed_media_ids(10).await.unwrap(), vec![100]);

        let u11 = db.get_up(11).await.unwrap();
        assert!(!u11.sync_complete);
        assert_eq!(u11.resume_page, None);
        assert!(db.get_up_feed_media_ids(11).await.unwrap().is_empty());
        assert_eq!(db.get_up_ids_of_media(103).await.unwrap(), vec![11]);

        let u12 = db.get_up(12).await.unwrap();
        assert!(!u12.sync_complete);
        assert_eq!(u12.resume_page, Some(2));
        assert_eq!(db.get_up_feed_media_ids(12).await.unwrap(), vec![104]);

        let u20 = db.get_up(20).await.unwrap();
        assert!(!u20.sync_complete);
        assert_eq!(u20.resume_page, None);
        assert!(db.get_up_feed_media_ids(20).await.unwrap().is_empty());
        assert_eq!(db.get_up_ids_of_media(101).await.unwrap(), vec![20]);

        let u30 = db.get_up(30).await.unwrap();
        assert!(!u30.sync_complete);
        assert_eq!(u30.resume_page, None);
        assert!(db.get_up_feed_media_ids(30).await.unwrap().is_empty());
        assert_eq!(db.get_up_ids_of_media(102).await.unwrap(), vec![30]);

        // Verify upsert_media_ups vs upsert_up_feed_media_ups monotonicity (false -> true, never true -> false).
        db.upsert_medias([
            sample_media(200, MediaState::Pending),
            sample_media(201, MediaState::Pending),
            sample_media(202, MediaState::Pending),
        ])
        .await
        .unwrap();

        // Favorite-set / backfill insert sets from_up_feed = false.
        db.upsert_media_ups([media_up::Model {
            id: 200,
            up_id: 10,
            from_up_feed: false,
        }])
        .await
        .unwrap();
        assert_eq!(db.get_up_ids_of_media(200).await.unwrap(), vec![10]);
        assert_eq!(db.get_up_feed_media_ids(10).await.unwrap(), vec![100]);

        // UP-feed upsert upgrades existing (200, 10) false -> true, inserts (201, 10) as true,
        // and enforces true even if caller passes from_up_feed = false.
        db.upsert_up_feed_media_ups([
            media_up::Model {
                id: 200,
                up_id: 10,
                from_up_feed: true,
            },
            media_up::Model {
                id: 201,
                up_id: 10,
                from_up_feed: true,
            },
            media_up::Model {
                id: 202,
                up_id: 10,
                from_up_feed: false,
            },
        ])
        .await
        .unwrap();
        let mut feed_ids = db.get_up_feed_media_ids(10).await.unwrap();
        feed_ids.sort_unstable();
        assert_eq!(feed_ids, vec![100, 200, 201, 202]);

        // Subsequent favorite-set / backfill upsert with from_up_feed = false MUST NOT downgrade true -> false.
        db.upsert_media_ups([
            media_up::Model {
                id: 200,
                up_id: 10,
                from_up_feed: false,
            },
            media_up::Model {
                id: 201,
                up_id: 10,
                from_up_feed: false,
            },
        ])
        .await
        .unwrap();
        let mut feed_ids_after = db.get_up_feed_media_ids(10).await.unwrap();
        feed_ids_after.sort_unstable();
        assert_eq!(feed_ids_after, vec![100, 200, 201, 202]);

        // Verify Migration #4 down/up roundtrip cleanly drops and re-adds from_up_feed.
        Migrator::down(db.conn(), Some(1)).await.unwrap();
        Migrator::up(db.conn(), None).await.unwrap();
        db.check_integrity().await.unwrap();
        let mut feed_ids_roundtrip = db.get_up_feed_media_ids(10).await.unwrap();
        feed_ids_roundtrip.sort_unstable();
        assert_eq!(feed_ids_roundtrip, vec![100, 200, 201, 202]);
        assert!(db.get_up_feed_media_ids(11).await.unwrap().is_empty());
        assert_eq!(db.get_up_feed_media_ids(12).await.unwrap(), vec![104]);
        assert!(db.get_up_feed_media_ids(20).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn safe_bulk_activate_and_deactivate_state_transitions() {
        let db = Db::open_in_memory().await.unwrap();

        // Accounts: Active (1), Inactive (2), Expired (3)
        db.upsert_account(sample_account(1, AccountState::Active))
            .await
            .unwrap();
        db.upsert_account(sample_account(2, AccountState::Inactive))
            .await
            .unwrap();
        db.upsert_account(sample_account(3, AccountState::Expired))
            .await
            .unwrap();

        // Sets:
        // 10: Inactive, owned by Active acct 1 -> should activate
        // 11: Active, owned by Active acct 1 -> already Active (unchanged, not recounted)
        // 12: Unreachable, owned by Active acct 1 -> MUST stay Unreachable
        // 13: Inactive, owned only by Inactive acct 2 -> MUST stay Inactive until acct 2 is active
        // 14: Inactive, owned only by Expired acct 3 -> MUST stay Inactive
        // 15: Inactive, shared by Inactive acct 2 and Active acct 1 -> should activate
        // 16: Inactive, orphan (no set_account link) -> MUST stay Inactive
        // 17: Active, owned only by Inactive acct 2 -> stays Active on activate_all, deactivates on deactivate_all
        db.upsert_sets([
            sample_set(10, "InactiveActiveAcct", 1, SetState::Inactive),
            sample_set(11, "ActiveActiveAcct", 1, SetState::Active),
            sample_set(12, "UnreachableActiveAcct", 1, SetState::Unreachable),
            sample_set(13, "InactiveInactiveAcct", 1, SetState::Inactive),
            sample_set(14, "InactiveExpiredAcct", 1, SetState::Inactive),
            sample_set(15, "InactiveShared", 1, SetState::Inactive),
            sample_set(16, "InactiveOrphan", 1, SetState::Inactive),
            sample_set(17, "ActiveInactiveAcct", 1, SetState::Active),
        ])
        .await
        .unwrap();
        db.upsert_set_accounts([
            set_account::Model {
                set_id: 10,
                account_id: 1,
            },
            set_account::Model {
                set_id: 11,
                account_id: 1,
            },
            set_account::Model {
                set_id: 12,
                account_id: 1,
            },
            set_account::Model {
                set_id: 13,
                account_id: 2,
            },
            set_account::Model {
                set_id: 14,
                account_id: 3,
            },
            set_account::Model {
                set_id: 15,
                account_id: 2,
            },
            set_account::Model {
                set_id: 15,
                account_id: 1,
            },
            set_account::Model {
                set_id: 17,
                account_id: 2,
            },
        ])
        .await
        .unwrap();

        // UPs:
        // 100: Inactive, followed by Active acct 1 -> should activate
        // 101: Active, followed by Active acct 1 -> already Active (unchanged)
        // 102: Deactivated, followed by Active acct 1 -> MUST stay Deactivated
        // 103: Inactive, followed only by Inactive acct 2 -> MUST stay Inactive
        // 104: Inactive, followed only by Expired acct 3 -> MUST stay Inactive
        // 105: Inactive, unfollowed (discovered via favorite set) -> MUST stay Inactive
        // 106: Inactive, shared by Inactive acct 2 and Active acct 1 -> should activate
        // 107: Active, unfollowed -> stays Active on activate_all, deactivates on deactivate_all
        db.upsert_ups([
            sample_up(100, "InactiveFollowedActive", UpState::Inactive),
            sample_up(101, "ActiveFollowedActive", UpState::Active),
            sample_up(102, "DeactivatedFollowedActive", UpState::Deactivated),
            sample_up(103, "InactiveFollowedInactive", UpState::Inactive),
            sample_up(104, "InactiveFollowedExpired", UpState::Inactive),
            sample_up(105, "InactiveUnfollowed", UpState::Inactive),
            sample_up(106, "InactiveShared", UpState::Inactive),
            sample_up(107, "ActiveUnfollowed", UpState::Active),
        ])
        .await
        .unwrap();
        db.upsert_up_accounts([
            up_account::Model {
                up_id: 100,
                account_id: 1,
            },
            up_account::Model {
                up_id: 101,
                account_id: 1,
            },
            up_account::Model {
                up_id: 102,
                account_id: 1,
            },
            up_account::Model {
                up_id: 103,
                account_id: 2,
            },
            up_account::Model {
                up_id: 104,
                account_id: 3,
            },
            up_account::Model {
                up_id: 106,
                account_id: 2,
            },
            up_account::Model {
                up_id: 106,
                account_id: 1,
            },
        ])
        .await
        .unwrap();

        // 1. activate_all_sets only activates Inactive sets owned by Active accounts (10 and 15).
        assert_eq!(db.activate_all_sets().await.unwrap(), 2);
        assert_eq!(
            db.get_set(10).await.unwrap().state,
            SetState::Active.as_str()
        );
        assert_eq!(
            db.get_set(11).await.unwrap().state,
            SetState::Active.as_str()
        );
        assert_eq!(
            db.get_set(12).await.unwrap().state,
            SetState::Unreachable.as_str()
        );
        assert_eq!(
            db.get_set(13).await.unwrap().state,
            SetState::Inactive.as_str()
        );
        assert_eq!(
            db.get_set(14).await.unwrap().state,
            SetState::Inactive.as_str()
        );
        assert_eq!(
            db.get_set(15).await.unwrap().state,
            SetState::Active.as_str()
        );
        assert_eq!(
            db.get_set(16).await.unwrap().state,
            SetState::Inactive.as_str()
        );
        assert_eq!(
            db.get_set(17).await.unwrap().state,
            SetState::Active.as_str()
        );
        // Second call is idempotent (0 newly activated).
        assert_eq!(db.activate_all_sets().await.unwrap(), 0);

        // 2. activate_all_ups only activates Inactive UPs followed by Active accounts (100 and 106).
        assert_eq!(db.activate_all_ups().await.unwrap(), 2);
        assert_eq!(
            db.get_up(100).await.unwrap().state,
            UpState::Active.as_str()
        );
        assert_eq!(
            db.get_up(101).await.unwrap().state,
            UpState::Active.as_str()
        );
        assert_eq!(
            db.get_up(102).await.unwrap().state,
            UpState::Deactivated.as_str()
        );
        assert_eq!(
            db.get_up(103).await.unwrap().state,
            UpState::Inactive.as_str()
        );
        assert_eq!(
            db.get_up(104).await.unwrap().state,
            UpState::Inactive.as_str()
        );
        assert_eq!(
            db.get_up(105).await.unwrap().state,
            UpState::Inactive.as_str()
        );
        assert_eq!(
            db.get_up(106).await.unwrap().state,
            UpState::Active.as_str()
        );
        assert_eq!(
            db.get_up(107).await.unwrap().state,
            UpState::Active.as_str()
        );
        assert_eq!(db.activate_all_ups().await.unwrap(), 0);

        // 3. activate_all_accounts transitions Inactive (2) -> Active, preserving Expired (3).
        assert_eq!(db.activate_all_accounts().await.unwrap(), 1);
        assert_eq!(
            db.get_account(1).await.unwrap().state,
            AccountState::Active.as_str()
        );
        assert_eq!(
            db.get_account(2).await.unwrap().state,
            AccountState::Active.as_str()
        );
        assert_eq!(
            db.get_account(3).await.unwrap().state,
            AccountState::Expired.as_str()
        );
        assert_eq!(db.activate_all_accounts().await.unwrap(), 0);

        // Now that acct 2 is Active, set 13 and UP 103 become eligible; Expired acct 3's set 14 / UP 104
        // and orphan set 16 / unfollowed UP 105 remain Inactive.
        assert_eq!(db.activate_all_sets().await.unwrap(), 1);
        assert_eq!(
            db.get_set(13).await.unwrap().state,
            SetState::Active.as_str()
        );
        assert_eq!(
            db.get_set(14).await.unwrap().state,
            SetState::Inactive.as_str()
        );
        assert_eq!(
            db.get_set(16).await.unwrap().state,
            SetState::Inactive.as_str()
        );

        assert_eq!(db.activate_all_ups().await.unwrap(), 1);
        assert_eq!(
            db.get_up(103).await.unwrap().state,
            UpState::Active.as_str()
        );
        assert_eq!(
            db.get_up(104).await.unwrap().state,
            UpState::Inactive.as_str()
        );
        assert_eq!(
            db.get_up(105).await.unwrap().state,
            UpState::Inactive.as_str()
        );

        // 4. deactivate_all_* transitions ONLY Active -> Inactive, preserving Expired/Unreachable/Deactivated.
        // Active sets: 10, 11, 13, 15, 17 (5 total)
        assert_eq!(db.deactivate_all_sets().await.unwrap(), 5);
        for id in [10, 11, 13, 14, 15, 16, 17] {
            assert_eq!(
                db.get_set(id).await.unwrap().state,
                SetState::Inactive.as_str(),
                "set {id} should be Inactive"
            );
        }
        assert_eq!(
            db.get_set(12).await.unwrap().state,
            SetState::Unreachable.as_str()
        );
        assert_eq!(db.deactivate_all_sets().await.unwrap(), 0);

        // Active UPs: 100, 101, 103, 106, 107 (5 total, including Active unfollowed UP 107)
        assert_eq!(db.deactivate_all_ups().await.unwrap(), 5);
        for id in [100, 101, 103, 104, 105, 106, 107] {
            assert_eq!(
                db.get_up(id).await.unwrap().state,
                UpState::Inactive.as_str(),
                "UP {id} should be Inactive"
            );
        }
        assert_eq!(
            db.get_up(102).await.unwrap().state,
            UpState::Deactivated.as_str()
        );
        assert_eq!(db.deactivate_all_ups().await.unwrap(), 0);

        // Active accounts: 1, 2 (2 total); Expired account 3 MUST remain Expired.
        assert_eq!(db.deactivate_all_accounts().await.unwrap(), 2);
        assert_eq!(
            db.get_account(1).await.unwrap().state,
            AccountState::Inactive.as_str()
        );
        assert_eq!(
            db.get_account(2).await.unwrap().state,
            AccountState::Inactive.as_str()
        );
        assert_eq!(
            db.get_account(3).await.unwrap().state,
            AccountState::Expired.as_str()
        );
        assert_eq!(db.deactivate_all_accounts().await.unwrap(), 0);

        // With 0 active accounts, activate_all_sets and activate_all_ups must activate 0 rows.
        assert_eq!(db.activate_all_sets().await.unwrap(), 0);
        assert_eq!(db.activate_all_ups().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn pull_candidates_combines_dimensions_and_filters_bulk_force_safely() {
        let db = Db::open_in_memory().await.unwrap();

        db.upsert_account(sample_account(1, AccountState::Active))
            .await
            .unwrap();
        db.upsert_sets([
            sample_set(10, "Set10", 4, SetState::Active),
            sample_set(20, "Set20", 4, SetState::Active),
        ])
        .await
        .unwrap();
        db.upsert_set_accounts([
            set_account::Model {
                set_id: 10,
                account_id: 1,
            },
            set_account::Model {
                set_id: 20,
                account_id: 1,
            },
        ])
        .await
        .unwrap();
        db.upsert_ups([
            sample_up(100, "Up100", UpState::Active),
            sample_up(200, "Up200", UpState::Active),
        ])
        .await
        .unwrap();
        db.upsert_up_accounts([
            up_account::Model {
                up_id: 100,
                account_id: 1,
            },
            up_account::Model {
                up_id: 200,
                account_id: 1,
            },
        ])
        .await
        .unwrap();

        // Dimension test medias (all Pending):
        // 1: set 10 + up 100
        // 2: set 10 + up 200
        // 3: set 20 + up 100
        // 4: set 20 only (no up)
        // 5: up 200 only (no set)
        // State test medias (all in set 10):
        // 11: Downloading, 12: Completed, 13: Failed, 14: Expired, 15: PermissionDenied, 16: Skipped
        db.upsert_medias([
            sample_media(1, MediaState::Pending),
            sample_media(2, MediaState::Pending),
            sample_media(3, MediaState::Pending),
            sample_media(4, MediaState::Pending),
            sample_media(5, MediaState::Pending),
            sample_media(11, MediaState::Downloading),
            sample_media(12, MediaState::Completed),
            sample_media(13, MediaState::Failed),
            sample_media(14, MediaState::Expired),
            sample_media(15, MediaState::PermissionDenied),
            sample_media(16, MediaState::Skipped),
        ])
        .await
        .unwrap();
        db.upsert_media_sets([
            media_set::Model { id: 1, set_id: 10 },
            media_set::Model { id: 2, set_id: 10 },
            media_set::Model { id: 3, set_id: 20 },
            media_set::Model { id: 4, set_id: 20 },
            media_set::Model { id: 11, set_id: 10 },
            media_set::Model { id: 12, set_id: 10 },
            media_set::Model { id: 13, set_id: 10 },
            media_set::Model { id: 14, set_id: 10 },
            media_set::Model { id: 15, set_id: 10 },
            media_set::Model { id: 16, set_id: 10 },
        ])
        .await
        .unwrap();
        db.upsert_media_ups([
            media_up::Model {
                id: 1,
                up_id: 100,
                from_up_feed: false,
            },
            media_up::Model {
                id: 2,
                up_id: 200,
                from_up_feed: false,
            },
            media_up::Model {
                id: 3,
                up_id: 100,
                from_up_feed: false,
            },
            media_up::Model {
                id: 5,
                up_id: 200,
                from_up_feed: false,
            },
        ])
        .await
        .unwrap();

        let ids_of =
            |rows: Vec<media::Model>| -> Vec<i64> { rows.into_iter().map(|m| m.id).collect() };

        // Fix 1: OR within set_ids, OR within up_ids, AND across ids / set_ids / up_ids.
        assert_eq!(
            ids_of(
                db.pull_candidates(&PullFilter {
                    set_ids: vec![10, 20],
                    ..Default::default()
                })
                .await
                .unwrap()
            ),
            vec![1, 2, 3, 4, 11]
        );
        assert_eq!(
            ids_of(
                db.pull_candidates(&PullFilter {
                    up_ids: vec![100, 200],
                    ..Default::default()
                })
                .await
                .unwrap()
            ),
            vec![1, 2, 3, 5]
        );
        assert_eq!(
            ids_of(
                db.pull_candidates(&PullFilter {
                    set_ids: vec![10, 20],
                    up_ids: vec![100],
                    ..Default::default()
                })
                .await
                .unwrap()
            ),
            vec![1, 3]
        );
        assert_eq!(
            ids_of(
                db.pull_candidates(&PullFilter {
                    set_ids: vec![10, 20],
                    up_ids: vec![100, 200],
                    ..Default::default()
                })
                .await
                .unwrap()
            ),
            vec![1, 2, 3]
        );
        assert_eq!(
            ids_of(
                db.pull_candidates(&PullFilter {
                    ids: vec![1, 2, 3, 4, 5],
                    set_ids: vec![10],
                    ..Default::default()
                })
                .await
                .unwrap()
            ),
            vec![1, 2]
        );
        assert_eq!(
            ids_of(
                db.pull_candidates(&PullFilter {
                    ids: vec![1, 2, 3],
                    set_ids: vec![10, 20],
                    up_ids: vec![200],
                    ..Default::default()
                })
                .await
                .unwrap()
            ),
            vec![2]
        );

        // Fix 2: State filtering for default, --retry-failed, bulk --force, bulk --force --retry-failed, and explicit ID --force.
        assert_eq!(
            ids_of(
                db.pull_candidates(&PullFilter {
                    set_ids: vec![10],
                    ..Default::default()
                })
                .await
                .unwrap()
            ),
            vec![1, 2, 11]
        );
        assert_eq!(
            ids_of(
                db.pull_candidates(&PullFilter {
                    set_ids: vec![10],
                    retry_failed: true,
                    ..Default::default()
                })
                .await
                .unwrap()
            ),
            vec![1, 2, 11, 13]
        );
        // Bulk --force includes Pending, Downloading, Completed; excludes Failed, Expired, PermissionDenied, Skipped.
        assert_eq!(
            ids_of(
                db.pull_candidates(&PullFilter {
                    set_ids: vec![10],
                    force: true,
                    ..Default::default()
                })
                .await
                .unwrap()
            ),
            vec![1, 2, 11, 12]
        );
        // Bulk --force --retry-failed adds Failed while still excluding Expired, PermissionDenied, Skipped.
        assert_eq!(
            ids_of(
                db.pull_candidates(&PullFilter {
                    set_ids: vec![10],
                    force: true,
                    retry_failed: true,
                    ..Default::default()
                })
                .await
                .unwrap()
            ),
            vec![1, 2, 11, 12, 13]
        );
        // Explicit IDs + --force allows ALL states (including Expired, PermissionDenied, Skipped, Failed, Completed).
        assert_eq!(
            ids_of(
                db.pull_candidates(&PullFilter {
                    ids: vec![1, 11, 12, 13, 14, 15, 16],
                    force: true,
                    ..Default::default()
                })
                .await
                .unwrap()
            ),
            vec![1, 11, 12, 13, 14, 15, 16]
        );
        // Explicit IDs + --force STILL respects set_ids intersection!
        assert!(
            db.pull_candidates(&PullFilter {
                ids: vec![14],
                set_ids: vec![20],
                force: true,
                ..Default::default()
            })
            .await
            .unwrap()
            .is_empty()
        );
    }

    #[tokio::test]
    async fn pull_candidates_per_account_scoping_and_reset_downloading() {
        let db = Db::open_in_memory().await.unwrap();

        db.upsert_account(sample_account(1, AccountState::Active))
            .await
            .unwrap();
        db.upsert_account(sample_account(2, AccountState::Active))
            .await
            .unwrap();
        db.upsert_account(sample_account(3, AccountState::Inactive))
            .await
            .unwrap();

        db.upsert_sets([
            sample_set(10, "A_Active", 2, SetState::Active),
            sample_set(11, "A_Inactive", 1, SetState::Inactive),
            sample_set(20, "B_Active", 3, SetState::Active),
            sample_set(30, "C_Active_On_InactiveAcct", 1, SetState::Active),
        ])
        .await
        .unwrap();
        db.upsert_set_accounts([
            set_account::Model {
                set_id: 10,
                account_id: 1,
            },
            set_account::Model {
                set_id: 11,
                account_id: 1,
            },
            set_account::Model {
                set_id: 20,
                account_id: 2,
            },
            set_account::Model {
                set_id: 30,
                account_id: 3,
            },
        ])
        .await
        .unwrap();

        db.upsert_ups([
            sample_up(100, "A_ActiveUp", UpState::Active),
            sample_up(200, "B_InactiveUp", UpState::Inactive),
        ])
        .await
        .unwrap();
        db.upsert_up_accounts([
            up_account::Model {
                up_id: 100,
                account_id: 1,
            },
            up_account::Model {
                up_id: 200,
                account_id: 2,
            },
        ])
        .await
        .unwrap();

        // 1: exclusive to A (Set 10)
        // 2: exclusive to B (Set 20)
        // 3: shared by A (Set 10) and B (Set 20)
        // 4: shared by A (Active UP 100) and B (Set 20)
        // 5: A's Inactive Set 11 only
        // 6: exclusive to A (Active UP 100)
        // 7: B's Inactive UP 200 only
        // 8: C's Active Set 30 (Inactive Account 3)
        db.upsert_medias([
            sample_media(1, MediaState::Pending),
            sample_media(2, MediaState::Pending),
            sample_media(3, MediaState::Pending),
            sample_media(4, MediaState::Pending),
            sample_media(5, MediaState::Pending),
            sample_media(6, MediaState::Downloading),
            sample_media(7, MediaState::Pending),
            sample_media(8, MediaState::Pending),
        ])
        .await
        .unwrap();
        db.upsert_media_sets([
            media_set::Model { id: 1, set_id: 10 },
            media_set::Model { id: 2, set_id: 20 },
            media_set::Model { id: 3, set_id: 10 },
            media_set::Model { id: 3, set_id: 20 },
            media_set::Model { id: 4, set_id: 20 },
            media_set::Model { id: 5, set_id: 11 },
            media_set::Model { id: 8, set_id: 30 },
        ])
        .await
        .unwrap();
        db.upsert_media_ups([
            media_up::Model {
                id: 4,
                up_id: 100,
                from_up_feed: false,
            },
            media_up::Model {
                id: 6,
                up_id: 100,
                from_up_feed: false,
            },
            media_up::Model {
                id: 7,
                up_id: 200,
                from_up_feed: false,
            },
        ])
        .await
        .unwrap();

        let ids_of =
            |rows: Vec<media::Model>| -> Vec<i64> { rows.into_iter().map(|m| m.id).collect() };

        assert_eq!(
            ids_of(
                db.pull_candidates(&PullFilter {
                    account_id: Some(1),
                    ..Default::default()
                })
                .await
                .unwrap()
            ),
            vec![1, 3, 4, 6]
        );
        assert_eq!(
            ids_of(
                db.pull_candidates(&PullFilter {
                    account_id: Some(2),
                    ..Default::default()
                })
                .await
                .unwrap()
            ),
            vec![2, 3, 4]
        );
        assert!(
            db.pull_candidates(&PullFilter {
                account_id: Some(3),
                ..Default::default()
            })
            .await
            .unwrap()
            .is_empty()
        );

        assert!(!db.is_media_active_for_any_account(1, &[]).await.unwrap());
        assert!(
            !db.is_media_active_for_any_account(1, &[2, 3])
                .await
                .unwrap()
        );
        assert!(
            db.is_media_active_for_any_account(1, &[1, 2])
                .await
                .unwrap()
        );
        assert!(db.is_media_active_for_any_account(3, &[2]).await.unwrap());
        assert!(db.is_media_active_for_any_account(4, &[2]).await.unwrap());
        assert!(
            !db.is_media_active_for_any_account(5, &[1, 2])
                .await
                .unwrap()
        );
        assert!(
            !db.is_media_active_for_any_account(7, &[1, 2])
                .await
                .unwrap()
        );
        assert!(
            !db.is_media_active_for_any_account(8, &[1, 2, 3])
                .await
                .unwrap()
        );

        // reset_downloading_medias resets only Downloading rows in the requested slice.
        db.set_media_state(1, MediaState::Completed).await.unwrap();
        db.record_media_failure(2, "transient failure")
            .await
            .unwrap();
        db.set_media_state(3, MediaState::Downloading)
            .await
            .unwrap();
        let reset = db.reset_downloading_medias(&[1, 2, 3, 999]).await.unwrap();
        assert_eq!(reset, 1);
        assert_eq!(
            db.get_media(1).await.unwrap().state,
            MediaState::Completed.as_str()
        );
        let m2 = db.get_media(2).await.unwrap();
        assert_eq!(m2.state, MediaState::Failed.as_str());
        assert_eq!(m2.retry_count, 1);
        assert_eq!(m2.last_error.as_deref(), Some("transient failure"));
        assert_eq!(
            db.get_media(3).await.unwrap().state,
            MediaState::Pending.as_str()
        );
        assert_eq!(
            db.get_media(6).await.unwrap().state,
            MediaState::Downloading.as_str()
        );
    }
}
