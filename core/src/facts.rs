// src/facts.rs — facts 子命令实现：会话事实捕获进 EverOS

use chrono::{DateTime, FixedOffset, Utc};
use clap::Args;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime};

#[derive(Args, Debug, Clone)]
pub struct FactsArgs {
    /// 配置文件路径
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// 演练模式：不发送 HTTP 请求，不拿锁，不写 state，不写日志；stdout 打印计划
    #[arg(long)]
    pub dry_run: bool,

    /// 本次最多 HTTP 请求数（add+flush 合计），覆盖配置；0 表示不限
    #[arg(long)]
    pub max_calls: Option<usize>,

    /// 回填模式：忽略空闲条件，每个有新消息的会话发完即 flush
    #[arg(long)]
    pub backfill: bool,
}

#[derive(Debug, Clone)]
pub struct FactsConfig {
    pub sessions_root: PathBuf,
    pub state_path: PathBuf,
    pub log_path: PathBuf,
    pub lock_path: PathBuf,
    pub endpoint: String,
    pub app_id: String,
    pub project_id: String,
    pub user_sender_id: String,
    pub user_sender_name: String,
    pub skip_recent_seconds: u64,
    pub idle_flush_seconds: u64,
    pub max_messages_per_call: usize,
    pub max_chars_per_call: usize,
    pub max_calls_per_run: usize,
    pub max_flushes_per_run: usize,
    pub timeout_seconds: u64,
    pub stale_lock_seconds: u64,
    pub curl_path: String,
}

#[derive(Debug)]
pub enum FactsError {
    Config(String),
    Runtime(String),
}

impl std::fmt::Display for FactsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FactsError::Config(msg) => write!(f, "config error: {}", msg),
            FactsError::Runtime(msg) => write!(f, "runtime error: {}", msg),
        }
    }
}

impl std::error::Error for FactsError {}

fn parse_str_field(v: &serde_json::Value, field: &str, default: &str) -> Result<String, String> {
    if let Some(val) = v.get(field) {
        if val.is_null() {
            return Ok(default.to_string());
        }
        val.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| format!("field '{}' must be a string", field))
    } else {
        Ok(default.to_string())
    }
}

fn parse_non_negative_u64(v: &serde_json::Value, field: &str, default: u64) -> Result<u64, String> {
    if let Some(val) = v.get(field) {
        if val.is_null() {
            return Ok(default);
        }
        let n = val
            .as_i64()
            .ok_or_else(|| format!("field '{}' must be an integer", field))?;
        if n < 0 {
            return Err(format!("field '{}' must be non-negative, got {}", field, n));
        }
        Ok(n as u64)
    } else {
        Ok(default)
    }
}

fn parse_positive_u64(v: &serde_json::Value, field: &str, default: u64) -> Result<u64, String> {
    if let Some(val) = v.get(field) {
        if val.is_null() {
            return Ok(default);
        }
        let n = val
            .as_i64()
            .ok_or_else(|| format!("field '{}' must be an integer", field))?;
        if n <= 0 {
            return Err(format!("field '{}' must be positive, got {}", field, n));
        }
        Ok(n as u64)
    } else {
        Ok(default)
    }
}

impl FactsConfig {
    pub fn load(args: &FactsArgs) -> Result<Self, FactsError> {
        let config_path = if let Some(ref p) = args.config {
            p.clone()
        } else if let Ok(env_path) = std::env::var("RIKO_FACTS_CONFIG") {
            if !env_path.trim().is_empty() {
                PathBuf::from(env_path)
            } else {
                PathBuf::from("/root/riko/var/facts.json")
            }
        } else {
            PathBuf::from("/root/riko/var/facts.json")
        };

        if !config_path.exists() {
            return Err(FactsError::Config(format!(
                "config file not found: {}",
                config_path.display()
            )));
        }

        let content = fs::read_to_string(&config_path).map_err(|e| {
            FactsError::Config(format!(
                "failed to read config file {}: {}",
                config_path.display(),
                e
            ))
        })?;

        let v: serde_json::Value = serde_json::from_str(&content).map_err(|e| {
            FactsError::Config(format!(
                "failed to parse config JSON {}: {}",
                config_path.display(),
                e
            ))
        })?;

        let sessions_root = PathBuf::from(
            parse_str_field(&v, "sessions_root", "/root/.dsh/sessions/--root-riko--")
                .map_err(FactsError::Config)?,
        );

        let state_path = PathBuf::from(
            parse_str_field(&v, "state_path", "/root/riko/var/facts.state.json")
                .map_err(FactsError::Config)?,
        );

        let log_path = PathBuf::from(
            parse_str_field(&v, "log_path", "/root/riko/var/facts.log")
                .map_err(FactsError::Config)?,
        );

        let lock_path = PathBuf::from(
            parse_str_field(&v, "lock_path", "/root/riko/var/facts.lock")
                .map_err(FactsError::Config)?,
        );

        let mut endpoint = parse_str_field(&v, "endpoint", "http://127.0.0.1:18100")
            .map_err(FactsError::Config)?;
        if let Ok(env_ep) = std::env::var("RIKO_FACTS_ENDPOINT") {
            if !env_ep.trim().is_empty() {
                endpoint = env_ep.trim().to_string();
            }
        }

        if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
            return Err(FactsError::Config(
                "endpoint must start with http:// or https://".to_string(),
            ));
        }

        let app_id = parse_str_field(&v, "app_id", "riko").map_err(FactsError::Config)?;
        let project_id =
            parse_str_field(&v, "project_id", "main").map_err(FactsError::Config)?;
        let user_sender_id =
            parse_str_field(&v, "user_sender_id", "aki").map_err(FactsError::Config)?;
        let user_sender_name =
            parse_str_field(&v, "user_sender_name", "爹地").map_err(FactsError::Config)?;

        let skip_recent_seconds = parse_non_negative_u64(&v, "skip_recent_seconds", 60)
            .map_err(FactsError::Config)?;
        let idle_flush_seconds = parse_non_negative_u64(&v, "idle_flush_seconds", 900)
            .map_err(FactsError::Config)?;

        let max_messages_per_call = parse_positive_u64(&v, "max_messages_per_call", 200)
            .map_err(FactsError::Config)? as usize;
        let max_chars_per_call = parse_positive_u64(&v, "max_chars_per_call", 200000)
            .map_err(FactsError::Config)? as usize;

        let config_max_calls = parse_non_negative_u64(&v, "max_calls_per_run", 40)
            .map_err(FactsError::Config)? as usize;
        let max_calls_per_run = match args.max_calls {
            Some(0) => usize::MAX,
            Some(n) => n,
            None => config_max_calls,
        };

        let max_flushes_per_run = parse_non_negative_u64(&v, "max_flushes_per_run", 2)
            .map_err(FactsError::Config)? as usize;
        let timeout_seconds = parse_positive_u64(&v, "timeout_seconds", 240)
            .map_err(FactsError::Config)?;
        let stale_lock_seconds = parse_positive_u64(&v, "stale_lock_seconds", 1800)
            .map_err(FactsError::Config)?;

        let curl_path = parse_str_field(&v, "curl_path", "/usr/bin/curl")
            .map_err(FactsError::Config)?;

        Ok(Self {
            sessions_root,
            state_path,
            log_path,
            lock_path,
            endpoint,
            app_id,
            project_id,
            user_sender_id,
            user_sender_name,
            skip_recent_seconds,
            idle_flush_seconds,
            max_messages_per_call,
            max_chars_per_call,
            max_calls_per_run,
            max_flushes_per_run,
            timeout_seconds,
            stale_lock_seconds,
            curl_path,
        })
    }
}

pub struct FactsLogger {
    path: PathBuf,
    offset: FixedOffset,
    dry_run: bool,
}

impl FactsLogger {
    pub fn new(path: PathBuf, offset: FixedOffset, dry_run: bool) -> Self {
        Self {
            path,
            offset,
            dry_run,
        }
    }

    pub fn log(&self, msg: &str) {
        if self.dry_run {
            return;
        }
        let now: DateTime<FixedOffset> = Utc::now().with_timezone(&self.offset);
        let time_str = now.format("%Y-%m-%dT%H:%M:%S%:z");
        let line = format!("{} [facts] {}\n", time_str, msg);
        if let Some(p) = self.path.parent() {
            let _ = fs::create_dir_all(p);
        }
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionState {
    pub last_seq: u64,
    pub last_user_seq: u64,
    pub flushed_seq: u64,
    pub last_activity_at: String,
}

#[derive(Debug, Clone)]
pub struct FactsState {
    pub version: u64,
    pub sessions: BTreeMap<String, SessionState>,
    pub path: PathBuf,
}

impl FactsState {
    pub fn load(path: &Path, logger: &FactsLogger) -> Self {
        if !path.exists() {
            return Self {
                version: 1,
                sessions: BTreeMap::new(),
                path: path.to_path_buf(),
            };
        }

        match fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(v) => {
                    let version = v.get("version").and_then(|n| n.as_u64()).unwrap_or(1);
                    let mut sessions = BTreeMap::new();
                    if let Some(sess_obj) = v.get("sessions").and_then(|s| s.as_object()) {
                        for (k, val) in sess_obj {
                            let last_seq =
                                val.get("last_seq").and_then(|n| n.as_u64()).unwrap_or(0);
                            let last_user_seq =
                                val.get("last_user_seq").and_then(|n| n.as_u64()).unwrap_or(0);
                            let flushed_seq = val
                                .get("flushed_seq")
                                .and_then(|n| n.as_u64())
                                .unwrap_or(0);
                            let last_activity_at = val
                                .get("last_activity_at")
                                .and_then(|s| s.as_str())
                                .unwrap_or("")
                                .to_string();
                            sessions.insert(
                                k.clone(),
                                SessionState {
                                    last_seq,
                                    last_user_seq,
                                    flushed_seq,
                                    last_activity_at,
                                },
                            );
                        }
                    }
                    Self {
                        version,
                        sessions,
                        path: path.to_path_buf(),
                    }
                }
                Err(e) => {
                    logger.log(&format!("warning: state file corrupted: {}", e));
                    Self {
                        version: 1,
                        sessions: BTreeMap::new(),
                        path: path.to_path_buf(),
                    }
                }
            },
            Err(e) => {
                logger.log(&format!("warning: failed to read state file: {}", e));
                Self {
                    version: 1,
                    sessions: BTreeMap::new(),
                    path: path.to_path_buf(),
                }
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let parent = self.path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;

        let mut sess_map = serde_json::Map::new();
        for (k, v) in &self.sessions {
            sess_map.insert(
                k.clone(),
                serde_json::json!({
                    "last_seq": v.last_seq,
                    "last_user_seq": v.last_user_seq,
                    "flushed_seq": v.flushed_seq,
                    "last_activity_at": v.last_activity_at,
                }),
            );
        }

        let val = serde_json::json!({
            "version": self.version,
            "sessions": sess_map,
        });

        let s = serde_json::to_string(&val)?;
        let now_nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let tmp_path = parent.join(format!(
            ".facts.state.tmp.{}.{}",
            std::process::id(),
            now_nanos
        ));

        {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp_path)?;
            file.write_all(s.as_bytes())?;
            file.flush()?;
            file.sync_all()?;
        }

        fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }
}

pub fn sanitize_session_id(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    sanitized.chars().take(128).collect()
}

pub fn batch_messages<'a>(
    messages: &'a [crate::dream::Message],
    max_messages: usize,
    max_chars: usize,
) -> Vec<&'a [crate::dream::Message]> {
    let mut batches = Vec::new();
    let mut start = 0;
    let mut current_chars = 0;
    let mut current_count = 0;

    for (i, msg) in messages.iter().enumerate() {
        let msg_chars = msg.text.chars().count();
        if current_count > 0
            && (current_count + 1 > max_messages || current_chars + msg_chars > max_chars)
        {
            batches.push(&messages[start..i]);
            start = i;
            current_count = 1;
            current_chars = msg_chars;
        } else {
            current_count += 1;
            current_chars += msg_chars;
        }
    }

    if start < messages.len() {
        batches.push(&messages[start..]);
    }

    batches
}

struct TempBodyFile {
    path: PathBuf,
}

impl TempBodyFile {
    fn create(dir: &Path, pid: u32, counter: u64, content: &str) -> anyhow::Result<Self> {
        fs::create_dir_all(dir)?;
        let filename = format!(".facts-body-{}-{}.json", pid, counter);
        let path = dir.join(filename);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;
        }

        #[cfg(not(unix))]
        {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;
        }

        Ok(Self { path })
    }
}

impl Drop for TempBodyFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn call_curl(
    curl_path: &str,
    timeout_seconds: u64,
    body_dir: &Path,
    url: &str,
    body_json: &str,
    counter: u64,
) -> anyhow::Result<(i32, String)> {
    let temp_file = TempBodyFile::create(body_dir, std::process::id(), counter, body_json)?;
    let output = Command::new(curl_path)
        .arg("--silent")
        .arg("--show-error")
        .arg("--fail-with-body")
        .arg("--max-time")
        .arg(timeout_seconds.to_string())
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-d")
        .arg(format!("@{}", temp_file.path.display()))
        .arg(url)
        .output()?;

    let exit_code = output.status.code().unwrap_or(-1);
    let mut resp = String::from_utf8_lossy(&output.stdout).to_string();
    if resp.is_empty() && !output.stderr.is_empty() {
        resp = String::from_utf8_lossy(&output.stderr).to_string();
    }
    Ok((exit_code, resp))
}

fn call_flush(
    config: &FactsConfig,
    logger: &FactsLogger,
    session_id: &str,
    body_dir: &Path,
    req_counter: &mut u64,
    total_calls: &mut usize,
    batch_info: Option<(usize, usize)>,
) -> Result<String, String> {
    let flush_body = serde_json::json!({
        "session_id": session_id,
        "app_id": config.app_id,
        "project_id": config.project_id,
    });
    let body_str = serde_json::to_string(&flush_body).unwrap_or_default();
    *req_counter += 1;
    let url = format!("{}/api/v2/memory/flush", config.endpoint.trim_end_matches('/'));
    let start_flush = Instant::now();

    let (exit_code, resp) = match call_curl(
        &config.curl_path,
        config.timeout_seconds,
        body_dir,
        &url,
        &body_str,
        *req_counter,
    ) {
        Ok(res) => res,
        Err(e) => {
            logger.log(&format!("session={} curl flush error: {}", session_id, e));
            return Err(format!("curl flush error: {}", e));
        }
    };

    *total_calls += 1;

    if exit_code != 0 {
        let prefix: String = resp.chars().take(500).collect();
        logger.log(&format!(
            "flush failed session={} exit_code={} output={}",
            session_id, exit_code, prefix
        ));
        return Err(format!("flush exit code {}: {}", exit_code, prefix));
    }

    let val: serde_json::Value = match serde_json::from_str(&resp) {
        Ok(v) => v,
        Err(e) => {
            let prefix: String = resp.chars().take(500).collect();
            logger.log(&format!(
                "flush failed invalid json session={} error={} output={}",
                session_id, e, prefix
            ));
            return Err(format!("flush invalid json: {}", e));
        }
    };

    let status = val
        .get("data")
        .and_then(|d| d.get("status"))
        .and_then(|s| s.as_str());

    let status_str = match status {
        Some(s) if s == "extracted" || s == "no_extraction" => s,
        _ => {
            let prefix: String = resp.chars().take(500).collect();
            logger.log(&format!(
                "flush failed invalid status session={} output={}",
                session_id, prefix
            ));
            return Err(format!("flush invalid status: {}", prefix));
        }
    };

    let ms = start_flush.elapsed().as_millis();
    if let Some((b, total_b)) = batch_info {
        logger.log(&format!(
            "flush session={} batch={}/{} status={} ms={}",
            session_id, b, total_b, status_str, ms
        ));
    } else {
        logger.log(&format!(
            "flush session={} status={} ms={}",
            session_id, status_str, ms
        ));
    }

    Ok(status_str.to_string())
}

pub fn run_facts(args: FactsArgs) -> Result<i32, FactsError> {
    let config = FactsConfig::load(&args)?;
    let offset = FixedOffset::east_opt(8 * 3600).unwrap();
    let logger = FactsLogger::new(config.log_path.clone(), offset, args.dry_run);
    let mut state = FactsState::load(&config.state_path, &logger);

    let _lock_guard = if !args.dry_run {
        match crate::dream::LockGuard::acquire(
            &config.lock_path,
            config.stale_lock_seconds,
            offset,
        ) {
            Ok(Some(guard)) => Some(guard),
            Ok(None) => {
                logger.log("lock busy, exiting");
                return Ok(0);
            }
            Err(e) => {
                logger.log(&format!("failed to acquire lock: {}", e));
                return Err(FactsError::Runtime(format!("failed to acquire lock: {}", e)));
            }
        }
    } else {
        None
    };

    let now: DateTime<FixedOffset> = Utc::now().with_timezone(&offset);

    // Scan sessions
    let mut session_entries = Vec::new();
    if config.sessions_root.exists() && config.sessions_root.is_dir() {
        if let Ok(read_dir) = fs::read_dir(&config.sessions_root) {
            for entry_res in read_dir {
                if let Ok(entry) = entry_res {
                    let path = entry.path();
                    if path.is_dir() {
                        let zstd_file = path.join("session.v4.jsonl.zstd");
                        if zstd_file.is_file() {
                            if let Ok(meta) = fs::metadata(&zstd_file) {
                                if let Ok(mtime) = meta.modified() {
                                    session_entries.push((path, zstd_file, mtime));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    session_entries.sort_by_key(|(_, _, mtime)| *mtime);

    let mut sessions_count = 0usize;
    let mut sent_count = 0usize;
    let mut flushed_count = 0usize;
    let mut total_calls = 0usize;
    let mut req_counter = 0u64;
    let mut has_failure = false;

    let body_dir = config.lock_path.parent().unwrap_or(Path::new("."));

    // In dry-run mode, track simulated session states to correctly report pending
    let mut dry_run_simulated_sessions: BTreeMap<String, (u64, u64)> = BTreeMap::new();

    for (session_dir, zstd_path, mtime) in session_entries {
        if total_calls >= config.max_calls_per_run {
            break;
        }

        let dir_name = session_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let session_id = sanitize_session_id(&dir_name);

        let elapsed = SystemTime::now()
            .duration_since(mtime)
            .unwrap_or_default()
            .as_secs();
        if elapsed < config.skip_recent_seconds {
            logger.log(&format!(
                "skip recent session={} elapsed={}s < {}s",
                session_id, elapsed, config.skip_recent_seconds
            ));
            continue;
        }

        let (content, _line_count, trailing_partial) =
            match crate::dream::decode_session_file(&zstd_path) {
                Ok(res) => res,
                Err(e) => {
                    logger.log(&format!(
                        "session={} decode error: {}, skipping",
                        session_id, e
                    ));
                    continue;
                }
            };

        if trailing_partial {
            logger.log(&format!(
                "session={} warning: trailing partial line detected",
                session_id
            ));
        }

        let (all_messages, _, _) = crate::dream::extract_messages(&content, 0, offset);
        let mut user_messages: Vec<crate::dream::Message> = all_messages
            .into_iter()
            .filter(|m| m.role == crate::dream::Role::User)
            .collect();

        let mut file_max_seq = 0u64;
        let mut time_fallback = false;
        let mut missing_time_messages = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(s) = val.get("seq").and_then(|s| s.as_u64()) {
                    if s > file_max_seq {
                        file_max_seq = s;
                    }
                }
                if val.get("type").and_then(|t| t.as_str()) == Some("user/message") {
                    let is_user = val
                        .get("data")
                        .and_then(|d| d.get("source"))
                        .and_then(|s| s.get("kind"))
                        .and_then(|k| k.as_str())
                        == Some("user");
                    if is_user {
                        let time_val = val.get("time").and_then(|t| t.as_i64());
                        if time_val.is_none() {
                            let mut text_parts = Vec::new();
                            if let Some(content_arr) = val
                                .get("data")
                                .and_then(|d| d.get("content"))
                                .and_then(|c| c.as_array())
                            {
                                for item in content_arr {
                                    if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                                        if let Some(txt) =
                                            item.get("text").and_then(|t| t.as_str())
                                        {
                                            text_parts.push(txt);
                                        }
                                    }
                                }
                            }
                            let combined = text_parts.join("");
                            if !combined.is_empty() && !combined.trim().starts_with("[自动探活]") {
                                if let Some(seq) = val.get("seq").and_then(|s| s.as_u64()) {
                                    time_fallback = true;
                                    let dt = DateTime::from_timestamp(
                                        mtime
                                            .duration_since(SystemTime::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs()
                                            as i64,
                                        mtime
                                            .duration_since(SystemTime::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .subsec_nanos(),
                                    )
                                    .unwrap_or_else(|| Utc::now())
                                    .with_timezone(&offset);
                                    missing_time_messages.push(crate::dream::Message {
                                        seq,
                                        role: crate::dream::Role::User,
                                        time: dt,
                                        text: combined,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        user_messages.extend(missing_time_messages);
        user_messages.sort_by_key(|m| m.seq);

        let mtime_dt = DateTime::from_timestamp(
            mtime
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
            mtime
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos(),
        )
        .unwrap_or_else(|| Utc::now())
        .with_timezone(&offset);

        let (session_last_activity_at, time_fallback_used) = if user_messages.is_empty() {
            (mtime_dt.to_rfc3339(), false)
        } else if time_fallback {
            (user_messages.last().unwrap().time.to_rfc3339(), true)
        } else {
            (user_messages.last().unwrap().time.to_rfc3339(), false)
        };

        if time_fallback_used {
            logger.log(&format!("session={} time_fallback=1", session_id));
        }

        let existing_state = state.sessions.get(&session_id).cloned();
        let is_new_session = existing_state.is_none();
        let prev_last_seq = existing_state.as_ref().map(|s| s.last_seq).unwrap_or(0);

        let mut sess_state = existing_state.unwrap_or(SessionState {
            last_seq: 0,
            last_user_seq: 0,
            flushed_seq: 0,
            last_activity_at: session_last_activity_at.clone(),
        });

        if user_messages.is_empty() {
            if args.dry_run {
                println!(
                    "facts: plan session={} new=0 add_batches=0 flush=no",
                    session_id
                );
                dry_run_simulated_sessions.insert(
                    session_id.clone(),
                    (sess_state.last_user_seq, sess_state.flushed_seq),
                );
                continue;
            }

            if is_new_session || prev_last_seq != file_max_seq {
                logger.log(&format!(
                    "session={} no_user_messages last_seq={}",
                    session_id, file_max_seq
                ));
            }
            sess_state.last_seq = file_max_seq;
            sess_state.last_activity_at = session_last_activity_at;
            state
                .sessions
                .insert(session_id.clone(), sess_state.clone());
            state.save().map_err(|e| {
                logger.log(&format!("state save error: {}", e));
                FactsError::Runtime(format!("failed to save state: {}", e))
            })?;
            continue;
        }

        let new_user_messages: Vec<crate::dream::Message> = user_messages
            .into_iter()
            .filter(|m| m.seq > sess_state.last_user_seq)
            .collect();

        let batches = batch_messages(
            &new_user_messages,
            config.max_messages_per_call,
            config.max_chars_per_call,
        );

        if args.dry_run {
            let new_n = new_user_messages.len();
            let add_batches = batches.len();
            let sim_last_user_seq = if new_n > 0 {
                new_user_messages.last().unwrap().seq
            } else {
                sess_state.last_user_seq
            };
            let sim_flushed_seq = sess_state.flushed_seq;

            let (flush_plan, final_sim_flushed_seq) = if add_batches >= 2 {
                let mut hit_limit = false;
                let mut sim_flushed = sim_flushed_seq;
                for batch in &batches {
                    let batch_last_seq = batch.last().unwrap().seq;
                    if total_calls + 2 > config.max_calls_per_run
                        || flushed_count + 1 > config.max_flushes_per_run
                    {
                        hit_limit = true;
                        break;
                    }
                    total_calls += 2;
                    flushed_count += 1;
                    sim_flushed = batch_last_seq;
                }
                let plan = if hit_limit {
                    "limit"
                } else {
                    "yes"
                };
                (plan, sim_flushed)
            } else if sim_flushed_seq < sim_last_user_seq {
                let idle_secs =
                    match DateTime::parse_from_rfc3339(&session_last_activity_at) {
                        Ok(dt) => (now - dt).num_seconds(),
                        Err(_) => config.idle_flush_seconds as i64 + 1,
                    };
                let is_idle = idle_secs > config.idle_flush_seconds as i64;
                if args.backfill || is_idle {
                    if flushed_count + 1 > config.max_flushes_per_run
                        || total_calls + add_batches + 1 > config.max_calls_per_run
                    {
                        ("limit", sim_flushed_seq)
                    } else {
                        flushed_count += 1;
                        total_calls += 1;
                        ("yes", sim_last_user_seq)
                    }
                } else {
                    ("wait:idle", sim_flushed_seq)
                }
            } else {
                ("no", sim_flushed_seq)
            };

            println!(
                "facts: plan session={} new={} add_batches={} flush={}",
                session_id, new_n, add_batches, flush_plan
            );

            if new_n > 0 {
                sessions_count += 1;
                sent_count += new_n;
                if add_batches < 2 {
                    total_calls += add_batches;
                }
            }

            dry_run_simulated_sessions
                .insert(session_id.clone(), (sim_last_user_seq, final_sim_flushed_seq));
            continue;
        }

        // Normal mode
        let is_multi_batch = batches.len() >= 2;
        let mut session_sent_any = false;
        let mut batches_fully_sent = true;
        let mut session_failed = false;

        for (b_idx, batch) in batches.iter().enumerate() {
            if is_multi_batch {
                if total_calls >= config.max_calls_per_run
                    || flushed_count >= config.max_flushes_per_run
                {
                    batches_fully_sent = false;
                    break;
                }
            } else {
                if total_calls >= config.max_calls_per_run {
                    batches_fully_sent = false;
                    break;
                }
            }

            let messages_json: Vec<serde_json::Value> = batch
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "sender_id": config.user_sender_id,
                        "sender_name": config.user_sender_name,
                        "role": "user",
                        "timestamp": m.time.timestamp_millis(),
                        "content": m.text,
                    })
                })
                .collect();

            let add_body = serde_json::json!({
                "session_id": session_id,
                "app_id": config.app_id,
                "project_id": config.project_id,
                "defer_extraction": true,
                "messages": messages_json,
            });
            let body_str = serde_json::to_string(&add_body).unwrap_or_default();
            req_counter += 1;
            let url = format!("{}/api/v2/memory/add", config.endpoint.trim_end_matches('/'));

            let (exit_code, resp) = match call_curl(
                &config.curl_path,
                config.timeout_seconds,
                body_dir,
                &url,
                &body_str,
                req_counter,
            ) {
                Ok(res) => res,
                Err(e) => {
                    logger.log(&format!("session={} curl execute error: {}", session_id, e));
                    has_failure = true;
                    session_failed = true;
                    batches_fully_sent = false;
                    break;
                }
            };

            total_calls += 1;

            if exit_code != 0 {
                let prefix: String = resp.chars().take(500).collect();
                logger.log(&format!(
                    "add failed session={} exit_code={} output={}",
                    session_id, exit_code, prefix
                ));
                has_failure = true;
                session_failed = true;
                batches_fully_sent = false;
                break;
            }

            let val: serde_json::Value = match serde_json::from_str(&resp) {
                Ok(v) => v,
                Err(e) => {
                    let prefix: String = resp.chars().take(500).collect();
                    logger.log(&format!(
                        "add failed invalid json session={} error={} output={}",
                        session_id, e, prefix
                    ));
                    has_failure = true;
                    session_failed = true;
                    batches_fully_sent = false;
                    break;
                }
            };

            if val
                .get("data")
                .and_then(|d| d.get("status"))
                .and_then(|s| s.as_str())
                .is_none()
            {
                let prefix: String = resp.chars().take(500).collect();
                logger.log(&format!(
                    "add failed invalid status session={} output={}",
                    session_id, prefix
                ));
                has_failure = true;
                session_failed = true;
                batches_fully_sent = false;
                break;
            }

            let batch_last_seq = batch.last().unwrap().seq;
            sess_state.last_user_seq = batch_last_seq;
            sess_state.last_seq = sess_state.last_seq.max(batch_last_seq);
            sess_state.last_activity_at = session_last_activity_at.clone();
            state
                .sessions
                .insert(session_id.clone(), sess_state.clone());
            state.save().map_err(|e| {
                logger.log(&format!("state save error: {}", e));
                FactsError::Runtime(format!("failed to save state: {}", e))
            })?;
            sent_count += batch.len();
            session_sent_any = true;

            // Multi-batch: flush immediately after each add batch
            if is_multi_batch {
                if total_calls >= config.max_calls_per_run
                    || flushed_count >= config.max_flushes_per_run
                {
                    batches_fully_sent = false;
                    break;
                }

                match call_flush(
                    &config,
                    &logger,
                    &session_id,
                    body_dir,
                    &mut req_counter,
                    &mut total_calls,
                    Some((b_idx + 1, batches.len())),
                ) {
                    Ok(_status) => {
                        sess_state.flushed_seq = batch_last_seq;
                        state
                            .sessions
                            .insert(session_id.clone(), sess_state.clone());
                        state.save().map_err(|e| {
                            logger.log(&format!("state save error: {}", e));
                            FactsError::Runtime(format!("failed to save state: {}", e))
                        })?;
                        flushed_count += 1;
                    }
                    Err(_err) => {
                        has_failure = true;
                        session_failed = true;
                        batches_fully_sent = false;
                        break;
                    }
                }
            }
        }

        if session_failed {
            continue;
        }

        if batches_fully_sent {
            if file_max_seq > sess_state.last_seq {
                sess_state.last_seq = file_max_seq;
            }
        }
        sess_state.last_activity_at = session_last_activity_at.clone();
        state
            .sessions
            .insert(session_id.clone(), sess_state.clone());
        state.save().map_err(|e| {
            logger.log(&format!("state save error: {}", e));
            FactsError::Runtime(format!("failed to save state: {}", e))
        })?;

        if session_sent_any {
            sessions_count += 1;
            logger.log(&format!(
                "session={} new={} batches={} last_seq={}",
                session_id,
                new_user_messages.len(),
                batches.len(),
                sess_state.last_seq
            ));
        }

        // Flush condition check for single batch or backlog flush
        if !is_multi_batch && sess_state.flushed_seq < sess_state.last_user_seq {
            let idle_secs = match DateTime::parse_from_rfc3339(&sess_state.last_activity_at) {
                Ok(dt) => (now - dt).num_seconds(),
                Err(_) => config.idle_flush_seconds as i64 + 1,
            };
            let is_idle = idle_secs > config.idle_flush_seconds as i64;
            if args.backfill || is_idle {
                if flushed_count < config.max_flushes_per_run
                    && total_calls < config.max_calls_per_run
                {
                    match call_flush(
                        &config,
                        &logger,
                        &session_id,
                        body_dir,
                        &mut req_counter,
                        &mut total_calls,
                        None,
                    ) {
                        Ok(_status) => {
                            sess_state.flushed_seq = sess_state.last_user_seq;
                            state
                                .sessions
                                .insert(session_id.clone(), sess_state.clone());
                            state.save().map_err(|e| {
                                logger.log(&format!("state save error: {}", e));
                                FactsError::Runtime(format!("failed to save state: {}", e))
                            })?;
                            flushed_count += 1;
                        }
                        Err(_err) => {
                            has_failure = true;
                            continue;
                        }
                    }
                }
            }
        }
    }

    let pending_count = if args.dry_run {
        let mut count = 0;
        for (sid, sess) in &state.sessions {
            if let Some((last_user, flushed)) = dry_run_simulated_sessions.get(sid) {
                if flushed < last_user {
                    count += 1;
                }
            } else if sess.flushed_seq < sess.last_user_seq {
                count += 1;
            }
        }
        for (sid, (last_user, flushed)) in &dry_run_simulated_sessions {
            if !state.sessions.contains_key(sid) && flushed < last_user {
                count += 1;
            }
        }
        count
    } else {
        state
            .sessions
            .values()
            .filter(|s| s.flushed_seq < s.last_user_seq)
            .count()
    };

    if !args.dry_run {
        let _ = state.save();
        logger.log(&format!(
            "run end sessions={} sent={} flushed={} pending={}",
            sessions_count, sent_count, flushed_count, pending_count
        ));
    }

    println!(
        "facts: sessions={} sent={} flushed={} pending={}",
        sessions_count, sent_count, flushed_count, pending_count
    );

    if has_failure {
        return Err(FactsError::Runtime(
            "one or more sessions failed during facts processing".to_string(),
        ));
    }

    Ok(0)
}

pub fn cmd_facts(args: FactsArgs) {
    match run_facts(args) {
        Ok(code) => std::process::exit(code),
        Err(FactsError::Config(msg)) => {
            eprintln!("facts config error: {}", msg);
            std::process::exit(2);
        }
        Err(FactsError::Runtime(msg)) => {
            eprintln!("facts error: {}", msg);
            std::process::exit(1);
        }
    }
}
