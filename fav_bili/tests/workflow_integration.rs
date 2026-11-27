mod common;

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use common::{
    MockAccount, MockBiliServer, MockCdnAsset, MockCollectedSet, MockMediaItem, MockPage,
    MockPlayurlResponse, MockSet, MockState, MockStreamSpec, MockUp, MockViewResponse,
    generate_tiny_av_streams, generate_tiny_durl_mp4,
};
use fav_bili::{
    action::{
        auth::{LogoutOptions, UseCookiesOptions, logout, use_cookies},
        doctor::{DoctorOptions, doctor_with_options},
        download::{DownloadOptions, download_with_env},
        fetch::{FetchOptions, fetch},
        like::{LikeOptions, like},
        media::{MediaResetOptions, reset as media_reset},
        pull::{PullOptions, pull},
    },
    config::{Config, Workspace},
    context::{AppContext, RuntimeOptions},
    cookies::parse_cookie_header,
    db::{Db, PullFilter},
    entity::{account, media, media_set, set, set_account, up, up_account},
    error::ExitKind,
    id_parse::aid_to_bvid,
    muxer::{FfmpegMuxer, Muxer, test_support::MockMuxer},
    reconcile::reconcile_completed_medias,
    state::{
        AccountState, AudioFormat, MediaPageState, MediaState, MediaType, SetKind, SetState,
        UpState, VideoClarity, VideoCodec,
    },
    table::OutputFormat,
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
            kind: SetKind::Created.to_string(),
            owner_mid: Some(1001),
            sync_complete: false,
            resume_page: None,
        },
        set::Model {
            set_id: 20,
            name: "InactiveFav".to_owned(),
            count: 2,
            state: SetState::Inactive.to_string(),
            kind: SetKind::Created.to_string(),
            owner_mid: Some(1001),
            sync_complete: false,
            resume_page: None,
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
            sync_complete: false,
            resume_page: None,
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
        kind: SetKind::Created.to_string(),
        owner_mid: Some(1001),
        sync_complete: false,
        resume_page: None,
    }])
    .await
    .unwrap();
    db.upsert_set_accounts([set_account::Model {
        set_id: 1,
        account_id: 1001,
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
        None,
        None,
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
        kind: SetKind::Created.to_string(),
        owner_mid: Some(2001),
        sync_complete: false,
        resume_page: None,
    }])
    .await
    .unwrap();
    db.upsert_set_accounts([set_account::Model {
        set_id: 1,
        account_id: 2001,
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

    for shell in ["bash", "zsh", "fish", "powershell", "pwsh", "nushell", "nu"] {
        let comp = Command::new(bin)
            .args(["completion", shell])
            .output()
            .unwrap();
        assert!(
            comp.status.success(),
            "completion {shell} failed: {}",
            String::from_utf8_lossy(&comp.stderr)
        );
        let script = String::from_utf8_lossy(&comp.stdout);
        assert!(!script.is_empty(), "empty completion script for {shell}");
        match shell {
            "powershell" | "pwsh" => {
                assert!(
                    script.contains("Register-ArgumentCompleter"),
                    "PowerShell completion missing Register-ArgumentCompleter"
                );
                assert!(
                    script.contains("'-C'") && script.contains("'--dir'"),
                    "PowerShell completion missing -C/--dir forwarding"
                );
            }
            "nushell" | "nu" => {
                assert!(
                    script.contains("nu-complete fav sets") && script.contains("export extern"),
                    "Nushell completion missing dynamic completers or externs"
                );
            }
            "bash" | "zsh" | "fish" => {
                assert!(
                    script.contains("-C") && script.contains("--dir"),
                    "{shell} completion missing -C/--dir forwarding"
                );
            }
            _ => {}
        }
    }

    let nu_static = Command::new(bin)
        .args(["completion", "nushell", "--static"])
        .output()
        .unwrap();
    assert!(nu_static.status.success());
    let nu_static_out = String::from_utf8_lossy(&nu_static.stdout);
    assert!(
        !nu_static_out.contains("nu-complete fav sets"),
        "nushell --static must omit dynamic DB completers"
    );

    // 6b. `fav download --help` and `fav dl --help`
    for sub in ["download", "dl"] {
        let dl_help = Command::new(bin).args([sub, "--help"]).output().unwrap();
        assert!(dl_help.status.success(), "{sub} --help failed");
        let text = String::from_utf8_lossy(&dl_help.stdout);
        for flag in [
            "--page",
            "--all-pages",
            "--sound-only",
            "--audio-format",
            "--clarity",
            "--codec",
            "--anonymous",
        ] {
            assert!(
                text.contains(flag),
                "expected {flag} in `fav {sub} --help`, got:\n{text}"
            );
        }
    }

    // 6c. `fav pull <untracked>` exits with code 2 (Usage) and suggests `fav download`, 0 DB rows!
    let untracked_pull = Command::new(bin)
        .args([
            "-C",
            ws_dir.path().to_str().unwrap(),
            "pull",
            "BV1Q541167Qg",
        ])
        .output()
        .unwrap();
    assert_eq!(
        untracked_pull.status.code(),
        Some(2),
        "expected exit code 2 for untracked pull, stderr={}",
        String::from_utf8_lossy(&untracked_pull.stderr)
    );
    assert!(
        untracked_pull.stdout.is_empty(),
        "untracked pull error must not pollute stdout"
    );
    assert!(
        String::from_utf8_lossy(&untracked_pull.stderr).contains("fav download"),
        "expected `fav download` hint in stderr: {}",
        String::from_utf8_lossy(&untracked_pull.stderr)
    );

    // 6d. Seed sets of each kind + legacy Completed media + >200 medias for CLI E2E checks:
    //     `fav ls set --kind`, `fav doctor -o`, `fav media reset --missing -o`, and `__complete medias`
    let custom_out = tempfile::tempdir().unwrap();
    {
        let db = Db::open_existing(&ws.db_path(), false).await.unwrap();
        assert!(
            db.all_medias().await.unwrap().is_empty(),
            "rejected untracked pull must insert 0 media rows"
        );
        db.upsert_sets([
            set::Model {
                set_id: 100,
                name: "MyCreated".to_owned(),
                count: 1,
                state: SetState::Active.to_string(),
                kind: SetKind::Created.to_string(),
                owner_mid: Some(42),
                sync_complete: true,
                resume_page: None,
            },
            set::Model {
                set_id: 200,
                name: "OtherCollected".to_owned(),
                count: 2,
                state: SetState::Inactive.to_string(),
                kind: SetKind::Collected.to_string(),
                owner_mid: Some(99),
                sync_complete: false,
                resume_page: None,
            },
            set::Model {
                set_id: 300,
                name: "RustSeason".to_owned(),
                count: 3,
                state: SetState::Inactive.to_string(),
                kind: SetKind::Season.to_string(),
                owner_mid: Some(88),
                sync_complete: false,
                resume_page: None,
            },
        ])
        .await
        .unwrap();

        // Seed 220 medias (media #220 is past index 200 for `__complete medias` filter-before-take check)
        // plus legacy Completed media 9001 missing from `custom_out`.
        let mut seed_medias: Vec<media::Model> = (1..=220)
            .map(|i| media::Model {
                id: i,
                bv_id: aid_to_bvid(i).unwrap(),
                title: if i == 220 {
                    "NeedlePastIndex200".to_owned()
                } else {
                    format!("FillerVideo_{i:03}")
                },
                r#type: MediaType::Video.to_string(),
                state: MediaState::Pending.to_string(),
                retry_count: 0,
                last_error: None,
                updated_at: None,
            })
            .collect();
        seed_medias.push(media::Model {
            id: 9001,
            bv_id: aid_to_bvid(9001).unwrap(),
            title: "LegacyMissingClip".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Completed.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        });
        db.upsert_medias(seed_medias).await.unwrap();
    }

    let ls_season = Command::new(bin)
        .args([
            "-C",
            ws_dir.path().to_str().unwrap(),
            "ls",
            "set",
            "--kind",
            "season",
            "--format",
            "tsv",
        ])
        .output()
        .unwrap();
    assert!(ls_season.status.success());
    let ls_season_out = String::from_utf8_lossy(&ls_season.stdout);
    assert!(ls_season_out.contains("RustSeason"));
    assert!(!ls_season_out.contains("MyCreated"));
    assert!(!ls_season_out.contains("OtherCollected"));

    let comp_past_200 = Command::new(bin)
        .args([
            "-C",
            ws_dir.path().to_str().unwrap(),
            "__complete",
            "medias",
            "NeedlePastIndex200",
        ])
        .output()
        .unwrap();
    assert!(comp_past_200.status.success());
    let comp_out = String::from_utf8_lossy(&comp_past_200.stdout);
    assert!(
        comp_out.contains("NeedlePastIndex200"),
        "expected __complete medias to find item past index 200, got: {comp_out}"
    );

    let doc_o = Command::new(bin)
        .args([
            "-C",
            ws_dir.path().to_str().unwrap(),
            "doctor",
            "--offline",
            "-o",
            custom_out.path().to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(doc_o.status.success());
    let doc_checks: serde_json::Value = serde_json::from_slice(&doc_o.stdout).unwrap();
    let disk_sync = doc_checks
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "disk_sync")
        .expect("disk_sync check present");
    assert_eq!(disk_sync["status"], "warn");
    assert!(
        disk_sync["detail"]
            .as_str()
            .unwrap()
            .contains("1 missing legacy media"),
        "expected missing legacy media in doctor -o output: {disk_sync}"
    );

    let reset_o = Command::new(bin)
        .args([
            "-C",
            ws_dir.path().to_str().unwrap(),
            "media",
            "reset",
            "--missing",
            "-o",
            custom_out.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        reset_o.status.success(),
        "media reset --missing -o failed: {}",
        String::from_utf8_lossy(&reset_o.stderr)
    );
    {
        let db = Db::open_existing(&ws.db_path(), true).await.unwrap();
        assert_eq!(
            db.get_media(9001).await.unwrap().state,
            MediaState::Pending.as_str(),
            "media reset --missing -o must reset missing legacy media to Pending"
        );
    }

    // 7. Workspace lock contention: holding `.fav/fav.lock` blocks mutating commands cleanly,
    //    while untracked `fav download --dry-run` still succeeds without taking the lock!
    let _held_lock = ws.acquire_lock().unwrap();
    let dl_while_locked = Command::new(bin)
        .args([
            "-C",
            ws_dir.path().to_str().unwrap(),
            "download",
            "--dry-run",
            "BV1Q541167Qg",
        ])
        .output()
        .unwrap();
    assert!(
        dl_while_locked.status.success(),
        "fav download must never block on WorkspaceLock: {}",
        String::from_utf8_lossy(&dl_while_locked.stderr)
    );
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

fn ffprobe_stream_types(path: &Path) -> Vec<String> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .expect("run ffprobe");
    assert!(
        out.status.success(),
        "ffprobe failed on {}: {}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn list_files_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    files
}

fn make_dash_spec(id: u32, path: &str, codecid: u32, bandwidth: u64) -> MockStreamSpec {
    MockStreamSpec {
        id,
        path: path.to_owned(),
        backup_paths: vec![],
        mime_type: "video/mp4".to_owned(),
        codecs: match codecid {
            12 => "hev1.1.6.L120.90".to_owned(),
            13 => "av01.0.04M.08".to_owned(),
            _ => "avc1.640032".to_owned(),
        },
        bandwidth,
        codecid,
    }
}

fn make_audio_spec(path: &str) -> MockStreamSpec {
    MockStreamSpec {
        id: 30280,
        path: path.to_owned(),
        backup_paths: vec![],
        mime_type: "audio/mp4".to_owned(),
        codecs: "mp4a.40.2".to_owned(),
        bandwidth: 192_000,
        codecid: 0,
    }
}

#[tokio::test]
async fn activation_strictness_enforced_across_fetch_pull_and_like() {
    let mut state = MockState::default();
    state
        .accounts_by_sessdata
        .insert("sess_a".to_owned(), MockAccount::active(1, "Alice"));
    state
        .accounts_by_sessdata
        .insert("sess_b".to_owned(), MockAccount::active(2, "Bob"));
    let server = MockBiliServer::start(state).await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = Workspace::new(tmp.path().to_path_buf());
    ws.ensure_dirs().unwrap();
    let db = Db::open_in_memory().await.unwrap();

    db.upsert_account(account::Model {
        account_id: 1,
        name: "Alice".to_owned(),
        cookies: "SESSDATA=sess_a; bili_jct=csrf_a".to_owned(),
        state: AccountState::Active.to_string(),
        refresh_token: None,
    })
    .await
    .unwrap();
    db.upsert_account(account::Model {
        account_id: 2,
        name: "Bob".to_owned(),
        cookies: "SESSDATA=sess_b; bili_jct=csrf_b".to_owned(),
        state: AccountState::Inactive.to_string(),
        refresh_token: None,
    })
    .await
    .unwrap();

    db.upsert_sets([
        set::Model {
            set_id: 10,
            name: "AliceInactiveSet".to_owned(),
            count: 1,
            state: SetState::Inactive.to_string(),
            kind: SetKind::Created.to_string(),
            owner_mid: Some(1),
            sync_complete: true,
            resume_page: None,
        },
        set::Model {
            set_id: 11,
            name: "BobActiveSet".to_owned(),
            count: 1,
            state: SetState::Active.to_string(),
            kind: SetKind::Created.to_string(),
            owner_mid: Some(2),
            sync_complete: true,
            resume_page: None,
        },
        set::Model {
            set_id: 12,
            name: "AliceActiveSet".to_owned(),
            count: 1,
            state: SetState::Active.to_string(),
            kind: SetKind::Created.to_string(),
            owner_mid: Some(1),
            sync_complete: true,
            resume_page: None,
        },
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
            account_id: 2,
        },
        set_account::Model {
            set_id: 12,
            account_id: 1,
        },
    ])
    .await
    .unwrap();

    db.upsert_ups([
        up::Model {
            up_id: 30,
            name: "AliceInactiveUp".to_owned(),
            state: UpState::Inactive.to_string(),
            sync_complete: true,
            resume_page: None,
        },
        up::Model {
            up_id: 31,
            name: "BobActiveUp".to_owned(),
            state: UpState::Active.to_string(),
            sync_complete: true,
            resume_page: None,
        },
    ])
    .await
    .unwrap();
    db.upsert_up_accounts([
        up_account::Model {
            up_id: 30,
            account_id: 1,
        },
        up_account::Model {
            up_id: 31,
            account_id: 2,
        },
    ])
    .await
    .unwrap();

    let bv_100 = aid_to_bvid(100).unwrap();
    db.upsert_medias([
        media::Model {
            id: 170_001,
            bv_id: "BV17x411w7KC".to_owned(),
            title: "InInactiveSet".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Pending.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        },
        media::Model {
            id: 2,
            bv_id: "BV1xx411c7mD".to_owned(),
            title: "UnderInactiveAccount".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Pending.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        },
        media::Model {
            id: 100,
            bv_id: bv_100.clone(),
            title: "ActiveMedia".to_owned(),
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
            id: 170_001,
            set_id: 10,
        },
        media_set::Model { id: 2, set_id: 11 },
        media_set::Model {
            id: 100,
            set_id: 12,
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

    // 1. `fetch --set/--up` rejects inactive sets/UPs and sets/UPs on inactive accounts with 0 media API calls.
    for opts in [
        FetchOptions {
            set_ids: vec![10],
            ..Default::default()
        },
        FetchOptions {
            set_ids: vec![11],
            ..Default::default()
        },
        FetchOptions {
            up_ids: vec![30],
            ..Default::default()
        },
        FetchOptions {
            up_ids: vec![31],
            ..Default::default()
        },
    ] {
        let err = fetch(&ctx, opts).await.unwrap_err();
        assert_eq!(err.kind(), ExitKind::Usage);
    }
    assert_eq!(server.call_count("/x/v3/fav/resource/list"), 0);
    assert_eq!(server.call_count("/x/space/wbi/arc/search"), 0);

    // 2. `pull <untracked>` rejected with ExitKind::Usage + `fav download` hint, 0 DB rows inserted.
    let err = pull(
        &ctx,
        PullOptions {
            targets: vec!["BV1Q541167Qg".to_owned()],
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind(), ExitKind::Usage);
    assert!(err.to_string().contains("fav download"));
    assert!(ctx.db.get_media(455_017_605).await.is_err());

    // 3. `pull --set/--up` inactive & `<tracked_but_inactive>` rejected.
    for opts in [
        PullOptions {
            set_ids: vec![10],
            ..Default::default()
        },
        PullOptions {
            set_ids: vec![11],
            ..Default::default()
        },
        PullOptions {
            up_ids: vec![30],
            ..Default::default()
        },
        PullOptions {
            up_ids: vec![31],
            ..Default::default()
        },
    ] {
        let err = pull(&ctx, opts).await.unwrap_err();
        assert_eq!(err.kind(), ExitKind::Usage);
    }
    for inactive_target in ["BV17x411w7KC", "BV1xx411c7mD"] {
        let err = pull(
            &ctx,
            PullOptions {
                targets: vec![inactive_target.to_owned()],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ExitKind::Usage);
        assert!(err.to_string().contains("fav download"));
    }

    // 4. Deactivating Account 1 immediately hides its active Set 12 from pull_candidates & active checks.
    assert!(ctx.db.is_media_active(100).await.unwrap());
    assert_eq!(
        ctx.db
            .pull_candidates(&PullFilter::default())
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect::<Vec<_>>(),
        vec![100]
    );
    ctx.db.deactivate_account(1).await.unwrap();
    assert!(!ctx.db.is_media_active(100).await.unwrap());
    assert!(
        ctx.db
            .pull_candidates(&PullFilter::default())
            .await
            .unwrap()
            .is_empty()
    );
    let err = pull(
        &ctx,
        PullOptions {
            targets: vec![bv_100],
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind(), ExitKind::Usage);
    ctx.db.activate_account(1).await.unwrap();

    // 5. `like --account/--set` rejects inactive accounts and inactive/unowned sets with 0 API calls.
    for opts in [
        LikeOptions {
            targets: vec!["av100".to_owned()],
            account_ids: vec![2],
            ..Default::default()
        },
        LikeOptions {
            set_ids: vec![10],
            ..Default::default()
        },
        LikeOptions {
            set_ids: vec![11],
            ..Default::default()
        },
    ] {
        let err = like(&ctx, opts).await.unwrap_err();
        assert_eq!(err.kind(), ExitKind::Usage);
    }
    assert_eq!(server.call_count("/x/web-interface/archive/like"), 0);
    assert_eq!(server.call_count("/x/web-interface/wbi/view"), 0);
}

#[tokio::test]
async fn untracked_download_zero_db_writes_cookie_fallback_pages_skip_force_and_staging() {
    let tmp = tempfile::tempdir().unwrap();
    let sample_dir = tmp.path().join("samples");
    std::fs::create_dir_all(&sample_dir).unwrap();
    let (sample_video, sample_audio) = generate_tiny_av_streams(&sample_dir);
    let v_bytes = std::fs::read(&sample_video).unwrap();
    let a_bytes = std::fs::read(&sample_audio).unwrap();

    let aid = 455_017_605i64; // BV1Q541167Qg
    let mut state = MockState::default();
    state
        .accounts_by_sessdata
        .insert("sess_active".to_owned(), MockAccount::active(1001, "Alice"));
    state.views.insert(
        aid,
        MockViewResponse::Ok {
            aid,
            bvid: "BV1Q541167Qg".to_owned(),
            title: "MultiPartDemo".to_owned(),
            owner: MockUp {
                mid: 55,
                name: "Creator".to_owned(),
            },
            pages: vec![
                MockPage {
                    cid: 501,
                    page: 1,
                    part: "Intro".to_owned(),
                },
                MockPage {
                    cid: 502,
                    page: 2,
                    part: "Main".to_owned(),
                },
                MockPage {
                    cid: 503,
                    page: 3,
                    part: "Outro".to_owned(),
                },
            ],
        },
    );
    for cid in [501, 502, 503] {
        let vp = format!("/cdn/dl_v_{cid}.mp4");
        let ap = format!("/cdn/dl_a_{cid}.m4a");
        state.playurls.insert(
            (aid, cid),
            MockPlayurlResponse::Dash {
                videos: vec![make_dash_spec(80, &vp, 7, 1_500_000)],
                audios: Some(vec![make_audio_spec(&ap)]),
            },
        );
        state
            .cdn_assets
            .insert(vp, MockCdnAsset::ok(v_bytes.clone()));
        state
            .cdn_assets
            .insert(ap, MockCdnAsset::ok(a_bytes.clone()));
    }
    let server = MockBiliServer::start(state).await;

    // Seed an on-disk workspace with one active account AND hold WorkspaceLock concurrently!
    let ws_dir = tmp.path().join("ws");
    let ws = Workspace::new(ws_dir);
    ws.ensure_dirs().unwrap();
    {
        let rw_db = Db::open_or_create(&ws.db_path()).await.unwrap();
        rw_db
            .upsert_account(account::Model {
                account_id: 1001,
                name: "Alice".to_owned(),
                cookies: "SESSDATA=sess_active; bili_jct=csrf_active".to_owned(),
                state: AccountState::Active.to_string(),
                refresh_token: None,
            })
            .await
            .unwrap();
    }
    let _held_lock = ws.acquire_lock().expect("hold workspace lock");
    let ro_db = Db::open_existing(&ws.db_path(), true).await.unwrap();

    let muxer: Arc<dyn Muxer> = Arc::new(FfmpegMuxer::new(None));
    let runtime = RuntimeOptions {
        no_progress: true,
        non_interactive: true,
        ..Default::default()
    };

    // Case A: URL `?p=2` with read-only active account cookies (while WorkspaceLock is held!).
    let out_p2 = tmp.path().join("out_p2");
    download_with_env(
        Some(&ro_db),
        Config::default(),
        muxer.clone(),
        server.client_config(),
        runtime.clone(),
        DownloadOptions {
            targets: vec!["https://www.bilibili.com/video/BV1Q541167Qg/?p=2".to_owned()],
            output_dir: Some(out_p2.clone()),
            ..Default::default()
        },
    )
    .await
    .expect("download ?p=2 with read-only workspace cookies should succeed");

    let files_p2 = list_files_in(&out_p2);
    assert_eq!(files_p2.len(), 1, "only page 2 must be downloaded");
    assert!(
        files_p2[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("(2)"),
        "expected (2) in filename: {:?}",
        files_p2[0]
    );
    assert_eq!(server.cdn_hits("/cdn/dl_v_501.mp4"), 0);
    assert_eq!(server.cdn_hits("/cdn/dl_v_502.mp4"), 1);
    assert_eq!(server.cdn_hits("/cdn/dl_v_503.mp4"), 0);
    assert!(
        ro_db.all_medias().await.unwrap().is_empty(),
        "fav download must never write to SQLite"
    );
    assert!(
        !out_p2.join(".fav-tmp").exists(),
        ".fav-tmp must be automatically removed when empty"
    );
    let playurl_reqs = server.requests_for("/x/player/wbi/playurl");
    assert_eq!(playurl_reqs.len(), 1);
    assert!(
        playurl_reqs[0]
            .headers
            .get("cookie")
            .is_some_and(|c| c.contains("SESSDATA=sess_active")),
        "expected active account cookie on authenticated download"
    );
    assert!(
        !playurl_reqs[0].query.contains("try_look=1"),
        "authenticated download should not set try_look=1"
    );

    // Case B: Skip-existing (force=false -> 0 new CDN hits) vs --force (force=true -> overwrites).
    download_with_env(
        Some(&ro_db),
        Config::default(),
        muxer.clone(),
        server.client_config(),
        runtime.clone(),
        DownloadOptions {
            targets: vec!["https://www.bilibili.com/video/BV1Q541167Qg/?p=2".to_owned()],
            output_dir: Some(out_p2.clone()),
            force: false,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        server.cdn_hits("/cdn/dl_v_502.mp4"),
        1,
        "existing non-empty file must be skipped without re-downloading"
    );

    download_with_env(
        Some(&ro_db),
        Config::default(),
        muxer.clone(),
        server.client_config(),
        runtime.clone(),
        DownloadOptions {
            targets: vec!["https://www.bilibili.com/video/BV1Q541167Qg/?p=2".to_owned()],
            output_dir: Some(out_p2.clone()),
            force: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        server.cdn_hits("/cdn/dl_v_502.mp4"),
        2,
        "--force must overwrite existing file"
    );

    // Case C: `--page 1-2` overriding `?p=3` + `--anonymous` (`try_look=1` recorded, no account cookie!).
    server.clear_requests();
    let out_anon = tmp.path().join("out_anon");
    download_with_env(
        Some(&ro_db),
        Config::default(),
        muxer.clone(),
        server.client_config(),
        runtime.clone(),
        DownloadOptions {
            targets: vec!["https://www.bilibili.com/video/BV1Q541167Qg/?p=3".to_owned()],
            pages: Some("1-2".to_owned()),
            output_dir: Some(out_anon.clone()),
            anonymous: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let files_anon = list_files_in(&out_anon);
    assert_eq!(files_anon.len(), 2, "expected only pages 1 and 2");
    assert_eq!(server.cdn_hits("/cdn/dl_v_501.mp4"), 1);
    assert_eq!(server.cdn_hits("/cdn/dl_v_503.mp4"), 0);
    assert!(!out_anon.join(".fav-tmp").exists());
    let anon_playurls = server.requests_for("/x/player/wbi/playurl");
    assert_eq!(anon_playurls.len(), 2);
    for req in &anon_playurls {
        assert!(
            req.query.contains("try_look=1"),
            "anonymous download must send try_look=1, got query={}",
            req.query
        );
        assert!(
            !req.headers
                .get("cookie")
                .is_some_and(|c| c.contains("SESSDATA=sess_active")),
            "--anonymous must not send workspace account cookies"
        );
    }

    // Case D: `--all-pages` overriding `?p=2` with NO workspace (`db: None` -> anonymous `try_look=1`).
    server.clear_requests();
    let out_all = tmp.path().join("out_all");
    download_with_env(
        None,
        Config::default(),
        muxer,
        server.client_config(),
        runtime,
        DownloadOptions {
            targets: vec!["https://www.bilibili.com/video/BV1Q541167Qg/?p=2".to_owned()],
            all_pages: true,
            output_dir: Some(out_all.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(list_files_in(&out_all).len(), 3, "expected all 3 pages");
    assert!(!out_all.join(".fav-tmp").exists());
    let no_ws_playurls = server.requests_for("/x/player/wbi/playurl");
    assert_eq!(no_ws_playurls.len(), 3);
    assert!(
        no_ws_playurls
            .iter()
            .all(|r| r.query.contains("try_look=1"))
    );
    assert!(ro_db.all_medias().await.unwrap().is_empty());
}

#[tokio::test]
async fn sound_only_and_audio_formats_dash_zero_video_hits_durl_and_comment_tag() {
    let tmp = tempfile::tempdir().unwrap();
    let sample_dir = tmp.path().join("samples");
    std::fs::create_dir_all(&sample_dir).unwrap();
    let (sample_video, sample_audio) = generate_tiny_av_streams(&sample_dir);
    let sample_durl = generate_tiny_durl_mp4(&sample_dir, &sample_video, &sample_audio);
    let v_bytes = std::fs::read(&sample_video).unwrap();
    let a_bytes = std::fs::read(&sample_audio).unwrap();
    let durl_bytes = std::fs::read(&sample_durl).unwrap();

    let dash_aid = 700i64;
    let dash_bvid = aid_to_bvid(dash_aid).unwrap();
    let durl_aid = 701i64;
    let durl_bvid = aid_to_bvid(durl_aid).unwrap();

    let mut state = MockState::default();
    state
        .accounts_by_sessdata
        .insert("sess_a".to_owned(), MockAccount::active(1001, "Alice"));
    state.views.insert(
        dash_aid,
        MockViewResponse::Ok {
            aid: dash_aid,
            bvid: dash_bvid.clone(),
            title: "DashSong".to_owned(),
            owner: MockUp {
                mid: 7,
                name: "Musician".to_owned(),
            },
            pages: vec![MockPage {
                cid: 7001,
                page: 1,
                part: "Track1".to_owned(),
            }],
        },
    );
    state.playurls.insert(
        (dash_aid, 7001),
        MockPlayurlResponse::Dash {
            videos: vec![make_dash_spec(80, "/cdn/so_video_unused.mp4", 7, 2_000_000)],
            audios: Some(vec![make_audio_spec("/cdn/so_audio.m4a")]),
        },
    );
    state.views.insert(
        durl_aid,
        MockViewResponse::Ok {
            aid: durl_aid,
            bvid: durl_bvid.clone(),
            title: "LegacySong".to_owned(),
            owner: MockUp {
                mid: 7,
                name: "Musician".to_owned(),
            },
            pages: vec![MockPage {
                cid: 7011,
                page: 1,
                part: "Legacy".to_owned(),
            }],
        },
    );
    state.playurls.insert(
        (durl_aid, 7011),
        MockPlayurlResponse::Durl {
            paths: vec!["/cdn/so_durl.mp4".to_owned()],
        },
    );
    state.cdn_assets.insert(
        "/cdn/so_video_unused.mp4".to_owned(),
        MockCdnAsset::ok(v_bytes),
    );
    state
        .cdn_assets
        .insert("/cdn/so_audio.m4a".to_owned(), MockCdnAsset::ok(a_bytes));
    state
        .cdn_assets
        .insert("/cdn/so_durl.mp4".to_owned(), MockCdnAsset::ok(durl_bytes));

    let server = MockBiliServer::start(state).await;
    let muxer: Arc<dyn Muxer> = Arc::new(FfmpegMuxer::new(None));

    // 1. Exercise `pull --sound-only --audio-format copy` on tracked DASH media -> `.m4a`, 0 video hits!
    let ws = Workspace::new(tmp.path().join("ws"));
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
        name: "Music".to_owned(),
        count: 1,
        state: SetState::Active.to_string(),
        kind: SetKind::Created.to_string(),
        owner_mid: Some(1001),
        sync_complete: true,
        resume_page: None,
    }])
    .await
    .unwrap();
    db.upsert_set_accounts([set_account::Model {
        set_id: 1,
        account_id: 1001,
    }])
    .await
    .unwrap();
    db.upsert_medias([media::Model {
        id: dash_aid,
        bv_id: dash_bvid.clone(),
        title: "DashSong".to_owned(),
        r#type: MediaType::Video.to_string(),
        state: MediaState::Pending.to_string(),
        retry_count: 0,
        last_error: None,
        updated_at: None,
    }])
    .await
    .unwrap();
    db.upsert_media_sets([media_set::Model {
        id: dash_aid,
        set_id: 1,
    }])
    .await
    .unwrap();

    let ctx = AppContext::for_test(
        ws,
        Config::default(),
        db,
        muxer.clone(),
        server.client_config(),
    );
    let pull_out = tmp.path().join("pull_copy");
    pull(
        &ctx,
        PullOptions {
            output_dir: Some(pull_out.clone()),
            sound_only: true,
            audio_format: Some(AudioFormat::Copy),
            ..Default::default()
        },
    )
    .await
    .expect("pull --sound-only copy should succeed");

    let copy_files = list_files_in(&pull_out);
    assert_eq!(copy_files.len(), 1);
    assert_eq!(
        copy_files[0].extension().and_then(|e| e.to_str()),
        Some("m4a")
    );
    assert_eq!(ffprobe_stream_types(&copy_files[0]), vec!["audio"]);
    assert_eq!(
        muxer
            .read_comment_tag(&copy_files[0])
            .await
            .unwrap()
            .as_deref(),
        Some("fav:aid=700;cid=7001")
    );

    // 2. Exercise mp3, flac, opus on DASH via `download_with_env` -> STILL 0 hits to video CDN!
    let runtime = RuntimeOptions {
        no_progress: true,
        non_interactive: true,
        ..Default::default()
    };
    for (fmt, expected_ext) in [
        (AudioFormat::Mp3, "mp3"),
        (AudioFormat::Flac, "flac"),
        (AudioFormat::Opus, "opus"),
    ] {
        let out_dir = tmp.path().join(format!("dash_{expected_ext}"));
        download_with_env(
            None,
            Config::default(),
            muxer.clone(),
            server.client_config(),
            runtime.clone(),
            DownloadOptions {
                targets: vec![dash_bvid.clone()],
                output_dir: Some(out_dir.clone()),
                sound_only: true,
                audio_format: Some(fmt),
                anonymous: true,
                ..Default::default()
            },
        )
        .await
        .unwrap_or_else(|e| panic!("DASH sound-only {expected_ext} failed: {e:#}"));

        let files = list_files_in(&out_dir);
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].extension().and_then(|e| e.to_str()),
            Some(expected_ext)
        );
        assert_eq!(ffprobe_stream_types(&files[0]), vec!["audio"]);
        assert_eq!(
            muxer.read_comment_tag(&files[0]).await.unwrap().as_deref(),
            Some("fav:aid=700;cid=7001")
        );
    }

    assert_eq!(
        server.cdn_hits("/cdn/so_video_unused.mp4"),
        0,
        "DASH --sound-only MUST make 0 hits to video CDN URL"
    );

    // 3. Legacy `durl` `-vn` extraction -> 0 video streams + embedded fav:aid;cid tag!
    let durl_out = tmp.path().join("durl_mp3");
    download_with_env(
        None,
        Config::default(),
        muxer.clone(),
        server.client_config(),
        runtime,
        DownloadOptions {
            targets: vec![durl_bvid],
            output_dir: Some(durl_out.clone()),
            sound_only: true,
            audio_format: Some(AudioFormat::Mp3),
            anonymous: true,
            ..Default::default()
        },
    )
    .await
    .expect("legacy durl -vn extraction should succeed");

    let durl_files = list_files_in(&durl_out);
    assert_eq!(durl_files.len(), 1);
    assert_eq!(
        durl_files[0].extension().and_then(|e| e.to_str()),
        Some("mp3")
    );
    assert_eq!(ffprobe_stream_types(&durl_files[0]), vec!["audio"]);
    assert_eq!(
        muxer
            .read_comment_tag(&durl_files[0])
            .await
            .unwrap()
            .as_deref(),
        Some("fav:aid=701;cid=7011")
    );
}

#[tokio::test]
async fn rename_safe_four_layer_recovery_surgical_multipage_and_legacy_v1() {
    let tmp = tempfile::tempdir().unwrap();
    let sample_dir = tmp.path().join("samples");
    std::fs::create_dir_all(&sample_dir).unwrap();
    let (sample_video, sample_audio) = generate_tiny_av_streams(&sample_dir);
    let v_bytes = std::fs::read(&sample_video).unwrap();
    let a_bytes = std::fs::read(&sample_audio).unwrap();

    let mut state = MockState::default();
    state
        .accounts_by_sessdata
        .insert("sess_a".to_owned(), MockAccount::active(1001, "Alice"));

    // Setup media 801 (same-disk rename), 802 (cross-disk rename), 803 (2-page partial delete),
    // and 805 (legacy v1.x missing).
    for (aid, title, pages) in [
        (
            801i64,
            "SameDiskClip",
            vec![MockPage {
                cid: 8011,
                page: 1,
                part: "P1".to_owned(),
            }],
        ),
        (
            802i64,
            "CrossDiskClip",
            vec![MockPage {
                cid: 8021,
                page: 1,
                part: "P1".to_owned(),
            }],
        ),
        (
            803i64,
            "TwoPartClip",
            vec![
                MockPage {
                    cid: 8031,
                    page: 1,
                    part: "PartOne".to_owned(),
                },
                MockPage {
                    cid: 8032,
                    page: 2,
                    part: "PartTwo".to_owned(),
                },
            ],
        ),
        (
            805i64,
            "LegacyMissingClip",
            vec![MockPage {
                cid: 8051,
                page: 1,
                part: "P1".to_owned(),
            }],
        ),
    ] {
        for p in &pages {
            let vp = format!("/cdn/rec_v_{}.mp4", p.cid);
            let ap = format!("/cdn/rec_a_{}.m4a", p.cid);
            state.playurls.insert(
                (aid, p.cid),
                MockPlayurlResponse::Dash {
                    videos: vec![make_dash_spec(80, &vp, 7, 1_000_000)],
                    audios: Some(vec![make_audio_spec(&ap)]),
                },
            );
            state
                .cdn_assets
                .insert(vp, MockCdnAsset::ok(v_bytes.clone()));
            state
                .cdn_assets
                .insert(ap, MockCdnAsset::ok(a_bytes.clone()));
        }
        state.views.insert(
            aid,
            MockViewResponse::Ok {
                aid,
                bvid: aid_to_bvid(aid).unwrap(),
                title: title.to_owned(),
                owner: MockUp {
                    mid: 88,
                    name: "Up88".to_owned(),
                },
                pages,
            },
        );
    }

    let server = MockBiliServer::start(state).await;

    let ws = Workspace::new(tmp.path().join("ws"));
    ws.ensure_dirs().unwrap();
    let out_dir = tmp.path().join("videos");
    std::fs::create_dir_all(&out_dir).unwrap();
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
        name: "Archive".to_owned(),
        count: 5,
        state: SetState::Active.to_string(),
        kind: SetKind::Created.to_string(),
        owner_mid: Some(1001),
        sync_complete: true,
        resume_page: None,
    }])
    .await
    .unwrap();
    db.upsert_set_accounts([set_account::Model {
        set_id: 1,
        account_id: 1001,
    }])
    .await
    .unwrap();
    db.upsert_medias([
        media::Model {
            id: 801,
            bv_id: aid_to_bvid(801).unwrap(),
            title: "SameDiskClip".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Pending.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        },
        media::Model {
            id: 802,
            bv_id: aid_to_bvid(802).unwrap(),
            title: "CrossDiskClip".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Pending.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        },
        media::Model {
            id: 803,
            bv_id: aid_to_bvid(803).unwrap(),
            title: "TwoPartClip".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Pending.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        },
        // Legacy v1.x Completed medias (0 media_page rows): 804 present, 805 missing!
        media::Model {
            id: 804,
            bv_id: aid_to_bvid(804).unwrap(),
            title: "LegacyPresentClip".to_owned(),
            r#type: MediaType::Video.to_string(),
            state: MediaState::Completed.to_string(),
            retry_count: 0,
            last_error: None,
            updated_at: None,
        },
        media::Model {
            id: 805,
            bv_id: aid_to_bvid(805).unwrap(),
            title: "LegacyMissingClip".to_owned(),
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
        media_set::Model { id: 801, set_id: 1 },
        media_set::Model { id: 802, set_id: 1 },
        media_set::Model { id: 803, set_id: 1 },
        media_set::Model { id: 804, set_id: 1 },
        media_set::Model { id: 805, set_id: 1 },
    ])
    .await
    .unwrap();

    // Place legacy v1.x file for 804 on disk; leave 805 missing!
    std::fs::write(out_dir.join("804-LegacyPresentClip.mp4"), b"legacy-v1-data").unwrap();

    let muxer: Arc<dyn Muxer> = Arc::new(FfmpegMuxer::new(None));
    let ctx = AppContext::for_test(ws, Config::default(), db, muxer, server.client_config());

    // Initial pull downloads 801, 802, 803 (both pages) using real FfmpegMuxer (embeds fav:aid;cid tags & records size/inode).
    pull(
        &ctx,
        PullOptions {
            output_dir: Some(out_dir.clone()),
            ..Default::default()
        },
    )
    .await
    .expect("initial pull should succeed");

    let p801_orig = out_dir.join(
        ctx.db.pages_for_media(801).await.unwrap()[0]
            .local_path
            .as_deref()
            .unwrap(),
    );
    let p802_orig = out_dir.join(
        ctx.db.pages_for_media(802).await.unwrap()[0]
            .local_path
            .as_deref()
            .unwrap(),
    );
    let p803_pages = ctx.db.pages_for_media(803).await.unwrap();
    let p803_p2_orig = out_dir.join(p803_pages[1].local_path.as_deref().unwrap());

    // 1. Same-disk rename for 801 -> subdir/totally_different.mp4 (same inode on Unix!)
    let renamed_801 = out_dir.join("subdir/totally_different.mp4");
    std::fs::create_dir_all(renamed_801.parent().unwrap()).unwrap();
    std::fs::rename(&p801_orig, &renamed_801).unwrap();

    // 2. Cross-disk rename simulation for 802 -> write fresh copy under completely unrelated filename (NEW inode!), delete original!
    let cross_802 = out_dir.join("cross/brand_new_unrelated.mp4");
    std::fs::create_dir_all(cross_802.parent().unwrap()).unwrap();
    let bytes_802 = std::fs::read(&p802_orig).unwrap();
    std::fs::write(&cross_802, &bytes_802).unwrap();
    std::fs::remove_file(&p802_orig).unwrap();

    // 3. Multi-page partial delete: remove ONLY P2 of 803 (keep P1 intact!)
    std::fs::remove_file(&p803_p2_orig).unwrap();

    // Archive-safe default: plain `pull` (without --recheck) must NOT re-download anything!
    pull(
        &ctx,
        PullOptions {
            output_dir: Some(out_dir.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(server.cdn_hits("/cdn/rec_v_8011.mp4"), 1);
    assert_eq!(server.cdn_hits("/cdn/rec_v_8021.mp4"), 1);
    assert_eq!(server.cdn_hits("/cdn/rec_v_8031.mp4"), 1);
    assert_eq!(server.cdn_hits("/cdn/rec_v_8032.mp4"), 1);
    assert_eq!(server.cdn_hits("/cdn/rec_v_8051.mp4"), 0);

    // Verify `doctor_with_options(-o)` & `reconcile_completed_medias` see all 4 layers + legacy v1.x!
    doctor_with_options(
        &ctx,
        DoctorOptions {
            format: OutputFormat::Json,
            offline: true,
            output_dir: Some(out_dir.clone()),
        },
    )
    .await
    .expect("doctor with warn-only disk drift should succeed");

    let report = reconcile_completed_medias(&ctx.db, ctx.muxer.as_ref(), &out_dir, false, None)
        .await
        .unwrap();
    assert_eq!(
        report.relocated.len(),
        2,
        "expected 801 & 802 relocated: {report:?}"
    );
    assert_eq!(report.missing_pages.len(), 1);
    assert_eq!(
        (
            report.missing_pages[0].media_id,
            report.missing_pages[0].cid
        ),
        (803, 8032)
    );
    assert_eq!(report.verified_legacy, 1);
    assert_eq!(report.missing_legacy_ids, vec![805]);

    // Run `pull --recheck -o out_dir`: relinks 801 & 802 with 0 re-downloads, downloads ONLY P2 of 803 (0 CDN hits for P1!), and recovers legacy 805!
    pull(
        &ctx,
        PullOptions {
            output_dir: Some(out_dir.clone()),
            recheck: true,
            ..Default::default()
        },
    )
    .await
    .expect("pull --recheck should relink and recover missing files");

    assert_eq!(
        server.cdn_hits("/cdn/rec_v_8011.mp4"),
        1,
        "same-disk renamed 801 must NOT be re-downloaded"
    );
    assert_eq!(
        server.cdn_hits("/cdn/rec_v_8021.mp4"),
        1,
        "cross-disk renamed 802 must NOT be re-downloaded"
    );
    assert_eq!(
        server.cdn_hits("/cdn/rec_v_8031.mp4"),
        1,
        "intact P1 of 803 must receive 0 re-download CDN hits"
    );
    assert_eq!(
        server.cdn_hits("/cdn/rec_v_8032.mp4"),
        2,
        "deleted P2 of 803 must be re-downloaded"
    );
    assert_eq!(
        server.cdn_hits("/cdn/rec_v_8051.mp4"),
        1,
        "missing legacy v1.x media 805 must be re-downloaded"
    );
    assert_eq!(
        ctx.db.pages_for_media(801).await.unwrap()[0]
            .local_path
            .as_deref(),
        Some("subdir/totally_different.mp4")
    );
    assert_eq!(
        ctx.db.pages_for_media(802).await.unwrap()[0]
            .local_path
            .as_deref(),
        Some("cross/brand_new_unrelated.mp4")
    );

    // Also verify `media reset --missing -o` directly: rename 801 again, delete P2 of 803, delete legacy 804!
    let renamed_801_again = out_dir.join("subdir2/moved_again.mp4");
    std::fs::create_dir_all(renamed_801_again.parent().unwrap()).unwrap();
    std::fs::rename(&renamed_801, &renamed_801_again).unwrap();
    std::fs::remove_file(&p803_p2_orig).unwrap();
    std::fs::remove_file(out_dir.join("804-LegacyPresentClip.mp4")).unwrap();

    media_reset(
        &ctx,
        MediaResetOptions {
            missing: true,
            output_dir: Some(out_dir.clone()),
            ..Default::default()
        },
    )
    .await
    .expect("media reset --missing -o should succeed");

    assert_eq!(
        ctx.db.get_media(801).await.unwrap().state,
        MediaState::Completed.as_str()
    );
    assert_eq!(
        ctx.db.pages_for_media(801).await.unwrap()[0]
            .local_path
            .as_deref(),
        Some("subdir2/moved_again.mp4")
    );
    assert_eq!(
        ctx.db.get_media(803).await.unwrap().state,
        MediaState::Pending.as_str()
    );
    let m803_pages = ctx.db.pages_for_media(803).await.unwrap();
    assert_eq!(
        m803_pages[0].state,
        MediaPageState::Completed.as_str(),
        "surgical reset must preserve intact P1"
    );
    assert_eq!(
        m803_pages[1].state,
        MediaPageState::Pending.as_str(),
        "surgical reset must reset only deleted P2"
    );
    assert_eq!(
        ctx.db.get_media(804).await.unwrap().state,
        MediaState::Pending.as_str(),
        "deleted legacy v1.x media 804 must be reset to Pending"
    );
}

#[tokio::test]
async fn archive_season_sync_and_incremental_cursor_resumable_backfill() {
    let mut state = MockState::default();
    let mut acc = MockAccount::active(1001, "Alice");
    acc.sets = vec![MockSet {
        id: 100,
        title: "ThreePageFav".to_owned(),
        media_count: 50,
    }];
    acc.collected_sets = vec![
        MockCollectedSet {
            id: 200,
            mid: 888,
            title: "CollectedFolder".to_owned(),
            media_count: 2,
            set_type: 11,
            attr: 22, // valid folder bitmask — must NOT be treated as invalidated!
            state: 0,
            upper: Some(MockUp {
                mid: 888,
                name: "FolderOwner".to_owned(),
            }),
        },
        MockCollectedSet {
            id: 300,
            mid: 777,
            title: "RustSeason".to_owned(),
            media_count: 3,
            set_type: 21,
            attr: 0,
            state: 0,
            upper: Some(MockUp {
                mid: 777,
                name: "SeasonCreator".to_owned(),
            }),
        },
        MockCollectedSet {
            id: 400,
            mid: 666,
            title: "DeletedSeason".to_owned(),
            media_count: 0,
            set_type: 21,
            attr: 0,
            state: 1, // remotely invalidated -> Unreachable
            upper: None,
        },
    ];
    // Start with collected/list failing on page 1 to verify partial failure preserves existing `set_account` links!
    acc.collected_error_page = Some(1);
    state.accounts_by_sessdata.insert("sess_a".to_owned(), acc);

    // 3-page set 100: 50 items newest-first (IDs 1050 down to 1001).
    let set100_items: Vec<MockMediaItem> = (1001..=1050)
        .rev()
        .map(|id| MockMediaItem {
            id,
            bvid: aid_to_bvid(id).unwrap(),
            title: format!("FavItem_{id}"),
            media_type: 2,
            attr: 0,
            upper: Some(MockUp {
                mid: 9000,
                name: "FavAuthor".to_owned(),
            }),
        })
        .collect();
    state.set_medias.insert(100, set100_items);

    // Season 300 episodes stored oldest-first -> `sort_reverse=true` returns newest-first!
    state.season_archives.insert(
        (777, 300),
        vec![
            MockMediaItem {
                id: 3001,
                bvid: aid_to_bvid(3001).unwrap(),
                title: "Ep1".to_owned(),
                media_type: 2,
                attr: 0,
                upper: None,
            },
            MockMediaItem {
                id: 3002,
                bvid: aid_to_bvid(3002).unwrap(),
                title: "Ep2".to_owned(),
                media_type: 2,
                attr: 0,
                upper: None,
            },
            MockMediaItem {
                id: 3003,
                bvid: aid_to_bvid(3003).unwrap(),
                title: "Ep3".to_owned(),
                media_type: 2,
                attr: 0,
                upper: None,
            },
        ],
    );

    let server = MockBiliServer::start(state).await;
    let tmp = tempfile::tempdir().unwrap();
    let ws = Workspace::new(tmp.path().to_path_buf());
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
    // Pre-existing set link 999 that must NOT be unlinked when collected/list fails partially!
    db.sync_account_sets(
        1001,
        vec![set::Model {
            set_id: 999,
            name: "PreExisting".to_owned(),
            count: 1,
            state: SetState::Inactive.to_string(),
            kind: SetKind::Collected.to_string(),
            owner_mid: Some(555),
            sync_complete: false,
            resume_page: None,
        }],
        true,
    )
    .await
    .unwrap();

    let ctx = AppContext::for_test(
        ws,
        Config::default(),
        db,
        Arc::new(MockMuxer::succeeding()),
        server.client_config(),
    );

    // Part A1: Partial collected/list failure preserves `set_account` link 999.
    fetch(
        &ctx,
        FetchOptions {
            sets_only: true,
            ignore_errors: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        ctx.db
            .get_set_ids_of_account(1001)
            .await
            .unwrap()
            .contains(&999),
        "partial collected/list failure must preserve existing set_account links"
    );

    // Part A2: Clear collected error -> discover type=11 Collected (Inactive), type=21 Season (Inactive),
    // and state=1 invalidated Season (Unreachable); Phase 2 ignores all until activated!
    {
        let mut st = server.state.lock().unwrap();
        st.accounts_by_sessdata
            .get_mut("sess_a")
            .unwrap()
            .collected_error_page = None;
    }
    server.clear_requests();
    fetch(
        &ctx,
        FetchOptions {
            sets_only: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let col_reqs = server.requests_for("/x/v3/fav/folder/collected/list");
    assert!(!col_reqs.is_empty());
    assert!(col_reqs.iter().all(|r| r.query.contains("platform=web")));
    assert_eq!(server.call_count("/x/v3/fav/resource/list"), 0);
    assert_eq!(
        server.call_count("/x/polymer/web-space/seasons_archives_list"),
        0
    );

    let s200 = ctx.db.get_set(200).await.unwrap();
    assert_eq!(s200.kind, SetKind::Collected.as_str());
    assert_eq!(s200.state, SetState::Inactive.as_str());
    let s300 = ctx.db.get_set(300).await.unwrap();
    assert_eq!(s300.kind, SetKind::Season.as_str());
    assert_eq!(s300.state, SetState::Inactive.as_str());
    assert_eq!(s300.owner_mid, Some(777));
    let s400 = ctx.db.get_set(400).await.unwrap();
    assert_eq!(s400.kind, SetKind::Season.as_str());
    assert_eq!(s400.state, SetState::Unreachable.as_str());

    // Part B: Activate Season 300 -> fetch pulls episodes via `seasons_archives_list?sort_reverse=true`
    // and links BOTH `media_set` and `media_up`!
    ctx.db.activate_set(300).await.unwrap();
    server.clear_requests();
    fetch(
        &ctx,
        FetchOptions {
            set_ids: vec![300],
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let season_reqs = server.requests_for("/x/polymer/web-space/seasons_archives_list");
    assert_eq!(season_reqs.len(), 1);
    assert!(season_reqs[0].query.contains("sort_reverse=true"));
    assert!(season_reqs[0].query.contains("season_id=300"));
    assert!(season_reqs[0].query.contains("mid=777"));
    let mut season_ids = ctx.db.get_media_ids_of_set(300).await.unwrap();
    season_ids.sort_unstable();
    assert_eq!(season_ids, vec![3001, 3002, 3003]);
    assert!(
        ctx.db
            .all_active_medias_without_up()
            .await
            .unwrap()
            .is_empty(),
        "season episodes must be linked in media_up (owner_mid=777)"
    );
    assert!(ctx.db.get_set(300).await.unwrap().sync_complete);

    // Part C: Resumable initial backfill on 3-page Set 100 — fail on page 2 -> saves resume_page=2!
    ctx.db.activate_set(100).await.unwrap();
    server.state.lock().unwrap().set_error_page.insert(100, 2);
    server.clear_requests();
    fetch(
        &ctx,
        FetchOptions {
            set_ids: vec![100],
            ignore_errors: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let s100_mid = ctx.db.get_set(100).await.unwrap();
    assert!(!s100_mid.sync_complete);
    assert_eq!(s100_mid.resume_page, Some(2));
    assert_eq!(ctx.db.get_media_ids_of_set(100).await.unwrap().len(), 20);

    // Clear page 2 error -> next fetch resumes at page 2, fetches page 2 & 3, and marks sync_complete=true!
    server.state.lock().unwrap().set_error_page.remove(&100);
    server.clear_requests();
    fetch(
        &ctx,
        FetchOptions {
            set_ids: vec![100],
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let resume_reqs = server.requests_for("/x/v3/fav/resource/list");
    let requested_pns: Vec<&str> = resume_reqs
        .iter()
        .filter_map(|r| r.query.split('&').find_map(|kv| kv.strip_prefix("pn=")))
        .collect();
    assert_eq!(
        requested_pns.iter().filter(|&&p| p == "2").count(),
        1,
        "page 2 must be requested once on resume, got pns={requested_pns:?}"
    );
    assert_eq!(
        requested_pns.iter().filter(|&&p| p == "3").count(),
        1,
        "page 3 must be requested once on resume, got pns={requested_pns:?}"
    );
    let s100_done = ctx.db.get_set(100).await.unwrap();
    assert!(s100_done.sync_complete);
    assert_eq!(s100_done.resume_page, None);
    assert_eq!(ctx.db.get_media_ids_of_set(100).await.unwrap().len(), 50);

    // Part D: Incremental early-stop (1 API call!) for `fetch` AND `fetch --prune`,
    //         versus full 3-page scan + expired marking + unfavorited unlinking for `fetch --full`!
    {
        let mut st = server.state.lock().unwrap();
        let list = st.set_medias.get_mut(&100).unwrap();
        // Prepend 1 new item at top of page 1
        list.insert(
            0,
            MockMediaItem {
                id: 1051,
                bvid: aid_to_bvid(1051).unwrap(),
                title: "BrandNew_1051".to_owned(),
                media_type: 2,
                attr: 0,
                upper: Some(MockUp {
                    mid: 9000,
                    name: "FavAuthor".to_owned(),
                }),
            },
        );
        // Mark item 1005 (on page 3) invalidated (`attr = 9`), and remove item 1001 (unfavorited!)
        for item in list.iter_mut() {
            if item.id == 1005 {
                item.attr = 9;
            }
        }
        list.retain(|item| item.id != 1001);
    }

    server.clear_requests();
    fetch(
        &ctx,
        FetchOptions {
            set_ids: vec![100],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let inc_reqs = server.requests_for("/x/v3/fav/resource/list");
    assert_eq!(
        inc_reqs.len(),
        1,
        "incremental fetch on synced 3-page set MUST stop after page 1 (1 API call)"
    );
    assert!(inc_reqs[0].query.contains("order=mtime"));
    assert!(inc_reqs[0].query.contains("platform=web"));
    assert!(ctx.db.get_media(1051).await.is_ok());

    // Prepend another new item and run `fetch --prune`: STILL strictly 1 API call!
    {
        let mut st = server.state.lock().unwrap();
        st.set_medias.get_mut(&100).unwrap().insert(
            0,
            MockMediaItem {
                id: 1052,
                bvid: aid_to_bvid(1052).unwrap(),
                title: "BrandNew_1052".to_owned(),
                media_type: 2,
                attr: 0,
                upper: Some(MockUp {
                    mid: 9000,
                    name: "FavAuthor".to_owned(),
                }),
            },
        );
    }
    server.clear_requests();
    fetch(
        &ctx,
        FetchOptions {
            set_ids: vec![100],
            prune: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        server.requests_for("/x/v3/fav/resource/list").len(),
        1,
        "fetch --prune MUST NOT trigger a full scan"
    );
    assert!(ctx.db.get_media(1052).await.is_ok());

    // Explicit `fetch --full`: scans all 3 pages, marks 1005 Expired, and unlinks unfavorited 1001!
    server.clear_requests();
    fetch(
        &ctx,
        FetchOptions {
            set_ids: vec![100],
            full: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        server.requests_for("/x/v3/fav/resource/list").len(),
        3,
        "fetch --full must scan all 3 pages"
    );
    assert_eq!(
        ctx.db.get_media(1005).await.unwrap().state,
        MediaState::Expired.as_str(),
        "fetch --full must mark remotely invalidated Pending item 1005 Expired"
    );
    assert!(
        !ctx.db
            .get_media_ids_of_set(100)
            .await
            .unwrap()
            .contains(&1001),
        "fetch --full must unlink unfavorited item 1001"
    );
}

#[tokio::test]
async fn clarity_and_codec_end_to_end_downloads_only_selected_stream() {
    let mut state = MockState::default();
    state
        .accounts_by_sessdata
        .insert("sess_a".to_owned(), MockAccount::active(1001, "Alice"));
    let aid = 900i64;
    let bvid = aid_to_bvid(aid).unwrap();
    state.views.insert(
        aid,
        MockViewResponse::Ok {
            aid,
            bvid: bvid.clone(),
            title: "ClarityTest".to_owned(),
            owner: MockUp {
                mid: 99,
                name: "Up99".to_owned(),
            },
            pages: vec![MockPage {
                cid: 9001,
                page: 1,
                part: "P1".to_owned(),
            }],
        },
    );
    state.playurls.insert(
        (aid, 9001),
        MockPlayurlResponse::Dash {
            videos: vec![
                make_dash_spec(120, "/cdn/cl_4k.mp4", 12, 8_000_000),
                make_dash_spec(80, "/cdn/cl_1080p.mp4", 12, 4_000_000),
                make_dash_spec(64, "/cdn/cl_720p_avc.mp4", 7, 1_500_000),
                make_dash_spec(64, "/cdn/cl_720p_hevc.mp4", 12, 1_000_000),
            ],
            audios: Some(vec![make_audio_spec("/cdn/cl_audio.m4a")]),
        },
    );
    for path in [
        "/cdn/cl_4k.mp4",
        "/cdn/cl_1080p.mp4",
        "/cdn/cl_720p_avc.mp4",
        "/cdn/cl_720p_hevc.mp4",
        "/cdn/cl_audio.m4a",
    ] {
        state
            .cdn_assets
            .insert(path.to_owned(), MockCdnAsset::ok(b"stream-bytes".to_vec()));
    }

    let server = MockBiliServer::start(state).await;
    let tmp = tempfile::tempdir().unwrap();
    let ws = Workspace::new(tmp.path().join("ws"));
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
        count: 1,
        state: SetState::Active.to_string(),
        kind: SetKind::Created.to_string(),
        owner_mid: Some(1001),
        sync_complete: true,
        resume_page: None,
    }])
    .await
    .unwrap();
    db.upsert_set_accounts([set_account::Model {
        set_id: 1,
        account_id: 1001,
    }])
    .await
    .unwrap();
    db.upsert_medias([media::Model {
        id: aid,
        bv_id: bvid,
        title: "ClarityTest".to_owned(),
        r#type: MediaType::Video.to_string(),
        state: MediaState::Pending.to_string(),
        retry_count: 0,
        last_error: None,
        updated_at: None,
    }])
    .await
    .unwrap();
    db.upsert_media_sets([media_set::Model { id: aid, set_id: 1 }])
        .await
        .unwrap();

    let ctx = AppContext::for_test(
        ws,
        Config::default(),
        db,
        Arc::new(MockMuxer::succeeding()),
        server.client_config(),
    );

    pull(
        &ctx,
        PullOptions {
            output_dir: Some(tmp.path().join("out")),
            clarity: Some(VideoClarity::P720),
            codec: Some(VideoCodec::Hevc),
            ..Default::default()
        },
    )
    .await
    .expect("pull --clarity 720p --codec hevc should succeed");

    let playurl_reqs = server.requests_for("/x/player/wbi/playurl");
    assert_eq!(playurl_reqs.len(), 1);
    assert!(
        playurl_reqs[0].query.contains("qn=64"),
        "expected qn=64 in playurl query, got: {}",
        playurl_reqs[0].query
    );
    assert_eq!(server.cdn_hits("/cdn/cl_4k.mp4"), 0);
    assert_eq!(server.cdn_hits("/cdn/cl_1080p.mp4"), 0);
    assert_eq!(server.cdn_hits("/cdn/cl_720p_avc.mp4"), 0);
    assert_eq!(server.cdn_hits("/cdn/cl_720p_hevc.mp4"), 1);
    assert_eq!(server.cdn_hits("/cdn/cl_audio.m4a"), 1);
}

#[tokio::test]
async fn temp_staging_uses_configured_tmp_dir_and_download_dot_fav_tmp() {
    let mut state = MockState::default();
    state
        .accounts_by_sessdata
        .insert("sess_a".to_owned(), MockAccount::active(1001, "Alice"));
    let aid = 950i64;
    let bvid = aid_to_bvid(aid).unwrap();
    state.views.insert(
        aid,
        MockViewResponse::Ok {
            aid,
            bvid: bvid.clone(),
            title: "StagingCheck".to_owned(),
            owner: MockUp {
                mid: 95,
                name: "Up95".to_owned(),
            },
            pages: vec![MockPage {
                cid: 9501,
                page: 1,
                part: "P1".to_owned(),
            }],
        },
    );
    state.playurls.insert(
        (aid, 9501),
        MockPlayurlResponse::Dash {
            videos: vec![make_dash_spec(80, "/cdn/st_v.mp4", 7, 1_000_000)],
            audios: Some(vec![make_audio_spec("/cdn/st_a.m4a")]),
        },
    );
    state.cdn_assets.insert(
        "/cdn/st_v.mp4".to_owned(),
        MockCdnAsset::ok(b"video".to_vec()),
    );
    state.cdn_assets.insert(
        "/cdn/st_a.m4a".to_owned(),
        MockCdnAsset::ok(b"audio".to_vec()),
    );

    let server = MockBiliServer::start(state).await;
    let tmp = tempfile::tempdir().unwrap();

    // 1. `pull` with `config.tmp_dir` override stages strictly inside `custom_stage` (never std::env::temp_dir() directly).
    let ws = Workspace::new(tmp.path().join("ws"));
    ws.ensure_dirs().unwrap();
    let custom_stage = tmp.path().join("custom_stage_dir");
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
        name: "S".to_owned(),
        count: 1,
        state: SetState::Active.to_string(),
        kind: SetKind::Created.to_string(),
        owner_mid: Some(1001),
        sync_complete: true,
        resume_page: None,
    }])
    .await
    .unwrap();
    db.upsert_set_accounts([set_account::Model {
        set_id: 1,
        account_id: 1001,
    }])
    .await
    .unwrap();
    db.upsert_medias([media::Model {
        id: aid,
        bv_id: bvid.clone(),
        title: "StagingCheck".to_owned(),
        r#type: MediaType::Video.to_string(),
        state: MediaState::Pending.to_string(),
        retry_count: 0,
        last_error: None,
        updated_at: None,
    }])
    .await
    .unwrap();
    db.upsert_media_sets([media_set::Model { id: aid, set_id: 1 }])
        .await
        .unwrap();

    let pull_muxer = MockMuxer::succeeding();
    let cfg = Config {
        tmp_dir: Some(custom_stage.clone()),
        ..Default::default()
    };
    let ctx = AppContext::for_test(
        ws,
        cfg,
        db,
        Arc::new(pull_muxer.clone()),
        server.client_config(),
    );
    let pull_out = tmp.path().join("pull_out");
    pull(
        &ctx,
        PullOptions {
            output_dir: Some(pull_out),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let pull_reqs = pull_muxer.requests();
    assert_eq!(pull_reqs.len(), 1);
    let v_part = pull_reqs[0].video.as_ref().unwrap();
    let a_part = pull_reqs[0].audio.as_ref().unwrap();
    assert_eq!(
        v_part.parent(),
        Some(custom_stage.as_path()),
        "pull video staging must reside in config.tmp_dir"
    );
    assert_eq!(
        a_part.parent(),
        Some(custom_stage.as_path()),
        "pull audio staging must reside in config.tmp_dir"
    );
    assert_ne!(v_part.parent(), Some(std::env::temp_dir().as_path()));
    assert!(
        !v_part.exists() && !a_part.exists(),
        "staged .part files must be cleaned up"
    );

    // 2. `fav download` stages strictly inside `<output_dir>/.fav-tmp/` and removes it when empty.
    let dl_muxer = MockMuxer::succeeding();
    let dl_out = tmp.path().join("dl_out");
    let expected_dl_tmp = dl_out.join(".fav-tmp");
    download_with_env(
        None,
        Config::default(),
        Arc::new(dl_muxer.clone()),
        server.client_config(),
        RuntimeOptions {
            no_progress: true,
            non_interactive: true,
            ..Default::default()
        },
        DownloadOptions {
            targets: vec![bvid],
            output_dir: Some(dl_out.clone()),
            anonymous: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let dl_reqs = dl_muxer.requests();
    assert_eq!(dl_reqs.len(), 1);
    let dl_v_part = dl_reqs[0].video.as_ref().unwrap();
    let dl_a_part = dl_reqs[0].audio.as_ref().unwrap();
    assert_eq!(
        dl_v_part.parent(),
        Some(expected_dl_tmp.as_path()),
        "download video staging must reside in <output_dir>/.fav-tmp"
    );
    assert_eq!(
        dl_a_part.parent(),
        Some(expected_dl_tmp.as_path()),
        "download audio staging must reside in <output_dir>/.fav-tmp"
    );
    assert_ne!(dl_v_part.parent(), Some(std::env::temp_dir().as_path()));
    assert!(
        !expected_dl_tmp.exists(),
        "<output_dir>/.fav-tmp must be removed when empty"
    );
}
