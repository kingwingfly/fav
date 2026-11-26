mod common;

use std::{path::PathBuf, process::Command, sync::Arc};

use common::{
    MockAccount, MockBiliServer, MockCdnAsset, MockMediaItem, MockPage, MockPlayurlResponse,
    MockSet, MockState, MockStreamSpec, MockUp, MockViewResponse, generate_tiny_av_streams,
};
use fav_bili::{
    action::{
        auth::{LogoutOptions, UseCookiesOptions, logout, use_cookies},
        fetch::{FetchOptions, fetch},
        like::{LikeOptions, like},
        pull::{PullOptions, pull},
    },
    config::{Config, Workspace},
    context::AppContext,
    cookies::parse_cookie_header,
    db::Db,
    entity::{account, media, media_set, set, up},
    muxer::{FfmpegMuxer, Muxer, test_support::MockMuxer},
    state::{AccountState, MediaPageState, MediaState, MediaType, SetState, UpState},
};

#[tokio::test]
async fn multi_account_cookie_isolation_like_ticket_and_logout() {
    let mut state = MockState::default();
    state
        .accounts_by_sessdata
        .insert("sess_a".to_owned(), MockAccount::active(1001, "Alice"));
    state
        .accounts_by_sessdata
        .insert("sess_b".to_owned(), MockAccount::active(1002, "Bob"));
    let server = MockBiliServer::start(state).await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = Workspace::new(tmp.path().to_path_buf());
    ws.ensure_dirs().unwrap();
    let db = Db::open_in_memory().await.unwrap();
    let ctx = AppContext::for_test(
        ws,
        Config::default(),
        db,
        Arc::new(MockMuxer::succeeding()),
        server.client_config(),
    );

    // Import two distinct accounts in a single `use_cookies` invocation (previously leaked cookies
    // between Account A and Account B via the global jar!).
    use_cookies(
        &ctx,
        UseCookiesOptions {
            cookies: vec![
                "SESSDATA=sess_a; bili_jct=csrf_a; extra_a=only_for_a".to_owned(),
                "SESSDATA=sess_b; bili_jct=csrf_b".to_owned(),
            ],
            ..Default::default()
        },
    )
    .await
    .expect("use_cookies should succeed for both accounts");

    let acc_a = ctx.db.get_account(1001).await.unwrap();
    let acc_b = ctx.db.get_account(1002).await.unwrap();
    let map_a = parse_cookie_header(&acc_a.cookies);
    let map_b = parse_cookie_header(&acc_b.cookies);
    assert_eq!(map_a.get("SESSDATA").map(String::as_str), Some("sess_a"));
    assert_eq!(map_b.get("SESSDATA").map(String::as_str), Some("sess_b"));
    assert!(
        !map_b.contains_key("extra_a"),
        "Account A cookies bled into Account B: {map_b:?}"
    );

    // Seed expired bili_ticket on Account A to prove `like` replaces it by cookie NAME (not value).
    ctx.db
        .update_account_cookies(
            1001,
            "SESSDATA=sess_a; bili_jct=csrf_a; bili_ticket=stale_ticket; bili_ticket_expires=1"
                .to_owned(),
            None,
        )
        .await
        .unwrap();

    like(
        &ctx,
        LikeOptions {
            targets: vec!["av170001".to_owned()],
            ..Default::default()
        },
    )
    .await
    .expect("like should succeed for both accounts");

    let refreshed_a = parse_cookie_header(&ctx.db.get_account(1001).await.unwrap().cookies);
    let refreshed_b = parse_cookie_header(&ctx.db.get_account(1002).await.unwrap().cookies);
    assert_eq!(
        refreshed_a.get("bili_ticket").map(String::as_str),
        Some("ticket_for_csrf_a")
    );
    assert_eq!(
        refreshed_b.get("bili_ticket").map(String::as_str),
        Some("ticket_for_csrf_b")
    );
    assert_eq!(
        refreshed_a.get("buvid3").map(String::as_str),
        Some("MOCK-BUVID3-0001")
    );
    assert!(!refreshed_a.contains_key("stale_ticket"));

    // Logout all accounts concurrently/sequentially without CSRF cross-talk.
    logout(
        &ctx,
        LogoutOptions {
            all: true,
            yes: true,
            ..Default::default()
        },
    )
    .await
    .expect("logout --all --yes should succeed without CSRF cross-talk");

    assert!(ctx.db.all_accounts().await.unwrap().is_empty());
}

#[tokio::test]
async fn fetch_partial_failure_media_set_unlink_and_prune_order() {
    let mut state = MockState::default();
    let mut acc = MockAccount::active(1001, "Alice");
    acc.sets = vec![
        MockSet {
            id: 10,
            title: "ActiveFav".to_owned(),
            media_count: 2,
        },
        MockSet {
            id: 20,
            title: "InactiveFav".to_owned(),
            media_count: 1,
        },
    ];
    // 55 followings -> 2 pages (50 + 5). Force page 2 to fail!
    acc.followings = (1..=55)
        .map(|i| MockUp {
            mid: 5000 + i,
            name: format!("Up{i}"),
        })
        .collect();
    acc.following_error_page = Some(2);
    state.accounts_by_sessdata.insert("sess_a".to_owned(), acc);

    // Set 10 initially has only media 100 (media 101 was unfavorited remotely!)
    state.set_medias.insert(
        10,
        vec![MockMediaItem {
            id: 100,
            bvid: "BV17x411w7KC".to_owned(),
            title: "Kept Video".to_owned(),
            media_type: 2,
            attr: 0,
            upper: Some(MockUp {
                mid: 9001,
                name: "Creator1".to_owned(),
            }),
        }],
    );

    let server = MockBiliServer::start(state).await;
    let tmp = tempfile::tempdir().unwrap();
    let ws = Workspace::new(tmp.path().to_path_buf());
    ws.ensure_dirs().unwrap();
    let db = Db::open_in_memory().await.unwrap();

    // Pre-populate account, active set 10, inactive set 20, stale media 101 in set 10,
    // completed media 200 in inactive set 20, pending media 201 in inactive set 20,
    // and existing followed UP 5055 (which lives on failing page 2!).
    db.upsert_account(account::Model {
        account_id: 1001,
        name: "Alice".to_owned(),
        cookies: "SESSDATA=sess_a; bili_jct=csrf_a".to_owned(),
        state: AccountState::Active.to_string(),
        refresh_token: None,
    })
    .await
    .unwrap();
    db.upsert_sets([
        set::Model {
            set_id: 10,
            name: "ActiveFav".to_owned(),
            count: 2,
            state: SetState::Active.to_string(),
        },
        set::Model {
            set_id: 20,
            name: "InactiveFav".to_owned(),
            count: 2,
            state: SetState::Inactive.to_string(),
        },
    ])
    .await
    .unwrap();
    db.sync_account_ups(
        1001,
        vec![up::Model {
            up_id: 5055,
            name: "Up55".to_owned(),
            state: UpState::Inactive.to_string(),
        }],
        true,
    )
    .await
    .unwrap();
    db.upsert_medias([
        media::Model {
            id: 101,
            bv_id: "BV1stale000101".to_owned(),
            title: "Unfavorited Video".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Pending.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        },
        media::Model {
            id: 200,
            bv_id: "BV1done0000200".to_owned(),
            title: "Already Completed".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Completed.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        },
        media::Model {
            id: 201,
            bv_id: "BV1drop0000201".to_owned(),
            title: "Inactive Pending".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Pending.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        },
    ])
    .await
    .unwrap();
    db.upsert_media_sets([
        media_set::Model {
            id: 101,
            set_id: 10,
        },
        media_set::Model {
            id: 200,
            set_id: 20,
        },
        media_set::Model {
            id: 201,
            set_id: 20,
        },
    ])
    .await
    .unwrap();

    let ctx = AppContext::for_test(
        ws,
        Config::default(),
        db,
        Arc::new(MockMuxer::succeeding()),
        server.client_config(),
    );

    // Fetch with `--prune --ignore-errors`: page 2 of followings fails so UP 5055 MUST survive!
    // Meanwhile Set 10 succeeds completely so stale media 101 IS unlinked & pruned, while
    // Completed media 200 is preserved!
    fetch(
        &ctx,
        FetchOptions {
            full: true,
            prune: true,
            ignore_errors: true,
            ..Default::default()
        },
    )
    .await
    .expect("fetch --ignore-errors should complete");

    let followed = ctx.db.get_up_ids_of_account(1001).await.unwrap();
    assert!(
        followed.contains(&5055),
        "partial following page failure must NOT unlink existing UP 5055"
    );

    let set10_medias = ctx.db.get_media_ids_of_set(10).await.unwrap();
    assert_eq!(
        set10_medias,
        vec![100],
        "unfavorited media 101 must be unlinked from set 10"
    );

    assert!(ctx.db.get_media(100).await.is_ok());
    assert!(
        ctx.db.get_media(101).await.is_err(),
        "unlinked pending media 101 should be pruned"
    );
    assert!(
        ctx.db.get_media(200).await.is_ok(),
        "Completed media 200 must be preserved by default prune"
    );
    assert!(
        ctx.db.get_media(201).await.is_err(),
        "inactive Pending media 201 should be pruned"
    );
}

#[tokio::test]
async fn pull_error_classification_resume_failover_multipage_cjk_and_no_mute_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let sample_dir = tmp.path().join("samples");
    std::fs::create_dir_all(&sample_dir).unwrap();
    let (sample_video, sample_audio) = generate_tiny_av_streams(&sample_dir);
    let video_bytes = std::fs::read(&sample_video).unwrap();
    let audio_bytes = std::fs::read(&sample_audio).unwrap();

    let mut state = MockState::default();
    state
        .accounts_by_sessdata
        .insert("sess_a".to_owned(), MockAccount::active(1001, "Alice"));

    // Media 10: multi-page + very long CJK title (>240 bytes) + Range resume + backup_url failover!
    let long_cjk_title = "测试超长中文标题防溢出校验".repeat(12);
    state.views.insert(
        10,
        MockViewResponse::Ok {
            aid: 10,
            bvid: "BV1multi000010".to_owned(),
            title: long_cjk_title.clone(),
            owner: MockUp {
                mid: 77,
                name: "Up77".to_owned(),
            },
            pages: vec![
                MockPage {
                    cid: 101,
                    page: 1,
                    part: "第一集".to_owned(),
                },
                MockPage {
                    cid: 102,
                    page: 2,
                    part: "第二集".to_owned(),
                },
            ],
        },
    );
    // Page 2 video fails primary URL (403) and succeeds on backup URL with Range resume!
    state.playurls.insert(
        (10, 102),
        MockPlayurlResponse::Dash {
            videos: vec![MockStreamSpec {
                id: 80,
                path: "/cdn/v10_fail.mp4".to_owned(),
                backup_paths: vec!["/cdn/v10_backup.mp4".to_owned()],
                mime_type: "video/mp4".to_owned(),
                codecs: "avc1.640032".to_owned(),
                bandwidth: 1_000_000,
                codecid: 7,
            }],
            audios: Some(vec![MockStreamSpec {
                id: 30280,
                path: "/cdn/a10.m4a".to_owned(),
                backup_paths: vec![],
                mime_type: "audio/mp4".to_owned(),
                codecs: "mp4a.40.2".to_owned(),
                bandwidth: 128_000,
                codecid: 0,
            }]),
        },
    );
    state
        .cdn_assets
        .insert("/cdn/v10_fail.mp4".to_owned(), MockCdnAsset::failing(403));
    let mut resumable_video = MockCdnAsset::ok(video_bytes.clone());
    resumable_video.truncate_first_unranged = Some(video_bytes.len() / 2);
    state
        .cdn_assets
        .insert("/cdn/v10_backup.mp4".to_owned(), resumable_video);
    state
        .cdn_assets
        .insert("/cdn/a10.m4a".to_owned(), MockCdnAsset::ok(audio_bytes));

    // Media 11: transient -412 rate limit -> must become Failed (NOT Expired!)
    state.views.insert(
        11,
        MockViewResponse::Error {
            code: -412,
            message: "rate limited".to_owned(),
        },
    );
    // Media 12: terminal -404 -> must become Expired!
    state.views.insert(
        12,
        MockViewResponse::Error {
            code: -404,
            message: "video deleted".to_owned(),
        },
    );
    // Media 13: dash.audio present but returns 403 -> must FAIL instead of saving mute video!
    state.views.insert(
        13,
        MockViewResponse::Ok {
            aid: 13,
            bvid: "BV1nomute00013".to_owned(),
            title: "No Mute Fallback".to_owned(),
            owner: MockUp {
                mid: 77,
                name: "Up77".to_owned(),
            },
            pages: vec![MockPage {
                cid: 131,
                page: 1,
                part: "P1".to_owned(),
            }],
        },
    );
    state.playurls.insert(
        (13, 131),
        MockPlayurlResponse::Dash {
            videos: vec![MockStreamSpec {
                id: 80,
                path: "/cdn/v13_ok.mp4".to_owned(),
                backup_paths: vec![],
                mime_type: "video/mp4".to_owned(),
                codecs: "avc1.640032".to_owned(),
                bandwidth: 1_000_000,
                codecid: 7,
            }],
            audios: Some(vec![MockStreamSpec {
                id: 30280,
                path: "/cdn/a13_403.m4a".to_owned(),
                backup_paths: vec![],
                mime_type: "audio/mp4".to_owned(),
                codecs: "mp4a.40.2".to_owned(),
                bandwidth: 128_000,
                codecid: 0,
            }]),
        },
    );
    state
        .cdn_assets
        .insert("/cdn/v13_ok.mp4".to_owned(), MockCdnAsset::ok(video_bytes));
    state
        .cdn_assets
        .insert("/cdn/a13_403.m4a".to_owned(), MockCdnAsset::failing(403));

    let server = MockBiliServer::start(state).await;

    let ws_dir = tmp.path().join("workspace");
    let out_dir = tmp.path().join("downloads");
    std::fs::create_dir_all(&out_dir).unwrap();
    let ws = Workspace::new(ws_dir);
    ws.ensure_dirs().unwrap();
    let db = Db::open_in_memory().await.unwrap();

    db.upsert_account(account::Model {
        account_id: 1001,
        name: "Alice".to_owned(),
        cookies: "SESSDATA=sess_a; bili_jct=csrf_a".to_owned(),
        state: AccountState::Active.to_string(),
        refresh_token: None,
    })
    .await
    .unwrap();
    db.upsert_sets([set::Model {
        set_id: 1,
        name: "Main".to_owned(),
        count: 4,
        state: SetState::Active.to_string(),
    }])
    .await
    .unwrap();
    db.upsert_medias([
        media::Model {
            id: 10,
            bv_id: "BV1multi000010".to_owned(),
            title: long_cjk_title,
            r#type: MediaType::Video.to_string(),
            state: MediaState::Pending.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        },
        media::Model {
            id: 11,
            bv_id: "BV1rate0000011".to_owned(),
            title: "Rate Limited".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Pending.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        },
        media::Model {
            id: 12,
            bv_id: "BV1gone0000012".to_owned(),
            title: "Deleted".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Pending.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        },
        media::Model {
            id: 13,
            bv_id: "BV1nomute00013".to_owned(),
            title: "No Mute Fallback".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Pending.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        },
    ])
    .await
    .unwrap();
    db.upsert_media_sets([
        media_set::Model { id: 10, set_id: 1 },
        media_set::Model { id: 11, set_id: 1 },
        media_set::Model { id: 12, set_id: 1 },
        media_set::Model { id: 13, set_id: 1 },
    ])
    .await
    .unwrap();

    // Pre-mark page 1 (cid 101) of media 10 as already Completed on disk so pull skips page 1!
    let p1_file = out_dir.join("10-existing_p1.mp4");
    std::fs::write(&p1_file, b"already downloaded p1").unwrap();
    db.mark_page_completed(
        10,
        101,
        1,
        "第一集".to_owned(),
        "10-existing_p1.mp4".to_owned(),
    )
    .await
    .unwrap();

    let muxer: Arc<dyn Muxer> = Arc::new(FfmpegMuxer::new(None));
    let ctx = AppContext::for_test(ws, Config::default(), db, muxer, server.client_config());

    let res = pull(
        &ctx,
        PullOptions {
            jobs: Some(2),
            output_dir: Some(out_dir.clone()),
            ..Default::default()
        },
    )
    .await;
    // Partial failure expected because medias 11, 12, 13 failed
    assert!(res.is_err());

    // Media 10 succeeded via real FfmpegMuxer, skipped cid 101, and kept filename <= 240 bytes with `(2)`!
    assert_eq!(
        ctx.db.get_media(10).await.unwrap().state,
        MediaState::Completed.as_str()
    );
    let pages = ctx.db.pages_for_media(10).await.unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[1].state, MediaPageState::Completed.as_str());
    let p2_rel = pages[1]
        .local_path
        .as_deref()
        .expect("page 2 local_path recorded");
    let p2_name = PathBuf::from(p2_rel)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(
        p2_name.len() <= 240,
        "filename exceeded 240 bytes ({}): {p2_name}",
        p2_name.len()
    );
    assert!(
        p2_name.contains("(2)"),
        "multi-page marker (2) lost during truncation: {p2_name}"
    );
    assert!(out_dir.join(p2_rel).is_file());

    // Media 11 (-412) MUST be Failed (retryable), NOT permanently Expired!
    assert_eq!(
        ctx.db.get_media(11).await.unwrap().state,
        MediaState::Failed.as_str()
    );
    // Media 12 (-404) MUST be Expired!
    assert_eq!(
        ctx.db.get_media(12).await.unwrap().state,
        MediaState::Expired.as_str()
    );
    // Media 13 (audio 403) MUST be Failed, and NO mute `.mp4` saved!
    assert_eq!(
        ctx.db.get_media(13).await.unwrap().state,
        MediaState::Failed.as_str()
    );
    let mute_files: Vec<_> = std::fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("13-"))
        .collect();
    assert!(
        mute_files.is_empty(),
        "mute video must NOT be saved when dash.audio fails"
    );
}

#[tokio::test]
async fn pull_auth_expired_101_marks_account_expired() {
    let mut state = MockState::default();
    state.accounts_by_sessdata.insert(
        "sess_exp".to_owned(),
        MockAccount::active(2001, "ExpiredUser"),
    );
    state.views.insert(
        50,
        MockViewResponse::Error {
            code: -101,
            message: "not logged in".to_owned(),
        },
    );
    let server = MockBiliServer::start(state).await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = Workspace::new(tmp.path().to_path_buf());
    ws.ensure_dirs().unwrap();
    let db = Db::open_in_memory().await.unwrap();

    db.upsert_account(account::Model {
        account_id: 2001,
        name: "ExpiredUser".to_owned(),
        cookies: "SESSDATA=sess_exp; bili_jct=csrf_exp".to_owned(),
        state: AccountState::Active.to_string(),
        refresh_token: None,
    })
    .await
    .unwrap();
    db.upsert_sets([set::Model {
        set_id: 1,
        name: "S".to_owned(),
        count: 1,
        state: SetState::Active.to_string(),
    }])
    .await
    .unwrap();
    db.upsert_medias([media::Model {
        id: 50,
        bv_id: "BV1exp00000050".to_owned(),
        title: "Need Login".to_owned(),
        r#type: MediaType::Video.to_string(),
        state: MediaState::Pending.to_string(),
        retry_count: 0,
        last_error: None,
        updated_at: None,
    }])
    .await
    .unwrap();
    db.upsert_media_sets([media_set::Model { id: 50, set_id: 1 }])
        .await
        .unwrap();

    let ctx = AppContext::for_test(
        ws,
        Config::default(),
        db,
        Arc::new(MockMuxer::succeeding()),
        server.client_config(),
    );

    let res = pull(&ctx, PullOptions::default()).await;
    assert!(res.is_err());
    assert_eq!(
        ctx.db.get_account(2001).await.unwrap().state,
        AccountState::Expired.as_str(),
        "-101 must transition account state to Expired"
    );
    assert_ne!(
        ctx.db.get_media(50).await.unwrap().state,
        MediaState::Expired.as_str(),
        "-101 auth error must NOT permanently mark the media as Expired"
    );
}

#[tokio::test]
async fn cli_binary_end_to_end_help_exit_codes_json_schedule_completion_doctor_and_lock() {
    let bin = env!("CARGO_BIN_EXE_fav");

    // 1. `--help` reports binary name `fav`
    let help = Command::new(bin).arg("--help").output().unwrap();
    assert!(help.status.success());
    let help_stdout = String::from_utf8_lossy(&help.stdout);
    assert!(
        help_stdout.contains("Usage: fav"),
        "expected `Usage: fav` in --help, got:\n{help_stdout}"
    );

    // 2. Missing workspace exits non-zero (Usage = 2), error on stderr, stdout empty!
    let empty_dir = tempfile::tempdir().unwrap();
    let missing = Command::new(bin)
        .args(["-C", empty_dir.path().to_str().unwrap(), "status"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    assert!(missing.stdout.is_empty(), "errors must not pollute stdout");
    assert!(
        !missing.stderr.is_empty(),
        "error message expected on stderr"
    );

    // 3. Seed a workspace DB with one account and verify JSON output redacts cookies
    let ws_dir = tempfile::tempdir().unwrap();
    let ws = Workspace::new(ws_dir.path().to_path_buf());
    ws.ensure_dirs().unwrap();
    let db = Db::open_or_create(&ws.db_path()).await.unwrap();
    db.upsert_account(account::Model {
        account_id: 42,
        name: "Alice".to_owned(),
        cookies: "SESSDATA=top_secret_cookie; bili_jct=secret_csrf".to_owned(),
        state: AccountState::Active.to_string(),
        refresh_token: Some("secret_refresh".to_owned()),
    })
    .await
    .unwrap();
    drop(db);

    let ls_json = Command::new(bin)
        .args([
            "-C",
            ws_dir.path().to_str().unwrap(),
            "ls",
            "account",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(
        ls_json.status.success(),
        "ls account --format json failed: {}",
        String::from_utf8_lossy(&ls_json.stderr)
    );
    let ls_stdout = String::from_utf8_lossy(&ls_json.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&ls_stdout).expect("valid JSON on stdout");
    assert!(parsed.is_array());
    assert!(!ls_stdout.contains("top_secret_cookie"));
    assert!(!ls_stdout.contains("secret_csrf"));
    assert!(!ls_stdout.contains("secret_refresh"));

    // 4. `fav status --json`
    let status = Command::new(bin)
        .args(["-C", ws_dir.path().to_str().unwrap(), "status", "--json"])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "status --json failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status_val: serde_json::Value =
        serde_json::from_slice(&status.stdout).expect("valid status JSON");
    assert_eq!(status_val["accounts"]["active"], 1);

    // 5. `fav doctor --offline --json`
    let doctor = Command::new(bin)
        .args([
            "-C",
            ws_dir.path().to_str().unwrap(),
            "doctor",
            "--offline",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        doctor.status.success(),
        "doctor --offline --json failed: stdout={} stderr={}",
        String::from_utf8_lossy(&doctor.stdout),
        String::from_utf8_lossy(&doctor.stderr),
    );
    let _: serde_json::Value = serde_json::from_slice(&doctor.stdout).expect("valid doctor JSON");

    // 6. `fav schedule` & `fav completion` (no DB required!)
    let sched = Command::new(bin)
        .args(["schedule", "--target", "systemd", "--interval", "3h"])
        .output()
        .unwrap();
    assert!(
        sched.status.success(),
        "schedule failed: {}",
        String::from_utf8_lossy(&sched.stderr)
    );
    assert!(String::from_utf8_lossy(&sched.stdout).contains("[Timer]"));

    for shell in ["bash", "zsh", "fish", "powershell"] {
        let comp = Command::new(bin)
            .args(["completion", shell])
            .output()
            .unwrap();
        assert!(
            comp.status.success(),
            "completion {shell} failed: {}",
            String::from_utf8_lossy(&comp.stderr)
        );
        assert!(!comp.stdout.is_empty());
    }

    // 7. Workspace lock contention: holding `.fav/fav.lock` blocks mutating commands cleanly!
    let _held_lock = ws.acquire_lock().unwrap();
    let locked = Command::new(bin)
        .args([
            "-C",
            ws_dir.path().to_str().unwrap(),
            "activate",
            "account",
            "42",
        ])
        .output()
        .unwrap();
    assert!(!locked.status.success());
    let locked_err = String::from_utf8_lossy(&locked.stderr);
    assert!(
        locked_err.to_ascii_lowercase().contains("lock")
            || locked_err.to_ascii_lowercase().contains("running"),
        "expected lock contention message on stderr, got: {locked_err}"
    );
}

#[tokio::test]
async fn cli_broken_pipe_exits_cleanly_without_panicking() {
    use std::io::{BufRead as _, BufReader, Read as _};
    use std::process::{Command, Stdio};

    let bin = env!("CARGO_BIN_EXE_fav");
    let ws_dir = tempfile::TempDir::new().unwrap();
    let ws = Workspace::discover_or_init(Some(ws_dir.path())).unwrap();
    ws.ensure_dirs().unwrap();
    let db = Db::open_or_create(&ws.db_path()).await.unwrap();

    // Insert enough rows (~200 KB output) to guarantee overflowing the OS pipe buffer
    // once the reader closes stdout after 1 line (just like `fav ls v | head -n 1`).
    let rows: Vec<media::Model> = (1..=1500)
        .map(|i| media::Model {
            id: 10_000 + i,
            bv_id: format!("BV1pipe{i:06}"),
            title: format!("洛天依，言和原创《普通DISCO》测试标题用于管道截断验证_{i:04}"),
            r#type: "Video".to_owned(),
            state: MediaState::Completed.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        })
        .collect();
    db.upsert_medias(rows).await.unwrap();
    drop(db);

    let mut child = Command::new(bin)
        .args([
            "-C",
            ws_dir.path().to_str().unwrap(),
            "--color",
            "always",
            "ls",
            "v",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut first_line = String::new();
    reader.read_line(&mut first_line).unwrap();
    assert!(
        first_line.starts_with('|'),
        "unexpected line: {first_line:?}"
    );
    // Close stdout read end immediately so `fav` receives EPIPE while writing the rest.
    drop(reader);

    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    let status = child.wait().unwrap();

    assert!(
        status.success(),
        "expected clean exit 0 on BrokenPipe, got {status:?}, stderr={stderr:?}"
    );
    assert!(
        !stderr.contains("panicked") && !stderr.contains("Broken pipe"),
        "unexpected panic/error on stderr: {stderr:?}"
    );
}
