// tests/integration_tests.rs — riko-memory 命令行端到端集成测试

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn get_bin() -> &'static str {
    env!("CARGO_BIN_EXE_riko-memory")
}

fn create_clean_dir(name: &str) -> PathBuf {
    let dir = env::temp_dir().join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_cli_add_and_increment() {
    let test_dir = create_clean_dir("riko_test_cli_add");
    let bin = get_bin();

    // T1: add 一条带 quote/tags/time
    let output = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args([
            "add",
            "--text",
            "这是第一条测试记录。后面还有一句。",
            "--type",
            "moment",
            "--quote",
            "引用的原话",
            "--source",
            "unit-test",
            "--tags",
            "tag1, tag2",
            "--time",
            "2026-10-31T15:56:00+08:00",
            "--title",
            "自定义标题",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let path_str = stdout.trim();
    assert!(path_str.ends_with("m_20261031_01.md"));
    assert!(PathBuf::from(path_str).exists());

    let file_content = fs::read_to_string(path_str).unwrap();
    assert!(file_content.contains("id: m_20261031_01"));
    assert!(file_content.contains("time: 2026-10-31T15:56:00+08:00"));
    assert!(file_content.contains("type: moment"));
    assert!(file_content.contains("title: 自定义标题"));
    assert!(file_content.contains("source: unit-test"));
    assert!(file_content.contains("quote: \"引用的原话\""));
    assert!(file_content.contains("status: active"));
    assert!(file_content.contains("tags: [tag1, tag2]"));
    assert!(file_content.contains("这是第一条测试记录。后面还有一句。"));

    // T2: 同一天连 add 第二条
    let output2 = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args([
            "add",
            "--text",
            "第二条测试内容",
            "--time",
            "2026-10-31T16:00:00+08:00",
        ])
        .output()
        .unwrap();

    assert!(output2.status.success());
    let stdout2 = String::from_utf8(output2.stdout).unwrap();
    let path_str2 = stdout2.trim();
    assert!(path_str2.ends_with("m_20261031_02.md"));
    assert!(PathBuf::from(path_str2).exists());

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cli_inbox_and_changelog() {
    let test_dir = create_clean_dir("riko_test_cli_inbox");
    let bin = get_bin();

    // inbox 命令
    let output = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["inbox", "待办事项一", "--source", "tg"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let inbox_file = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    assert!(inbox_file.exists());
    let inbox_content = fs::read_to_string(&inbox_file).unwrap();
    assert!(inbox_content.starts_with("\n- [ ] "));
    assert!(inbox_content.contains("待办事项一（来源：tg）"));

    // changelog 命令
    let output_cl = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["changelog", "重构核心模块完成"])
        .output()
        .unwrap();

    assert!(output_cl.status.success());
    let cl_file = PathBuf::from(String::from_utf8(output_cl.stdout).unwrap().trim());
    assert!(cl_file.exists());
    let cl_content = fs::read_to_string(&cl_file).unwrap();
    assert!(cl_content.contains("- "));
    assert!(cl_content.contains("重构核心模块完成"));

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cli_search_and_ranking() {
    let test_dir = create_clean_dir("riko_test_cli_search");
    let bin = get_bin();

    // 建立 3 条：一条同时含 AB、一条只含 A、一条只含 B
    Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args([
            "add",
            "--text",
            "苹果和香蕉混合汁，苹果香蕉都好吃",
            "--time",
            "2026-10-31T10:00:00+08:00",
            "--title",
            "条目AB",
        ])
        .output()
        .unwrap();

    Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args([
            "add",
            "--text",
            "单纯的苹果汁",
            "--time",
            "2026-10-31T11:00:00+08:00",
            "--title",
            "条目A",
        ])
        .output()
        .unwrap();

    Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args([
            "add",
            "--text",
            "单纯的香蕉牛奶",
            "--time",
            "2026-10-31T12:00:00+08:00",
            "--title",
            "条目B",
        ])
        .output()
        .unwrap();

    // 搜索 苹果 香蕉
    let search_out = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["search", "苹果", "香蕉"])
        .output()
        .unwrap();

    assert!(search_out.status.success());
    let lines: Vec<String> = String::from_utf8(search_out.stdout)
        .unwrap()
        .lines()
        .map(|s| s.to_string())
        .collect();

    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("条目AB"));
    assert!(lines[0].contains("(hit 4)"));

    // 过滤不存在的 tag（未命中，受过滤器限制输出为空，exit 0）
    let tag_none_out = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["search", "香蕉", "--tag", "不存在的标签"])
        .output()
        .unwrap();

    assert!(tag_none_out.status.success());
    let tag_stdout = String::from_utf8(tag_none_out.stdout).unwrap();
    assert!(tag_stdout.trim().is_empty());

    // 搜索全库不存在的词（exit 0，输出为空）
    let none_out = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["search", "绝不存在的关键词"])
        .output()
        .unwrap();
    assert!(none_out.status.success());
    assert!(String::from_utf8(none_out.stdout).unwrap().trim().is_empty());

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cli_reveal_open_resident_summary() {
    let test_dir = create_clean_dir("riko_test_cli_reveal");
    let bin = get_bin();

    Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args([
            "add",
            "--text",
            "用于测试 reveal 的正文",
            "--time",
            "2026-10-31T10:00:00+08:00",
            "--title",
            "Reveal测试条目",
        ])
        .output()
        .unwrap();

    // 1. reveal id
    let out1 = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["reveal", "m_20261031_01"])
        .output()
        .unwrap();
    assert!(out1.status.success());
    assert!(String::from_utf8(out1.stdout).unwrap().contains("用于测试 reveal 的正文"));

    // 2. reveal 文件名
    let out2 = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["reveal", "m_20261031_01.md"])
        .output()
        .unwrap();
    assert!(out2.status.success());
    assert!(String::from_utf8(out2.stdout).unwrap().contains("用于测试 reveal 的正文"));

    // 3. reveal 找不到
    let out3 = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["reveal", "non_existing_key"])
        .output()
        .unwrap();
    assert_eq!(out3.status.code(), Some(1));
    assert!(String::from_utf8(out3.stderr).unwrap().contains("找不到: non_existing_key"));

    // 4. open 不存在
    let out_open = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["open"])
        .output()
        .unwrap();
    assert!(out_open.status.success());
    assert!(String::from_utf8(out_open.stdout).unwrap().contains("（没有 open-loops.md）"));

    // 5. resident 不存在
    let out_res = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["resident"])
        .output()
        .unwrap();
    assert_eq!(out_res.status.code(), Some(1));
    assert!(String::from_utf8(out_res.stderr).unwrap().contains("没有 resident.md"));

    // 6. summary 不存在
    let out_sum = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["summary"])
        .output()
        .unwrap();
    assert_eq!(out_sum.status.code(), Some(1));
    assert!(String::from_utf8(out_sum.stderr).unwrap().contains("没有 MEMORY.md"));

    let _ = fs::remove_dir_all(&test_dir);
}


#[test]
fn test_cli_lint_clean_and_dirty() {
    let test_dir = create_clean_dir("riko_test_cli_lint");
    let bin = get_bin();

    // 建立一条合法条目
    Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args([
            "add",
            "--text",
            "干净条目",
            "--time",
            "2026-10-31T10:00:00+08:00",
        ])
        .output()
        .unwrap();

    // 干净 fixture: lint 应报告 0 处问题
    let lint1 = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["lint"])
        .output()
        .unwrap();
    assert!(lint1.status.success());
    let stdout1 = String::from_utf8(lint1.stdout).unwrap();
    assert!(stdout1.contains("lint: 0 处问题"));

    // 塞入一条缺 time 的文件
    let bad_file = test_dir.join("moments/2026-10/bad.md");
    fs::write(
        &bad_file,
        "---\nid: m_bad\ntype: moment\ntitle: 缺时间\nsource: manual\nstatus: active\ntags: []\n---\n\n内容\n",
    )
    .unwrap();

    // 坏条: lint 应报告 1 处问题
    let lint2 = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["lint"])
        .output()
        .unwrap();
    assert!(lint2.status.success());
    let stdout2 = String::from_utf8(lint2.stdout).unwrap();
    assert!(stdout2.contains("缺 time:"));
    assert!(stdout2.contains("lint: 1 处问题"));

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cli_sync_agents_and_index() {
    let test_dir = create_clean_dir("riko_test_cli_sync");
    let bin = get_bin();

    let resident_file = test_dir.join("resident.md");
    fs::write(
        &resident_file,
        "# 标题1\n> 引用1\n核心状态行A\n核心状态行B\n\n",
    )
    .unwrap();

    let agents_file = test_dir.join("AGENTS.md");
    fs::write(
        &agents_file,
        "# 系统说明\n\n<!-- memory-index:begin（旧） -->\n旧状态\n<!-- memory-index:end -->\n\n末尾说明\n",
    )
    .unwrap();

    let sync_out = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .env("RIKO_AGENTS_PATH", &agents_file)
        .args(["sync-agents"])
        .output()
        .unwrap();

    assert!(sync_out.status.success());
    let updated_agents = fs::read_to_string(&agents_file).unwrap();
    assert!(updated_agents.contains("核心状态行A"));
    assert!(updated_agents.contains("核心状态行B"));
    assert!(!updated_agents.contains("# 标题1"));
    assert!(!updated_agents.contains("> 引用1"));
    assert!(!updated_agents.contains("旧状态"));

    // index 命令
    let idx_out = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["index"])
        .output()
        .unwrap();

    assert!(idx_out.status.success());
    let idx_stdout = String::from_utf8(idx_out.stdout).unwrap();
    assert!(idx_stdout.contains("== 文件 =="));
    assert!(idx_stdout.contains("== 统计 =="));
    assert!(idx_stdout.contains("moments: 0｜people: 0｜self: 0｜inbox: 0"));

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cli_t4b_search_fallback_with_filter() {
    // delta §1 新增验收 T4b：
    // people/ 下有「apple 的记录」，moments/ 下无 apple；
    // riko-memory search apple --type moment 必须无输出（exit 0）；
    // search apple --type person 必须列出该 people 文件。
    let test_dir = create_clean_dir("riko_test_cli_t4b");
    let bin = get_bin();

    // 在 people/ 下建一条包含 apple 的记录
    let add_person = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args([
            "add",
            "--text",
            "这是关于 apple 的记录内容",
            "--type",
            "person",
            "--time",
            "2026-10-31T10:00:00+08:00",
            "--title",
            "乔布斯与apple",
        ])
        .output()
        .unwrap();
    assert!(add_person.status.success());

    // 在 moments/ 下建一条不含 apple 的普通记录
    let add_moment = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args([
            "add",
            "--text",
            "今天天气晴朗，散步了一小时",
            "--type",
            "moment",
            "--time",
            "2026-10-31T11:00:00+08:00",
            "--title",
            "秋日散步",
        ])
        .output()
        .unwrap();
    assert!(add_moment.status.success());

    // 1. search apple --type moment 必须无输出（exit 0）
    let out_moment = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["search", "apple", "--type", "moment"])
        .output()
        .unwrap();
    assert!(out_moment.status.success());
    let stdout_m = String::from_utf8(out_moment.stdout).unwrap();
    assert_eq!(stdout_m.trim(), "", "search apple --type moment 必须无输出");

    // 2. search apple --type person 必须列出该 people 文件
    let out_person = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["search", "apple", "--type", "person"])
        .output()
        .unwrap();
    assert!(out_person.status.success());
    let stdout_p = String::from_utf8(out_person.stdout).unwrap();
    assert!(stdout_p.contains("people/2026-10/m_20261031_01.md"));
    assert!(stdout_p.contains("乔布斯与apple"));

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cli_t12_title_and_quote_safety() {
    // delta §2 新增验收 T12：
    // add --text '他说"你好"。第二句' 的 title 必须是 他说'你好'；
    // add --quote $'第一行\n第二行' 的 quote 必须是一行 第一行 第二行；
    // add --title $'标题\n第二行' 的 title 必须是 标题 第二行；
    // 三个文件都能被 lint 通过。
    let test_dir = create_clean_dir("riko_test_cli_t12");
    let bin = get_bin();

    // 1. add --text '他说"你好"。第二句'
    let out1 = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["add", "--text", "他说\"你好\"。第二句"])
        .output()
        .unwrap();
    assert!(out1.status.success());
    let path1 = PathBuf::from(String::from_utf8(out1.stdout).unwrap().trim());
    let c1 = fs::read_to_string(&path1).unwrap();
    assert!(c1.contains("title: 他说'你好'"), "title 必须是 他说'你好'");

    // 2. add --quote $'第一行\n第二行'
    let out2 = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["add", "--text", "正文测试2", "--quote", "第一行\n第二行"])
        .output()
        .unwrap();
    assert!(out2.status.success());
    let path2 = PathBuf::from(String::from_utf8(out2.stdout).unwrap().trim());
    let c2 = fs::read_to_string(&path2).unwrap();
    assert!(c2.contains("quote: \"第一行 第二行\"\n"), "quote 必须是一行 第一行 第二行");

    // 3. add --title $'标题\n第二行'
    let out3 = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["add", "--text", "正文测试3", "--title", "标题\n第二行"])
        .output()
        .unwrap();
    assert!(out3.status.success());
    let path3 = PathBuf::from(String::from_utf8(out3.stdout).unwrap().trim());
    let c3 = fs::read_to_string(&path3).unwrap();
    assert!(c3.contains("title: 标题 第二行"), "title 必须是 标题 第二行");

    // 4. 三个文件都能被 lint 通过
    let lint_out = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["lint"])
        .output()
        .unwrap();
    assert!(lint_out.status.success());
    let lint_stdout = String::from_utf8(lint_out.stdout).unwrap();
    assert!(lint_stdout.contains("lint: 0 处问题"), "三个文件都能被 lint 通过");

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cli_t13_lint_front_matter_line_check() {
    // delta §3 新增验收 T13：
    // 手写一个 front-matter 里带多行值的文件，lint 必须至少报 1 条 front-matter 行无法解析。
    let test_dir = create_clean_dir("riko_test_cli_t13");
    let bin = get_bin();

    let moment_dir = test_dir.join("moments/2026-10");
    fs::create_dir_all(&moment_dir).unwrap();
    let bad_file = moment_dir.join("bad_multiline.md");

    let bad_content = r#"---
id: m_20261031_99
time: 2026-10-31T15:56:00+08:00
type: moment
title: 第一行
第二行不含冒号
source: manual
status: active
tags: [test]
---

正文
"#;
    fs::write(&bad_file, bad_content).unwrap();

    let lint_out = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["lint"])
        .output()
        .unwrap();
    assert!(lint_out.status.success());
    let stdout = String::from_utf8(lint_out.stdout).unwrap();
    assert!(
        stdout.contains("front-matter 行无法解析:"),
        "必须报出 front-matter 行无法解析"
    );
    assert!(
        stdout.contains("(第二行不含冒号)"),
        "必须包含该行内容"
    );
    assert!(
        stdout.contains("lint: 1 处问题"),
        "计入问题数"
    );

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cli_t14_sync_agents_empty_agents_md() {
    // delta §4 新增验收 T14：
    // 空 AGENTS.md 跑 sync-agents 后，文件第一行必须是 <!-- memory-index:begin（由 riko-memory sync-agents 生成，勿手改） -->
    let test_dir = create_clean_dir("riko_test_cli_t14");
    let bin = get_bin();

    let res_file = test_dir.join("resident.md");
    fs::write(&res_file, "核心状态追踪").unwrap();

    let agents_file = test_dir.join("AGENTS.md");
    fs::write(&agents_file, "").unwrap(); // 空文件

    let sync_out = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .env("RIKO_AGENTS_PATH", &agents_file)
        .args(["sync-agents"])
        .output()
        .unwrap();
    assert!(sync_out.status.success());

    let agents_content = fs::read_to_string(&agents_file).unwrap();
    let first_line = agents_content.lines().next().unwrap_or("");
    assert_eq!(
        first_line,
        "<!-- memory-index:begin（由 riko-memory sync-agents 生成，勿手改） -->",
        "空 AGENTS.md 跑 sync-agents 后第一行必须是 begin 标记，且绝无前导空行"
    );
    assert!(!agents_content.starts_with('\n'), "不能有前导空行");

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cli_t15_lint_unknown_check_order() {
    // delta v1.1b §1 新增验收 T15：
    // 造一个 time 为 2026-10-31T15:56:00+08:00 Unknown 的条目，lint 必须报 time 为 Unknown（不再报不合法）。
    let test_dir = create_clean_dir("riko_test_cli_t15");
    let bin = get_bin();

    let moment_dir = test_dir.join("moments/2026-10");
    fs::create_dir_all(&moment_dir).unwrap();
    let unknown_file = moment_dir.join("m_unknown.md");
    fs::write(
        &unknown_file,
        r#"---
id: m_unknown
time: 2026-10-31T15:56:00+08:00 Unknown
type: moment
title: 测试Unknown
source: manual
status: active
tags: []
---

正文内容
"#,
    )
    .unwrap();

    let lint_out = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["lint"])
        .output()
        .unwrap();

    assert!(lint_out.status.success());
    let stdout = String::from_utf8(lint_out.stdout).unwrap();
    assert!(
        stdout.contains("time 为 Unknown:"),
        "必须报告 time 为 Unknown"
    );
    assert!(
        !stdout.contains("time 不合法:"),
        "绝不能再报告 time 不合法"
    );
    assert!(
        stdout.contains("lint: 1 处问题"),
        "问题总数应为 1"
    );

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cli_t16_recent_char_slice_no_panic() {
    // delta v1.1b §2 新增验收 T16：
    // 手写一个 time 值为 未知时间2026-10-31T15:56:00+08:00 的条目，recent --days 30 必须不 panic、正常输出（时间列显示前 16 个字符）。
    let test_dir = create_clean_dir("riko_test_cli_t16");
    let bin = get_bin();

    let moment_dir = test_dir.join("moments/2026-10");
    fs::create_dir_all(&moment_dir).unwrap();
    let t16_file = moment_dir.join("m_t16.md");
    fs::write(
        &t16_file,
        r#"---
id: m_t16
time: 未知时间2026-10-31T15:56:00+08:00
type: moment
title: 测试最近时间截取
source: manual
status: active
tags: []
---

正文内容
"#,
    )
    .unwrap();

    let recent_out = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["recent", "--days", "30"])
        .output()
        .unwrap();

    assert!(recent_out.status.success(), "recent 命令不能 panic 崩溃");
    let stdout = String::from_utf8(recent_out.stdout).unwrap();
    // 前 16 个字符：未知时间2026-10-31 1（第 15 个字符 T 替换为空格）
    assert!(
        stdout.contains("未知时间2026-10-31 1\tmoments/2026-10/m_t16.md\t测试最近时间截取"),
        "输出应包含前 16 个字符并替换 T 为空格: {}",
        stdout
    );

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cli_t17_title_of_no_truncation() {
    // delta v1.1b §3 新增验收 T17：
    // 手写一条 title 长度 120 个字符的条目，search 和 recent 输出的标题必须是完整 120 字符。
    let test_dir = create_clean_dir("riko_test_cli_t17");
    let bin = get_bin();

    // 构造长度恰为 120 个字符的标题
    let title_120 = "一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十";
    assert_eq!(title_120.chars().count(), 120);

    let moment_dir = test_dir.join("moments/2026-10");
    fs::create_dir_all(&moment_dir).unwrap();
    let t17_file = moment_dir.join("m_t17.md");
    fs::write(
        &t17_file,
        format!(
            r#"---
id: m_t17
time: 2026-10-31T15:56:00+08:00
type: moment
title: {}
source: manual
status: active
tags: []
---

测试全文搜索关键词匹配
"#,
            title_120
        ),
    )
    .unwrap();

    // 1. 测试 search 输出完整 120 字符
    let search_out = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["search", "关键词"])
        .output()
        .unwrap();

    assert!(search_out.status.success());
    let search_stdout = String::from_utf8(search_out.stdout).unwrap();
    let search_line = search_stdout.lines().next().expect("search 应有输出行");
    let search_cols: Vec<&str> = search_line.split('\t').collect();
    assert!(search_cols.len() >= 3, "search 输出格式为 path\ttime\ttitle\thit");
    assert_eq!(
        search_cols[2], title_120,
        "search 输出的标题必须完整保留 120 字符，不应被截断"
    );

    // 2. 测试 recent 输出完整 120 字符
    let recent_out = Command::new(bin)
        .env("RIKO_MEM_DIR", &test_dir)
        .args(["recent", "--days", "30"])
        .output()
        .unwrap();

    assert!(recent_out.status.success());
    let recent_stdout = String::from_utf8(recent_out.stdout).unwrap();
    let recent_line = recent_stdout.lines().next().expect("recent 应有输出行");
    let recent_cols: Vec<&str> = recent_line.split('\t').collect();
    assert!(recent_cols.len() >= 3, "recent 输出格式为 time\tpath\ttitle");
    assert_eq!(
        recent_cols[2], title_120,
        "recent 输出的标题必须完整保留 120 字符，不应被截断"
    );

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cli_t18_index_sigpipe_no_panic() {
    // delta v1.1c §1 新增验收 T18：
    // 造 200 条条目让 index 输出超过管道缓冲，riko-memory index | head -1：
    // - 不能出现 panicked 字样（stderr 为空）
    // - 进程被 SIGPIPE 终止（shell 状态 141）或正常 0，二者取一，只要不是 101/panic
    // - 输出首行仍是 == 文件 ==
    let test_dir = create_clean_dir("riko_test_cli_t18");
    let bin = get_bin();

    // 1. 造 200 条条目让 index 输出超过管道缓冲
    let moment_dir = test_dir.join("moments/2026-10");
    fs::create_dir_all(&moment_dir).unwrap();
    for i in 1..=200 {
        let file = moment_dir.join(format!("m_20261031_{:03}.md", i));
        fs::write(
            &file,
            format!(
                r#"---
id: m_20261031_{:03}
time: 2026-10-31T15:56:00+08:00
type: moment
title: 测试条目 {:03}
source: unit-test
status: active
tags: [tag{:03}]
---

这是第 {:03} 条用于填充满管道缓冲区的测试数据正文。填充若干文字以确保数据量足够大，触发 SIGPIPE。
"#,
                i, i, i, i
            ),
        )
        .unwrap();
    }

    let stderr_file = test_dir.join("stderr.log");
    let bash_cmd = format!(
        "export RIKO_MEM_DIR=\"{}\"; \"{}\" index 2>\"{}\" | head -1; echo PIPESTATUS_0=${{PIPESTATUS[0]}}",
        test_dir.display(),
        bin,
        stderr_file.display()
    );

    let output = Command::new("bash")
        .arg("-c")
        .arg(&bash_cmd)
        .output()
        .expect("执行 bash 管道命令失败");

    assert!(output.status.success());
    let stdout_str = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout_str.lines().collect();
    assert!(!lines.is_empty(), "管道必须有输出行");

    // 输出首行仍是 `== 文件 ==`
    assert_eq!(lines[0], "== 文件 ==", "首行必须是 '== 文件 =='");

    // 检查 stderr 内容（必须为空，绝不能出现 panicked）
    let stderr_content = fs::read_to_string(&stderr_file).unwrap();
    assert!(
        stderr_content.trim().is_empty(),
        "stderr 必须为空，实际内容: {}",
        stderr_content
    );
    assert!(
        !stderr_content.contains("panicked"),
        "stderr 不能出现 panicked 字样"
    );

    // 检查进程退出码：通过 PIPESTATUS[0]，进程被 SIGPIPE 终止为 141，或正常为 0，不能是 101/panic
    let pipestatus_line = lines
        .iter()
        .find(|l| l.starts_with("PIPESTATUS_0="))
        .expect("必须找到 PIPESTATUS_0 输出行");
    let exit_code: i32 = pipestatus_line
        .strip_prefix("PIPESTATUS_0=")
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        exit_code == 0 || exit_code == 141,
        "进程退出码必须是 0 或 141 (SIGPIPE)，实际退出码: {}",
        exit_code
    );
    assert_ne!(exit_code, 101, "退出码绝不能是 101 (panic)");

    let _ = fs::remove_dir_all(&test_dir);
}
