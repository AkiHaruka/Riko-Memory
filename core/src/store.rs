// src/store.rs — 路径、时间、front-matter、遍历、标题等公共逻辑

use chrono::{DateTime, FixedOffset, NaiveDateTime, SecondsFormat, Utc};
use regex::Regex;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// 默认记忆目录
pub const DEFAULT_MEM_DIR: &str = "/root/riko/memory";
/// 默认 AGENTS.md 路径
pub const DEFAULT_AGENTS_PATH: &str = "/root/riko/AGENTS.md";

/// 获取东八区固定时区 (+08:00)
pub fn beijing_offset() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).expect("东八区偏移量必须有效")
}

/// 获取当前东八区时间
pub fn now_beijing() -> DateTime<FixedOffset> {
    Utc::now().with_timezone(&beijing_offset())
}

/// 格式化时间为 RFC3339（秒级精度，带 +08:00 时区）
pub fn format_rfc3339(dt: &DateTime<FixedOffset>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// 获取记忆库根目录（优先使用 RIKO_MEM_DIR 环境变量）
pub fn mem_dir() -> PathBuf {
    match env::var("RIKO_MEM_DIR") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v.trim()),
        _ => PathBuf::from(DEFAULT_MEM_DIR),
    }
}

/// 获取 AGENTS.md 路径（优先使用 RIKO_AGENTS_PATH 环境变量）
pub fn agents_path() -> PathBuf {
    match env::var("RIKO_AGENTS_PATH") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v.trim()),
        _ => PathBuf::from(DEFAULT_AGENTS_PATH),
    }
}

/// 解析用户输入的时间字符串
/// 支持格式：
/// 1. 完整 RFC3339: 2026-10-31T15:56:00+08:00
/// 2. 带时区无秒: 2026-10-31T15:56+08:00
/// 3. 无时区带秒: 2026-10-31T15:56:00 (自动补 +08:00)
/// 4. 无时区无秒: 2026-10-31T15:56 (自动补 +08:00)
/// 5. 空格分隔日期时间: 2026-10-31 15:56:00 或 2026-10-31 15:56 (自动补 +08:00)
pub fn parse_time_opt(input: &str) -> Result<DateTime<FixedOffset>, String> {
    let s = input.trim();
    let tz = beijing_offset();

    // 尝试直接按 RFC3339 解析
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&tz));
    }

    // 尝试带时区的常见格式 "%Y-%m-%dT%H:%M%:z"
    if let Ok(dt) = DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M%:z") {
        return Ok(dt.with_timezone(&tz));
    }

    // 尝试各种无时区的格式，补全东八区
    let naive_formats = [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ];

    for fmt in &naive_formats {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
            return Ok(DateTime::from_naive_utc_and_offset(
                naive - chrono::Duration::hours(8),
                tz,
            ));
        }
    }

    Err(format!("无法解析时间: {}", input))
}

/// 获取文本中的原始 front-matter 文本切片（不含包裹的 ---）
pub fn raw_front_matter(txt: &str) -> Option<&str> {
    if !txt.starts_with("---\n") && !txt.starts_with("---\r\n") {
        return None;
    }

    let end_idx = txt[4..].find("\n---")? + 4;
    Some(&txt[4..end_idx])
}

/// 解析文本中的 front-matter 和正文
/// 返回 (front_matter_map, body)
pub fn read_front_matter(txt: &str) -> (HashMap<String, String>, String) {
    if !txt.starts_with("---\n") && !txt.starts_with("---\r\n") {
        return (HashMap::new(), txt.to_string());
    }

    let end_idx = if let Some(idx) = txt[4..].find("\n---") {
        idx + 4
    } else {
        return (HashMap::new(), txt.to_string());
    };

    let fm_raw = &txt[4..end_idx];
    let mut rest = &txt[end_idx + 4..];
    // 跳过紧接着的可能存在的换行符
    if rest.starts_with('\n') {
        rest = &rest[1..];
    } else if rest.starts_with("\r\n") {
        rest = &rest[2..];
    }

    let mut fm = HashMap::new();
    for line in fm_raw.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_string();
            let val_trimmed = v.trim();
            let val = if val_trimmed.starts_with('"')
                && val_trimmed.ends_with('"')
                && val_trimmed.len() >= 2
            {
                &val_trimmed[1..val_trimmed.len() - 1]
            } else {
                val_trimmed
            };
            fm.insert(key, val.to_string());
        }
    }

    (fm, rest.to_string())
}

/// 读取文件的 front-matter 和正文
pub fn read_file_front(path: &Path) -> (HashMap<String, String>, String) {
    match fs::read_to_string(path) {
        Ok(txt) => read_front_matter(&txt),
        Err(_) => (HashMap::new(), String::new()),
    }
}

/// 获取文件的标题：
/// 1. front-matter 的 title 字段
/// 2. 正文第一个非空行（去除 # 和空白）
/// 3. 文件名（去扩展名或保留）
pub fn title_of(path: &Path) -> String {
    let (fm, body) = read_file_front(path);
    if let Some(t) = fm.get("title") {
        let trimmed = t.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    for line in body.lines() {
        let s = line.trim().trim_start_matches('#').trim();
        if !s.is_empty() {
            return s.to_string();
        }
    }

    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// 递归遍历目录下所有 .md 文件，跳过 .git 目录，结果按字典序排序
pub fn walk_md_files(base: &Path) -> Vec<PathBuf> {
    if !base.exists() {
        return Vec::new();
    }
    let mut files = Vec::new();
    for entry in WalkDir::new(base)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            let p = entry.path();
            if p.extension().map(|ext| ext == "md").unwrap_or(false) {
                files.push(p.to_path_buf());
            }
        }
    }
    files.sort();
    files
}

/// 类别映射到目录名
pub fn type_to_dir(typ: &str) -> Result<&'static str, String> {
    match typ {
        "moment" => Ok("moments"),
        "person" => Ok("people"),
        "self" => Ok("self"),
        _ => Err("type 只支持: moment, person, self".to_string()),
    }
}

/// 计算新条目的文件路径与 ID
/// 返回 (文件绝对路径, id)
pub fn next_entry_path(
    mem_base: &Path,
    typ: &str,
    time: &DateTime<FixedOffset>,
) -> Result<(PathBuf, String), String> {
    let dir_name = type_to_dir(typ)?;
    let month = time.format("%Y-%m").to_string();
    let dir_path = mem_base.join(dir_name).join(month);
    fs::create_dir_all(&dir_path).map_err(|e| format!("创建目录失败: {}", e))?;

    let day = time.format("%Y%m%d").to_string();
    let mut n = 1;
    loop {
        let candidate_id = format!("m_{}_{:02}", day, n);
        let candidate_path = dir_path.join(format!("{}.md", candidate_id));
        if !candidate_path.exists() {
            return Ok((candidate_path, candidate_id));
        }
        n += 1;
    }
}

/// 净化 title（无论是默认提取还是 --title 传入）：
/// 先把 \r\n、\r、\n 全部换成空格，再把双引号 " 换成单引号 '，trim，最后按字符截断到 30 个字符。
pub fn sanitize_title(raw_title: &str, fallback_id: &str) -> String {
    let replaced_crlf = raw_title.replace("\r\n", " ").replace('\r', " ").replace('\n', " ");
    let replaced_quotes = replaced_crlf.replace('"', "'");
    let trimmed = replaced_quotes.trim();
    if trimmed.is_empty() {
        fallback_id.to_string()
    } else {
        trimmed.chars().take(30).collect()
    }
}

/// 净化 quote：
/// \r\n、\r、\n 换成空格；双引号用 \" 转义；trim 后若为空则返回 None。
pub fn sanitize_quote(raw_quote: &str) -> Option<String> {
    let replaced_crlf = raw_quote.replace("\r\n", " ").replace('\r', " ").replace('\n', " ");
    let trimmed = replaced_crlf.trim();
    if trimmed.is_empty() {
        None
    } else {
        let escaped = trimmed.replace('"', "\\\"");
        Some(escaped)
    }
}

/// 从正文中自动提取标题（取按 。！？\n 切分的第一句前 30 个字符，并经过安全净化）
pub fn extract_title(text: &str, fallback_id: &str) -> String {
    let body = text.trim();
    if body.is_empty() {
        return fallback_id.to_string();
    }

    // 按 。 ！？ \n 切分
    let re = Regex::new(r"[。！？\n]").expect("正则表达式必须有效");
    let first_part = match re.split(body).next() {
        Some(s) => s.trim(),
        None => body,
    };

    sanitize_title(first_part, fallback_id)
}

/// 统计给定词在内容中出现的总次数（不区分大小写）
pub fn score_content(quote: Option<&str>, body: &str, terms: &[String]) -> usize {
    let mut combined = String::new();
    if let Some(q) = quote {
        combined.push_str(q);
    }
    combined.push('\n');
    combined.push_str(body);
    let lower_text = combined.to_lowercase();

    let mut total_hits = 0;
    for term in terms {
        let t = term.to_lowercase();
        if t.is_empty() {
            continue;
        }
        total_hits += lower_text.matches(&t).count();
    }
    total_hits
}

/// 判断给定的词是否至少有一个出现在内容中（OR 匹配）
pub fn matches_any_term(quote: Option<&str>, body: &str, terms: &[String]) -> bool {
    let mut combined = String::new();
    if let Some(q) = quote {
        combined.push_str(q);
    }
    combined.push('\n');
    combined.push_str(body);
    let lower_text = combined.to_lowercase();

    for term in terms {
        let t = term.to_lowercase();
        if !t.is_empty() && lower_text.contains(&t) {
            return true;
        }
    }
    false
}

/// 根据 resident.md 内容生成 sync-agents 块
/// 返回 (块的全部行列表, 完整块字符串)
pub fn generate_agents_block(resident_content: Option<&str>) -> (Vec<String>, String) {
    let mut block = vec![
        "<!-- memory-index:begin（由 riko-memory sync-agents 生成，勿手改） -->".to_string(),
        "### 我的记忆状态板（源：memory/resident.md；完整索引 memory/MEMORY.md）".to_string(),
        "".to_string(),
    ];

    if let Some(content) = resident_content {
        let mut filtered: Vec<String> = Vec::new();
        for line in content.lines() {
            let s = line.trim_end();
            if s.trim().is_empty() || s.starts_with("# ") || s.starts_with('>') {
                continue;
            }
            filtered.push(s.to_string());
        }

        if filtered.len() <= 60 {
            block.extend(filtered);
        } else {
            let mut cut_point = None;
            for c in (1..=60).rev() {
                let next_is_boundary = c >= filtered.len() || filtered[c].starts_with("## ");
                let prev_not_header = !filtered[c - 1].starts_with("## ");
                if next_is_boundary && prev_not_header {
                    cut_point = Some(c);
                    break;
                }
            }
            let c = cut_point.unwrap_or(60);
            block.extend(filtered.into_iter().take(c));
        }
    } else {
        block.push("（还没有 resident.md）".to_string());
    }

    block.push("".to_string());
    block.push("查：riko-memory search 关键词｜看：riko-memory reveal 条目｜未完成：riko-memory open".to_string());
    block.push("更新状态板：改 memory/resident.md 后跑 riko-memory sync-agents".to_string());
    block.push("<!-- memory-index:end -->".to_string());

    let combined = block.join("\n");
    (block, combined)
}

/// 替换或追加 AGENTS.md 中的记忆块
pub fn replace_or_append_agents_block(agents_content: &str, new_block: &str) -> String {
    // 1. 若原内容为空或 trim 后为空：直接写入「块内容 + 一个末尾换行」，不要前导空行
    if agents_content.trim().is_empty() {
        return format!("{}\n", new_block);
    }

    // 2. 检查是否有标记
    let begin_marker = "<!-- memory-index:begin";
    let end_marker = "<!-- memory-index:end -->";

    if let Some(start_idx) = agents_content.find(begin_marker) {
        if let Some(end_rel_idx) = agents_content[start_idx..].find(end_marker) {
            let end_idx = start_idx + end_rel_idx + end_marker.len();
            let mut result = String::new();
            result.push_str(&agents_content[..start_idx]);
            result.push_str(new_block);
            result.push_str(&agents_content[end_idx..]);
            return result;
        }
    }

    // 3. 非空且没有成对标记时：先 rstrip，再空一行，再块，再末尾换行
    let trimmed = agents_content.trim_end();
    format!("{}\n\n{}\n", trimmed, new_block)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_time_opt() {
        let t1 = parse_time_opt("2026-10-31T15:56:00+08:00").unwrap();
        assert_eq!(format_rfc3339(&t1), "2026-10-31T15:56:00+08:00");

        let t2 = parse_time_opt("2026-10-31T15:56").unwrap();
        assert_eq!(format_rfc3339(&t2), "2026-10-31T15:56:00+08:00");

        let t3 = parse_time_opt("2026-10-31 15:56:00").unwrap();
        assert_eq!(format_rfc3339(&t3), "2026-10-31T15:56:00+08:00");
    }

    #[test]
    fn test_front_matter_parsing_and_escaping() {
        let content = r#"---
id: m_20261031_01
time: 2026-10-31T15:56:00+08:00
type: moment
title: 测试标题
source: manual
quote: "原话包含\"双引号\""
status: active
tags: [tag1, tag2]
---

这是正文内容。
"#;
        let (fm, body) = read_front_matter(content);
        assert_eq!(fm.get("id").unwrap(), "m_20261031_01");
        assert_eq!(fm.get("time").unwrap(), "2026-10-31T15:56:00+08:00");
        assert_eq!(fm.get("type").unwrap(), "moment");
        assert_eq!(fm.get("title").unwrap(), "测试标题");
        assert_eq!(fm.get("quote").unwrap(), "原话包含\\\"双引号\\\"");
        assert_eq!(fm.get("status").unwrap(), "active");
        assert_eq!(fm.get("tags").unwrap(), "[tag1, tag2]");
        assert_eq!(body.trim(), "这是正文内容。");
    }

    #[test]
    fn test_next_entry_path_increment() {
        let temp_dir = env::temp_dir().join("riko_test_increment");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let time = parse_time_opt("2026-10-31T10:00:00+08:00").unwrap();

        let (p1, id1) = next_entry_path(&temp_dir, "moment", &time).unwrap();
        assert_eq!(id1, "m_20261031_01");
        fs::write(&p1, "first").unwrap();

        let (p2, id2) = next_entry_path(&temp_dir, "moment", &time).unwrap();
        assert_eq!(id2, "m_20261031_02");
        fs::write(&p2, "second").unwrap();

        let (_p3, id3) = next_entry_path(&temp_dir, "moment", &time).unwrap();
        assert_eq!(id3, "m_20261031_03");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_search_scoring_and_ranking() {
        let terms = vec!["Rust".to_string(), "Memory".to_string()];

        // 包含 Rust 2次, Memory 1次 -> hit 3
        let quote1 = Some("Rust is fast");
        let body1 = "Rust Memory model";
        let score1 = score_content(quote1, body1, &terms);
        assert_eq!(score1, 3);

        // 只含 Rust 1次 -> hit 1
        let score2 = score_content(None, "Learning Rust programming", &terms);
        assert_eq!(score2, 1);

        // 只含 Memory 1次 -> hit 1
        let score3 = score_content(None, "Computer Memory architecture", &terms);
        assert_eq!(score3, 1);

        // 都不含 -> hit 0
        let score4 = score_content(None, "Python scripting", &terms);
        assert_eq!(score4, 0);

        assert!(score1 > score2);
    }

    #[test]
    fn test_sync_agents_block_generation_and_replace() {
        let resident_md = "# 状态板标题\n> 这是一个引用\n\n核心任务：完成 Rust 重写\n正在进行中\n";
        let (lines, block) = generate_agents_block(Some(resident_md));

        assert!(block.contains("核心任务：完成 Rust 重写"));
        assert!(block.contains("正在进行中"));
        assert!(!block.contains("# 状态板标题"));
        assert!(!block.contains("> 这是一个引用"));

        // 测试替换现有标记
        let original_agents = "# AGENTS\n\n<!-- memory-index:begin（旧） -->\n旧内容\n<!-- memory-index:end -->\n\n其他配置";
        let replaced = replace_or_append_agents_block(original_agents, &block);
        assert!(replaced.contains("核心任务：完成 Rust 重写"));
        assert!(!replaced.contains("旧内容"));
        assert!(replaced.contains("其他配置"));

        // 测试无标记时追加
        let agents_no_mark = "# AGENTS\n\n纯文本说明";
        let appended = replace_or_append_agents_block(agents_no_mark, &block);
        assert!(appended.starts_with("# AGENTS\n\n纯文本说明\n\n<!-- memory-index:begin"));
        assert!(appended.contains("核心任务：完成 Rust 重写"));

        // 测试空内容或全空白时直接写入块且无前导空行 (T14)
        let empty_agents = "";
        let appended_empty = replace_or_append_agents_block(empty_agents, &block);
        assert!(appended_empty.starts_with("<!-- memory-index:begin（由 riko-memory sync-agents 生成，勿手改） -->"));
        assert!(!appended_empty.starts_with('\n'));

        let ws_agents = "   \n\n  \t";
        let appended_ws = replace_or_append_agents_block(ws_agents, &block);
        assert!(appended_ws.starts_with("<!-- memory-index:begin（由 riko-memory sync-agents 生成，勿手改） -->"));
        assert!(!appended_ws.starts_with('\n'));

        assert_eq!(lines.len(), 9);
    }

    #[test]
    fn test_sync_agents_block_35_lines_all_retained() {
        let mut content = String::from("# 标题\n> 引用说明\n\n");
        // 构造 3 个小节，总共 35 行有效内容
        content.push_str("## 第一节\n");
        for i in 1..=10 {
            content.push_str(&format!("- 内容 1.{}\n", i));
        }
        content.push_str("## 第二节\n");
        for i in 1..=10 {
            content.push_str(&format!("- 内容 2.{}\n", i));
        }
        content.push_str("## 第三节\n");
        for i in 1..=12 {
            content.push_str(&format!("- 内容 3.{}\n", i));
        }

        let (lines, block) = generate_agents_block(Some(&content));
        // 头部 3 行 + 内容 35 行 + 尾部 4 行 = 42 行
        assert_eq!(lines.len(), 42);
        assert!(block.contains("## 第一节"));
        assert!(block.contains("- 内容 1.10"));
        assert!(block.contains("## 第二节"));
        assert!(block.contains("- 内容 2.10"));
        assert!(block.contains("## 第三节"));
        assert!(block.contains("- 内容 3.12"));
    }

    #[test]
    fn test_sync_agents_block_exact_60_lines_retained() {
        let mut content = String::from("# 标题\n> 引用说明\n\n");
        // 构造 3 个小节，各 20 行（1 标题 + 19 内容），总计 60 行
        for s in 1..=3 {
            content.push_str(&format!("## 小节 {}\n", s));
            for i in 1..=19 {
                content.push_str(&format!("- 条目 {}.{}\n", s, i));
            }
        }

        let (lines, block) = generate_agents_block(Some(&content));
        // 头部 3 行 + 内容 60 行 + 尾部 4 行 = 67 行
        assert_eq!(lines.len(), 67);
        assert!(block.contains("## 小节 1"));
        assert!(block.contains("## 小节 2"));
        assert!(block.contains("## 小节 3"));
        assert!(block.contains("- 条目 3.19"));
    }

    #[test]
    fn test_sync_agents_block_truncation_drops_incomplete_section() {
        let mut content = String::from("# 标题\n> 引用说明\n\n");
        // 小节 1: 标题 + 19 行 = 20 行
        content.push_str("## 小节 1\n");
        for i in 1..=19 {
            content.push_str(&format!("- 1.{}\n", i));
        }
        // 小节 2: 标题 + 19 行 = 20 行 (累计 40 行)
        content.push_str("## 小节 2\n");
        for i in 1..=19 {
            content.push_str(&format!("- 2.{}\n", i));
        }
        // 小节 3: 标题 + 24 行 = 25 行 (累计 65 行)
        // 前 60 行会截断在小节 3 的第 19 行内容处，小节 3 剩余 5 行超标
        content.push_str("## 小节 3\n");
        for i in 1..=24 {
            content.push_str(&format!("- 3.{}\n", i));
        }

        let (lines, block) = generate_agents_block(Some(&content));
        // 按新算法：从 c=60 递减到 1 查找合法切点：
        // 在 c=41..=60 处，filtered[c] 均为小节 3 的正文内容（以 "- 3." 开头），不满足下一行是边界条件；
        // 在 c=40 处，filtered[40] 为 "## 小节 3"（满足下一行是边界），且 filtered[39] 为 "- 2.19"（满足上一行非孤立标题）；
        // 故在 c=40 命中最大合法切点，小节 3（第 41~65 行）被丢弃，保留小节 1 (20 行) + 小节 2 (20 行) = 40 行内容。
        // 总行数 = 3 (头部) + 40 (内容) + 4 (尾部) = 47。
        assert_eq!(lines.len(), 47);
        assert!(block.contains("## 小节 1"));
        assert!(block.contains("## 小节 2"));
        assert!(block.contains("- 2.19"));
        assert!(!block.contains("## 小节 3"));
        assert!(!block.contains("- 3.1"));

        // 内容行不超过 60 行
        let content_lines = &lines[3..lines.len() - 4];
        assert_eq!(content_lines.len(), 40);
        assert!(content_lines.len() <= 60);
        // 最后一节是完整的：不以 "## " 标题结尾
        assert!(!content_lines.last().unwrap().starts_with("## "));
        // 且每个保留下来的小节标题后面至少有一行内容
        let mut last_was_header = false;
        for line in content_lines {
            if line.starts_with("## ") {
                assert!(!last_was_header, "连续标题，标题后没有内容");
                last_was_header = true;
            } else {
                last_was_header = false;
            }
        }
        assert!(!last_was_header, "最后一个保留行不能是标题");
    }

    #[test]
    fn test_sync_agents_block_truncation_at_exact_section_boundary() {
        let mut content = String::from("# 标题\n> 引用说明\n\n");
        // 小节 1: 标题 + 19 行 = 20 行
        content.push_str("## 小节 1\n");
        for i in 1..=19 {
            content.push_str(&format!("- 1.{}\n", i));
        }
        // 小节 2: 标题 + 19 行 = 20 行
        content.push_str("## 小节 2\n");
        for i in 1..=19 {
            content.push_str(&format!("- 2.{}\n", i));
        }
        // 小节 3: 标题 + 19 行 = 20 行 (前 3 节刚好 60 行)
        content.push_str("## 小节 3\n");
        for i in 1..=19 {
            content.push_str(&format!("- 3.{}\n", i));
        }
        // 小节 4: 标题 + 4 行 = 5 行 (总计 65 行)
        content.push_str("## 小节 4\n");
        for i in 1..=4 {
            content.push_str(&format!("- 4.{}\n", i));
        }

        let (lines, block) = generate_agents_block(Some(&content));
        // 小节 4 刚好从第 61 行开始，整节被丢弃；保留前 3 节共 60 行内容
        // 总行数 = 3 + 60 + 4 = 67
        assert_eq!(lines.len(), 67);
        assert!(block.contains("## 小节 1"));
        assert!(block.contains("## 小节 2"));
        assert!(block.contains("## 小节 3"));
        assert!(block.contains("- 3.19"));
        assert!(!block.contains("## 小节 4"));

        let content_lines = &lines[3..lines.len() - 4];
        assert_eq!(content_lines.len(), 60);
        assert!(!content_lines.last().unwrap().starts_with("## "));
    }

    #[test]
    fn test_sync_agents_block_no_empty_when_first_line_is_header() {
        let mut content = String::from("# 标题\n> 引用说明\n\n");
        // filtered 首行是 "## " 标题、单小节共 80 行正文（过滤后有效内容共 81 行）
        content.push_str("## 最近变化\n");
        for i in 1..=80 {
            content.push_str(&format!("- 变化记录 {}\n", i));
        }

        let (lines, block) = generate_agents_block(Some(&content));
        // 输出非空
        assert!(!block.is_empty());
        // 包含该首行标题
        assert!(block.contains("## 最近变化"));

        // 内容行不超过 60 行（由于后续无其他 "## " 标题，无合法小节切点，触发保底 c = 60）
        let content_lines = &lines[3..lines.len() - 4];
        assert!(content_lines.len() <= 60);
        assert_eq!(content_lines.len(), 60);
        // 总行数 = 3 (头部) + 60 (内容) + 4 (尾部) = 67
        assert_eq!(lines.len(), 67);
        // 断言保留了首行标题及前 59 行正文
        assert_eq!(content_lines[0], "## 最近变化");
        assert!(block.contains("- 变化记录 1"));
        assert!(block.contains("- 变化记录 59"));
        assert!(!block.contains("- 变化记录 60"));
    }

    #[test]
    fn test_sync_agents_block_no_headers_fallback_60() {
        let mut content = String::from("# 标题\n> 引用说明\n\n");
        // 没有任何 "## " 标题、共 70 行有效内容
        for i in 1..=70 {
            content.push_str(&format!("- 有效内容 {}\n", i));
        }

        let (lines, block) = generate_agents_block(Some(&content));
        // 输出非空
        assert!(!block.is_empty());

        // 正好保留前 60 行（触发保底 c = 60）
        let content_lines = &lines[3..lines.len() - 4];
        assert_eq!(content_lines.len(), 60);
        // 总行数 = 3 (头部) + 60 (内容) + 4 (尾部) = 67
        assert_eq!(lines.len(), 67);
        assert!(block.contains("- 有效内容 1"));
        assert!(block.contains("- 有效内容 60"));
        assert!(!block.contains("- 有效内容 61"));
    }

    #[test]
    fn test_sanitize_title_and_quote() {
        // T12 相关单元测试
        let title_with_quotes = extract_title("他说\"你好\"。第二句", "m_fallback");
        assert_eq!(title_with_quotes, "他说'你好'");

        let title_with_newline = sanitize_title("标题\n第二行", "m_fallback");
        assert_eq!(title_with_newline, "标题 第二行");

        let quote_with_newline = sanitize_quote("第一行\n第二行").unwrap();
        assert_eq!(quote_with_newline, "第一行 第二行");

        let quote_with_double_quotes = sanitize_quote("带有\"双引号\"的原话").unwrap();
        assert_eq!(quote_with_double_quotes, "带有\\\"双引号\\\"的原话");

        // 空 quote
        assert_eq!(sanitize_quote("   \r\n  "), None);

        // title 30 字符截断
        let long_title = "这是一个非常长非常长非常长非常长非常长非常长非常长超过三十个字的标题";
        let truncated = sanitize_title(long_title, "m_fallback");
        assert_eq!(truncated.chars().count(), 30);
    }

    #[test]
    fn test_raw_front_matter() {
        let good = "---\nid: test\ntime: 2026-10-31\n---\nbody";
        assert_eq!(raw_front_matter(good).unwrap(), "id: test\ntime: 2026-10-31");

        let bad = "no front matter";
        assert_eq!(raw_front_matter(bad), None);
    }
}
