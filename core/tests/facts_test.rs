// tests/facts_test.rs — facts 子命令集成测试 (F1–F23)

use chrono::Utc;
use regex::Regex;
use std::env;
use std::fs::{self, File};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

fn get_bin() -> &'static str {
    env!("CARGO_BIN_EXE_riko-memory")
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn create_test_env(test_name: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let now_nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let base_dir = env::temp_dir().join(format!(
        "riko_facts_test_{}_{}_{}",
        test_name,
        std::process::id(),
        now_nanos
    ));
    let _ = fs::remove_dir_all(&base_dir);
    fs::create_dir_all(&base_dir).unwrap();

    let sessions_root = base_dir.join("sessions");
    let var_dir = base_dir.join("var");
    let capture_dir = base_dir.join("capture");
    fs::create_dir_all(&sessions_root).unwrap();
    fs::create_dir_all(&var_dir).unwrap();
    fs::create_dir_all(&capture_dir).unwrap();

    (base_dir, sessions_root, var_dir, capture_dir)
}

fn make_fake_curl(base_dir: &Path, capture_dir: &Path) -> PathBuf {
    let curl_path = base_dir.join("fake_curl.sh");
    let script = format!(
        r#"#!/bin/bash
echo "$@" >> "{base_dir}/curl_args.log"
body_file=""
url=""
while [ $# -gt 0 ]; do
  case "$1" in
    -d)
      shift
      body_file="${{1#@}}"
      ;;
    http://*|https://*)
      url="$1"
      ;;
  esac
  shift
done

if [ -n "$body_file" ] && [ -f "$body_file" ]; then
  cnt=$(ls -1 "{capture_dir}" 2>/dev/null | wc -l)
  if [[ "$url" == *"/api/v2/memory/add"* ]]; then
    cp "$body_file" "{capture_dir}/add_${{cnt}}.json"
  elif [[ "$url" == *"/api/v2/memory/flush"* ]]; then
    cp "$body_file" "{capture_dir}/flush_${{cnt}}.json"
  else
    cp "$body_file" "{capture_dir}/other_${{cnt}}.json"
  fi
fi

if [ -f "{base_dir}/fake_curl_fail" ]; then
  echo '{{"error":"internal server error","detail":"simulated failure"}}'
  exit 22
fi

if [ -f "{base_dir}/fake_curl_fail_session" ]; then
  fail_target=$(cat "{base_dir}/fake_curl_fail_session")
  if [ -n "$fail_target" ] && [ -n "$body_file" ] && grep -q "\"session_id\":\"$fail_target\"" "$body_file"; then
    echo '{{"error":"simulated failure for session","detail":"fail_session"}}'
    exit 22
  fi
fi

if [ -f "{base_dir}/fake_curl_fail_flush_nth" ]; then
  target_idx=$(cat "{base_dir}/fake_curl_fail_flush_nth")
  flush_cnt_file="{base_dir}/.flush_counter"
  curr_cnt=0
  if [ -f "$flush_cnt_file" ]; then
    curr_cnt=$(cat "$flush_cnt_file")
  fi
  if [[ "$url" == *"/api/v2/memory/flush"* ]]; then
    curr_cnt=$((curr_cnt + 1))
    echo "$curr_cnt" > "$flush_cnt_file"
    if [ "$curr_cnt" -eq "$target_idx" ]; then
      echo '{{"error":"simulated nth flush failure","detail":"fail_nth_flush"}}'
      exit 22
    fi
  fi
fi

if [[ "$url" == *"/api/v2/memory/add"* ]]; then
  echo '{{"request_id":"test-add","data":{{"message_count":1,"status":"accumulated"}}}}'
  exit 0
elif [[ "$url" == *"/api/v2/memory/flush"* ]]; then
  if [ -f "{base_dir}/fake_curl_flush_no_extraction" ]; then
    echo '{{"request_id":"test-flush","data":{{"status":"no_extraction"}}}}'
  else
    echo '{{"request_id":"test-flush","data":{{"status":"extracted"}}}}'
  fi
  exit 0
else
  echo '{{"error":"unknown endpoint"}}'
  exit 1
fi
"#,
        base_dir = base_dir.display(),
        capture_dir = capture_dir.display()
    );

    fs::write(&curl_path, script).unwrap();
    let mut perms = fs::metadata(&curl_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&curl_path, perms).unwrap();
    curl_path
}

fn write_config(
    base_dir: &Path,
    sessions_root: &Path,
    curl_path: &Path,
    extra_json: Option<serde_json::Value>,
) -> PathBuf {
    let config_path = base_dir.join("var").join("facts.json");
    let mut map = serde_json::json!({
        "sessions_root": sessions_root.to_str().unwrap(),
        "state_path": base_dir.join("var").join("facts.state.json").to_str().unwrap(),
        "log_path": base_dir.join("var").join("facts.log").to_str().unwrap(),
        "lock_path": base_dir.join("var").join("facts.lock").to_str().unwrap(),
        "endpoint": "http://127.0.0.1:18100",
        "app_id": "riko",
        "project_id": "main",
        "user_sender_id": "aki",
        "user_sender_name": "爹地",
        "skip_recent_seconds": 60,
        "idle_flush_seconds": 900,
        "max_messages_per_call": 200,
        "max_chars_per_call": 200000,
        "max_calls_per_run": 40,
        "max_flushes_per_run": 2,
        "timeout_seconds": 240,
        "stale_lock_seconds": 1800,
        "curl_path": curl_path.to_str().unwrap()
    });

    if let Some(extra) = extra_json {
        if let Some(extra_obj) = extra.as_object() {
            for (k, v) in extra_obj {
                map[k] = v.clone();
            }
        }
    }

    fs::write(&config_path, serde_json::to_string_pretty(&map).unwrap()).unwrap();
    config_path
}

fn setup_session_with_fixture(
    sessions_root: &Path,
    session_name: &str,
    fixture_filename: &str,
    mtime_seconds_ago: u64,
) -> PathBuf {
    let session_dir = sessions_root.join(session_name);
    fs::create_dir_all(&session_dir).unwrap();

    let dest_file = session_dir.join("session.v4.jsonl.zstd");
    let src_file = fixtures_dir().join(fixture_filename);
    fs::copy(&src_file, &dest_file).unwrap();

    if mtime_seconds_ago > 0 {
        let f = File::options().write(true).open(&dest_file).unwrap();
        let target_time = SystemTime::now() - Duration::from_secs(mtime_seconds_ago);
        f.set_modified(target_time).unwrap();
    }

    dest_file
}

fn create_session_with_events(
    sessions_root: &Path,
    session_name: &str,
    events: &[serde_json::Value],
    mtime_seconds_ago: u64,
) -> PathBuf {
    let session_dir = sessions_root.join(session_name);
    fs::create_dir_all(&session_dir).unwrap();
    let zstd_file = session_dir.join("session.v4.jsonl.zstd");

    let mut raw_bytes = Vec::new();
    for ev in events {
        let line = serde_json::to_string(ev).unwrap();
        raw_bytes.extend_from_slice(line.as_bytes());
        raw_bytes.push(b'\n');
    }

    let compressed = zstd::encode_all(&raw_bytes[..], 3).unwrap();
    fs::write(&zstd_file, compressed).unwrap();

    if mtime_seconds_ago > 0 {
        let f = File::options().write(true).open(&zstd_file).unwrap();
        let target_time = SystemTime::now() - Duration::from_secs(mtime_seconds_ago);
        f.set_modified(target_time).unwrap();
    }

    zstd_file
}

fn get_captured_bodies(capture_dir: &Path) -> Vec<serde_json::Value> {
    let mut files: Vec<_> = fs::read_dir(capture_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().map_or(false, |ext| ext == "json"))
        .collect();
    files.sort();

    let mut bodies = Vec::new();
    for f in files {
        let content = fs::read_to_string(&f).unwrap();
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            bodies.push(v);
        }
    }
    bodies
}

fn get_captured_add_bodies(capture_dir: &Path) -> Vec<serde_json::Value> {
    let mut files: Vec<_> = fs::read_dir(capture_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .map_or(false, |name| name.to_string_lossy().starts_with("add_"))
        })
        .collect();
    files.sort();

    let mut bodies = Vec::new();
    for f in files {
        let content = fs::read_to_string(&f).unwrap();
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            bodies.push(v);
        }
    }
    bodies
}

fn get_captured_flush_bodies(capture_dir: &Path) -> Vec<serde_json::Value> {
    let mut files: Vec<_> = fs::read_dir(capture_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .map_or(false, |name| name.to_string_lossy().starts_with("flush_"))
        })
        .collect();
    files.sort();

    let mut bodies = Vec::new();
    for f in files {
        let content = fs::read_to_string(&f).unwrap();
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            bodies.push(v);
        }
    }
    bodies
}

// =========================================================================
// F1: 发送 body 只含 user 消息；sender_id=aki、role=user；断言 assistant 文本不出现
// =========================================================================
#[test]
fn test_f1_only_user_messages_sent() {
    let (base_dir, sessions_root, _var_dir, capture_dir) = create_test_env("f1");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(&base_dir, &sessions_root, &curl_path, None);

    setup_session_with_fixture(
        &sessions_root,
        "session-f1",
        "facts-fixture.zstd",
        120,
    );

    let output = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success(), "facts should exit 0");

    let bodies = get_captured_add_bodies(&capture_dir);
    assert!(!bodies.is_empty(), "should have captured at least 1 add call");

    for body in &bodies {
        if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
            assert!(!messages.is_empty(), "messages must not be empty in add call");
            for msg in messages {
                assert_eq!(msg["role"], "user", "role must be user");
                assert_eq!(msg["sender_id"], "aki", "sender_id must be aki");
                let content = msg["content"].as_str().unwrap();
                assert!(!content.contains("系统激活"), "must not contain system text");
                assert!(!content.contains("璃"), "must not contain assistant text");
            }
        }
    }
}

// =========================================================================
// F2: 注入类消息（source.kind != user）、[自动探活]、tool 事件都被过滤
// =========================================================================
#[test]
fn test_f2_filter_injected_heartbeat_tool_events() {
    let (base_dir, sessions_root, _var_dir, capture_dir) = create_test_env("f2");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(&base_dir, &sessions_root, &curl_path, None);

    let events = vec![
        // 1. 注入消息: source.kind = system
        serde_json::json!({
            "type": "user/message",
            "seq": 1,
            "time": 1720000001000i64,
            "data": {
                "source": { "kind": "system" },
                "content": [{ "type": "text", "text": "system injected prompt" }]
            }
        }),
        // 2. 自动探活
        serde_json::json!({
            "type": "user/message",
            "seq": 2,
            "time": 1720000002000i64,
            "data": {
                "source": { "kind": "user" },
                "content": [{ "type": "text", "text": "[自动探活] ping 123" }]
            }
        }),
        // 3. tool 事件
        serde_json::json!({
            "type": "tool/call",
            "seq": 3,
            "time": 1720000003000i64,
            "data": { "call_id": "c1" }
        }),
        // 4. 正常爹地消息
        serde_json::json!({
            "type": "user/message",
            "seq": 4,
            "time": 1720000004000i64,
            "data": {
                "source": { "kind": "user" },
                "content": [{ "type": "text", "text": "爹地的有效事实" }]
            }
        }),
    ];

    create_session_with_events(&sessions_root, "session-f2", &events, 120);

    let output = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());

    let bodies = get_captured_add_bodies(&capture_dir);
    assert_eq!(bodies.len(), 1, "exactly 1 add call expected");
    let messages = bodies[0]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1, "only 1 message should pass filtering");
    assert_eq!(messages[0]["content"], "爹地的有效事实");
}

// =========================================================================
// F3: body 顶层字段正确：session_id/app_id/project_id/defer_extraction=true；timestamp 为 13 位 ms
// =========================================================================
#[test]
fn test_f3_body_fields_and_timestamp() {
    let (base_dir, sessions_root, _var_dir, capture_dir) = create_test_env("f3");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(&base_dir, &sessions_root, &curl_path, None);

    let events = vec![
        serde_json::json!({
            "type": "user/message",
            "seq": 10,
            "time": 1729000000123i64,
            "data": {
                "source": { "kind": "user" },
                "content": [{ "type": "text", "text": "test timestamp message" }]
            }
        }),
    ];

    create_session_with_events(&sessions_root, "session-f3", &events, 120);

    let output = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());

    let bodies = get_captured_add_bodies(&capture_dir);
    assert_eq!(bodies.len(), 1);
    let body = &bodies[0];

    assert_eq!(body["session_id"], "session-f3");
    assert_eq!(body["app_id"], "riko");
    assert_eq!(body["project_id"], "main");
    assert_eq!(body["defer_extraction"], true);

    let ts = body["messages"][0]["timestamp"].as_i64().unwrap();
    assert_eq!(ts, 1729000000123i64);
    assert_eq!(ts.to_string().len(), 13, "timestamp must be 13-digit ms");
}

// =========================================================================
// F4: 增量：第二次运行只发 seq>last_seq 的 user 消息；只有 assistant 新增时不发但 last_seq 推进到文件最大事件 seq
// =========================================================================
#[test]
fn test_f4_incremental_and_assistant_advance() {
    let (base_dir, sessions_root, var_dir, capture_dir) = create_test_env("f4");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(&base_dir, &sessions_root, &curl_path, None);

    let events1 = vec![
        serde_json::json!({
            "type": "user/message",
            "seq": 1,
            "time": 1720000001000i64,
            "data": {
                "source": { "kind": "user" },
                "content": [{ "type": "text", "text": "message 1" }]
            }
        }),
        serde_json::json!({
            "type": "assistant/message",
            "seq": 2,
            "time": 1720000002000i64,
            "data": {
                "message": { "content": [{ "type": "text", "text": "reply 1" }] }
            }
        }),
    ];

    create_session_with_events(&sessions_root, "session-f4", &events1, 120);

    // Run 1
    let output1 = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output1.status.success());

    let state_file = var_dir.join("facts.state.json");
    let state_raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();
    assert_eq!(state_raw["sessions"]["session-f4"]["last_seq"], 2);

    // Clear capture
    for f in fs::read_dir(&capture_dir).unwrap() {
        let _ = fs::remove_file(f.unwrap().path());
    }

    // Now add assistant message only (seq 5)
    let events2 = vec![
        serde_json::json!({
            "type": "user/message",
            "seq": 1,
            "time": 1720000001000i64,
            "data": {
                "source": { "kind": "user" },
                "content": [{ "type": "text", "text": "message 1" }]
            }
        }),
        serde_json::json!({
            "type": "assistant/message",
            "seq": 2,
            "time": 1720000002000i64,
            "data": {
                "message": { "content": [{ "type": "text", "text": "reply 1" }] }
            }
        }),
        serde_json::json!({
            "type": "assistant/message",
            "seq": 5,
            "time": 1720000005000i64,
            "data": {
                "message": { "content": [{ "type": "text", "text": "reply 2" }] }
            }
        }),
    ];

    create_session_with_events(&sessions_root, "session-f4", &events2, 120);

    // Run 2
    let output2 = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output2.status.success());

    // Assert zero add calls made
    let bodies2 = get_captured_add_bodies(&capture_dir);
    assert_eq!(bodies2.len(), 0, "no HTTP add calls should be made when only assistant added");

    // But last_seq advances to 5
    let state_raw2: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();
    assert_eq!(state_raw2["sessions"]["session-f4"]["last_seq"], 5);
}

// =========================================================================
// F5: 分批：消息数/字符数超限时切多批，每批不超上限
// =========================================================================
#[test]
fn test_f5_batching_messages_and_chars() {
    let (base_dir, sessions_root, _var_dir, capture_dir) = create_test_env("f5");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);

    // max_messages_per_call = 2, max_chars_per_call = 10
    let config_path = write_config(
        &base_dir,
        &sessions_root,
        &curl_path,
        Some(serde_json::json!({
            "max_messages_per_call": 2,
            "max_chars_per_call": 10,
        })),
    );

    let events = vec![
        serde_json::json!({
            "type": "user/message", "seq": 1, "time": 1720000001000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "1234" }] }
        }),
        serde_json::json!({
            "type": "user/message", "seq": 2, "time": 1720000002000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "5678" }] }
        }),
        serde_json::json!({
            "type": "user/message", "seq": 3, "time": 1720000003000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "abcdefg" }] }
        }),
        serde_json::json!({
            "type": "user/message", "seq": 4, "time": 1720000004000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "hijklmn" }] }
        }),
    ];

    create_session_with_events(&sessions_root, "session-f5", &events, 120);

    let output = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());

    let bodies = get_captured_add_bodies(&capture_dir);
    assert!(bodies.len() >= 2, "must be split into multiple batches");

    for body in bodies {
        let msgs = body["messages"].as_array().unwrap();
        assert!(msgs.len() <= 2, "messages per batch must be <= 2");
        let total_chars: usize = msgs
            .iter()
            .map(|m| m["content"].as_str().unwrap().chars().count())
            .sum();
        if msgs.len() > 1 {
            assert!(total_chars <= 10, "batch chars must be <= 10");
        }
    }
}

// =========================================================================
// F6: add 失败（fake curl exit 22）：state 不推进、退出码 1、日志有 500 字符片段
// =========================================================================
#[test]
fn test_f6_add_failure_exit_code_and_log() {
    let (base_dir, sessions_root, var_dir, capture_dir) = create_test_env("f6");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(&base_dir, &sessions_root, &curl_path, None);

    let events = vec![
        serde_json::json!({
            "type": "user/message", "seq": 1, "time": 1720000001000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "fail test" }] }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f6", &events, 120);

    // Request failure mode in fake curl
    fs::write(base_dir.join("fake_curl_fail"), "1").unwrap();

    let output = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1), "must exit 1 on add failure");

    let state_file = var_dir.join("facts.state.json");
    if state_file.exists() {
        let state_raw: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();
        if let Some(s) = state_raw["sessions"].get("session-f6") {
            assert_eq!(s["last_seq"], 0, "state must not advance on failure");
        }
    }

    let log_file = var_dir.join("facts.log");
    assert!(log_file.exists());
    let log_content = fs::read_to_string(&log_file).unwrap();
    assert!(
        log_content.contains("simulated failure"),
        "log must contain error snippet from stdout"
    );
}

// =========================================================================
// F7: flush 条件：idle 未超不发；超了发且 flushed_seq 推进
// =========================================================================
#[test]
fn test_f7_flush_idle_condition() {
    let (base_dir, sessions_root, var_dir, capture_dir) = create_test_env("f7");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);

    // idle_flush_seconds = 900
    let config_path = write_config(
        &base_dir,
        &sessions_root,
        &curl_path,
        Some(serde_json::json!({
            "idle_flush_seconds": 900,
        })),
    );

    // Session active 100 seconds ago (100 < 900, NOT idle)
    let now_ms = Utc::now().timestamp_millis();
    let events = vec![
        serde_json::json!({
            "type": "user/message", "seq": 10, "time": now_ms - 100 * 1000,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "recent msg" }] }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f7", &events, 100);

    // Run 1: Should add, but not flush
    let output1 = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output1.status.success());

    let state_file = var_dir.join("facts.state.json");
    let state1: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();
    assert_eq!(state1["sessions"]["session-f7"]["last_seq"], 10);
    assert_eq!(state1["sessions"]["session-f7"]["flushed_seq"], 0);

    // Now update session mtime and message timestamp to 1200 seconds ago (> 900, idle!)
    let events2 = vec![
        serde_json::json!({
            "type": "user/message", "seq": 10, "time": now_ms - 1200 * 1000,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "recent msg" }] }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f7", &events2, 1200);

    // Run 2: Should trigger flush
    let output2 = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output2.status.success());

    let state2: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();
    assert_eq!(state2["sessions"]["session-f7"]["flushed_seq"], 10);
    assert_eq!(get_captured_flush_bodies(&capture_dir).len(), 1);
}

// =========================================================================
// F8: --backfill 忽略 idle、发完即 flush；max_flushes_per_run 生效
// =========================================================================
#[test]
fn test_f8_backfill_and_max_flushes_limit() {
    let (base_dir, sessions_root, var_dir, capture_dir) = create_test_env("f8");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);

    // max_flushes_per_run = 2, idle_flush_seconds = 99999
    let config_path = write_config(
        &base_dir,
        &sessions_root,
        &curl_path,
        Some(serde_json::json!({
            "max_flushes_per_run": 2,
            "idle_flush_seconds": 99999,
        })),
    );

    let now_ms = Utc::now().timestamp_millis();
    for i in 1..=3 {
        let events = vec![
            serde_json::json!({
                "type": "user/message", "seq": i * 10, "time": now_ms - 100 * 1000,
                "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": format!("msg {}", i) }] }
            }),
        ];
        create_session_with_events(&sessions_root, &format!("session-f8-{}", i), &events, 100 + i as u64);
    }

    let output = Command::new(get_bin())
        .args([
            "facts",
            "--config",
            config_path.to_str().unwrap(),
            "--backfill",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let state_file = var_dir.join("facts.state.json");
    let state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();

    let mut flushed_count = 0;
    for (_k, v) in state["sessions"].as_object().unwrap() {
        if v["flushed_seq"] == v["last_seq"] && v["flushed_seq"].as_u64().unwrap() > 0 {
            flushed_count += 1;
        }
    }
    assert_eq!(flushed_count, 2, "exactly max_flushes_per_run sessions should be flushed");
}

// =========================================================================
// F9: --dry-run：零 curl 调用、state 不变、stdout 有 facts: plan ... 与汇总行
// =========================================================================
#[test]
fn test_f9_dry_run() {
    let (base_dir, sessions_root, var_dir, capture_dir) = create_test_env("f9");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(&base_dir, &sessions_root, &curl_path, None);

    let events = vec![
        serde_json::json!({
            "type": "user/message", "seq": 1, "time": 1720000001000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "dry run msg" }] }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f9", &events, 120);

    let output = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap(), "--dry-run"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("facts: plan session=session-f9"));
    assert!(stdout.contains("facts: sessions="));

    assert!(!var_dir.join("facts.state.json").exists(), "state must not be created");
    assert!(!var_dir.join("facts.log").exists(), "log must not be created");
    assert!(!base_dir.join("curl_args.log").exists(), "zero curl calls");
}

// =========================================================================
// F10: 锁：已有未过期 lock → 退出 0、零 curl；过期 lock → 正常继续
// =========================================================================
#[test]
fn test_f10_lock_concurrency_and_stale_handling() {
    let (base_dir, sessions_root, var_dir, capture_dir) = create_test_env("f10");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(&base_dir, &sessions_root, &curl_path, None);

    let events = vec![
        serde_json::json!({
            "type": "user/message", "seq": 1, "time": 1720000001000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "lock test msg" }] }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f10", &events, 120);

    let lock_file = var_dir.join("facts.lock");

    // Part A: Fresh active lock -> exit 0, zero curl
    fs::write(&lock_file, "pid=9999 time=2026-10-31T21:00:00+08:00\n").unwrap();

    let output1 = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output1.status.success());
    assert!(!base_dir.join("curl_args.log").exists(), "active lock must cause 0 curl calls");

    // Part B: Stale lock (mtime 3600 seconds ago, stale_lock_seconds is 1800)
    let f = File::options().write(true).open(&lock_file).unwrap();
    f.set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();

    let output2 = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output2.status.success());
    assert!(base_dir.join("curl_args.log").exists(), "stale lock must be broken and curl executed");
}

// =========================================================================
// F11: state 原子写：正常退出后 JSON 可解析；父路径是普通文件导致无法写时退出 1
// =========================================================================
#[test]
fn test_f11_state_atomic_write_and_failure() {
    let (base_dir, sessions_root, var_dir, capture_dir) = create_test_env("f11");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);

    // Normal run
    let config_path = write_config(&base_dir, &sessions_root, &curl_path, None);
    let events = vec![
        serde_json::json!({
            "type": "user/message", "seq": 1, "time": 1720000001000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "atomic test" }] }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f11", &events, 120);

    let output1 = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output1.status.success());

    let state_file = var_dir.join("facts.state.json");
    let parsed: Result<serde_json::Value, _> =
        serde_json::from_str(&fs::read_to_string(&state_file).unwrap());
    assert!(parsed.is_ok(), "state file must be valid JSON");

    // Parent path is a regular file
    let bad_state_parent = base_dir.join("bad_dir_file");
    fs::write(&bad_state_parent, "i am a file").unwrap();
    let bad_state_path = bad_state_parent.join("facts.state.json");

    let bad_config = write_config(
        &base_dir,
        &sessions_root,
        &curl_path,
        Some(serde_json::json!({
            "state_path": bad_state_path.to_str().unwrap(),
        })),
    );

    let output2 = Command::new(get_bin())
        .args(["facts", "--config", bad_config.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output2.status.code(), Some(1), "must exit 1 when state parent is a file");
}

// =========================================================================
// F12: max_calls_per_run：到上限就停、退出 0、curl 调用数 == 上限
// =========================================================================
#[test]
fn test_f12_max_calls_per_run_limit() {
    let (base_dir, sessions_root, _var_dir, capture_dir) = create_test_env("f12");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);

    // max_calls_per_run = 2
    let config_path = write_config(
        &base_dir,
        &sessions_root,
        &curl_path,
        Some(serde_json::json!({
            "max_calls_per_run": 2,
        })),
    );

    for i in 1..=4 {
        let events = vec![
            serde_json::json!({
                "type": "user/message", "seq": i, "time": 1720000000000i64 + i as i64 * 1000,
                "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": format!("msg {}", i) }] }
            }),
        ];
        create_session_with_events(&sessions_root, &format!("session-f12-{}", i), &events, 100 + i as u64);
    }

    let output = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());
    let bodies = get_captured_bodies(&capture_dir);
    assert_eq!(bodies.len(), 2, "curl calls must exactly equal max_calls_per_run");
}

// =========================================================================
// F13: 配置错误：负数/类型错/endpoint 非 http → 退出 2
// =========================================================================
#[test]
fn test_f13_config_validation_errors() {
    let (base_dir, sessions_root, _var_dir, capture_dir) = create_test_env("f13");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);

    // Negative timeout
    let cfg1 = write_config(
        &base_dir,
        &sessions_root,
        &curl_path,
        Some(serde_json::json!({
            "timeout_seconds": -10,
        })),
    );
    let out1 = Command::new(get_bin())
        .args(["facts", "--config", cfg1.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out1.status.code(), Some(2), "negative timeout must exit 2");

    // Invalid endpoint scheme
    let cfg2 = write_config(
        &base_dir,
        &sessions_root,
        &curl_path,
        Some(serde_json::json!({
            "endpoint": "ftp://127.0.0.1:18100",
        })),
    );
    let out2 = Command::new(get_bin())
        .args(["facts", "--config", cfg2.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out2.status.code(), Some(2), "non http endpoint must exit 2");

    // Wrong type for integer
    let cfg3 = write_config(
        &base_dir,
        &sessions_root,
        &curl_path,
        Some(serde_json::json!({
            "max_messages_per_call": "two_hundred",
        })),
    );
    let out3 = Command::new(get_bin())
        .args(["facts", "--config", cfg3.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out3.status.code(), Some(2), "wrong type must exit 2");
}

// =========================================================================
// F14: session_id 安全化：非法字符替换、超 128 截断
// =========================================================================
#[test]
fn test_f14_session_id_sanitization() {
    let (base_dir, sessions_root, _var_dir, capture_dir) = create_test_env("f14");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(&base_dir, &sessions_root, &curl_path, None);

    let raw_name = "session@test:weird#chars_and_very_long_padding_".to_string()
        + &"a".repeat(120);

    let events = vec![
        serde_json::json!({
            "type": "user/message", "seq": 1, "time": 1720000001000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "sanitize test" }] }
        }),
    ];
    create_session_with_events(&sessions_root, &raw_name, &events, 120);

    let output = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());
    let bodies = get_captured_add_bodies(&capture_dir);
    assert_eq!(bodies.len(), 1);
    let sid = bodies[0]["session_id"].as_str().unwrap();

    assert!(sid.len() <= 128, "session_id length must not exceed 128");
    assert!(!sid.contains('@'));
    assert!(!sid.contains(':'));
    assert!(!sid.contains('#'));
    assert!(sid.starts_with("session-test-weird-chars_"));
}

// =========================================================================
// F15: truncated zstd fixture (facts-fixture-partial.zstd) 不 panic、只处理完整行
// =========================================================================
#[test]
fn test_f15_truncated_fixture_handling() {
    let (base_dir, sessions_root, _var_dir, capture_dir) = create_test_env("f15");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(&base_dir, &sessions_root, &curl_path, None);

    setup_session_with_fixture(
        &sessions_root,
        "session-f15",
        "facts-fixture-partial.zstd",
        120,
    );

    let output = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success(), "truncated file must not panic");
    let bodies = get_captured_bodies(&capture_dir);
    assert!(!bodies.is_empty(), "complete lines should be processed");
}

// =========================================================================
// F16: 缺时间事件：user 消息 time 缺失时用 mtime 兜底，日志含 time_fallback=1
// =========================================================================
#[test]
fn test_f16_missing_time_event_fallback() {
    let (base_dir, sessions_root, var_dir, capture_dir) = create_test_env("f16");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(&base_dir, &sessions_root, &curl_path, None);

    // Event without time field
    let events = vec![
        serde_json::json!({
            "type": "user/message",
            "seq": 1,
            "data": {
                "source": { "kind": "user" },
                "content": [{ "type": "text", "text": "missing time msg" }]
            }
        }),
    ];
    let zstd_file = create_session_with_events(&sessions_root, "session-f16", &events, 120);

    let mtime = fs::metadata(&zstd_file).unwrap().modified().unwrap();
    let expected_ms = mtime
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let output = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());

    let bodies = get_captured_add_bodies(&capture_dir);
    assert_eq!(bodies.len(), 1);
    let ts = bodies[0]["messages"][0]["timestamp"].as_i64().unwrap();
    assert_eq!(ts, expected_ms, "timestamp must fallback to mtime ms");

    let log_file = var_dir.join("facts.log");
    let log_content = fs::read_to_string(&log_file).unwrap();
    assert!(
        log_content.contains("time_fallback=1"),
        "log must contain time_fallback=1"
    );
}

// =========================================================================
// F17: curl argv 含 --max-time 与配置值（如 240）
// =========================================================================
#[test]
fn test_f17_curl_argv_contains_max_time() {
    let (base_dir, sessions_root, _var_dir, capture_dir) = create_test_env("f17");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(
        &base_dir,
        &sessions_root,
        &curl_path,
        Some(serde_json::json!({
            "timeout_seconds": 240,
        })),
    );

    let events = vec![
        serde_json::json!({
            "type": "user/message", "seq": 1, "time": 1720000001000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "max-time test" }] }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f17", &events, 120);

    let output = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());
    let args_log = fs::read_to_string(base_dir.join("curl_args.log")).unwrap();
    assert!(
        args_log.contains("--max-time 240"),
        "curl argv must contain --max-time 240"
    );
}

// =========================================================================
// F18: 汇总行严格匹配正则 ^facts: sessions=\d+ sent=\d+ flushed=\d+ pending=\d+$
// =========================================================================
#[test]
fn test_f18_summary_line_regex_format() {
    let (base_dir, sessions_root, _var_dir, capture_dir) = create_test_env("f18");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(&base_dir, &sessions_root, &curl_path, None);

    let events = vec![
        serde_json::json!({
            "type": "user/message", "seq": 1, "time": 1720000001000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "summary regex test" }] }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f18", &events, 120);

    let output = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let re = Regex::new(r"^facts: sessions=\d+ sent=\d+ flushed=\d+ pending=\d+$").unwrap();

    let lines: Vec<_> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "normal run stdout must be strictly 1 line");
    assert!(re.is_match(lines[0]), "stdout must strictly match summary regex: {}", lines[0]);
}

// =========================================================================
// F19: 空会话不 flush：
// 1. 只有 assistant/tool 事件的会话，mtime 超过 idle：正常模式不发 flush，state flushed_seq == 0, last_seq == file_max_seq
// 2. --backfill 模式同样不发 flush
// 3. --dry-run：该会话 flush=no，汇总 flushed=0, pending=0
// 4. 同目录同时放一个正常会话：正常会话照常 add + flush
// =========================================================================
#[test]
fn test_f19_empty_session_no_flush() {
    let (base_dir, sessions_root, var_dir, capture_dir) = create_test_env("f19");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(&base_dir, &sessions_root, &curl_path, None);

    let empty_events = vec![
        serde_json::json!({
            "type": "assistant/message",
            "seq": 1,
            "time": 1720000001000i64,
            "data": { "message": { "content": [{ "type": "text", "text": "assistant only" }] } }
        }),
        serde_json::json!({
            "type": "tool/call",
            "seq": 2,
            "time": 1720000002000i64,
            "data": { "call_id": "c1" }
        }),
    ];
    create_session_with_events(&sessions_root, "session-empty", &empty_events, 1200);

    // 1. --dry-run
    let out_dry = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap(), "--dry-run"])
        .output()
        .unwrap();
    assert!(out_dry.status.success());
    let dry_stdout = String::from_utf8(out_dry.stdout).unwrap();
    assert!(
        dry_stdout.contains("plan session=session-empty new=0 add_batches=0 flush=no"),
        "dry-run should plan flush=no for empty session, got:\n{}",
        dry_stdout
    );
    assert!(
        dry_stdout.contains("flushed=0 pending=0"),
        "dry-run summary should show flushed=0 pending=0, got:\n{}",
        dry_stdout
    );

    // 2. --backfill
    let out_backfill = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap(), "--backfill"])
        .output()
        .unwrap();
    assert!(out_backfill.status.success());
    assert_eq!(get_captured_flush_bodies(&capture_dir).len(), 0, "backfill must not flush empty session");

    // Check state after backfill
    let state_file = var_dir.join("facts.state.json");
    let state_val: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();
    assert_eq!(state_val["sessions"]["session-empty"]["flushed_seq"], 0);
    assert_eq!(state_val["sessions"]["session-empty"]["last_seq"], 2);

    // 3. Normal run with both empty session and a normal session
    let normal_events = vec![
        serde_json::json!({
            "type": "user/message",
            "seq": 10,
            "time": 1720000010000i64,
            "data": {
                "source": { "kind": "user" },
                "content": [{ "type": "text", "text": "normal user message" }]
            }
        }),
    ];
    create_session_with_events(&sessions_root, "session-normal", &normal_events, 1200);

    let out_normal = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out_normal.status.success());

    // Normal session must be added and flushed
    let add_bodies = get_captured_add_bodies(&capture_dir);
    assert_eq!(add_bodies.len(), 1);
    assert_eq!(add_bodies[0]["session_id"], "session-normal");

    let flush_bodies = get_captured_flush_bodies(&capture_dir);
    assert_eq!(flush_bodies.len(), 1);
    assert_eq!(flush_bodies[0]["session_id"], "session-normal");

    let state_val2: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();
    assert_eq!(state_val2["sessions"]["session-empty"]["flushed_seq"], 0);
    assert_eq!(state_val2["sessions"]["session-empty"]["last_seq"], 2);
    assert_eq!(state_val2["sessions"]["session-normal"]["flushed_seq"], 10);
    assert_eq!(state_val2["sessions"]["session-normal"]["last_seq"], 10);
}

// =========================================================================
// F20: 日志：空会话运行两次，两次日志里 time_fallback 出现 0 次，no_user_messages 首次出现、第二次不重复
// =========================================================================
#[test]
fn test_f20_empty_session_logging_noise_suppressed() {
    let (base_dir, sessions_root, var_dir, capture_dir) = create_test_env("f20");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(&base_dir, &sessions_root, &curl_path, None);

    let empty_events = vec![
        serde_json::json!({
            "type": "assistant/message",
            "seq": 1,
            "time": 1720000001000i64,
            "data": { "message": { "content": [{ "type": "text", "text": "assistant greeting" }] } }
        }),
        serde_json::json!({
            "type": "tool/call",
            "seq": 2,
            "time": 1720000002000i64,
            "data": { "call_id": "c2" }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f20", &empty_events, 1200);

    // Run 1
    let out1 = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out1.status.success());

    let log_file = var_dir.join("facts.log");
    assert!(log_file.exists());
    let log1 = fs::read_to_string(&log_file).unwrap();
    assert!(
        !log1.contains("time_fallback"),
        "time_fallback must not appear in log for empty session"
    );
    assert!(
        log1.contains("no_user_messages"),
        "no_user_messages must appear on first run for empty session"
    );
    assert_eq!(
        log1.matches("no_user_messages").count(),
        1,
        "no_user_messages must appear exactly once in run 1"
    );

    // Run 2
    let out2 = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out2.status.success());

    let log2 = fs::read_to_string(&log_file).unwrap();
    assert!(
        !log2.contains("time_fallback"),
        "time_fallback must still not appear in log after run 2"
    );
    assert_eq!(
        log2.matches("no_user_messages").count(),
        1,
        "no_user_messages must not repeat on second run"
    );
}

// =========================================================================
// F21: assistant-only 增长不重 flush：一个会话先 add + flush 成功（flushed_seq == last_user_seq）；
// 然后往文件追加只有 assistant/tool 的事件（mtime 改旧），再跑一次 idle 条件成立的运行：
// 不发 add、不发 flush，pending=0
// =========================================================================
#[test]
fn test_f21_assistant_only_growth_does_not_reflush() {
    let (base_dir, sessions_root, var_dir, capture_dir) = create_test_env("f21");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(
        &base_dir,
        &sessions_root,
        &curl_path,
        Some(serde_json::json!({
            "idle_flush_seconds": 900,
        })),
    );

    let initial_events = vec![
        serde_json::json!({
            "type": "user/message",
            "seq": 1,
            "time": 1720000001000i64,
            "data": {
                "source": { "kind": "user" },
                "content": [{ "type": "text", "text": "initial user message" }]
            }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f21", &initial_events, 1200);

    // Run 1: add + flush should succeed
    let out1 = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out1.status.success());

    let state_file = var_dir.join("facts.state.json");
    let state1: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();
    assert_eq!(state1["sessions"]["session-f21"]["last_user_seq"], 1);
    assert_eq!(state1["sessions"]["session-f21"]["flushed_seq"], 1);
    assert_eq!(get_captured_add_bodies(&capture_dir).len(), 1);
    assert_eq!(get_captured_flush_bodies(&capture_dir).len(), 1);

    // Clear capture & curl_args.log
    for f in fs::read_dir(&capture_dir).unwrap() {
        let _ = fs::remove_file(f.unwrap().path());
    }
    let curl_args_log = base_dir.join("curl_args.log");
    if curl_args_log.exists() {
        let _ = fs::remove_file(&curl_args_log);
    }

    // Now append assistant and tool events (seq 2, seq 3), mtime set to 1200s ago (> 900)
    let updated_events = vec![
        serde_json::json!({
            "type": "user/message",
            "seq": 1,
            "time": 1720000001000i64,
            "data": {
                "source": { "kind": "user" },
                "content": [{ "type": "text", "text": "initial user message" }]
            }
        }),
        serde_json::json!({
            "type": "assistant/message",
            "seq": 2,
            "time": 1720000002000i64,
            "data": { "message": { "content": [{ "type": "text", "text": "assistant reply" }] } }
        }),
        serde_json::json!({
            "type": "tool/call",
            "seq": 3,
            "time": 1720000003000i64,
            "data": { "call_id": "c3" }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f21", &updated_events, 1200);

    // Run 2: idle condition holds, but no new user messages -> no add, no flush, pending=0
    let out2 = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out2.status.success());

    let stdout2 = String::from_utf8(out2.stdout).unwrap();
    assert!(
        stdout2.contains("sent=0 flushed=0 pending=0"),
        "stdout should show sent=0 flushed=0 pending=0, got:\n{}",
        stdout2
    );

    assert_eq!(
        get_captured_add_bodies(&capture_dir).len(),
        0,
        "must not send any add HTTP calls"
    );
    assert_eq!(
        get_captured_flush_bodies(&capture_dir).len(),
        0,
        "must not send any flush HTTP calls"
    );

    let state2: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();
    assert_eq!(state2["sessions"]["session-f21"]["last_user_seq"], 1);
    assert_eq!(state2["sessions"]["session-f21"]["flushed_seq"], 1);
    assert_eq!(state2["sessions"]["session-f21"]["last_seq"], 3);
}

// =========================================================================
// F22: flush 的 no_extraction 视为成功：
// fake curl 对 /flush 返回 {"request_id":"x","data":{"status":"no_extraction"}}；
// 断言：退出码 0、state 的 flushed_seq == last_user_seq、日志含 status=no_extraction
// =========================================================================
#[test]
fn test_f22_flush_no_extraction_treated_as_success() {
    let (base_dir, sessions_root, var_dir, capture_dir) = create_test_env("f22");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(
        &base_dir,
        &sessions_root,
        &curl_path,
        Some(serde_json::json!({
            "idle_flush_seconds": 900,
        })),
    );

    // Create session with user message, mtime 1200s ago (> 900s idle)
    let events = vec![
        serde_json::json!({
            "type": "user/message",
            "seq": 10,
            "time": 1720000010000i64,
            "data": {
                "source": { "kind": "user" },
                "content": [{ "type": "text", "text": "f22 test message" }]
            }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f22", &events, 1200);

    // Enable no_extraction response for flush
    fs::write(base_dir.join("fake_curl_flush_no_extraction"), "1").unwrap();

    let output = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success(), "facts should exit 0 when flush returns no_extraction");

    let state_file = var_dir.join("facts.state.json");
    let state_raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();
    let sess = &state_raw["sessions"]["session-f22"];
    assert_eq!(
        sess["flushed_seq"], 10,
        "flushed_seq should be advanced to last_user_seq"
    );
    assert_eq!(
        sess["flushed_seq"], sess["last_user_seq"],
        "flushed_seq must equal last_user_seq"
    );

    let log_file = var_dir.join("facts.log");
    let log_content = fs::read_to_string(&log_file).unwrap();
    assert!(
        log_content.contains("status=no_extraction"),
        "log must contain status=no_extraction, got:\n{}",
        log_content
    );
}

// =========================================================================
// F23: 单会话失败不得阻塞后续会话：
// fake curl 对某个会话的 add 返回非 0 退出，对其它会话正常；
// 断言：后续会话仍被 add/flush 处理（capture 里有它们的 body）、退出码 1、日志有失败行、
// 失败会话 state 未推进、成功会话 state 推进
// =========================================================================
#[test]
fn test_f23_session_failure_isolation() {
    let (base_dir, sessions_root, var_dir, capture_dir) = create_test_env("f23");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(
        &base_dir,
        &sessions_root,
        &curl_path,
        Some(serde_json::json!({
            "idle_flush_seconds": 900,
        })),
    );

    // Create session 1: session-f23-fail, mtime 1500s ago
    let events1 = vec![
        serde_json::json!({
            "type": "user/message",
            "seq": 10,
            "time": 1720000010000i64,
            "data": {
                "source": { "kind": "user" },
                "content": [{ "type": "text", "text": "fail message" }]
            }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f23-fail", &events1, 1500);

    // Create session 2: session-f23-ok, mtime 1200s ago (mtime is newer, so it sorts after session-f23-fail)
    let events2 = vec![
        serde_json::json!({
            "type": "user/message",
            "seq": 20,
            "time": 1720000020000i64,
            "data": {
                "source": { "kind": "user" },
                "content": [{ "type": "text", "text": "ok message" }]
            }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f23-ok", &events2, 1200);

    // Configure fake curl to fail on session-f23-fail
    fs::write(
        base_dir.join("fake_curl_fail_session"),
        "session-f23-fail",
    )
    .unwrap();

    let output = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "must exit 1 when any session failed"
    );

    // Assert session-f23-ok was still processed (add and flush captured)
    let add_bodies = get_captured_add_bodies(&capture_dir);
    let ok_add_exists = add_bodies
        .iter()
        .any(|b| b["session_id"] == "session-f23-ok");
    assert!(
        ok_add_exists,
        "session-f23-ok add call must be captured despite previous session failure"
    );

    let flush_bodies = get_captured_flush_bodies(&capture_dir);
    let ok_flush_exists = flush_bodies
        .iter()
        .any(|b| b["session_id"] == "session-f23-ok");
    assert!(
        ok_flush_exists,
        "session-f23-ok flush call must be captured despite previous session failure"
    );

    // Assert log contains failure line
    let log_file = var_dir.join("facts.log");
    let log_content = fs::read_to_string(&log_file).unwrap();
    assert!(
        log_content.contains("add failed session=session-f23-fail"),
        "log must contain add failure line for failed session"
    );

    // Assert state of failed session is NOT advanced
    let state_file = var_dir.join("facts.state.json");
    assert!(state_file.exists());
    let state_raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();
    if let Some(fail_sess) = state_raw["sessions"].get("session-f23-fail") {
        assert_eq!(
            fail_sess["last_seq"], 0,
            "failed session last_seq must not advance"
        );
        assert_eq!(
            fail_sess["last_user_seq"], 0,
            "failed session last_user_seq must not advance"
        );
        assert_eq!(
            fail_sess["flushed_seq"], 0,
            "failed session flushed_seq must not advance"
        );
    }

    // Assert state of successful session IS advanced
    let ok_sess = &state_raw["sessions"]["session-f23-ok"];
    assert_eq!(
        ok_sess["last_user_seq"], 20,
        "successful session last_user_seq must advance"
    );
    assert_eq!(
        ok_sess["flushed_seq"], 20,
        "successful session flushed_seq must advance"
    );
}

// =========================================================================
// F24: 多批会话每批 add 后立即 flush：
// 一个会话 5 条 user 消息，max_messages_per_call=2（→3 批）；fake curl 全成功。
// 断言：退出码 0；capture 里 add/flush 交替（3 次 add、3 次 flush）；
// state flushed_seq == last_user_seq == 最后一条 seq；日志出现 3 次 flush 行。
// =========================================================================
#[test]
fn test_f24_multi_batch_flush_after_every_add() {
    let (base_dir, sessions_root, var_dir, capture_dir) = create_test_env("f24");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(
        &base_dir,
        &sessions_root,
        &curl_path,
        Some(serde_json::json!({
            "max_messages_per_call": 2,
            "max_flushes_per_run": 10,
        })),
    );

    let events = vec![
        serde_json::json!({
            "type": "user/message", "seq": 1, "time": 1720000001000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "msg 1" }] }
        }),
        serde_json::json!({
            "type": "user/message", "seq": 2, "time": 1720000002000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "msg 2" }] }
        }),
        serde_json::json!({
            "type": "user/message", "seq": 3, "time": 1720000003000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "msg 3" }] }
        }),
        serde_json::json!({
            "type": "user/message", "seq": 4, "time": 1720000004000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "msg 4" }] }
        }),
        serde_json::json!({
            "type": "user/message", "seq": 5, "time": 1720000005000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "msg 5" }] }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f24", &events, 1200);

    let output = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success(), "facts should exit 0 on success");

    let add_bodies = get_captured_add_bodies(&capture_dir);
    assert_eq!(add_bodies.len(), 3, "should have 3 add batches");
    let flush_bodies = get_captured_flush_bodies(&capture_dir);
    assert_eq!(flush_bodies.len(), 3, "should have 3 flush calls");

    // Capture checks: curl calls alternate add/flush
    let curl_args = fs::read_to_string(base_dir.join("curl_args.log")).unwrap();
    let lines: Vec<&str> = curl_args.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 6, "total 6 curl calls");
    assert!(lines[0].contains("/api/v2/memory/add"), "call 1 must be add");
    assert!(lines[1].contains("/api/v2/memory/flush"), "call 2 must be flush");
    assert!(lines[2].contains("/api/v2/memory/add"), "call 3 must be add");
    assert!(lines[3].contains("/api/v2/memory/flush"), "call 4 must be flush");
    assert!(lines[4].contains("/api/v2/memory/add"), "call 5 must be add");
    assert!(lines[5].contains("/api/v2/memory/flush"), "call 6 must be flush");

    // State check: flushed_seq == last_user_seq == 5
    let state_file = var_dir.join("facts.state.json");
    let state_raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();
    let sess = &state_raw["sessions"]["session-f24"];
    assert_eq!(sess["flushed_seq"], 5, "flushed_seq should be 5");
    assert_eq!(sess["last_user_seq"], 5, "last_user_seq should be 5");
    assert_eq!(sess["flushed_seq"], sess["last_user_seq"], "flushed_seq must equal last_user_seq");

    // Log check: exactly 3 flush lines
    let log_file = var_dir.join("facts.log");
    let log_content = fs::read_to_string(&log_file).unwrap();
    let flush_line_count = log_content
        .lines()
        .filter(|l| l.contains("flush session=session-f24"))
        .count();
    assert_eq!(flush_line_count, 3, "log must have exactly 3 flush lines");
    assert!(log_content.contains("batch=1/3"), "log must contain batch=1/3");
    assert!(log_content.contains("batch=2/3"), "log must contain batch=2/3");
    assert!(log_content.contains("batch=3/3"), "log must contain batch=3/3");
}

// =========================================================================
// F25: 多批会话第 2 次 flush 失败：
// 同 F24 配置，但 fake curl 让第 2 次 flush 失败（exit 22）。
// 断言：第 1 批 flush 后 flushed_seq 已推进；失败后本会话第 3 批不再发送（capture 里 add 只有 2 次）；
// 退出码 1；其它会话（再放一个正常的 1 批会话）照常处理。
// =========================================================================
#[test]
fn test_f25_multi_batch_second_flush_failure_isolation() {
    let (base_dir, sessions_root, var_dir, capture_dir) = create_test_env("f25");
    let curl_path = make_fake_curl(&base_dir, &capture_dir);
    let config_path = write_config(
        &base_dir,
        &sessions_root,
        &curl_path,
        Some(serde_json::json!({
            "max_messages_per_call": 2,
            "max_flushes_per_run": 10,
            "idle_flush_seconds": 900,
        })),
    );

    // Multi-batch session: 5 messages, mtime 1500s ago
    let events_multi = vec![
        serde_json::json!({
            "type": "user/message", "seq": 1, "time": 1720000001000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "multi msg 1" }] }
        }),
        serde_json::json!({
            "type": "user/message", "seq": 2, "time": 1720000002000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "multi msg 2" }] }
        }),
        serde_json::json!({
            "type": "user/message", "seq": 3, "time": 1720000003000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "multi msg 3" }] }
        }),
        serde_json::json!({
            "type": "user/message", "seq": 4, "time": 1720000004000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "multi msg 4" }] }
        }),
        serde_json::json!({
            "type": "user/message", "seq": 5, "time": 1720000005000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "multi msg 5" }] }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f25-multi", &events_multi, 1500);

    // Other normal session: 1 message, seq 10, mtime 1200s ago (newer than 1500, so sorted after)
    let events_other = vec![
        serde_json::json!({
            "type": "user/message", "seq": 10, "time": 1720000010000i64,
            "data": { "source": { "kind": "user" }, "content": [{ "type": "text", "text": "other msg 1" }] }
        }),
    ];
    create_session_with_events(&sessions_root, "session-f25-other", &events_other, 1200);

    // Configure fake curl to fail on the 2nd flush call
    fs::write(base_dir.join("fake_curl_fail_flush_nth"), "2").unwrap();

    let output = Command::new(get_bin())
        .args(["facts", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1), "facts must exit 1 when a session failed");

    // Capture checks:
    // Multi-batch session should only have sent 2 add batches (batch 3 not sent due to 2nd flush failure)
    let add_bodies = get_captured_add_bodies(&capture_dir);
    let multi_add_bodies: Vec<_> = add_bodies
        .iter()
        .filter(|b| b["session_id"] == "session-f25-multi")
        .collect();
    assert_eq!(
        multi_add_bodies.len(),
        2,
        "session-f25-multi must only have 2 add batches captured"
    );

    // Other session should have been processed (1 add batch, 1 flush)
    let other_add_bodies: Vec<_> = add_bodies
        .iter()
        .filter(|b| b["session_id"] == "session-f25-other")
        .collect();
    assert_eq!(
        other_add_bodies.len(),
        1,
        "session-f25-other must have 1 add batch captured"
    );

    let flush_bodies = get_captured_flush_bodies(&capture_dir);
    let other_flush_bodies: Vec<_> = flush_bodies
        .iter()
        .filter(|b| b["session_id"] == "session-f25-other")
        .collect();
    assert_eq!(
        other_flush_bodies.len(),
        1,
        "session-f25-other must have 1 flush call captured"
    );

    // State checks:
    let state_file = var_dir.join("facts.state.json");
    let state_raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_file).unwrap()).unwrap();

    let multi_sess = &state_raw["sessions"]["session-f25-multi"];
    assert_eq!(
        multi_sess["flushed_seq"], 2,
        "session-f25-multi flushed_seq must be advanced to batch 1 last seq (2)"
    );
    assert_eq!(
        multi_sess["last_user_seq"], 4,
        "session-f25-multi last_user_seq must be 4 after batch 2 add"
    );

    let other_sess = &state_raw["sessions"]["session-f25-other"];
    assert_eq!(
        other_sess["flushed_seq"], 10,
        "session-f25-other flushed_seq must be 10"
    );
    assert_eq!(
        other_sess["last_user_seq"], 10,
        "session-f25-other last_user_seq must be 10"
    );

    // Log checks: contains failure line for session-f25-multi
    let log_file = var_dir.join("facts.log");
    let log_content = fs::read_to_string(&log_file).unwrap();
    assert!(
        log_content.contains("flush failed session=session-f25-multi"),
        "log must contain flush failure line for session-f25-multi"
    );
}
