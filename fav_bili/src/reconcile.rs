use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::{
    db::Db,
    downloader::{metadata_size_and_inode, parse_comment_tag, resolve_recorded_path},
    entity::{media, media_page},
    muxer::Muxer,
    state::{MediaPageState, MediaState},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelocatedPage {
    pub media_id: i64,
    pub cid: i64,
    pub old_local_path: Option<String>,
    pub new_local_path: String,
    pub file_size: Option<i64>,
    pub file_inode: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MissingPage {
    pub media_id: i64,
    pub cid: i64,
}

#[derive(Debug, Clone, Default)]
pub struct ReconcileReport {
    pub total_checked_pages: usize,
    pub verified_pages: usize,
    pub total_checked_legacy: usize,
    pub verified_legacy: usize,
    pub relocated: Vec<RelocatedPage>,
    pub missing_pages: Vec<MissingPage>,
    pub missing_legacy_ids: Vec<i64>,
}

impl ReconcileReport {
    pub fn total_checked(&self) -> usize {
        self.total_checked_pages + self.total_checked_legacy
    }

    pub fn total_verified(&self) -> usize {
        self.verified_pages + self.verified_legacy
    }

    pub fn total_missing_items(&self) -> usize {
        self.missing_pages.len() + self.missing_legacy_ids.len()
    }

    pub fn affected_missing_media_ids(&self) -> BTreeSet<i64> {
        let mut ids: BTreeSet<i64> = self.missing_pages.iter().map(|m| m.media_id).collect();
        ids.extend(self.missing_legacy_ids.iter().copied());
        ids
    }

    pub fn has_drift(&self) -> bool {
        !self.relocated.is_empty()
            || !self.missing_pages.is_empty()
            || !self.missing_legacy_ids.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScopedReconcileReport {
    pub active: ReconcileReport,
    pub inactive: ReconcileReport,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReconcileApplied {
    pub relinked_pages: usize,
    pub reset_pages: u64,
    pub reset_medias: usize,
}

#[derive(Debug, Default)]
struct DiskIndex {
    #[cfg_attr(not(unix), allow(dead_code))]
    by_inode: HashMap<u64, PathBuf>,
    by_aid_prefix: HashMap<i64, Vec<PathBuf>>,
    by_bvid_lower: Vec<(String, PathBuf)>,
    by_size: HashMap<u64, Vec<PathBuf>>,
}

impl DiskIndex {
    fn scan(output_dir: &Path) -> Self {
        let mut index = Self::default();
        if !output_dir.is_dir() {
            return index;
        }

        let mut stack = vec![output_dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let file_name = entry.file_name();
                let name = file_name.to_string_lossy();
                if name == ".fav" || name == ".fav-tmp" {
                    continue;
                }
                let Ok(ft) = entry.file_type() else {
                    continue;
                };
                let path = entry.path();
                if ft.is_dir() {
                    if !name.starts_with('.') {
                        stack.push(path);
                    }
                    continue;
                }

                // Skip symlinks pointing to directories (avoid cycles) and incomplete `.part` files.
                if name.ends_with(".part") {
                    continue;
                }
                let Ok(meta) = std::fs::metadata(&path) else {
                    continue;
                };
                if !meta.is_file() || meta.len() == 0 {
                    continue;
                }

                let (size, inode) = metadata_size_and_inode(&meta);
                if let Some(ino) = inode {
                    index.by_inode.entry(ino).or_insert_with(|| path.clone());
                }
                if let Some(aid) = extract_aid_prefix(&name) {
                    index
                        .by_aid_prefix
                        .entry(aid)
                        .or_default()
                        .push(path.clone());
                }
                let lower = name.to_ascii_lowercase();
                if lower.contains("bv1") {
                    index.by_bvid_lower.push((lower, path.clone()));
                }
                index.by_size.entry(size).or_default().push(path);
            }
        }

        // Deterministic traversal order across platforms/filesystems.
        for list in index.by_aid_prefix.values_mut() {
            list.sort();
        }
        index.by_bvid_lower.sort_by(|a, b| a.1.cmp(&b.1));
        for list in index.by_size.values_mut() {
            list.sort();
        }
        index
    }
}

fn extract_aid_prefix(file_name: &str) -> Option<i64> {
    let bytes = file_name.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i >= bytes.len() {
        return None;
    }
    if !matches!(bytes[i], b'-' | b'.' | b'(') {
        return None;
    }
    file_name[..i].parse::<i64>().ok().filter(|&n| n > 0)
}

fn path_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn relative_local_path(output_dir: &Path, full_path: &Path) -> String {
    full_path
        .strip_prefix(output_dir)
        .unwrap_or(full_path)
        .to_string_lossy()
        .into_owned()
}

fn make_relocated(
    page: &media_page::Model,
    output_dir: &Path,
    new_full_path: &Path,
) -> RelocatedPage {
    let (size, inode) = std::fs::metadata(new_full_path)
        .map(|m| {
            let (sz, ino) = metadata_size_and_inode(&m);
            (
                i64::try_from(sz).ok(),
                ino.and_then(|v| i64::try_from(v).ok()),
            )
        })
        .unwrap_or((page.file_size, page.file_inode));
    RelocatedPage {
        media_id: page.media_id,
        cid: page.cid,
        old_local_path: page.local_path.clone(),
        new_local_path: relative_local_path(output_dir, new_full_path),
        file_size: size,
        file_inode: inode,
    }
}

fn is_real_bvid(bv_id: &str) -> bool {
    let lower = bv_id.trim().to_ascii_lowercase();
    lower.len() == 12 && lower.starts_with("bv1")
}

fn matches_page_marker(path: &Path, page_num: i32) -> bool {
    if page_num <= 0 {
        return false;
    }
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };

    let bytes = stem.as_bytes();
    let mut last_canonical: Option<i32> = None;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > start && j < bytes.len() && bytes[j] == b')' {
                let digits = &stem[start..j];
                if !digits.starts_with('0')
                    && let Ok(k) = digits.parse::<i32>()
                    && k > 0
                {
                    let followed_by_boundary = j + 1 == bytes.len() || bytes[j + 1] == b'-';
                    if followed_by_boundary {
                        last_canonical = Some(k);
                    }
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }

    if let Some(canonical_page) = last_canonical {
        canonical_page == page_num
    } else {
        let marker = format!("({page_num})");
        stem.contains(&marker)
    }
}

struct PendingCheck<'a> {
    media: &'a media::Model,
    page: &'a media_page::Model,
    is_multipage: bool,
}

#[derive(Default)]
struct ReconcileScratch {
    claimed: HashSet<PathBuf>,
    tag_cache: HashMap<PathBuf, Option<(i64, i64)>>,
}

async fn reconcile_slice(
    completed_medias: &[media::Model],
    pages_by_media: &HashMap<i64, Vec<media_page::Model>>,
    muxer: &dyn Muxer,
    output_dir: &Path,
    disk_index: &DiskIndex,
    scratch: &mut ReconcileScratch,
) -> ReconcileReport {
    if completed_medias.is_empty() {
        return ReconcileReport::default();
    }

    let mut report = ReconcileReport::default();
    let mut unresolved: Vec<PendingCheck<'_>> = Vec::new();
    let mut legacy_medias: Vec<&media::Model> = Vec::new();

    // Pass 1 (Layer 1): Exact `local_path` exists and is non-empty.
    // Run across all pages FIRST so intact pages claim their files before relocation heuristics run.
    for media_row in completed_medias {
        let Some(pages) = pages_by_media.get(&media_row.id) else {
            legacy_medias.push(media_row);
            continue;
        };
        if pages.is_empty() {
            legacy_medias.push(media_row);
            continue;
        }
        let media_has_multiple_pages = pages.len() > 1;
        for page in pages {
            if page.state != MediaPageState::Completed.as_str() {
                continue;
            }
            report.total_checked_pages += 1;
            let exact_present = page.local_path.as_deref().and_then(|raw| {
                let candidate = resolve_recorded_path(output_dir, raw);
                std::fs::metadata(&candidate)
                    .ok()
                    .filter(|m| m.is_file() && m.len() > 0)
                    .map(|_| candidate)
            });
            if let Some(present_path) = exact_present {
                report.verified_pages += 1;
                scratch.claimed.insert(path_key(&present_path));
            } else {
                unresolved.push(PendingCheck {
                    media: media_row,
                    page,
                    is_multipage: media_has_multiple_pages || page.page > 1,
                });
            }
        }
    }

    // Pass 2 (Layer 2): Same-disk inode match (survives arbitrary rename / move into subdirs).
    let mut after_layer2: Vec<PendingCheck<'_>> = Vec::new();
    for item in unresolved {
        let matched = item
            .page
            .file_inode
            .and_then(|ino| u64::try_from(ino).ok())
            .and_then(|ino| disk_index.by_inode.get(&ino))
            .filter(|candidate| !scratch.claimed.contains(&path_key(candidate)));

        if let Some(new_path) = matched {
            scratch.claimed.insert(path_key(new_path));
            report
                .relocated
                .push(make_relocated(item.page, output_dir, new_path));
        } else {
            after_layer2.push(item);
        }
    }

    // Pass 3 (Layer 3): Recursive `{aid}` prefix or `{bv_id}` filename match among unclaimed files.
    let mut after_layer3: Vec<PendingCheck<'_>> = Vec::new();
    for item in after_layer2 {
        let mut matched: Option<&PathBuf> = None;

        if let Some(candidates) = disk_index.by_aid_prefix.get(&item.media.id) {
            matched = candidates.iter().find(|p| {
                !scratch.claimed.contains(&path_key(p))
                    && (!item.is_multipage || matches_page_marker(p, item.page.page))
            });
        }

        if matched.is_none() && is_real_bvid(&item.media.bv_id) {
            let bv_lower = item.media.bv_id.to_ascii_lowercase();
            matched = disk_index
                .by_bvid_lower
                .iter()
                .find(|(name_lower, p)| {
                    name_lower.contains(&bv_lower)
                        && !scratch.claimed.contains(&path_key(p))
                        && (!item.is_multipage || matches_page_marker(p, item.page.page))
                })
                .map(|(_, p)| p);
        }

        if let Some(new_path) = matched {
            scratch.claimed.insert(path_key(new_path));
            report
                .relocated
                .push(make_relocated(item.page, output_dir, new_path));
        } else {
            after_layer3.push(item);
        }
    }

    // Pass 4 (Layer 4): Cross-disk rename with changed filename — match by exact file size +
    // embedded `fav:aid=...;cid=...` comment tag via `muxer.read_comment_tag`.
    for item in after_layer3 {
        let mut matched: Option<PathBuf> = None;
        if let Some(size_u64) = item.page.file_size.and_then(|s| u64::try_from(s).ok())
            && size_u64 > 0
            && let Some(candidates) = disk_index.by_size.get(&size_u64)
        {
            for candidate in candidates {
                let key = path_key(candidate);
                if scratch.claimed.contains(&key) {
                    continue;
                }
                let parsed = if let Some(&cached) = scratch.tag_cache.get(&key) {
                    cached
                } else {
                    let raw_tag = muxer.read_comment_tag(candidate).await.ok().flatten();
                    let parsed_tag = raw_tag.as_deref().and_then(parse_comment_tag);
                    scratch.tag_cache.insert(key.clone(), parsed_tag);
                    parsed_tag
                };
                if parsed == Some((item.page.media_id, item.page.cid)) {
                    matched = Some(candidate.clone());
                    break;
                }
            }
        }

        if let Some(new_path) = matched {
            scratch.claimed.insert(path_key(&new_path));
            report
                .relocated
                .push(make_relocated(item.page, output_dir, &new_path));
        } else {
            report.missing_pages.push(MissingPage {
                media_id: item.page.media_id,
                cid: item.page.cid,
            });
        }
    }

    // Legacy v1.x `Completed` medias with 0 `media_page` rows.
    for media_row in legacy_medias {
        report.total_checked_legacy += 1;
        let mut matched: Option<&PathBuf> =
            disk_index
                .by_aid_prefix
                .get(&media_row.id)
                .and_then(|list| {
                    list.iter()
                        .find(|p| !scratch.claimed.contains(&path_key(p)))
                });

        if matched.is_none() && is_real_bvid(&media_row.bv_id) {
            let bv_lower = media_row.bv_id.to_ascii_lowercase();
            matched = disk_index
                .by_bvid_lower
                .iter()
                .find(|(name_lower, p)| {
                    name_lower.contains(&bv_lower) && !scratch.claimed.contains(&path_key(p))
                })
                .map(|(_, p)| p);
        }

        if let Some(found_path) = matched {
            scratch.claimed.insert(path_key(found_path));
            report.verified_legacy += 1;
        } else {
            report.missing_legacy_ids.push(media_row.id);
        }
    }

    report
}

/// Reconcile `Completed` medias in SQLite against files on disk under `output_dir` using
/// 4 rename-safe layers (exact path -> same-disk inode -> `{aid}`/`{bvid}` recursive scan ->
/// size-matched embedded `fav:aid=...;cid=...` container tag) plus legacy v1.x media checks.
pub async fn reconcile_completed_medias(
    db: &Db,
    muxer: &dyn Muxer,
    output_dir: &Path,
    active_only: bool,
    filter_media_ids: Option<&HashSet<i64>>,
) -> Result<ReconcileReport> {
    let mut completed_medias = db.completed_medias(active_only).await?;
    if let Some(filter) = filter_media_ids {
        completed_medias.retain(|m| filter.contains(&m.id));
    }
    if completed_medias.is_empty() {
        return Ok(ReconcileReport::default());
    }

    let media_ids: Vec<i64> = completed_medias.iter().map(|m| m.id).collect();
    let all_pages = db.pages_for_media_ids(&media_ids).await?;
    let mut pages_by_media: HashMap<i64, Vec<media_page::Model>> = HashMap::new();
    for page in all_pages {
        pages_by_media.entry(page.media_id).or_default().push(page);
    }

    let output_dir_owned = output_dir.to_path_buf();
    let disk_index = tokio::task::spawn_blocking(move || DiskIndex::scan(&output_dir_owned))
        .await
        .unwrap_or_default();

    let mut scratch = ReconcileScratch::default();
    Ok(reconcile_slice(
        &completed_medias,
        &pages_by_media,
        muxer,
        output_dir,
        &disk_index,
        &mut scratch,
    )
    .await)
}

/// Reconcile active and inactive `Completed` medias in a single disk scan of `output_dir`.
/// Active medias are reconciled first so `active` matches
/// `reconcile_completed_medias(..., active_only = true, None)` 1-to-1 and active claims take
/// priority over inactive medias.
pub async fn reconcile_completed_medias_scoped(
    db: &Db,
    muxer: &dyn Muxer,
    output_dir: &Path,
) -> Result<ScopedReconcileReport> {
    let all_completed = db.completed_medias(false).await?;
    if all_completed.is_empty() {
        return Ok(ScopedReconcileReport::default());
    }

    let active_ids = db.active_media_ids().await?;
    let mut active_medias = Vec::new();
    let mut inactive_medias = Vec::new();
    for m in all_completed {
        if active_ids.contains(&m.id) {
            active_medias.push(m);
        } else {
            inactive_medias.push(m);
        }
    }

    let all_ids: Vec<i64> = active_medias
        .iter()
        .chain(inactive_medias.iter())
        .map(|m| m.id)
        .collect();
    let all_pages = db.pages_for_media_ids(&all_ids).await?;
    let mut pages_by_media: HashMap<i64, Vec<media_page::Model>> = HashMap::new();
    for page in all_pages {
        pages_by_media.entry(page.media_id).or_default().push(page);
    }

    let output_dir_owned = output_dir.to_path_buf();
    let disk_index = tokio::task::spawn_blocking(move || DiskIndex::scan(&output_dir_owned))
        .await
        .unwrap_or_default();

    let mut scratch = ReconcileScratch::default();
    let active = reconcile_slice(
        &active_medias,
        &pages_by_media,
        muxer,
        output_dir,
        &disk_index,
        &mut scratch,
    )
    .await;
    let inactive = reconcile_slice(
        &inactive_medias,
        &pages_by_media,
        muxer,
        output_dir,
        &disk_index,
        &mut scratch,
    )
    .await;

    Ok(ScopedReconcileReport { active, inactive })
}

/// Apply a [`ReconcileReport`] to SQLite:
/// - Relinks every `Relocated` page via `db.update_page_local_path`
/// - Surgically resets ONLY `missing_pages` via `db.reset_specific_pages`
/// - Transitions affected medias (and `missing_legacy_ids`) back to `MediaState::Pending`
pub async fn apply_reconciliation(db: &Db, report: &ReconcileReport) -> Result<ReconcileApplied> {
    let mut applied = ReconcileApplied::default();

    for reloc in &report.relocated {
        db.update_page_local_path(
            reloc.media_id,
            reloc.cid,
            reloc.new_local_path.clone(),
            reloc.file_size,
            reloc.file_inode,
        )
        .await?;
        applied.relinked_pages += 1;
    }

    if !report.missing_pages.is_empty() {
        let pairs: Vec<(i64, i64)> = report
            .missing_pages
            .iter()
            .map(|m| (m.media_id, m.cid))
            .collect();
        applied.reset_pages = db.reset_specific_pages(&pairs).await?;
    }

    for media_id in report.affected_missing_media_ids() {
        db.set_media_state(media_id, MediaState::Pending).await?;
        applied.reset_medias += 1;
    }

    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        downloader::file_size_and_inode,
        entity::{account, media, media_set, set, set_account},
        muxer::test_support::MockMuxer,
        state::{AccountState, MediaType, SetKind, SetState},
    };

    async fn seed_completed_media(db: &Db, id: i64, bv_id: &str, title: &str) {
        db.upsert_account(account::Model {
            account_id: 1,
            name: "Alice".to_owned(),
            cookies: "SESSDATA=s".to_owned(),
            state: AccountState::Active.to_string(),
            refresh_token: None,
        })
        .await
        .unwrap();
        db.upsert_sets([set::Model {
            set_id: 10,
            name: "Fav".to_owned(),
            count: 1,
            state: SetState::Active.to_string(),
            kind: SetKind::Created.to_string(),
            owner_mid: Some(1),
            sync_complete: true,
            resume_page: None,
        }])
        .await
        .unwrap();
        db.upsert_set_accounts([set_account::Model {
            set_id: 10,
            account_id: 1,
        }])
        .await
        .unwrap();
        db.upsert_medias([media::Model {
            id,
            bv_id: bv_id.to_owned(),
            title: title.to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Completed.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        }])
        .await
        .unwrap();
        db.upsert_media_sets([media_set::Model { id, set_id: 10 }])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn layer1_exact_and_layer2_same_disk_inode_rename_relink() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("videos");
        std::fs::create_dir_all(&out_dir).unwrap();

        let db = Db::open_in_memory().await.unwrap();
        seed_completed_media(&db, 10, "BV17x411w7KC", "Video 10").await;

        let orig = out_dir.join("10-Video 10.mp4");
        std::fs::write(&orig, b"video payload bytes").unwrap();
        let (size, inode) = file_size_and_inode(&orig);
        db.mark_page_completed(
            10,
            1001,
            1,
            "P1".to_owned(),
            "10-Video 10.mp4".to_owned(),
            size,
            inode,
        )
        .await
        .unwrap();

        let muxer = MockMuxer::succeeding();

        // Layer 1: exact path present -> 0 drift
        let rep1 = reconcile_completed_medias(&db, &muxer, &out_dir, true, None)
            .await
            .unwrap();
        assert_eq!(rep1.verified_pages, 1);
        assert!(!rep1.has_drift());

        // Move & rename into a nested subdir with a completely unrelated filename (same inode on Unix)
        let nested = out_dir.join("music/favorites");
        std::fs::create_dir_all(&nested).unwrap();
        let renamed = nested.join("my_custom_clip.mp4");
        std::fs::rename(&orig, &renamed).unwrap();

        #[cfg(unix)]
        {
            let rep2 = reconcile_completed_medias(&db, &muxer, &out_dir, true, None)
                .await
                .unwrap();
            assert_eq!(rep2.verified_pages, 0);
            assert_eq!(rep2.relocated.len(), 1);
            assert!(rep2.missing_pages.is_empty());
            assert_eq!(
                rep2.relocated[0].new_local_path,
                "music/favorites/my_custom_clip.mp4"
            );

            let applied = apply_reconciliation(&db, &rep2).await.unwrap();
            assert_eq!(applied.relinked_pages, 1);
            assert_eq!(applied.reset_medias, 0);

            // DB media remains Completed and local_path is updated
            assert_eq!(
                db.get_media(10).await.unwrap().state,
                MediaState::Completed.as_str()
            );
            let pages = db.pages_for_media(10).await.unwrap();
            assert_eq!(
                pages[0].local_path.as_deref(),
                Some("music/favorites/my_custom_clip.mp4")
            );
        }
    }

    #[tokio::test]
    async fn layer3_filename_aid_bvid_and_layer4_cross_disk_tag_relink() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("videos");
        std::fs::create_dir_all(&out_dir).unwrap();

        let db = Db::open_in_memory().await.unwrap();
        seed_completed_media(&db, 20, "BV1Q541167Qg", "Clip 20").await;
        seed_completed_media(&db, 30, "BV1xx411c7mD", "Clip 30").await;

        // Media 20: renamed with BVID in filename, no inode recorded (simulates cross-disk / upgraded DB)
        let bv_renamed = out_dir.join("archive/custom_BV1Q541167Qg_saved.mp4");
        std::fs::create_dir_all(bv_renamed.parent().unwrap()).unwrap();
        std::fs::write(&bv_renamed, b"content-20").unwrap();
        db.mark_page_completed(
            20,
            2001,
            1,
            "P1".to_owned(),
            "20-Clip 20.mp4".to_owned(),
            Some(10),
            None,
        )
        .await
        .unwrap();

        // Media 30: completely unrelated filename, different inode, matching size + embedded tag!
        let cross_disk = out_dir.join("totally_renamed.mp4");
        let bytes_30 = b"mock-mp4\nfav:aid=30;cid=3001\n";
        std::fs::write(&cross_disk, bytes_30).unwrap();
        db.mark_page_completed(
            30,
            3001,
            1,
            "P1".to_owned(),
            "30-Clip 30.mp4".to_owned(),
            Some(bytes_30.len() as i64),
            Some(999_999_999), // mismatched inode simulating another disk!
        )
        .await
        .unwrap();

        let muxer = MockMuxer::succeeding();
        let rep = reconcile_completed_medias(&db, &muxer, &out_dir, true, None)
            .await
            .unwrap();
        assert_eq!(rep.relocated.len(), 2);
        assert!(rep.missing_pages.is_empty());

        let applied = apply_reconciliation(&db, &rep).await.unwrap();
        assert_eq!(applied.relinked_pages, 2);
        assert_eq!(applied.reset_medias, 0);
        assert_eq!(
            db.pages_for_media(20).await.unwrap()[0]
                .local_path
                .as_deref(),
            Some("archive/custom_BV1Q541167Qg_saved.mp4")
        );
        assert_eq!(
            db.pages_for_media(30).await.unwrap()[0]
                .local_path
                .as_deref(),
            Some("totally_renamed.mp4")
        );
    }

    #[tokio::test]
    async fn surgical_multipage_reset_and_legacy_v1_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("videos");
        std::fs::create_dir_all(&out_dir).unwrap();

        let db = Db::open_in_memory().await.unwrap();
        seed_completed_media(&db, 40, "BV17x411w7KC", "Multi").await;
        seed_completed_media(&db, 50, "BV1Q541167Qg", "LegacyPresent").await;
        seed_completed_media(&db, 60, "BV1xx411c7mD", "LegacyMissing").await;

        // Multi-page media 40: P1 present, P2 deleted!
        let p1 = out_dir.join("40-Multi(1)-P1.mp4");
        std::fs::write(&p1, b"page1-data").unwrap();
        let (sz1, ino1) = file_size_and_inode(&p1);
        db.mark_page_completed(
            40,
            4001,
            1,
            "P1".to_owned(),
            "40-Multi(1)-P1.mp4".to_owned(),
            sz1,
            ino1,
        )
        .await
        .unwrap();
        db.mark_page_completed(
            40,
            4002,
            2,
            "P2".to_owned(),
            "40-Multi(2)-P2.mp4".to_owned(),
            Some(999),
            None,
        )
        .await
        .unwrap();

        // Legacy v1.x media 50 (0 media_page rows): file exists under aid prefix
        std::fs::write(out_dir.join("50-LegacyPresent.mp4"), b"legacy-ok").unwrap();
        // Legacy v1.x media 60 (0 media_page rows): file missing!
        // Also create a hidden `.fav-tmp/60-temp.part` which MUST be ignored!
        std::fs::create_dir_all(out_dir.join(".fav-tmp")).unwrap();
        std::fs::write(out_dir.join(".fav-tmp/60-LegacyMissing.mp4"), b"ignored").unwrap();
        std::fs::write(out_dir.join("60-LegacyMissing.mp4.part"), b"ignored").unwrap();

        let muxer = MockMuxer::succeeding();

        // Scoped reconciliation with `filter_media_ids` only inspects matching media IDs.
        let scoped =
            reconcile_completed_medias(&db, &muxer, &out_dir, true, Some(&HashSet::from([40])))
                .await
                .unwrap();
        assert_eq!(scoped.verified_pages, 1);
        assert_eq!(scoped.total_checked_legacy, 0);
        assert_eq!(
            scoped.missing_pages,
            vec![MissingPage {
                media_id: 40,
                cid: 4002
            }]
        );
        assert!(scoped.missing_legacy_ids.is_empty());

        let rep = reconcile_completed_medias(&db, &muxer, &out_dir, true, None)
            .await
            .unwrap();
        assert_eq!(rep.verified_pages, 1);
        assert_eq!(rep.verified_legacy, 1);
        assert_eq!(
            rep.missing_pages,
            vec![MissingPage {
                media_id: 40,
                cid: 4002
            }]
        );
        assert_eq!(rep.missing_legacy_ids, vec![60]);

        let applied = apply_reconciliation(&db, &rep).await.unwrap();
        assert_eq!(applied.reset_pages, 1);
        assert_eq!(applied.reset_medias, 2); // medias 40 and 60

        // Media 40: P1 stays Completed, P2 becomes Pending, media becomes Pending!
        assert_eq!(
            db.get_media(40).await.unwrap().state,
            MediaState::Pending.as_str()
        );
        let m40_pages = db.pages_for_media(40).await.unwrap();
        assert_eq!(m40_pages[0].state, MediaPageState::Completed.as_str());
        assert_eq!(m40_pages[1].state, MediaPageState::Pending.as_str());

        // Legacy 50 stays Completed; Legacy 60 becomes Pending
        assert_eq!(
            db.get_media(50).await.unwrap().state,
            MediaState::Completed.as_str()
        );
        assert_eq!(
            db.get_media(60).await.unwrap().state,
            MediaState::Pending.as_str()
        );
    }

    #[tokio::test]
    async fn scoped_reconcile_partitions_active_and_inactive_in_single_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("videos");
        std::fs::create_dir_all(&out_dir).unwrap();

        let db = Db::open_in_memory().await.unwrap();
        seed_completed_media(&db, 10, "BV17x411w7KC", "ActivePresent").await;
        // Inactive set 20 owned by active account 1
        db.upsert_sets([set::Model {
            set_id: 20,
            name: "InactiveFav".to_owned(),
            count: 2,
            state: SetState::Inactive.to_string(),
            kind: SetKind::Created.to_string(),
            owner_mid: Some(1),
            sync_complete: true,
            resume_page: None,
        }])
        .await
        .unwrap();
        db.upsert_set_accounts([set_account::Model {
            set_id: 20,
            account_id: 1,
        }])
        .await
        .unwrap();
        db.upsert_medias([
            media::Model {
                id: 20,
                bv_id: "BV1Q541167Qg".to_owned(),
                title: "InactiveMissingLegacy".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
            media::Model {
                id: 30,
                bv_id: "BV1xx411c7mD".to_owned(),
                title: "InactivePresentLegacy".to_owned(),
                r#type: MediaType::Video.to_string(),
                state: MediaState::Completed.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            },
        ])
        .await
        .unwrap();
        db.upsert_media_sets([
            media_set::Model { id: 20, set_id: 20 },
            media_set::Model { id: 30, set_id: 20 },
        ])
        .await
        .unwrap();

        std::fs::write(out_dir.join("10-ActivePresent.mp4"), b"active").unwrap();
        std::fs::write(out_dir.join("30-InactivePresentLegacy.mp4"), b"inactive").unwrap();

        let muxer = MockMuxer::succeeding();
        let scoped = reconcile_completed_medias_scoped(&db, &muxer, &out_dir)
            .await
            .unwrap();
        let direct_active = reconcile_completed_medias(&db, &muxer, &out_dir, true, None)
            .await
            .unwrap();

        assert_eq!(scoped.active.total_checked(), direct_active.total_checked());
        assert_eq!(
            scoped.active.total_verified(),
            direct_active.total_verified()
        );
        assert!(!scoped.active.has_drift());
        assert_eq!(scoped.active.verified_legacy, 1);

        assert_eq!(scoped.inactive.verified_legacy, 1);
        assert_eq!(scoped.inactive.missing_legacy_ids, vec![20]);
    }

    #[test]
    fn matches_page_marker_prefers_canonical_markers_and_supports_custom_renames() {
        // Video title contains `(2)` while canonical page marker is `(1)`
        assert!(matches_page_marker(Path::new("100-Clip (2)(1)-P1.mp4"), 1));
        assert!(!matches_page_marker(Path::new("100-Clip (2)(1)-P1.mp4"), 2));
        assert!(!matches_page_marker(Path::new("100-Clip (2)(1)-P1.mp4"), 3));

        // No part suffix: canonical `(1)` at end of stem
        assert!(matches_page_marker(Path::new("100-Clip (2)(1).mp4"), 1));
        assert!(!matches_page_marker(Path::new("100-Clip (2)(1).mp4"), 2));

        // Title contains `(2)-` before the real `(1)-` marker: last canonical marker wins
        assert!(matches_page_marker(
            Path::new("100-Season (2)-Live(1)-P1.mp4"),
            1
        ));
        assert!(!matches_page_marker(
            Path::new("100-Season (2)-Live(1)-P1.mp4"),
            2
        ));

        // Multi-digit page numbers and nested parent directories containing `(2)`
        assert!(matches_page_marker(
            Path::new("season (2)/100-Clip (2)(12)-P12.mp4"),
            12
        ));
        assert!(!matches_page_marker(
            Path::new("season (2)/100-Clip (2)(12)-P12.mp4"),
            2
        ));

        // UTF-8 CJK titles and part names
        assert!(matches_page_marker(
            Path::new("100-测试 (2)(1)-第一集.mp4"),
            1
        ));
        assert!(!matches_page_marker(
            Path::new("100-测试 (2)(1)-第一集.mp4"),
            2
        ));

        // Custom renamed filename without canonical `(<k>)-` or trailing `(<k>)` falls back to substring in stem
        assert!(matches_page_marker(Path::new("custom_(2)_final.mp4"), 2));
        assert!(!matches_page_marker(Path::new("custom_(2)_final.mp4"), 1));

        // Non-positive or missing markers never match
        assert!(!matches_page_marker(Path::new("100-Clip (0)-P1.mp4"), 0));
        assert!(!matches_page_marker(Path::new("100-Clip (0)-P1.mp4"), 1));
        assert!(!matches_page_marker(Path::new("100-Clip.mp4"), 1));
    }

    #[tokio::test]
    async fn layer3_multipage_reconcile_ignores_parenthesized_numbers_in_title() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("videos");
        let moved_dir = out_dir.join("moved");
        std::fs::create_dir_all(&moved_dir).unwrap();

        let db = Db::open_in_memory().await.unwrap();
        seed_completed_media(&db, 100, "BV17x411w7KC", "Clip (2)").await;
        seed_completed_media(&db, 101, "BV1Q541167Qg", "Clip (1)").await;
        seed_completed_media(&db, 102, "BV1xx411c7mD", "Clip (2)").await;

        // Media 100 ("Clip (2)"): cross-disk move (no inode). Only P1 exists on disk; P2 deleted.
        std::fs::write(moved_dir.join("100-Clip (2)(1)-P1.mp4"), b"m100-p1").unwrap();
        db.mark_page_completed(
            100,
            1001,
            1,
            "P1".to_owned(),
            "old/100-Clip (2)(1)-P1.mp4".to_owned(),
            Some(999),
            None,
        )
        .await
        .unwrap();
        db.mark_page_completed(
            100,
            1002,
            2,
            "P2".to_owned(),
            "old/100-Clip (2)(2)-P2.mp4".to_owned(),
            Some(999),
            None,
        )
        .await
        .unwrap();

        // Media 101 ("Clip (1)"): cross-disk move (no inode). P1 deleted, only P2 exists on disk.
        // P1 MUST NOT falsely steal P2's file despite `(1)` appearing in the title!
        std::fs::write(moved_dir.join("101-Clip (1)(2)-P2.mp4"), b"m101-p2").unwrap();
        db.mark_page_completed(
            101,
            1101,
            1,
            "P1".to_owned(),
            "old/101-Clip (1)(1)-P1.mp4".to_owned(),
            Some(999),
            None,
        )
        .await
        .unwrap();
        db.mark_page_completed(
            101,
            1102,
            2,
            "P2".to_owned(),
            "old/101-Clip (1)(2)-P2.mp4".to_owned(),
            Some(999),
            None,
        )
        .await
        .unwrap();

        // Media 102 ("Clip (2)"): only P2 is Completed in DB; only P1 file (without part suffix) is on disk.
        // P2 MUST NOT falsely match `102-Clip (2)(1).mp4` despite `(2)` in the title!
        std::fs::write(moved_dir.join("102-Clip (2)(1).mp4"), b"m102-p1").unwrap();
        db.mark_page_completed(
            102,
            1202,
            2,
            "P2".to_owned(),
            "old/102-Clip (2)(2).mp4".to_owned(),
            Some(999),
            None,
        )
        .await
        .unwrap();

        let muxer = MockMuxer::succeeding();
        let rep = reconcile_completed_medias(&db, &muxer, &out_dir, true, None)
            .await
            .unwrap();

        // Relocated: (100, 1001) -> moved/100-Clip (2)(1)-P1.mp4, (101, 1102) -> moved/101-Clip (1)(2)-P2.mp4
        assert_eq!(rep.relocated.len(), 2);
        assert!(rep.relocated.iter().any(|r| {
            r.media_id == 100 && r.cid == 1001 && r.new_local_path == "moved/100-Clip (2)(1)-P1.mp4"
        }));
        assert!(rep.relocated.iter().any(|r| {
            r.media_id == 101 && r.cid == 1102 && r.new_local_path == "moved/101-Clip (1)(2)-P2.mp4"
        }));

        // Missing: (100, 1002), (101, 1101), (102, 1202)
        let missing: HashSet<MissingPage> = rep.missing_pages.iter().copied().collect();
        assert_eq!(
            missing,
            HashSet::from([
                MissingPage {
                    media_id: 100,
                    cid: 1002
                },
                MissingPage {
                    media_id: 101,
                    cid: 1101
                },
                MissingPage {
                    media_id: 102,
                    cid: 1202
                },
            ])
        );
    }
}
