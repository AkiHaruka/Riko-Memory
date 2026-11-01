// src/commands.rs — 12 个子命令实现

use crate::store::*;
use chrono::{DateTime, Duration};
use clap::{Args, Subcommand};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// 添加一条记忆
    Add(AddArgs),
    /// 追加内容到待办收集箱
    Inbox(InboxArgs),
    /// 搜索记忆条目
    Search(SearchArgs),
    /// 查看近期修改的文件
    Recent(RecentArgs),
    /// 查看指定条目的全文
    Reveal(RevealArgs),
    /// 查看未闭环事件列表
    Open,
    /// 查看记忆状态板
    Resident,
    /// 查看记忆索引前 N 行
    Summary(SummaryArgs),
    /// 检查记忆条目格式规范
    Lint,
    /// 同步状态板到 AGENTS.md
    SyncAgents,
    /// 列出所有条目与分类统计
    Index,
    /// 追加变更日志
    Changelog(ChangelogArgs),
    /// 后台记忆整理
    Dream(crate::dream::DreamArgs),
    /// 会话事实捕获
    Facts(crate::facts::FactsArgs),
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// 记忆正文
    #[arg(long, required = true)]
    pub text: String,

    /// 类别 (moment, person, self)
    #[arg(long, default_value = "moment")]
    pub r#type: String,

    /// 引用原话
    #[arg(long)]
    pub quote: Option<String>,

    /// 来源渠道
    #[arg(long)]
    pub source: Option<String>,

    /// 标签列表，逗号切分
    #[arg(long)]
    pub tags: Option<String>,

    /// 发生时间 (ISO 格式)
    #[arg(long)]
    pub time: Option<String>,

    /// 自定义标题
    #[arg(long)]
    pub title: Option<String>,
}

#[derive(Args, Debug)]
pub struct InboxArgs {
    /// 候选内容
    pub text: String,

    /// 来源描述
    #[arg(long)]
    pub source: Option<String>,
}

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// 搜索关键词
    #[arg(required = true)]
    pub terms: Vec<String>,

    /// 按类别筛选 (moment, person, self, inbox)
    #[arg(long)]
    pub r#type: Option<String>,

    /// 按标签筛选
    #[arg(long)]
    pub tag: Option<String>,

    /// 只看 N 天内发生的条目
    #[arg(long)]
    pub days: Option<i64>,

    /// 返回条数上限
    #[arg(long, default_value_t = 12)]
    pub limit: usize,
}

#[derive(Args, Debug)]
pub struct RecentArgs {
    /// 最近 N 天内修改过的文件
    #[arg(long, default_value_t = 7)]
    pub days: i64,

    /// 按类别筛选
    #[arg(long)]
    pub r#type: Option<String>,
}

#[derive(Args, Debug)]
pub struct RevealArgs {
    /// 条目 ID、完整文件名或路径片段
    pub key: String,
}

#[derive(Args, Debug)]
pub struct SummaryArgs {
    /// 查看行数
    #[arg(long, default_value_t = 60)]
    pub lines: usize,
}

#[derive(Args, Debug)]
pub struct ChangelogArgs {
    /// 日志内容
    pub text: String,
}

/// 执行 add 命令
pub fn cmd_add(args: AddArgs) {
    let typ = args.r#type.trim();
    if typ != "moment" && typ != "person" && typ != "self" {
        eprintln!("type 只支持: moment, person, self");
        std::process::exit(1);
    }

    let time_dt = match args.time {
        Some(ref t_str) => match parse_time_opt(t_str) {
            Ok(dt) => dt,
            Err(e) => {
                eprintln!("{}", e);
                std::process::exit(1);
            }
        },
        None => now_beijing(),
    };

    let base = mem_dir();
    let (path, id) = match next_entry_path(&base, typ, &time_dt) {
        Ok(res) => res,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    let time_str = format_rfc3339(&time_dt);
    let src = args.source.unwrap_or_else(|| "manual".to_string());

    let tags_list: Vec<&str> = args
        .tags
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let tags_formatted = format!("[{}]", tags_list.join(", "));

    let quote_line = match args.quote {
        Some(ref q) => match sanitize_quote(q) {
            Some(escaped) => format!("quote: \"{}\"\n", escaped),
            None => String::new(),
        },
        None => String::new(),
    };

    let body = args.text.trim();
    let title = match args.title {
        Some(ref t) => sanitize_title(t, &id),
        None => extract_title(body, &id),
    };

    // 字段顺序固定: id, time, type, title, source, [quote 可选], status, tags
    let content = format!(
        "---\nid: {}\ntime: {}\ntype: {}\ntitle: {}\nsource: {}\n{}status: active\ntags: {}\n---\n\n{}\n",
        id, time_str, typ, title, src, quote_line, tags_formatted, body
    );

    if let Err(e) = fs::write(&path, content) {
        eprintln!("写入文件失败: {}", e);
        std::process::exit(1);
    }

    println!("{}", path.display());
}

/// 执行 inbox 命令
pub fn cmd_inbox(args: InboxArgs) {
    let t = now_beijing();
    let base = mem_dir();
    let inbox_dir = base.join("inbox");
    if let Err(e) = fs::create_dir_all(&inbox_dir) {
        eprintln!("创建 inbox 目录失败: {}", e);
        std::process::exit(1);
    }

    let file_name = format!("{}.md", t.format("%Y-%m-%d"));
    let path = inbox_dir.join(file_name);

    let src = args.source.unwrap_or_else(|| "manual".to_string());
    let line = format!(
        "\n- [ ] {} — {}（来源：{}）\n",
        format_rfc3339(&t),
        args.text.trim(),
        src
    );

    let mut file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("打开 inbox 文件失败: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = file.write_all(line.as_bytes()) {
        eprintln!("写入 inbox 失败: {}", e);
        std::process::exit(1);
    }

    println!("{}", path.display());
}

/// 执行 search 命令
pub fn cmd_search(args: SearchArgs) {
    let terms: Vec<String> = args
        .terms
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if terms.is_empty() {
        eprintln!("需要 query");
        std::process::exit(1);
    }

    let base = mem_dir();
    let cutoff = args.days.map(|d| now_beijing() - Duration::days(d));

    let all_files = walk_md_files(&base);
    let mut filtered_files: Vec<PathBuf> = Vec::new();

    for p in all_files {
        let p_str = p.to_string_lossy();

        if let Some(ref typ) = args.r#type {
            let filter_dir = match typ.as_str() {
                "moment" => "/moments/",
                "person" => "/people/",
                "self" => "/self/",
                "inbox" => "/inbox/",
                _ => "",
            };
            if !filter_dir.is_empty() && !p_str.contains(filter_dir) {
                continue;
            }
        }

        let (fm, _) = read_file_front(&p);

        if let Some(ref tag) = args.tag {
            let tags_val = fm.get("tags").cloned().unwrap_or_default();
            if !tags_val.contains(tag) {
                continue;
            }
        }

        if let Some(ref cut) = cutoff {
            let time_val = fm.get("time").cloned().unwrap_or_default();
            match parse_time_opt(&time_val) {
                Ok(dt) if dt < *cut => continue,
                Err(_) => continue,
                _ => {}
            }
        }

        filtered_files.push(p);
    }

    let mut scored: Vec<(usize, PathBuf)> = Vec::new();
    for p in &filtered_files {
        let (fm, body) = read_file_front(p);
        let hit = score_content(fm.get("quote").map(|s| s.as_str()), &body, &terms);
        if hit > 0 {
            scored.push((hit, p.clone()));
        }
    }

    // 有命中条目：按 hit 降序，路径升序排
    if !scored.is_empty() {
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    } else {
        // 回退成 OR 匹配，在过滤后的集合中匹配，按文件 mtime 降序排，hit 显示为空
        let mut fallback: Vec<(SystemTime, PathBuf)> = Vec::new();
        for p in &filtered_files {
            let (fm, body) = read_file_front(p);
            if matches_any_term(fm.get("quote").map(|s| s.as_str()), &body, &terms) {
                let mtime = fs::metadata(p)
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                fallback.push((mtime, p.clone()));
            }
        }
        fallback.sort_by(|a, b| b.0.cmp(&a.0));
        scored = fallback.into_iter().map(|(_, p)| (0, p)).collect();
    }

    for (h, p) in scored.into_iter().take(args.limit) {
        let (fm, _) = read_file_front(&p);
        let rel = p
            .strip_prefix(&base)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| p.to_string_lossy().to_string());
        let time_disp = fm.get("time").map(|s| s.as_str()).unwrap_or("?");
        let title_disp = title_of(&p);
        let hit_disp = if h > 0 {
            format!("(hit {})", h)
        } else {
            String::new()
        };
        println!("{}\t{}\t{}\t{}", rel, time_disp, title_disp, hit_disp);
    }
}

/// 执行 recent 命令
pub fn cmd_recent(args: RecentArgs) {
    let base = mem_dir();
    let cutoff_system_time = SystemTime::now()
        - std::time::Duration::from_secs((args.days.max(0) as u64) * 86400);

    let all_files = walk_md_files(&base);
    let mut rows: Vec<(SystemTime, PathBuf)> = Vec::new();

    for p in all_files {
        let p_str = p.to_string_lossy();
        if !p_str.contains("/moments/")
            && !p_str.contains("/people/")
            && !p_str.contains("/self/")
            && !p_str.contains("/inbox/")
        {
            continue;
        }

        if let Some(ref typ) = args.r#type {
            let filter_dir = match typ.as_str() {
                "moment" => "/moments/",
                "person" => "/people/",
                "self" => "/self/",
                "inbox" => "/inbox/",
                _ => "",
            };
            if !filter_dir.is_empty() && !p_str.contains(filter_dir) {
                continue;
            }
        }

        if let Ok(meta) = fs::metadata(&p) {
            if let Ok(mtime) = meta.modified() {
                if mtime >= cutoff_system_time {
                    rows.push((mtime, p));
                }
            }
        }
    }

    // 按 mtime 降序排
    rows.sort_by(|a, b| b.0.cmp(&a.0));

    for (_, p) in rows {
        let (fm, _) = read_file_front(&p);
        let time_raw = fm.get("time").map(|s| s.as_str()).unwrap_or("?");
        let time_disp = time_raw
            .chars()
            .take(16)
            .collect::<String>()
            .replace('T', " ");

        let rel = p
            .strip_prefix(&base)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| p.to_string_lossy().to_string());
        let title_disp = title_of(&p);
        println!("{}\t{}\t{}", time_disp, rel, title_disp);
    }
}

/// 执行 reveal 命令
pub fn cmd_reveal(args: RevealArgs) {
    let base = mem_dir();
    let all_files = walk_md_files(&base);
    let key = &args.key;

    // 优先级 1: 文件名去 .md 等于 KEY
    for p in &all_files {
        if let Some(stem) = p.file_stem() {
            if stem.to_string_lossy() == *key {
                if let Ok(content) = fs::read_to_string(p) {
                    print!("{}", content);
                    return;
                }
            }
        }
    }

    // 优先级 2: 完整文件名等于 KEY
    for p in &all_files {
        if let Some(name) = p.file_name() {
            if name.to_string_lossy() == *key {
                if let Ok(content) = fs::read_to_string(p) {
                    print!("{}", content);
                    return;
                }
            }
        }
    }

    // 优先级 3: 路径包含 KEY
    for p in &all_files {
        if p.to_string_lossy().contains(key) {
            if let Ok(content) = fs::read_to_string(p) {
                print!("{}", content);
                return;
            }
        }
    }

    eprintln!("找不到: {}", key);
    std::process::exit(1);
}

/// 执行 open 命令
pub fn cmd_open() {
    let p = mem_dir().join("open-loops.md");
    if p.exists() {
        match fs::read_to_string(&p) {
            Ok(content) => print!("{}", content),
            Err(_) => println!("（没有 open-loops.md）"),
        }
    } else {
        println!("（没有 open-loops.md）");
    }
}

/// 执行 resident 命令
pub fn cmd_resident() {
    let p = mem_dir().join("resident.md");
    if !p.exists() {
        eprintln!("没有 resident.md");
        std::process::exit(1);
    }
    match fs::read_to_string(&p) {
        Ok(content) => print!("{}", content),
        Err(e) => {
            eprintln!("读取 resident.md 失败: {}", e);
            std::process::exit(1);
        }
    }
}

/// 执行 summary 命令
pub fn cmd_summary(args: SummaryArgs) {
    let p = mem_dir().join("MEMORY.md");
    if !p.exists() {
        eprintln!("没有 MEMORY.md");
        std::process::exit(1);
    }
    let content = match fs::read_to_string(&p) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("读取 MEMORY.md 失败: {}", e);
            std::process::exit(1);
        }
    };
    let lines: Vec<&str> = content.lines().take(args.lines).collect();
    println!("{}", lines.join("\n"));
}

/// 执行 lint 命令
pub fn cmd_lint() {
    let base = mem_dir();
    let all_files = walk_md_files(&base);
    let skip_names = [
        "RULES.md",
        "CHANGELOG.md",
        "MEMORY.md",
        "open-loops.md",
        "resident.md",
        "README.md",
        "AGENTS.md",
    ];

    let mut bad = 0;
    for p in &all_files {
        let p_str = p.to_string_lossy();
        if p_str.contains("/inbox/") {
            continue;
        }
        if let Some(name) = p.file_name() {
            if skip_names.contains(&name.to_string_lossy().as_ref()) {
                continue;
            }
        }

        let content = match fs::read_to_string(p) {
            Ok(c) => c,
            Err(_) => {
                println!("无 front-matter: {}", p.display());
                bad += 1;
                continue;
            }
        };

        let fm_raw = match raw_front_matter(&content) {
            Some(raw) => raw,
            None => {
                println!("无 front-matter: {}", p.display());
                bad += 1;
                continue;
            }
        };

        // 检查 front-matter 块内每一行是否包含冒号
        for line in fm_raw.lines() {
            let trimmed = line.trim();
            if !trimmed.contains(':') {
                println!("front-matter 行无法解析: {} ({})", p.display(), trimmed);
                bad += 1;
            }
        }

        let (fm, _) = read_front_matter(&content);
        if fm.is_empty() {
            println!("无 front-matter: {}", p.display());
            bad += 1;
            continue;
        }

        let mut has_missing = false;
        for k in &["id", "time", "type", "source", "status"] {
            if !fm.contains_key(*k) || fm.get(*k).map(|s| s.trim().is_empty()).unwrap_or(true) {
                println!("缺 {}: {}", k, p.display());
                bad += 1;
                if *k == "time" {
                    has_missing = true;
                }
            }
        }

        if !has_missing {
            let time_val = fm.get("time").cloned().unwrap_or_default();
            let trimmed_time = time_val.trim();
            if trimmed_time.ends_with("Unknown") {
                println!("time 为 Unknown: {}", p.display());
                bad += 1;
            } else if DateTime::parse_from_rfc3339(trimmed_time).is_err() {
                println!("time 不合法: {} ({:?})", p.display(), time_val);
                bad += 1;
            }
        }
    }

    println!("lint: {} 处问题", bad);
}

/// 执行 sync-agents 命令
pub fn cmd_sync_agents() {
    let mem_base = mem_dir();
    let rp = mem_base.join("resident.md");
    let resident_content = fs::read_to_string(&rp).ok();

    let (block_lines, new_block) = generate_agents_block(resident_content.as_deref());

    let ap = agents_path();
    let original_agents = fs::read_to_string(&ap).unwrap_or_default();
    let updated_agents = replace_or_append_agents_block(&original_agents, &new_block);

    if let Err(e) = fs::write(&ap, updated_agents) {
        eprintln!("写入 AGENTS.md 失败: {}", e);
        std::process::exit(1);
    }

    println!("AGENTS.md 状态板已更新（{} 行）", block_lines.len());
}

/// 执行 index 命令
pub fn cmd_index() {
    let base = mem_dir();
    let all_files = walk_md_files(&base);

    println!("== 文件 ==");
    for p in &all_files {
        let rel = p
            .strip_prefix(&base)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| p.to_string_lossy().to_string());
        println!("{}", rel);
    }

    println!("\n== 统计 ==");
    let mut moments = 0;
    let mut people = 0;
    let mut self_cnt = 0;
    let mut inbox = 0;

    for p in &all_files {
        let s = p.to_string_lossy();
        if s.contains("/moments/") {
            moments += 1;
        }
        if s.contains("/people/") {
            people += 1;
        }
        if s.contains("/self/") {
            self_cnt += 1;
        }
        if s.contains("/inbox/") {
            inbox += 1;
        }
    }

    println!(
        "moments: {}｜people: {}｜self: {}｜inbox: {}",
        moments, people, self_cnt, inbox
    );
}

/// 执行 changelog 命令
pub fn cmd_changelog(args: ChangelogArgs) {
    let base = mem_dir();
    let p = base.join("CHANGELOG.md");
    let t = now_beijing();
    let time_str = t.format("%Y-%m-%d %H:%M").to_string();
    let line = format!("- {} {}\n", time_str, args.text.trim());

    let mut file = match OpenOptions::new().create(true).append(true).open(&p) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("打开 CHANGELOG.md 失败: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = file.write_all(line.as_bytes()) {
        eprintln!("写入 CHANGELOG.md 失败: {}", e);
        std::process::exit(1);
    }

    println!("{}", p.display());
}

/// 执行 dream 命令
pub fn cmd_dream(args: crate::dream::DreamArgs) {
    crate::dream::cmd_dream(args);
}

/// 执行 facts 命令
pub fn cmd_facts(args: crate::facts::FactsArgs) {
    crate::facts::cmd_facts(args);
}
