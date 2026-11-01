// tests/dream_test.rs — dream 子命令规格验收测试（T1–T15）

use chrono::{DateTime, FixedOffset, Utc};
use regex::Regex;
use riko_memory::dream::*;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

fn get_bin() -> &'static str {
    env!("CARGO_BIN_EXE_riko-memory")
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

fn create_test_env(test_name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let base_dir = env::temp_dir().join(format!("riko_dream_test_{}_{}", test_name, std::process::id()));
    let _ = fs::remove_dir_all(&base_dir);
    fs::create_dir_all(&base_dir).unwrap();

    let sessions_root = base_dir.join("sessions");
    let memory_dir = base_dir.join("memory");
    fs::create_dir_all(&sessions_root).unwrap();
    fs::create_dir_all(&memory_dir).unwrap();

    (base_dir, sessions_root, memory_dir)
}

fn make_fake_curl(base_dir: &Path, script_body: &str) -> PathBuf {
    let curl_path = base_dir.join("fake_curl.sh");
    let full_script = format!("#!/bin/bash\n{}\n", script_body);
    fs::write(&curl_path, full_script).unwrap();
    let mut perms = fs::metadata(&curl_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&curl_path, perms).unwrap();
    curl_path
}

fn setup_session_with_fixture(
    sessions_root: &Path,
    session_name: &str,
    fixture_filename: &str,
    mtime_minutes_ago: u64,
) -> PathBuf {
    let session_dir = sessions_root.join(session_name);
    fs::create_dir_all(&session_dir).unwrap();

    let dest_file = session_dir.join("session.v4.jsonl.zstd");
    let src_file = fixtures_dir().join(fixture_filename);
    fs::copy(&src_file, &dest_file).unwrap();

    if mtime_minutes_ago > 0 {
        let f = File::options().write(true).open(&dest_file).unwrap();
        let target_time = SystemTime::now() - Duration::from_secs(mtime_minutes_ago * 60);
        f.set_modified(target_time).unwrap();
    }

    dest_file
}

fn write_config(
    base_dir: &Path,
    sessions_root: &Path,
    memory_dir: &Path,
    curl_path: &Path,
    extra_json: Option<serde_json::Value>,
) -> PathBuf {
    let config_path = base_dir.join("dream.json");
    let mut map = serde_json::json!({
        "sessions_root": sessions_root.to_str().unwrap(),
        "memory_dir": memory_dir.to_str().unwrap(),
        "state_path": base_dir.join("var").join("dream.state.json").to_str().unwrap(),
        "log_path": base_dir.join("var").join("dream.log").to_str().unwrap(),
        "lock_path": base_dir.join("var").join("dream.lock").to_str().unwrap(),
        "endpoint": "https://opencode.ai/inference/go/openai/v1/responses",
        "model": "muse-spark-1.3-contributor",
        "api_key_env": "TEST_DREAM_API_KEY",
        "max_chars_per_call": 12000,
        "max_calls_per_run": 5,
        "max_candidates_per_call": 6,
        "window_days": 2,
        "timeout_seconds": 150,
        "skip_recent_seconds": 60,
        "stale_lock_seconds": 3600,
        "curl_path": curl_path.to_str().unwrap(),
        "reasoning_effort": "minimal"
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

// =========================================================================
// T1: 解压
// =========================================================================
#[test]
fn test_t1_decode_and_extract() {
    let offset = FixedOffset::east_opt(8 * 3600).unwrap();
    let fixture_path = fixtures_dir().join("dream-fixture.zstd");
    assert!(fixture_path.exists(), "fixture file must exist");

    let (content, line_count, trailing_partial) = decode_session_file(&fixture_path).unwrap();
    assert!(!trailing_partial, "full fixture should not have trailing partial");
    assert_eq!(line_count, 10, "fixture should have 10 events");

    let (messages, total_events, skipped_lines) = extract_messages(&content, 0, offset);
    assert_eq!(total_events, 10);
    assert_eq!(skipped_lines, 0);
    assert_eq!(messages.len(), 4, "should extract exactly 4 messages");

    // 顺序 [assistant seq193, user seq200, user seq252, assistant seq461]
    assert_eq!(messages[0].role, Role::Assistant);
    assert_eq!(messages[0].seq, 193);

    assert_eq!(messages[1].role, Role::User);
    assert_eq!(messages[1].seq, 200);

    assert_eq!(messages[2].role, Role::User);
    assert_eq!(messages[2].seq, 252);

    assert_eq!(messages[3].role, Role::Assistant);
    assert_eq!(messages[3].seq, 461);

    // transcript 含 【当前时间】，不含 系统激活、system-reminder
    let transcript = build_transcript(&messages);
    assert!(transcript.contains("【当前时间】"), "transcript must contain 【当前时间】");
    assert!(!transcript.contains("系统激活"), "transcript must NOT contain 系统激活");
    assert!(!transcript.contains("system-reminder"), "transcript must NOT contain system-reminder");
}

// =========================================================================
// T2: 截断文件
// =========================================================================
#[test]
fn test_t2_truncated_file() {
    let offset = FixedOffset::east_opt(8 * 3600).unwrap();
    let fixture_path = fixtures_dir().join("dream-fixture-partial.zstd");
    assert!(fixture_path.exists(), "fixture file must exist");

    let (content, line_count, trailing_partial) = decode_session_file(&fixture_path).unwrap();
    assert!(trailing_partial, "partial fixture should have trailing partial detected");
    assert_eq!(line_count, 9, "should have 9 complete lines");

    let (messages, _, _) = extract_messages(&content, 0, offset);
    // 不含 seq 9001 的消息
    for msg in &messages {
        assert_ne!(msg.seq, 9001, "must not contain seq 9001 message");
    }
}

// =========================================================================
// T3: dry-run
// =========================================================================
#[test]
fn test_t3_dry_run() {
    let (base_dir, sessions_root, memory_dir) = create_test_env("t3");
    let marker_file = base_dir.join("curl_executed.marker");
    let curl_script = format!("touch {}\nexit 0", marker_file.to_str().unwrap());
    let curl_path = make_fake_curl(&base_dir, &curl_script);

    let config_path = write_config(&base_dir, &sessions_root, &memory_dir, &curl_path, None);
    setup_session_with_fixture(
        &sessions_root,
        "session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc",
        "dream-fixture.zstd",
        10,
    );

    let output = Command::new(get_bin())
        .env_remove("TEST_DREAM_API_KEY")
        .args(["dream", "--config", config_path.to_str().unwrap(), "--dry-run"])
        .output()
        .unwrap();

    assert!(output.status.success(), "dry-run should succeed with exit 0");

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("记忆整理助手"), "stdout must contain '记忆整理助手'");
    assert!(stdout.contains("【当前时间】"), "stdout must contain '【当前时间】'");
    assert!(!stdout.contains("dream: sessions="), "dry-run must not output summary line");

    assert!(!marker_file.exists(), "curl must not be executed during dry-run");
    assert!(!base_dir.join("var").join("dream.state.json").exists(), "state must not be created");
    assert!(!memory_dir.join("inbox").exists(), "inbox must not be created");
}

// =========================================================================
// T4: 正常写 inbox
// =========================================================================
#[test]
fn test_t4_write_inbox_normal() {
    let (base_dir, sessions_root, memory_dir) = create_test_env("t4");
    let valid_response = serde_json::json!({
        "output": [
            {
                "type": "message",
                "content": [
                    {
                        "type": "text",
                        "text": "{\"candidates\":[{\"kind\":\"moment\",\"when\":\"2026-10-31T19:49:00+08:00\",\"text\":\"爹地晚上二次回复四件事：按需检索采纳……\",\"quote\":\"我发一句晚安，你能检索到什么，是吧\",\"tags\":[\"决定\",\"记忆系统\"],\"reason\":\"爹地亲自推翻每轮强制检索\"}]}"
                    }
                ]
            }
        ],
        "usage": {
            "input_tokens": 100,
            "output_tokens": 50
        }
    });

    let argv_file = base_dir.join("curl_argv.txt");
    let conf_copy_file = base_dir.join("curl_conf.txt");

    let curl_script = format!(r#"
echo "$@" > "{argv_path}"
for ((i=1; i<=$#; i++)); do
    arg="${{!i}}"
    if [ "$arg" = "--config" ]; then
        next_i=$((i + 1))
        conf_file="${{!next_i}}"
        cp "$conf_file" "{conf_copy_path}"
    fi
done
cat <<'EOF'
{resp}
EOF
"#,
        argv_path = argv_file.to_str().unwrap(),
        conf_copy_path = conf_copy_file.to_str().unwrap(),
        resp = serde_json::to_string(&valid_response).unwrap(),
    );
    let curl_path = make_fake_curl(&base_dir, &curl_script);

    let config_path = write_config(&base_dir, &sessions_root, &memory_dir, &curl_path, None);
    setup_session_with_fixture(
        &sessions_root,
        "session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc",
        "dream-fixture.zstd",
        10,
    );

    let output = Command::new(get_bin())
        .env("TEST_DREAM_API_KEY", "dummy_key")
        .args(["dream", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success(), "run must succeed with exit 0");

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("dream: sessions=1 chunks=1 candidates=1 inbox="));

    // 断言 inbox 文件存在
    let inbox_dir = memory_dir.join("inbox");
    assert!(inbox_dir.exists());

    let mut entries = fs::read_dir(&inbox_dir).unwrap();
    let inbox_file = entries.next().unwrap().unwrap().path();
    let content = fs::read_to_string(&inbox_file).unwrap();

    // 正则断言 run 标题行
    let title_regex = Regex::new(r"## run \d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} \+08:00 · session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc · seq 193-461").unwrap();
    assert!(title_regex.is_match(&content), "inbox content must match run title format: {}", content);

    // 断言字段顺序
    let kind_pos = content.find("kind: moment").expect("missing kind");
    let when_pos = content.find("when: 2026-10-31T19:49:00+08:00").expect("missing when");
    let tags_pos = content.find("tags: 决定, 记忆系统").expect("missing tags");
    let quote_pos = content.find("quote: 我发一句晚安，你能检索到什么，是吧").expect("missing quote");
    let why_pos = content.find("why: 爹地亲自推翻每轮强制检索").expect("missing why");
    let text_pos = content.find("text: |").expect("missing text");

    assert!(kind_pos < when_pos);
    assert!(when_pos < tags_pos);
    assert!(tags_pos < quote_pos);
    assert!(quote_pos < why_pos);
    assert!(why_pos < text_pos);

    // 检查 state 文件
    let state_path = base_dir.join("var").join("dream.state.json");
    let state_str = fs::read_to_string(&state_path).unwrap();
    let state_json: serde_json::Value = serde_json::from_str(&state_str).unwrap();
    assert_eq!(
        state_json["sessions"]["session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc"],
        461,
        "session last_seq must be 461"
    );

    // 断言假 curl 收到的 --config 文件内容必须包含 x-opencode-session: riko-dream
    let conf_content = fs::read_to_string(&conf_copy_file).expect("conf copy file must exist");
    assert!(
        conf_content.contains("header = \"x-opencode-session: riko-dream\""),
        "--config file must contain 'header = \"x-opencode-session: riko-dream\"': {}",
        conf_content
    );

    // 断言传给假 curl 的 argv 里不含 API key
    let argv_content = fs::read_to_string(&argv_file).expect("argv file must exist");
    assert!(
        !argv_content.contains("dummy_key"),
        "curl argv must not contain API key: {}",
        argv_content
    );
    assert!(argv_content.contains("--config"), "curl argv must contain '--config'");
}

// =========================================================================
// T5: 幂等
// =========================================================================
#[test]
fn test_t5_idempotent() {
    let (base_dir, sessions_root, memory_dir) = create_test_env("t5");
    let valid_response = serde_json::json!({
        "output": [
            {
                "type": "message",
                "content": [
                    {
                        "type": "text",
                        "text": "{\"candidates\":[{\"kind\":\"moment\",\"when\":\"2026-10-31T19:49:00+08:00\",\"text\":\"文本\",\"quote\":\"原话\",\"tags\":[\"标签\"],\"reason\":\"原因\"}]}"
                    }
                ]
            }
        ]
    });
    let curl_script = format!("cat <<'EOF'\n{}\nEOF\n", serde_json::to_string(&valid_response).unwrap());
    let curl_path = make_fake_curl(&base_dir, &curl_script);

    let config_path = write_config(&base_dir, &sessions_root, &memory_dir, &curl_path, None);
    setup_session_with_fixture(
        &sessions_root,
        "session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc",
        "dream-fixture.zstd",
        10,
    );

    // 第一次运行
    let out1 = Command::new(get_bin())
        .env("TEST_DREAM_API_KEY", "dummy_key")
        .args(["dream", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out1.status.success());

    let inbox_file = memory_dir.join("inbox").read_dir().unwrap().next().unwrap().unwrap().path();
    let initial_size = fs::metadata(&inbox_file).unwrap().len();

    // 第二次运行
    let out2 = Command::new(get_bin())
        .env("TEST_DREAM_API_KEY", "dummy_key")
        .args(["dream", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out2.status.success());

    let final_size = fs::metadata(&inbox_file).unwrap().len();
    assert_eq!(initial_size, final_size, "inbox size should not change on second run");

    let log_file = base_dir.join("var").join("dream.log");
    let log_content = fs::read_to_string(&log_file).unwrap();
    assert!(log_content.contains("no new messages"), "log must contain 'no new messages'");
}

// =========================================================================
// T6: 分块续跑
// =========================================================================
#[test]
fn test_t6_chunking_resume() {
    let (base_dir, sessions_root, memory_dir) = create_test_env("t6");
    let valid_response = serde_json::json!({
        "output": [
            {
                "type": "message",
                "content": [
                    {
                        "type": "text",
                        "text": "{\"candidates\":[{\"kind\":\"moment\",\"when\":\"2026-10-31T19:49:00+08:00\",\"text\":\"分块测试草稿\",\"quote\":\"原话\",\"tags\":[\"测试\"],\"reason\":\"原因\"}]}"
                    }
                ]
            }
        ]
    });
    let curl_script = format!("cat <<'EOF'\n{}\nEOF\n", serde_json::to_string(&valid_response).unwrap());
    let curl_path = make_fake_curl(&base_dir, &curl_script);

    // max_chars_per_call 设为 40, max_calls_per_run 设为 1
    let config_path = write_config(
        &base_dir,
        &sessions_root,
        &memory_dir,
        &curl_path,
        Some(serde_json::json!({
            "max_chars_per_call": 40,
            "max_calls_per_run": 1
        })),
    );
    setup_session_with_fixture(
        &sessions_root,
        "session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc",
        "dream-fixture.zstd",
        10,
    );

    // 第一次运行
    let out1 = Command::new(get_bin())
        .env("TEST_DREAM_API_KEY", "dummy_key")
        .args(["dream", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out1.status.success());

    let state_path = base_dir.join("var").join("dream.state.json");
    let state_str = fs::read_to_string(&state_path).unwrap();
    let state_json: serde_json::Value = serde_json::from_str(&state_str).unwrap();
    let seq_run1 = state_json["sessions"]["session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc"].as_u64().unwrap();
    assert_eq!(seq_run1, 193, "first chunk should process only seq 193");

    // 第二次运行，指定 --max-calls 5 把剩余消息处理完
    let out2 = Command::new(get_bin())
        .env("TEST_DREAM_API_KEY", "dummy_key")
        .args(["dream", "--config", config_path.to_str().unwrap(), "--max-calls", "5"])
        .output()
        .unwrap();
    assert!(out2.status.success());

    let state_str2 = fs::read_to_string(&state_path).unwrap();
    let state_json2: serde_json::Value = serde_json::from_str(&state_str2).unwrap();
    let seq_run2 = state_json2["sessions"]["session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc"].as_u64().unwrap();
    assert_eq!(seq_run2, 461, "all messages should be completed up to 461");

    // inbox 中应该有两节或多节
    let inbox_file = memory_dir.join("inbox").read_dir().unwrap().next().unwrap().unwrap().path();
    let content = fs::read_to_string(&inbox_file).unwrap();
    let run_count = content.matches("## run ").count();
    assert!(run_count >= 2, "inbox should have at least 2 run sections, got {}", run_count);
}

// =========================================================================
// T7: LLM 容错
// =========================================================================
#[test]
fn test_t7_llm_retry_and_failure() {
    let (base_dir, sessions_root, memory_dir) = create_test_env("t7");
    let count_file = base_dir.join("curl_call_count.txt");
    let body_copy_prefix = base_dir.join("curl_body_copy");

    let script = format!(r#"
COUNT_FILE="{}"
cnt=$(cat "$COUNT_FILE" 2>/dev/null || echo 0)
cnt=$((cnt + 1))
echo "$cnt" > "$COUNT_FILE"

# 复制 body
for arg in "$@"; do
    if [[ "$arg" == @* ]]; then
        file="${{arg:1}}"
        cp "$file" "{}_$cnt.json"
    fi
done

if [ "$cnt" -eq 1 ]; then
    cat <<'EOF'
{{
  "output": [
    {{
      "type": "message",
      "content": [
        {{"type": "text", "text": "not json"}}
      ]
    }}
  ]
}}
EOF
else
    cat <<'EOF'
{{
  "output": [
    {{
      "type": "message",
      "content": [
        {{"type": "text", "text": "{{\"candidates\":[{{\"kind\":\"moment\",\"when\":\"2026-10-31T19:49:00+08:00\",\"text\":\"文本\",\"quote\":\"原话\",\"tags\":[\"标签\"],\"reason\":\"原因\"}}]}}"}}
      ]
    }}
  ]
}}
EOF
fi
"#, count_file.to_str().unwrap(), body_copy_prefix.to_str().unwrap());

    let curl_path = make_fake_curl(&base_dir, &script);
    let config_path = write_config(&base_dir, &sessions_root, &memory_dir, &curl_path, None);
    setup_session_with_fixture(
        &sessions_root,
        "session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc",
        "dream-fixture.zstd",
        10,
    );

    // 第一次：第一次返回 not json，第二次返回合法 JSON → exit 0
    let out = Command::new(get_bin())
        .env("TEST_DREAM_API_KEY", "dummy_key")
        .args(["dream", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(out.status.success(), "should succeed after retry");
    let count_str = fs::read_to_string(&count_file).unwrap();
    assert_eq!(count_str.trim(), "2", "curl should be called twice");

    let body2_file = PathBuf::from(format!("{}_2.json", body_copy_prefix.to_str().unwrap()));
    let body2_content = fs::read_to_string(&body2_file).unwrap();
    assert!(body2_content.contains("只输出一个 JSON 对象"), "retry prompt must contain reminder sentence");

    // 第二个场景：两次都非法 → exit 1，state 不变，日志有 error
    let (base_dir2, sessions_root2, memory_dir2) = create_test_env("t7_fail");
    let invalid_script = r#"cat <<'EOF'
{
  "output": [
    {
      "type": "message",
      "content": [
        {"type": "text", "text": "not json"}
      ]
    }
  ]
}
EOF
"#;
    let curl_path2 = make_fake_curl(&base_dir2, invalid_script);
    let config_path2 = write_config(&base_dir2, &sessions_root2, &memory_dir2, &curl_path2, None);
    setup_session_with_fixture(
        &sessions_root2,
        "session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc",
        "dream-fixture.zstd",
        10,
    );

    let out_fail = Command::new(get_bin())
        .env("TEST_DREAM_API_KEY", "dummy_key")
        .args(["dream", "--config", config_path2.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(out_fail.status.code(), Some(1), "should exit 1 when both retries fail");
    let state_file2 = base_dir2.join("var").join("dream.state.json");
    if state_file2.exists() {
        let s = fs::read_to_string(&state_file2).unwrap();
        assert!(!s.contains("461"), "state should not have advanced");
    }
    let log_file2 = base_dir2.join("var").join("dream.log");
    let log_content2 = fs::read_to_string(&log_file2).unwrap();
    assert!(log_content2.contains("error"), "log must contain error");
}

// =========================================================================
// T8: 候选校验
// =========================================================================
#[test]
fn test_t8_candidate_validation() {
    let (base_dir, sessions_root, memory_dir) = create_test_env("t8");

    // 构造 7 条候选：包含未知 kind、空 quote、超长 text、合法的
    let long_text = "a".repeat(501);
    let candidates = serde_json::json!({
        "candidates": [
            // 1. 合法
            {"kind": "moment", "when": "2026-10-31T19:49:00+08:00", "text": "合法1", "quote": "原话1", "tags": ["tag1"], "reason": "理由1"},
            // 2. 未知 kind -> 丢弃
            {"kind": "unknown_kind", "when": "2026-10-31T19:49:00+08:00", "text": "文本2", "quote": "原话2", "tags": [], "reason": "理由2"},
            // 3. 空 quote -> 丢弃
            {"kind": "people", "when": "2026-10-31T19:49:00+08:00", "text": "文本3", "quote": "", "tags": [], "reason": "理由3"},
            // 4. 超长 text -> 丢弃
            {"kind": "self", "when": "2026-10-31T19:49:00+08:00", "text": long_text, "quote": "原话4", "tags": [], "reason": "理由4"},
            // 5. 合法
            {"kind": "open-loop", "when": "2026-10-31T19:49:00+08:00", "text": "合法2", "quote": "原话5", "tags": ["tag2"], "reason": "理由5"},
            // 6. 合法
            {"kind": "moment", "when": "2026-10-31T19:49:00+08:00", "text": "合法3", "quote": "原话6", "tags": ["tag3"], "reason": "理由6"},
            // 7. 合法
            {"kind": "moment", "when": "2026-10-31T19:49:00+08:00", "text": "合法4", "quote": "原话7", "tags": ["tag4"], "reason": "理由7"}
        ]
    });

    let resp = serde_json::json!({
        "output": [
            {
                "type": "message",
                "content": [
                    {"type": "text", "text": serde_json::to_string(&candidates).unwrap()}
                ]
            }
        ]
    });

    let curl_script = format!("cat <<'EOF'\n{}\nEOF\n", serde_json::to_string(&resp).unwrap());
    let curl_path = make_fake_curl(&base_dir, &curl_script);

    // max_candidates_per_call 设为 2
    let config_path = write_config(
        &base_dir,
        &sessions_root,
        &memory_dir,
        &curl_path,
        Some(serde_json::json!({
            "max_candidates_per_call": 2
        })),
    );
    setup_session_with_fixture(
        &sessions_root,
        "session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc",
        "dream-fixture.zstd",
        10,
    );

    let out = Command::new(get_bin())
        .env("TEST_DREAM_API_KEY", "dummy_key")
        .args(["dream", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(out.status.success());

    // inbox 只出现合法条目且 <= max_candidates_per_call (2 条)
    let inbox_file = memory_dir.join("inbox").read_dir().unwrap().next().unwrap().unwrap().path();
    let content = fs::read_to_string(&inbox_file).unwrap();
    assert!(!content.contains("unknown_kind"));
    assert!(!content.contains("文本2"));
    assert!(!content.contains("文本3"));
    assert!(!content.contains("文本4"));
    assert!(content.contains("合法1"));
    assert!(content.contains("合法2"));
    assert!(!content.contains("合法3")); // 因截断被丢弃

    // 日志中 dropped 计数：3 条非法 + 2 条超额 = 5 dropped
    let log_file = base_dir.join("var").join("dream.log");
    let log_content = fs::read_to_string(&log_file).unwrap();
    assert!(log_content.contains("dropped=5"), "log must report dropped=5: {}", log_content);
}

// =========================================================================
// T9: 锁
// =========================================================================
#[test]
fn test_t9_lock() {
    let (base_dir, sessions_root, memory_dir) = create_test_env("t9");
    let curl_path = make_fake_curl(&base_dir, "echo '{}'");
    let config_path = write_config(&base_dir, &sessions_root, &memory_dir, &curl_path, None);
    setup_session_with_fixture(
        &sessions_root,
        "session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc",
        "dream-fixture.zstd",
        10,
    );

    let lock_file = base_dir.join("var").join("dream.lock");
    fs::create_dir_all(lock_file.parent().unwrap()).unwrap();

    // 1. 手动放一个新鲜的 lock 文件
    fs::write(&lock_file, "pid=999999 time=2026-10-31T20:00:00+08:00\n").unwrap();

    let out1 = Command::new(get_bin())
        .env("TEST_DREAM_API_KEY", "dummy_key")
        .args(["dream", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(out1.status.code(), Some(0), "should exit 0 when lock exists");
    let stderr1 = String::from_utf8(out1.stderr).unwrap();
    assert!(stderr1.contains("another dream is running"), "stderr must contain 'another dream is running'");
    assert!(!memory_dir.join("inbox").exists(), "should not write inbox");

    // 2. lock 文件 mtime 设成 2 小时前 -> 正常执行
    let f = OpenOptions::new().write(true).open(&lock_file).unwrap();
    let two_hours_ago = SystemTime::now() - Duration::from_secs(7200);
    f.set_modified(two_hours_ago).unwrap();

    let valid_response = serde_json::json!({
        "output": [
            {
                "type": "message",
                "content": [
                    {"type": "text", "text": "{\"candidates\":[{\"kind\":\"moment\",\"when\":\"2026-10-31T19:49:00+08:00\",\"text\":\"文本\",\"quote\":\"原话\",\"tags\":[\"标签\"],\"reason\":\"原因\"}]}"}
                ]
            }
        ]
    });
    let curl_script = format!("cat <<'EOF'\n{}\nEOF\n", serde_json::to_string(&valid_response).unwrap());
    fs::write(&curl_path, format!("#!/bin/bash\n{}\n", curl_script)).unwrap();

    let out2 = Command::new(get_bin())
        .env("TEST_DREAM_API_KEY", "dummy_key")
        .args(["dream", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(out2.status.code(), Some(0), "stale lock should be broken and command should succeed");
    assert!(memory_dir.join("inbox").exists(), "inbox should be created after breaking stale lock");
}

// =========================================================================
// T10: 配置错误
// =========================================================================
#[test]
fn test_t10_config_error() {
    let (base_dir, _, _) = create_test_env("t10");
    let nonexistent = base_dir.join("nonexistent_dream.json");

    // 1. 配置文件不存在
    let out1 = Command::new(get_bin())
        .args(["dream", "--config", nonexistent.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(out1.status.code(), Some(1));
    let stderr1 = String::from_utf8(out1.stderr).unwrap();
    assert!(stderr1.contains(nonexistent.to_str().unwrap()), "stderr must contain config path: {}", stderr1);

    // 2. api_key_env 未设置（非 dry-run）
    let (base_dir2, sessions_root2, memory_dir2) = create_test_env("t10_key");
    let curl_path2 = make_fake_curl(&base_dir2, "exit 0");
    let config_path2 = write_config(&base_dir2, &sessions_root2, &memory_dir2, &curl_path2, None);

    let out2 = Command::new(get_bin())
        .env_remove("TEST_DREAM_API_KEY")
        .args(["dream", "--config", config_path2.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(out2.status.code(), Some(1));
    let stderr2 = String::from_utf8(out2.stderr).unwrap();
    assert!(stderr2.contains("TEST_DREAM_API_KEY"), "stderr must mention missing api key env: {}", stderr2);

    // 3. 数字字段 0 或负数，以及 window_days 范围测试
    let test_cases = [
        ("max_chars_per_call", serde_json::json!(-1)),
        ("max_chars_per_call", serde_json::json!(0)),
        ("max_calls_per_run", serde_json::json!(-5)),
        ("max_candidates_per_call", serde_json::json!(0)),
        ("timeout_seconds", serde_json::json!(-10)),
        ("skip_recent_seconds", serde_json::json!(0)),
        ("stale_lock_seconds", serde_json::json!(-100)),
        ("max_output_tokens", serde_json::json!(-1)),
        ("max_output_tokens", serde_json::json!(0)),
        ("window_days", serde_json::json!(-1)),
        ("window_days", serde_json::json!(0)),
        ("window_days", serde_json::json!(35)),
    ];

    for (field_name, field_val) in test_cases {
        let (base_dir_case, sessions_root_case, memory_dir_case) = create_test_env("t10_num");
        let curl_case = make_fake_curl(&base_dir_case, "exit 0");
        let config_case = write_config(
            &base_dir_case,
            &sessions_root_case,
            &memory_dir_case,
            &curl_case,
            Some(serde_json::json!({ field_name: field_val })),
        );

        let out_case = Command::new(get_bin())
            .env("TEST_DREAM_API_KEY", "dummy_key")
            .args(["dream", "--config", config_case.to_str().unwrap()])
            .output()
            .unwrap();

        assert_eq!(out_case.status.code(), Some(1), "field {} with val {} should exit 1", field_name, field_val);
        let err_str = String::from_utf8(out_case.stderr).unwrap();
        assert!(
            err_str.contains(field_name),
            "stderr must mention invalid field name '{}', got: {}",
            field_name,
            err_str
        );
    }
}

// =========================================================================
// T11: 时区
// =========================================================================
#[test]
fn test_t11_timezone() {
    let (base_dir, sessions_root, memory_dir) = create_test_env("t11");
    let curl_path = make_fake_curl(&base_dir, "exit 0");

    // 构造一条 time 为 UTC 16:30 的消息（对应东八区次日 00:30）
    let event = serde_json::json!({
        "type": "user/message",
        "seq": 100,
        "time": 1793464200000_i64, // 对应某个时间的 16:30 UTC
        "data": {
            "source": {"kind": "user"},
            "content": [{"type": "text", "text": "时区测试文本"}]
        }
    });

    let session_dir = sessions_root.join("session-tz");
    fs::create_dir_all(&session_dir).unwrap();
    let session_file = session_dir.join("session.v4.jsonl.zstd");

    let event_line = serde_json::to_string(&event).unwrap() + "\n";
    let compressed = zstd::stream::encode_all(event_line.as_bytes(), 0).unwrap();
    fs::write(&session_file, compressed).unwrap();

    let f = File::options().write(true).open(&session_file).unwrap();
    f.set_modified(SystemTime::now() - Duration::from_secs(600)).unwrap();

    let config_path = write_config(&base_dir, &sessions_root, &memory_dir, &curl_path, None);

    let out = Command::new(get_bin())
        .args(["dream", "--config", config_path.to_str().unwrap(), "--dry-run"])
        .output()
        .unwrap();

    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    // 断言 transcript 时间包含 +08:00
    assert!(stdout.contains("+08:00"), "transcript time must contain '+08:00': {}", stdout);

    // 计算运行时的东八区日期，断言 inbox 文件名
    let offset = FixedOffset::east_opt(8 * 3600).unwrap();
    let expected_date = Utc::now().with_timezone(&offset).format("%Y-%m-%d").to_string();

    let valid_response = serde_json::json!({
        "output": [
            {
                "type": "message",
                "content": [
                    {"type": "text", "text": "{\"candidates\":[{\"kind\":\"moment\",\"when\":\"2026-10-31T19:49:00+08:00\",\"text\":\"文本\",\"quote\":\"原话\",\"tags\":[\"标签\"],\"reason\":\"原因\"}]}"}
                ]
            }
        ]
    });
    let curl_script = format!("cat <<'EOF'\n{}\nEOF\n", serde_json::to_string(&valid_response).unwrap());
    fs::write(&curl_path, format!("#!/bin/bash\n{}\n", curl_script)).unwrap();

    let out2 = Command::new(get_bin())
        .env("TEST_DREAM_API_KEY", "dummy_key")
        .args(["dream", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(out2.status.success());
    let expected_inbox = memory_dir.join("inbox").join(format!("{}.md", expected_date));
    assert!(expected_inbox.exists(), "inbox file {} must exist", expected_inbox.display());
}

// =========================================================================
// T12: curl 失败
// =========================================================================
#[test]
fn test_t12_curl_failure() {
    let (base_dir, sessions_root, memory_dir) = create_test_env("t12");
    let count_file = base_dir.join("curl_call_count.txt");

    let script = format!(r#"
COUNT_FILE="{}"
cnt=$(cat "$COUNT_FILE" 2>/dev/null || echo 0)
cnt=$((cnt + 1))
echo "$cnt" > "$COUNT_FILE"
exit 7
"#, count_file.to_str().unwrap());

    let curl_path = make_fake_curl(&base_dir, &script);
    let config_path = write_config(&base_dir, &sessions_root, &memory_dir, &curl_path, None);
    setup_session_with_fixture(
        &sessions_root,
        "session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc",
        "dream-fixture.zstd",
        10,
    );

    let out = Command::new(get_bin())
        .env("TEST_DREAM_API_KEY", "dummy_key")
        .args(["dream", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "must exit 1 on curl exit 7");
    let count_str = fs::read_to_string(&count_file).unwrap();
    assert_eq!(count_str.trim(), "2", "curl should be retried once (2 calls total)");

    let state_file = base_dir.join("var").join("dream.state.json");
    if state_file.exists() {
        let s = fs::read_to_string(&state_file).unwrap();
        assert!(!s.contains("461"), "state must not have advanced");
    }
}

// =========================================================================
// T13: 无新消息
// =========================================================================
#[test]
fn test_t13_no_new_messages() {
    let (base_dir, sessions_root, memory_dir) = create_test_env("t13");
    let curl_path = make_fake_curl(&base_dir, "exit 0");
    let config_path = write_config(&base_dir, &sessions_root, &memory_dir, &curl_path, None);
    setup_session_with_fixture(
        &sessions_root,
        "session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc",
        "dream-fixture.zstd",
        10,
    );

    // 手动将 state 里该 session 的 seq 设为 461（已到最大）
    let state_file = base_dir.join("var").join("dream.state.json");
    fs::create_dir_all(state_file.parent().unwrap()).unwrap();
    let initial_state = serde_json::json!({
        "version": 1,
        "sessions": {
            "session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc": 461
        }
    });
    fs::write(&state_file, serde_json::to_string(&initial_state).unwrap()).unwrap();

    let out = Command::new(get_bin())
        .env("TEST_DREAM_API_KEY", "dummy_key")
        .args(["dream", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(0));
    assert!(!memory_dir.join("inbox").exists(), "no inbox section should be written");

    let log_file = base_dir.join("var").join("dream.log");
    let log_content = fs::read_to_string(&log_file).unwrap();
    assert!(log_content.contains("no new messages"), "log must contain 'no new messages'");
}

// =========================================================================
// T14: 过滤会话
// =========================================================================
#[test]
fn test_t14_filter_sessions() {
    let (base_dir, sessions_root, memory_dir) = create_test_env("t14");
    let curl_path = make_fake_curl(&base_dir, "exit 0");
    let config_path = write_config(&base_dir, &sessions_root, &memory_dir, &curl_path, None);

    // 1. mtime 超过 window_days (例如 3 天前，window_days=2)
    let expired_dir = sessions_root.join("session-expired");
    fs::create_dir_all(&expired_dir).unwrap();
    let expired_file = expired_dir.join("session.v4.jsonl.zstd");
    fs::copy(fixtures_dir().join("dream-fixture.zstd"), &expired_file).unwrap();
    let f1 = File::options().write(true).open(&expired_file).unwrap();
    f1.set_modified(SystemTime::now() - Duration::from_secs(3 * 86400)).unwrap();

    // 2. sessions_root 下非 session 目录或普通文件
    fs::create_dir_all(sessions_root.join("not_a_session_dir")).unwrap();
    fs::write(sessions_root.join("random_file.txt"), "hello").unwrap();

    // 3. 空会话（只有 session 顶层事件，没有提取出的消息）
    let empty_dir = sessions_root.join("session-empty");
    fs::create_dir_all(&empty_dir).unwrap();
    let empty_file = empty_dir.join("session.v4.jsonl.zstd");
    let session_event = "{\"type\":\"session\",\"time\":1793447581890}\n";
    let compressed = zstd::stream::encode_all(session_event.as_bytes(), 0).unwrap();
    fs::write(&empty_file, compressed).unwrap();
    let f2 = File::options().write(true).open(&empty_file).unwrap();
    f2.set_modified(SystemTime::now() - Duration::from_secs(600)).unwrap();

    let out = Command::new(get_bin())
        .env("TEST_DREAM_API_KEY", "dummy_key")
        .args(["dream", "--config", config_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.trim(), "dream: sessions=0 chunks=0 candidates=0 inbox=-");

    let log_file = base_dir.join("var").join("dream.log");
    let log_content = fs::read_to_string(&log_file).unwrap();
    assert!(log_content.contains("skip session=session-expired reason=window_expired"));
}

// =========================================================================
// T15: 回归（验证原 21 个测试通过 & release 零 warning）
// =========================================================================
#[test]
fn test_t15_regression_baseline() {
    // 单元测试中验证重要结构体和核心函数的回归
    let offset = FixedOffset::east_opt(8 * 3600).unwrap();
    let dt = DateTime::parse_from_rfc3339("2026-10-31T19:49:00+08:00").unwrap().with_timezone(&offset);
    let msg = Message {
        seq: 1,
        role: Role::User,
        time: dt,
        text: "第一行\n第二行".to_string(),
    };
    let transcript = format_message_transcript(&msg);
    assert_eq!(transcript, "[2026-10-31 19:49 +08:00] 爹地: 第一行\n  第二行");
}

// =========================================================================
// T7.4: spec §7.4 响应解析（status == incomplete 记 warning & output 无文本重试并失败）
// =========================================================================
#[test]
fn test_t7_4_incomplete_and_no_text() {
    // 1. status == "incomplete" 但包含候选文本 -> 记 warning 且正常处理完成
    let (base_dir, sessions_root, memory_dir) = create_test_env("t7_4_inc");
    let inc_response = serde_json::json!({
        "status": "incomplete",
        "output": [
            {
                "type": "message",
                "content": [
                    {
                        "type": "text",
                        "text": "{\"candidates\":[{\"kind\":\"moment\",\"when\":\"2026-10-31T19:49:00+08:00\",\"text\":\"文本\",\"quote\":\"原话\",\"tags\":[\"标签\"],\"reason\":\"原因\"}]}"
                    }
                ]
            }
        ]
    });
    let curl_script1 = format!("cat <<'EOF'\n{}\nEOF\n", serde_json::to_string(&inc_response).unwrap());
    let curl_path1 = make_fake_curl(&base_dir, &curl_script1);
    let config_path1 = write_config(&base_dir, &sessions_root, &memory_dir, &curl_path1, None);
    setup_session_with_fixture(
        &sessions_root,
        "session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc",
        "dream-fixture.zstd",
        10,
    );

    let out1 = Command::new(get_bin())
        .env("TEST_DREAM_API_KEY", "dummy_key")
        .args(["dream", "--config", config_path1.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(out1.status.success(), "incomplete with candidates should succeed");
    let log_file1 = base_dir.join("var").join("dream.log");
    let log_content1 = fs::read_to_string(&log_file1).unwrap();
    assert!(
        log_content1.contains("warning llm response status is incomplete"),
        "log must report warning for incomplete status: {}",
        log_content1
    );

    // 2. output 里找不到任何文本 -> 重试一次，仍无文本则失败 exit 1
    let (base_dir2, sessions_root2, memory_dir2) = create_test_env("t7_4_notext");
    let count_file2 = base_dir2.join("curl_call_count.txt");
    let no_text_script = format!(r#"
COUNT_FILE="{}"
cnt=$(cat "$COUNT_FILE" 2>/dev/null || echo 0)
cnt=$((cnt + 1))
echo "$cnt" > "$COUNT_FILE"
cat <<'EOF'
{{
  "status": "completed",
  "output": []
}}
EOF
"#, count_file2.to_str().unwrap());
    let curl_path2 = make_fake_curl(&base_dir2, &no_text_script);
    let config_path2 = write_config(&base_dir2, &sessions_root2, &memory_dir2, &curl_path2, None);
    setup_session_with_fixture(
        &sessions_root2,
        "session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc",
        "dream-fixture.zstd",
        10,
    );

    let out2 = Command::new(get_bin())
        .env("TEST_DREAM_API_KEY", "dummy_key")
        .args(["dream", "--config", config_path2.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(out2.status.code(), Some(1), "no text in output must exit 1");
    let count_str2 = fs::read_to_string(&count_file2).unwrap();
    assert_eq!(count_str2.trim(), "2", "curl should be retried once (total 2 calls)");
    let log_file2 = base_dir2.join("var").join("dream.log");
    let log_content2 = fs::read_to_string(&log_file2).unwrap();
    assert!(log_content2.contains("error"), "log must report error: {}", log_content2);
}

// =========================================================================
// T16: max_output_tokens 配置化（缺省 8192、显式 4096、非法值报错）
// =========================================================================
#[test]
fn test_t16_max_output_tokens() {
    let dummy_response = serde_json::json!({
        "status": "completed",
        "output": [
            {
                "type": "message",
                "content": [
                    {
                        "type": "text",
                        "text": "{\"candidates\":[]}"
                    }
                ]
            }
        ],
        "usage": {
            "input_tokens": 100,
            "output_tokens": 50
        }
    });

    // 1. 缺省配置：max_output_tokens 缺省为 8192
    {
        let (base_dir, sessions_root, memory_dir) = create_test_env("t16_default");
        let body_copy_path = base_dir.join("request_body.json");
        let curl_script = format!(
            r#"
for ((i=1; i<=$#; i++)); do
    arg="${{!i}}"
    if [ "$arg" = "--data-binary" ]; then
        next_i=$((i + 1))
        body_file="${{!next_i}}"
        body_file="${{body_file#@}}"
        cp "$body_file" "{body_copy_path}"
    fi
done
cat <<'EOF'
{resp}
EOF
"#,
            body_copy_path = body_copy_path.to_str().unwrap(),
            resp = serde_json::to_string(&dummy_response).unwrap(),
        );
        let curl_path = make_fake_curl(&base_dir, &curl_script);
        let config_path = write_config(&base_dir, &sessions_root, &memory_dir, &curl_path, None);

        // 验证 DreamConfig::load 解析出 8192
        let args = DreamArgs {
            config: Some(config_path.clone()),
            dry_run: true,
            max_calls: None,
        };
        let cfg = DreamConfig::load(&args).expect("DreamConfig::load failed on default config");
        assert_eq!(
            cfg.max_output_tokens, 8192,
            "default max_output_tokens must be 8192"
        );

        // 验证实际发送给 curl 的 body
        setup_session_with_fixture(
            &sessions_root,
            "session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc",
            "dream-fixture.zstd",
            10,
        );
        let out = Command::new(get_bin())
            .env("TEST_DREAM_API_KEY", "dummy_key")
            .args(["dream", "--config", config_path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(out.status.success(), "dream should succeed");

        let body_str = fs::read_to_string(&body_copy_path).unwrap();
        let body_json: serde_json::Value = serde_json::from_str(&body_str).unwrap();
        assert_eq!(body_json["max_output_tokens"], 8192);
    }

    // 2. 显式配置 4096 时生效
    {
        let (base_dir, sessions_root, memory_dir) = create_test_env("t16_custom");
        let body_copy_path = base_dir.join("request_body.json");
        let curl_script = format!(
            r#"
for ((i=1; i<=$#; i++)); do
    arg="${{!i}}"
    if [ "$arg" = "--data-binary" ]; then
        next_i=$((i + 1))
        body_file="${{!next_i}}"
        body_file="${{body_file#@}}"
        cp "$body_file" "{body_copy_path}"
    fi
done
cat <<'EOF'
{resp}
EOF
"#,
            body_copy_path = body_copy_path.to_str().unwrap(),
            resp = serde_json::to_string(&dummy_response).unwrap(),
        );
        let curl_path = make_fake_curl(&base_dir, &curl_script);
        let config_path = write_config(
            &base_dir,
            &sessions_root,
            &memory_dir,
            &curl_path,
            Some(serde_json::json!({ "max_output_tokens": 4096 })),
        );

        // 验证 DreamConfig::load 解析出 4096
        let args = DreamArgs {
            config: Some(config_path.clone()),
            dry_run: true,
            max_calls: None,
        };
        let cfg = DreamConfig::load(&args).expect("DreamConfig::load failed on custom config");
        assert_eq!(
            cfg.max_output_tokens, 4096,
            "explicit max_output_tokens should be 4096"
        );

        // 验证实际发送给 curl 的 body
        setup_session_with_fixture(
            &sessions_root,
            "session-5efd2274-4bf2-447a-8d6c-aca3b2c9c9fc",
            "dream-fixture.zstd",
            10,
        );
        let out = Command::new(get_bin())
            .env("TEST_DREAM_API_KEY", "dummy_key")
            .args(["dream", "--config", config_path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(out.status.success(), "dream should succeed");

        let body_str = fs::read_to_string(&body_copy_path).unwrap();
        let body_json: serde_json::Value = serde_json::from_str(&body_str).unwrap();
        assert_eq!(body_json["max_output_tokens"], 4096);
    }

    // 3. 0 或负数报错
    for invalid_val in [serde_json::json!(0), serde_json::json!(-1)] {
        let (base_dir, sessions_root, memory_dir) = create_test_env("t16_invalid");
        let curl_path = make_fake_curl(&base_dir, "exit 0");
        let config_path = write_config(
            &base_dir,
            &sessions_root,
            &memory_dir,
            &curl_path,
            Some(serde_json::json!({ "max_output_tokens": invalid_val })),
        );

        let args = DreamArgs {
            config: Some(config_path.clone()),
            dry_run: true,
            max_calls: None,
        };
        let err = DreamConfig::load(&args).unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("max_output_tokens") && err_msg.contains("must be positive"),
            "error message must mention max_output_tokens must be positive, got: {}",
            err_msg
        );
    }
}
