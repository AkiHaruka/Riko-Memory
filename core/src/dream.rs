// src/dream.rs — 后台记忆整理子命令 dream 实现

use chrono::{DateTime, FixedOffset, NaiveDateTime, Utc};
use clap::Args;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const SYSTEM_PROMPT: &str = r#"你是璃的记忆整理助手，只做一件事：从对话里挑出值得存进第一人称叙事记忆的候选，写成草稿放进收集箱。你不写正式记忆，不做人物画像，不给建议。

规则：
1. 只提取对话里真实出现的事实、原话、情绪、承诺、生活切片。
2. 每条必须能在原文里找到依据；quote 必须是原文片段（可截断，不能改写）。
3. 禁止：性格归纳、心理分析、建议、总结对方是什么样的人、推测动机。
4. 禁止：寒暄、玩笑、临时事务（改配置、查命令）这类不需要长期记的内容。
5. 时间用对话里给出的具体时间；不确定就不写这条。
6. 宁少不多：没有值得记的就返回空数组。
7. 只输出 JSON，不要解释、不要 markdown 代码块。

输出格式：
{"candidates":[{"kind":"moment","when":"2026-10-31T19:49:00+08:00","text":"第一人称草稿，璃的视角，具体，1-3 句","quote":"原文片段","tags":["决定","记忆系统"],"reason":"为什么值得记，一句话"}]}

kind 只能是：
- moment：一个时刻/对话片段（谁说了什么、当时情境）
- people：关于某个人稳定下来的具体事实（偏好、习惯、经历），只写事实不写判断
- self：璃自己的决定、判断、想法
- open-loop：没做完/以后要跟进的事"#;

#[derive(Args, Debug, Clone)]
pub struct DreamArgs {
    /// 配置文件路径；优先级：本参数 > 环境变量 RIKO_DREAM_CONFIG > /root/riko/var/dream.json
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// 只构建并打印将要发送的 prompt（含 system/user 全文），不联网、不拿锁、不写 state、不写 inbox、不要求 API key
    #[arg(long)]
    pub dry_run: bool,

    /// 本次最多处理几个 chunk，覆盖配置
    #[arg(long)]
    pub max_calls: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct DreamConfig {
    pub sessions_root: PathBuf,
    pub memory_dir: PathBuf,
    pub state_path: PathBuf,
    pub log_path: PathBuf,
    pub lock_path: PathBuf,
    pub endpoint: String,
    pub model: String,
    pub api_key_env: String,
    pub max_chars_per_call: usize,
    pub max_calls_per_run: usize,
    pub max_candidates_per_call: usize,
    pub window_days: u64,
    pub timeout_seconds: u64,
    pub skip_recent_seconds: u64,
    pub stale_lock_seconds: u64,
    pub curl_path: String,
    pub reasoning_effort: String,
    pub max_output_tokens: u64,
}

fn parse_positive_i64(v: &serde_json::Value, field_name: &str, default: i64) -> anyhow::Result<i64> {
    if let Some(val) = v.get(field_name) {
        if val.is_null() {
            return Ok(default);
        }
        let n = val.as_i64().ok_or_else(|| {
            anyhow::anyhow!("invalid config: field '{}' must be an integer", field_name)
        })?;
        if n <= 0 {
            anyhow::bail!("invalid config: field '{}' must be positive, got {}", field_name, n);
        }
        Ok(n)
    } else {
        Ok(default)
    }
}

impl DreamConfig {
    pub fn load(args: &DreamArgs) -> anyhow::Result<Self> {
        let config_path = if let Some(ref p) = args.config {
            p.clone()
        } else if let Ok(env_path) = std::env::var("RIKO_DREAM_CONFIG") {
            if !env_path.trim().is_empty() {
                PathBuf::from(env_path)
            } else {
                PathBuf::from("/root/riko/var/dream.json")
            }
        } else {
            PathBuf::from("/root/riko/var/dream.json")
        };

        if !config_path.exists() {
            anyhow::bail!("config file not found: {}", config_path.display());
        }

        let content = fs::read_to_string(&config_path)
            .map_err(|e| anyhow::anyhow!("failed to read config file {}: {}", config_path.display(), e))?;
        let v: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("failed to parse config JSON {}: {}", config_path.display(), e))?;

        let sessions_root = v.get("sessions_root")
            .and_then(|s| s.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/root/.dsh/sessions/--root-riko--"));

        let memory_dir = if let Some(m) = v.get("memory_dir").and_then(|s| s.as_str()) {
            if !m.trim().is_empty() {
                PathBuf::from(m)
            } else if let Ok(env_m) = std::env::var("RIKO_MEM_DIR") {
                if !env_m.trim().is_empty() {
                    PathBuf::from(env_m)
                } else {
                    PathBuf::from("/root/riko/memory")
                }
            } else {
                PathBuf::from("/root/riko/memory")
            }
        } else if let Ok(env_m) = std::env::var("RIKO_MEM_DIR") {
            if !env_m.trim().is_empty() {
                PathBuf::from(env_m)
            } else {
                PathBuf::from("/root/riko/memory")
            }
        } else {
            PathBuf::from("/root/riko/memory")
        };

        let state_path = if let Ok(env_s) = std::env::var("RIKO_DREAM_STATE") {
            if !env_s.trim().is_empty() {
                PathBuf::from(env_s)
            } else {
                v.get("state_path")
                    .and_then(|s| s.as_str())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/root/riko/var/dream.state.json"))
            }
        } else {
            v.get("state_path")
                .and_then(|s| s.as_str())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/root/riko/var/dream.state.json"))
        };

        let log_path = if let Ok(env_l) = std::env::var("RIKO_DREAM_LOG") {
            if !env_l.trim().is_empty() {
                PathBuf::from(env_l)
            } else {
                v.get("log_path")
                    .and_then(|s| s.as_str())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/root/riko/var/dream.log"))
            }
        } else {
            v.get("log_path")
                .and_then(|s| s.as_str())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/root/riko/var/dream.log"))
        };

        let lock_path = if let Ok(env_k) = std::env::var("RIKO_DREAM_LOCK") {
            if !env_k.trim().is_empty() {
                PathBuf::from(env_k)
            } else {
                v.get("lock_path")
                    .and_then(|s| s.as_str())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("/root/riko/var/dream.lock"))
            }
        } else {
            v.get("lock_path")
                .and_then(|s| s.as_str())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/root/riko/var/dream.lock"))
        };

        let endpoint = v.get("endpoint")
            .and_then(|s| s.as_str())
            .unwrap_or("https://opencode.ai/inference/go/openai/v1/responses")
            .to_string();

        let model = v.get("model")
            .and_then(|s| s.as_str())
            .unwrap_or("muse-spark-1.3-contributor")
            .to_string();

        let api_key_env = v.get("api_key_env")
            .and_then(|s| s.as_str())
            .unwrap_or("RIKO_DREAM_API_KEY")
            .to_string();

        let max_chars_per_call = parse_positive_i64(&v, "max_chars_per_call", 12000)? as usize;

        let mut max_calls_per_run = parse_positive_i64(&v, "max_calls_per_run", 5)? as usize;

        if let Some(m) = args.max_calls {
            if m == 0 {
                anyhow::bail!("invalid config: field 'max_calls' must be positive, got 0");
            }
            max_calls_per_run = m;
        }

        let max_candidates_per_call = parse_positive_i64(&v, "max_candidates_per_call", 6)? as usize;

        let window_days_i64 = parse_positive_i64(&v, "window_days", 2)?;
        if !(1..=30).contains(&window_days_i64) {
            anyhow::bail!("invalid config: window_days must be in range 1..=30, got {}", window_days_i64);
        }
        let window_days = window_days_i64 as u64;

        let timeout_seconds = parse_positive_i64(&v, "timeout_seconds", 150)? as u64;

        let skip_recent_seconds = parse_positive_i64(&v, "skip_recent_seconds", 60)? as u64;

        let stale_lock_seconds = parse_positive_i64(&v, "stale_lock_seconds", 3600)? as u64;

        let max_output_tokens = parse_positive_i64(&v, "max_output_tokens", 8192)? as u64;

        let curl_path = v.get("curl_path")
            .and_then(|s| s.as_str())
            .unwrap_or("/usr/bin/curl")
            .to_string();

        let reasoning_effort = v.get("reasoning_effort")
            .and_then(|s| s.as_str())
            .unwrap_or("minimal")
            .to_string();

        // 检查 API key
        if !args.dry_run {
            let key = std::env::var(&api_key_env).unwrap_or_default();
            if key.trim().is_empty() {
                anyhow::bail!("api key environment variable '{}' is not set or empty", api_key_env);
            }
        }

        Ok(Self {
            sessions_root,
            memory_dir,
            state_path,
            log_path,
            lock_path,
            endpoint,
            model,
            api_key_env,
            max_chars_per_call,
            max_calls_per_run,
            max_candidates_per_call,
            window_days,
            timeout_seconds,
            skip_recent_seconds,
            stale_lock_seconds,
            curl_path,
            reasoning_effort,
            max_output_tokens,
        })
    }
}

pub struct DreamLogger {
    path: PathBuf,
    offset: FixedOffset,
}

impl DreamLogger {
    pub fn new(path: PathBuf, offset: FixedOffset) -> Self {
        Self { path, offset }
    }

    pub fn log(&self, msg: &str) {
        let now: DateTime<FixedOffset> = Utc::now().with_timezone(&self.offset);
        let time_str = now.format("%Y-%m-%dT%H:%M:%S%:z");
        let line = format!("{} [dream] {}\n", time_str, msg);
        if let Some(p) = self.path.parent() {
            let _ = fs::create_dir_all(p);
        }
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

pub struct LockGuard {
    lock_path: PathBuf,
}

impl LockGuard {
    pub fn acquire(
        lock_path: &Path,
        stale_lock_seconds: u64,
        offset: FixedOffset,
    ) -> anyhow::Result<Option<Self>> {
        if let Some(p) = lock_path.parent() {
            fs::create_dir_all(p)?;
        }

        let write_lock_file = |file: &mut File| -> anyhow::Result<()> {
            let now: DateTime<FixedOffset> = Utc::now().with_timezone(&offset);
            let content = format!("pid={} time={}\n", std::process::id(), now.to_rfc3339());
            file.write_all(content.as_bytes())?;
            Ok(())
        };

        match OpenOptions::new().write(true).create_new(true).open(lock_path) {
            Ok(mut f) => {
                write_lock_file(&mut f)?;
                Ok(Some(LockGuard {
                    lock_path: lock_path.to_path_buf(),
                }))
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // 检查 mtime 是否超时
                let metadata = fs::metadata(lock_path);
                let is_stale = if let Ok(meta) = metadata {
                    if let Ok(mtime) = meta.modified() {
                        if let Ok(elapsed) = SystemTime::now().duration_since(mtime) {
                            elapsed > Duration::from_secs(stale_lock_seconds)
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                if is_stale {
                    let _ = fs::remove_file(lock_path);
                    match OpenOptions::new().write(true).create_new(true).open(lock_path) {
                        Ok(mut f) => {
                            write_lock_file(&mut f)?;
                            return Ok(Some(LockGuard {
                                lock_path: lock_path.to_path_buf(),
                            }));
                        }
                        Err(_) => {}
                    }
                }
                Ok(None)
            }
            Err(e) => Err(anyhow::anyhow!("failed to create lock file {}: {}", lock_path.display(), e)),
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock_path);
    }
}

#[derive(Debug, Clone)]
pub struct DreamState {
    pub version: u64,
    pub last_run: Option<String>,
    pub sessions: BTreeMap<String, u64>,
    path: PathBuf,
}

impl DreamState {
    pub fn load(path: &Path, logger: &DreamLogger) -> Self {
        if !path.exists() {
            return Self {
                version: 1,
                last_run: None,
                sessions: BTreeMap::new(),
                path: path.to_path_buf(),
            };
        }

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => {
                return Self {
                    version: 1,
                    last_run: None,
                    sessions: BTreeMap::new(),
                    path: path.to_path_buf(),
                };
            }
        };

        match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(v) => {
                let version = v.get("version").and_then(|n| n.as_u64()).unwrap_or(1);
                let last_run = v.get("last_run").and_then(|s| s.as_str()).map(|s| s.to_string());
                let mut sessions = BTreeMap::new();
                if let Some(sess_obj) = v.get("sessions").and_then(|s| s.as_object()) {
                    for (k, val) in sess_obj {
                        if let Some(seq) = val.as_u64() {
                            sessions.insert(k.clone(), seq);
                        }
                    }
                }
                Self {
                    version,
                    last_run,
                    sessions,
                    path: path.to_path_buf(),
                }
            }
            Err(_) => {
                let ts = Utc::now().timestamp();
                let broken_name = format!("{}.broken-{}", path.display(), ts);
                let _ = fs::rename(path, &broken_name);
                logger.log(&format!("state corrupted, renamed to {}", broken_name));
                Self {
                    version: 1,
                    last_run: None,
                    sessions: BTreeMap::new(),
                    path: path.to_path_buf(),
                }
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let v = serde_json::json!({
            "version": self.version,
            "last_run": self.last_run,
            "sessions": self.sessions,
        });
        let s = serde_json::to_string(&v)?;
        if let Some(p) = self.path.parent() {
            fs::create_dir_all(p)?;
        }
        let tmp_path = self.path.with_extension("tmp");
        fs::write(&tmp_path, s.as_bytes())?;
        fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub seq: u64,
    pub role: Role,
    pub time: DateTime<FixedOffset>,
    pub text: String,
}

pub fn format_message_transcript(msg: &Message) -> String {
    let role_name = match msg.role {
        Role::User => "爹地",
        Role::Assistant => "璃",
    };
    let time_str = msg.time.format("%Y-%m-%d %H:%M +08:00").to_string();
    let mut lines = msg.text.lines();
    let first = lines.next().unwrap_or("");
    let mut res = format!("[{}] {}: {}", time_str, role_name, first);
    for line in lines {
        res.push('\n');
        res.push_str("  ");
        res.push_str(line);
    }
    res
}

pub fn decode_session_file(path: &Path) -> anyhow::Result<(String, usize, bool)> {
    let bytes = fs::read(path)?;
    let mut dec = zstd::stream::read::Decoder::new(&bytes[..])?;
    let mut out = String::new();
    let mut trailing_partial = false;
    match dec.read_to_string(&mut out) {
        Ok(_) => {}
        Err(_) => {
            trailing_partial = true;
            if let Some(pos) = out.rfind('\n') {
                out.truncate(pos + 1);
            } else {
                out.clear();
            }
        }
    }
    let lines_count = out.lines().count();
    Ok((out, lines_count, trailing_partial))
}

pub fn extract_messages(
    content: &str,
    last_seq: u64,
    offset: FixedOffset,
) -> (Vec<Message>, usize, usize) {
    let mut messages = Vec::new();
    let mut total_events = 0;
    let mut skipped_lines = 0;

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        total_events += 1;
        let obj: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                skipped_lines += 1;
                continue;
            }
        };

        let ev_type = match obj.get("type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => continue,
        };

        let seq = match obj.get("seq").and_then(|v| v.as_u64()) {
            Some(s) => s,
            None => continue,
        };

        let time_ms = match obj.get("time").and_then(|v| v.as_i64()) {
            Some(t) => t,
            None => continue,
        };

        let time: DateTime<FixedOffset> = match DateTime::from_timestamp_millis(time_ms) {
            Some(dt) => dt.with_timezone(&offset),
            None => continue,
        };

        match ev_type {
            "user/message" => {
                // A. user/message
                let kind = obj.get("data")
                    .and_then(|d| d.get("source"))
                    .and_then(|s| s.get("kind"))
                    .and_then(|k| k.as_str());
                if kind != Some("user") {
                    continue;
                }

                let mut text_parts = Vec::new();
                if let Some(content_arr) = obj.get("data").and_then(|d| d.get("content")).and_then(|c| c.as_array()) {
                    for item in content_arr {
                        if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                            if let Some(t) = item.get("text").and_then(|txt| txt.as_str()) {
                                text_parts.push(t);
                            }
                        }
                    }
                }
                if text_parts.is_empty() {
                    continue;
                }
                let combined_text = text_parts.join("");
                if combined_text.trim().starts_with("[自动探活]") {
                    continue;
                }

                if seq > last_seq {
                    messages.push(Message {
                        seq,
                        role: Role::User,
                        time,
                        text: combined_text,
                    });
                }
            }
            "assistant/message" => {
                // B. assistant/message
                let mut text_parts = Vec::new();
                if let Some(content_arr) = obj.get("data")
                    .and_then(|d| d.get("message"))
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for item in content_arr {
                        if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                            if let Some(t) = item.get("text").and_then(|txt| txt.as_str()) {
                                text_parts.push(t);
                            }
                        }
                    }
                }
                if text_parts.is_empty() {
                    continue;
                }
                let combined_text = text_parts.join("");

                if seq > last_seq {
                    messages.push(Message {
                        seq,
                        role: Role::Assistant,
                        time,
                        text: combined_text,
                    });
                }
            }
            _ => {}
        }
    }

    messages.sort_by_key(|m| m.seq);
    (messages, total_events, skipped_lines)
}

pub fn chunk_messages(
    messages: Vec<Message>,
    max_chars_per_call: usize,
    max_calls_per_run: usize,
) -> Vec<Vec<Message>> {
    let mut chunks: Vec<Vec<Message>> = Vec::new();
    let mut current_chunk: Vec<Message> = Vec::new();
    let mut current_chars: usize = 0;

    for mut msg in messages {
        // 单条消息本身超过上限时截断到上限并追加 …
        if msg.text.chars().count() > max_chars_per_call {
            let truncated: String = msg.text.chars().take(max_chars_per_call).collect();
            msg.text = format!("{}…", truncated);
        }

        let msg_rendered = format_message_transcript(&msg);
        let msg_chars = msg_rendered.chars().count();

        if !current_chunk.is_empty() && current_chars + msg_chars > max_chars_per_call {
            chunks.push(std::mem::take(&mut current_chunk));
            current_chars = 0;
        }

        current_chars += msg_chars;
        current_chunk.push(msg);
    }

    if !current_chunk.is_empty() {
        chunks.push(current_chunk);
    }

    if chunks.len() > max_calls_per_run {
        chunks.truncate(max_calls_per_run);
    }

    chunks
}

pub fn build_transcript(chunk: &[Message]) -> String {
    let mut parts = Vec::new();
    for msg in chunk {
        parts.push(format_message_transcript(msg));
    }
    parts.join("\n")
}

pub fn build_user_prompt(transcript: &str, is_retry: bool) -> String {
    let mut prompt = format!(
        "下面是一段对话记录（SGT 时间，可能跨天）：\n\n{}\n\n按规则输出候选 JSON。",
        transcript
    );
    if is_retry {
        prompt.push_str("\n注意：上次输出无法解析。这次只输出一个 JSON 对象，不要代码块标记，不要任何前后文字。");
    }
    prompt
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub kind: String,
    pub when: String,
    pub tags: Vec<String>,
    pub quote: String,
    pub why: String,
    pub text: String,
}

fn parse_when(s: &str) -> bool {
    DateTime::parse_from_rfc3339(s).is_ok()
        || NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").is_ok()
}

pub fn validate_candidate(val: &serde_json::Value) -> Option<Candidate> {
    let kind = val.get("kind").and_then(|v| v.as_str())?;
    if !["moment", "people", "self", "open-loop"].contains(&kind) {
        return None;
    }

    let when = val.get("when").and_then(|v| v.as_str())?;
    if !parse_when(when) {
        return None;
    }

    let text = val.get("text").and_then(|v| v.as_str())?;
    let text_trim = text.trim();
    let text_chars = text_trim.chars().count();
    if text_chars == 0 || text_chars > 500 {
        return None;
    }

    let quote = val.get("quote").and_then(|v| v.as_str())?;
    let quote_trim = quote.trim();
    let quote_chars = quote_trim.chars().count();
    if quote_chars == 0 || quote_chars > 300 {
        return None;
    }

    let reason = val.get("reason").and_then(|v| v.as_str())?;
    let reason_trim = reason.trim();
    let reason_chars = reason_trim.chars().count();
    if reason_chars == 0 || reason_chars > 200 {
        return None;
    }

    let tags_val = val.get("tags").and_then(|v| v.as_array())?;
    if tags_val.len() > 6 {
        return None;
    }
    let mut tags = Vec::new();
    for t_item in tags_val {
        let t_str = t_item.as_str()?;
        let t_trim = t_str.trim();
        let t_chars = t_trim.chars().count();
        if t_chars == 0 || t_chars > 20 {
            return None;
        }
        tags.push(t_trim.to_string());
    }

    // quote 单行化（内部换行替换为空格）
    let single_line_quote = quote_trim
        .replace("\r\n", " ")
        .replace('\n', " ")
        .replace('\r', " ");

    Some(Candidate {
        kind: kind.to_string(),
        when: when.to_string(),
        tags,
        quote: single_line_quote,
        why: reason_trim.to_string(),
        text: text_trim.to_string(),
    })
}

pub fn extract_candidates_from_text(
    raw_text: &str,
    max_candidates: usize,
) -> Result<(Vec<Candidate>, usize), String> {
    let mut text = raw_text.trim();
    // 去除 markdown 代码块围栏
    if text.starts_with("```") {
        if let Some(first_nl) = text.find('\n') {
            text = &text[first_nl + 1..];
        } else {
            text = "";
        }
        if let Some(last_fence) = text.rfind("```") {
            text = &text[..last_fence];
        }
    }
    text = text.trim();

    let start = text.find('{').ok_or_else(|| "no JSON object start found".to_string())?;
    let end = text.rfind('}').ok_or_else(|| "no JSON object end found".to_string())?;
    if end < start {
        return Err("invalid JSON range".to_string());
    }
    let json_str = &text[start..=end];

    let root: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("failed to parse candidates JSON: {}", e))?;

    let candidates_arr = root.get("candidates")
        .and_then(|c| c.as_array())
        .ok_or_else(|| "missing candidates array in JSON".to_string())?;

    let mut valid = Vec::new();
    let mut dropped = 0;

    for item in candidates_arr {
        if let Some(cand) = validate_candidate(item) {
            valid.push(cand);
        } else {
            dropped += 1;
        }
    }

    if valid.len() > max_candidates {
        let extra = valid.len() - max_candidates;
        valid.truncate(max_candidates);
        dropped += extra;
    }

    Ok((valid, dropped))
}

pub struct CallLlmResult {
    pub candidates: Vec<Candidate>,
    pub dropped: usize,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

struct TempFileGuard(PathBuf);
impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn run_curl_and_parse(
    config: &DreamConfig,
    body_path: &Path,
    conf_path: &Path,
    logger: &DreamLogger,
) -> anyhow::Result<(String, u64, u64)> {
    let output = std::process::Command::new(&config.curl_path)
        .arg("-sS")
        .arg("--max-time")
        .arg(config.timeout_seconds.to_string())
        .arg("-X")
        .arg("POST")
        .arg(&config.endpoint)
        .arg("--config")
        .arg(conf_path)
        .arg("--data-binary")
        .arg(format!("@{}", body_path.display()))
        .output()
        .map_err(|e| anyhow::anyhow!("failed to execute curl: {}", e))?;

    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("curl failed with status {}: {}", output.status, err_msg.trim());
    }

    if output.stdout.is_empty() {
        anyhow::bail!("curl returned empty stdout");
    }

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let resp: serde_json::Value = serde_json::from_str(&stdout_str)
        .map_err(|e| anyhow::anyhow!("failed to parse curl response JSON: {}", e))?;

    if resp.get("status").and_then(|s| s.as_str()) == Some("incomplete") {
        logger.log("warning llm response status is incomplete");
    }

    let tokens_in = resp.get("usage")
        .and_then(|u| u.get("input_tokens"))
        .and_then(|n| n.as_u64())
        .unwrap_or(0);
    let tokens_out = resp.get("usage")
        .and_then(|u| u.get("output_tokens"))
        .and_then(|n| n.as_u64())
        .unwrap_or(0);

    // 解析 Responses API: 取 output 数组中 type=="message" 的 item 的 content 中 type 为 text/output_text 的 text
    let mut collected_text = String::new();
    if let Some(out_arr) = resp.get("output").and_then(|o| o.as_array()) {
        for item in out_arr {
            if item.get("type").and_then(|t| t.as_str()) == Some("message") {
                if let Some(content_arr) = item.get("content").and_then(|c| c.as_array()) {
                    for c in content_arr {
                        let c_type = c.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        if c_type == "text" || c_type == "output_text" {
                            if let Some(txt) = c.get("text").and_then(|t| t.as_str()) {
                                collected_text.push_str(txt);
                            }
                        }
                    }
                }
            }
        }
    }

    if collected_text.trim().is_empty() {
        anyhow::bail!("no text found in response output");
    }

    Ok((collected_text, tokens_in, tokens_out))
}

pub fn execute_curl(
    config: &DreamConfig,
    user_prompt: &str,
    logger: &DreamLogger,
) -> anyhow::Result<(String, u64, u64)> {
    let api_key = std::env::var(&config.api_key_env).unwrap_or_default();
    let temp_dir = std::env::temp_dir();
    let id = format!("{}_{}", std::process::id(), Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let body_path = temp_dir.join(format!("riko_dream_body_{}.json", id));
    let conf_path = temp_dir.join(format!("riko_dream_curl_{}.conf", id));

    let _guard_body = TempFileGuard(body_path.clone());
    let _guard_conf = TempFileGuard(conf_path.clone());

    let body_json = serde_json::json!({
        "model": &config.model,
        "instructions": SYSTEM_PROMPT,
        "input": [
            {"role": "user", "content": user_prompt}
        ],
        "max_output_tokens": config.max_output_tokens,
        "temperature": 0.1,
        "reasoning": {
            "effort": &config.reasoning_effort
        }
    });

    let body_bytes = serde_json::to_vec(&body_json)?;
    fs::write(&body_path, body_bytes)?;

    let conf_content = format!(
        "header = \"Content-Type: application/json\"\nheader = \"Authorization: Bearer {}\"\nheader = \"x-opencode-session: riko-dream\"\n",
        api_key
    );

    let mut open_opts = OpenOptions::new();
    open_opts.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open_opts.mode(0o600);
    }
    let mut conf_file = open_opts.open(&conf_path)?;
    conf_file.write_all(conf_content.as_bytes())?;
    drop(conf_file);

    match run_curl_and_parse(config, &body_path, &conf_path, logger) {
        Ok(res) => Ok(res),
        Err(err1) => {
            logger.log(&format!("warning curl failed, retrying in 3s: {}", err1));
            std::thread::sleep(Duration::from_secs(3));
            run_curl_and_parse(config, &body_path, &conf_path, logger)
                .map_err(|err2| anyhow::anyhow!("curl failed after retry: {}", err2))
        }
    }
}

pub fn call_llm(
    config: &DreamConfig,
    transcript: &str,
    logger: &DreamLogger,
    session_name: &str,
    seq_range: (u64, u64),
) -> anyhow::Result<CallLlmResult> {
    // 首次尝试
    let prompt1 = build_user_prompt(transcript, false);
    let (text1, tokens_in1, tokens_out1) = match execute_curl(config, &prompt1, logger) {
        Ok(res) => res,
        Err(e) => {
            logger.log(&format!("error chunk session={} seq={}-{} reason=llm: {}", session_name, seq_range.0, seq_range.1, e));
            return Err(e);
        }
    };

    match extract_candidates_from_text(&text1, config.max_candidates_per_call) {
        Ok((candidates, dropped)) => {
            return Ok(CallLlmResult {
                candidates,
                dropped,
                tokens_in: tokens_in1,
                tokens_out: tokens_out1,
            });
        }
        Err(e1) => {
            logger.log(&format!("warning chunk session={} parse failed: {}, retrying with reminder", session_name, e1));
        }
    }

    // 第二次尝试（重试，带有格式提示）
    let prompt2 = build_user_prompt(transcript, true);
    let (text2, tokens_in2, tokens_out2) = match execute_curl(config, &prompt2, logger) {
        Ok(res) => res,
        Err(e) => {
            logger.log(&format!("error chunk session={} seq={}-{} reason=llm retry: {}", session_name, seq_range.0, seq_range.1, e));
            return Err(e);
        }
    };

    match extract_candidates_from_text(&text2, config.max_candidates_per_call) {
        Ok((candidates, dropped)) => Ok(CallLlmResult {
            candidates,
            dropped,
            tokens_in: tokens_in2,
            tokens_out: tokens_out2,
        }),
        Err(e2) => {
            let err_msg = format!("llm json parse failed after retry: {}", e2);
            logger.log(&format!("error chunk session={} seq={}-{} reason={}", session_name, seq_range.0, seq_range.1, err_msg));
            anyhow::bail!("{}", err_msg);
        }
    }
}

pub fn write_inbox(
    memory_dir: &Path,
    run_time: &DateTime<FixedOffset>,
    session_name: &str,
    seq_range: (u64, u64),
    candidates: &[Candidate],
) -> anyhow::Result<PathBuf> {
    if candidates.is_empty() {
        anyhow::bail!("no candidates to write");
    }

    let inbox_dir = memory_dir.join("inbox");
    fs::create_dir_all(&inbox_dir)?;

    let date_str = run_time.format("%Y-%m-%d").to_string();
    let file_path = inbox_dir.join(format!("{}.md", date_str));

    let file_exists = file_path.exists();
    let mut content_to_append = String::new();

    if !file_exists {
        content_to_append.push_str(&format!(
            "# 候选箱 · {}\n\ndream 自动生成，璃审过才算数。审完把文件挪走或删掉。\n",
            date_str
        ));
    } else {
        content_to_append.push('\n');
    }

    let run_time_str = run_time.format("%Y-%m-%d %H:%M:%S +08:00").to_string();
    content_to_append.push_str(&format!(
        "## run {} · {} · seq {}-{}\n",
        run_time_str, session_name, seq_range.0, seq_range.1
    ));

    for (i, cand) in candidates.iter().enumerate() {
        if i > 0 {
            content_to_append.push('\n');
        }
        content_to_append.push_str(&format!("### {}\n", i + 1));
        content_to_append.push_str(&format!("kind: {}\n", cand.kind));
        content_to_append.push_str(&format!("when: {}\n", cand.when));
        content_to_append.push_str(&format!("tags: {}\n", cand.tags.join(", ")));
        content_to_append.push_str(&format!("quote: {}\n", cand.quote));
        content_to_append.push_str(&format!("why: {}\n", cand.why));
        content_to_append.push_str("text: |\n");
        for line in cand.text.lines() {
            content_to_append.push_str("  ");
            content_to_append.push_str(line);
            content_to_append.push('\n');
        }
    }

    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)?;
    f.write_all(content_to_append.as_bytes())?;

    Ok(file_path)
}

pub fn run_dream(args: DreamArgs) -> anyhow::Result<i32> {
    let offset = FixedOffset::east_opt(8 * 3600).unwrap();
    let run_time = Utc::now().with_timezone(&offset);

    let config = match DreamConfig::load(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {}", e);
            return Ok(1);
        }
    };

    let logger = DreamLogger::new(config.log_path.clone(), offset);

    // 拿锁
    let _lock = if !args.dry_run {
        match LockGuard::acquire(&config.lock_path, config.stale_lock_seconds, offset) {
            Ok(Some(guard)) => Some(guard),
            Ok(None) => {
                eprintln!("another dream is running");
                return Ok(0);
            }
            Err(e) => {
                eprintln!("error: {}", e);
                return Ok(1);
            }
        }
    } else {
        None
    };

    if !args.dry_run {
        logger.log(&format!("run start pid={}", std::process::id()));
    }

    let mut state = DreamState::load(&config.state_path, &logger);

    // 扫描 sessions_root
    if !config.sessions_root.exists() {
        if !args.dry_run {
            logger.log(&format!("run end sessions=0 chunks=0 candidates=0 inbox=-"));
        }
        println!("dream: sessions=0 chunks=0 candidates=0 inbox=-");
        return Ok(0);
    }

    let mut session_dirs = Vec::new();
    if let Ok(entries) = fs::read_dir(&config.sessions_root) {
        let now = SystemTime::now();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let session_file = path.join("session.v4.jsonl.zstd");
            if !session_file.is_file() {
                continue;
            }

            let session_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();

            let metadata = match fs::metadata(&session_file) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let mtime = match metadata.modified() {
                Ok(t) => t,
                Err(_) => continue,
            };

            // 检查 window_days
            let window_duration = Duration::from_secs(config.window_days * 86400);
            if let Ok(elapsed) = now.duration_since(mtime) {
                if elapsed > window_duration {
                    logger.log(&format!("skip session={} reason=window_expired", session_name));
                    continue;
                }
                if elapsed < Duration::from_secs(config.skip_recent_seconds) {
                    logger.log(&format!("skip session={} reason=too_recent", session_name));
                    continue;
                }
            }

            session_dirs.push((mtime, path, session_name));
        }
    }

    session_dirs.sort_by_key(|(mtime, _, _)| *mtime);

    let mut total_sessions_processed = 0;
    let mut total_chunks_processed = 0;
    let mut total_candidates_found = 0;
    let mut last_written_inbox: Option<PathBuf> = None;
    let mut remaining_calls = config.max_calls_per_run;
    let mut all_prompts_to_print: Vec<(String, String)> = Vec::new();

    for (_mtime, session_dir, session_name) in session_dirs {
        if remaining_calls == 0 {
            break;
        }

        let session_file = session_dir.join("session.v4.jsonl.zstd");
        let (content, _line_count, trailing_partial) = match decode_session_file(&session_file) {
            Ok(c) => c,
            Err(_) => {
                logger.log(&format!("skip session={} reason=decode_error", session_name));
                continue;
            }
        };

        if content.is_empty() {
            logger.log(&format!("skip session={} reason=no_complete_lines", session_name));
            continue;
        }

        let last_seq = state.sessions.get(&session_name).copied().unwrap_or(0);
        let (messages, events_count, skipped_lines) = extract_messages(&content, last_seq, offset);

        logger.log(&format!(
            "decode session={} events={} skipped_lines={} trailing_partial={}",
            session_name, events_count, skipped_lines, trailing_partial
        ));

        if messages.is_empty() {
            continue;
        }

        let chunks = chunk_messages(messages, config.max_chars_per_call, remaining_calls);
        if chunks.is_empty() {
            continue;
        }

        total_sessions_processed += 1;

        for chunk in chunks {
            if remaining_calls == 0 {
                break;
            }

            let start_seq = chunk.first().map(|m| m.seq).unwrap_or(0);
            let end_seq = chunk.last().map(|m| m.seq).unwrap_or(0);
            let transcript = build_transcript(&chunk);

            if args.dry_run {
                let user_prompt = build_user_prompt(&transcript, false);
                all_prompts_to_print.push((SYSTEM_PROMPT.to_string(), user_prompt));
                remaining_calls -= 1;
                total_chunks_processed += 1;
                continue;
            }

            match call_llm(&config, &transcript, &logger, &session_name, (start_seq, end_seq)) {
                Ok(res) => {
                    total_chunks_processed += 1;
                    remaining_calls -= 1;

                    logger.log(&format!(
                        "chunk session={} seq={}-{} chars={} tokens_in={} tokens_out={} candidates={} dropped={}",
                        session_name,
                        start_seq,
                        end_seq,
                        transcript.chars().count(),
                        res.tokens_in,
                        res.tokens_out,
                        res.candidates.len(),
                        res.dropped
                    ));

                    if !res.candidates.is_empty() {
                        total_candidates_found += res.candidates.len();
                        match write_inbox(&config.memory_dir, &run_time, &session_name, (start_seq, end_seq), &res.candidates) {
                            Ok(p) => {
                                last_written_inbox = Some(p);
                            }
                            Err(e) => {
                                eprintln!("error: {}", e);
                                return Ok(1);
                            }
                        }
                    }

                    // 立即更新 state
                    state.sessions.insert(session_name.clone(), end_seq);
                    if let Err(e) = state.save() {
                        eprintln!("error saving state: {}", e);
                        return Ok(1);
                    }
                }
                Err(e) => {
                    eprintln!("error: {}", e);
                    return Ok(1);
                }
            }
        }
    }

    if args.dry_run {
        if all_prompts_to_print.is_empty() {
            println!("dream: no new messages");
        } else {
            for (sys, usr) in all_prompts_to_print {
                println!("[system]\n{}\n\n[user]\n{}", sys, usr);
            }
        }
        return Ok(0);
    }

    if total_chunks_processed == 0 {
        logger.log("no new messages");
    }

    state.last_run = Some(run_time.to_rfc3339());
    let _ = state.save();

    let inbox_repr = last_written_inbox
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "-".to_string());

    logger.log(&format!(
        "run end sessions={} chunks={} candidates={} inbox={}",
        total_sessions_processed, total_chunks_processed, total_candidates_found, inbox_repr
    ));

    println!(
        "dream: sessions={} chunks={} candidates={} inbox={}",
        total_sessions_processed, total_chunks_processed, total_candidates_found, inbox_repr
    );

    Ok(0)
}

pub fn cmd_dream(args: DreamArgs) {
    match run_dream(args) {
        Ok(code) => {
            std::process::exit(code);
        }
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}
