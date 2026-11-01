# riko-memory 命令行实现差异文档 (CLI-DIFFERENCES.md)

本文档逐条记录 Rust 重写版 `riko-memory` 与原 Python 版本的行为差异及其设计理由。

---

## 1. search 回退过滤（修正原版行为，非兼容）

- **Python 版行为**：
  主查询未命中时，回退的 OR 匹配逻辑使用 `walk_md(MEM)` 遍历整库，丢失了命令行传入的 `--type`、`--tag`、`--days` 过滤限制。例如在 `people/` 存在含 `apple` 的条目而 `moments/` 不存在时，执行 `riko-memory search apple --type moment` 仍会跨目录返回 `people/` 下的条目。
- **Rust 版行为**：
  主查询与回退逻辑统一受 `--type`、`--tag`、`--days` 过滤器的严格约束。未命中回退时，仅在符合过滤条件的候选条目集合中做 OR 匹配，绝不跨越类别目录或超出时间/标签范围；若过滤后集合中无匹配项，输出为空（退出码 0）。
- **理由**：
  过滤条件属于硬约束，指定 `--type moment` 却返回 `people/` 下的文件属于逻辑错误。Rust 版依规格 v1.1 修复该设计缺陷，不继承原版 Bug。

---

## 2. recent 输出时间格式（空格替换 T）

- **Python 版行为**：
  直接对 ISO 字符串切片取前 16 位（如 `2026-10-31T15:56`），保留了中间的字符 `T`。
- **Rust 版行为**：
  按规格 §3.4 规范将时间字符串第 11 位的 `T` 替换为空格，输出格式为 `YYYY-MM-DD HH:MM`（如 `2026-10-31 15:56`）。
- **理由**：
  规格 §3.4 明确规定“time 前16位(把T换成空格)”，提升控制台输出在终端下的可读性。

---

## 3. index 统计行输出格式

- **Python 版行为**：
  直接通过 `print(str(stats))` 输出 Python 字典字面量字符串（如 `{'moments': 0, 'people': 0, 'self': 0, 'inbox': 0}`）。
- **Rust 版行为**：
  按规格 §3.9 规范，输出为竖线分隔的排版行：`moments: N｜people: N｜self: N｜inbox: N`。
- **理由**：
  符合规格 §3.9 的明确格式定义，格式规整且更适合作为状态摘要展示。

---

## 4. lint 校验规则与跳过名单

- **Python 版行为**：
  1. 若文件缺少 `time` 字段，Python 版先报 `缺 time`，接着因空值无法被 `fromisoformat` 解析再次抛错，连带报出 `time 不合法: ('')`，导致单处缺失重复累计为 2 处问题。
  2. 未在忽略名单中包含 `AGENTS.md`，当记忆目录与项目根目录重合或同级遍历时容易被误检为缺失 front-matter。
- **Rust 版行为**：
  1. 缺 `time` 字段时准确报告 `缺 time: <path>` 1 条，不再追报 `time 不合法`，严格计为 1 处问题。
  2. 跳过名单中明确包含 `AGENTS.md`（与 RULES.md、CHANGELOG.md 等并列）。
  3. 新增 front-matter 块内逐行检查：任何无法解析为 `key: value` 的行（如未安全转义换行的多行值、折行破坏）均被检出并计入问题数（报 `front-matter 行无法解析: <path> (<该行内容>)`）。
- **理由**：
  避免同一问题重复计数（符合 §4 T7 要求）；防止外部文件或同目录下的状态文件干扰检查；杜绝因换行破坏 front-matter 结构导致的静默解析丢弃。

---

## 5. reveal / open / resident 打印末尾空行

- **Python 版行为**：
  使用 `print(open(p).read())`，因为文件正文本身通常以 `\n` 结尾，`print` 默认再次追加 `\n`，导致末尾多出一个多余的空行。
- **Rust 版行为**：
  使用 `print!("{}", content)`（标准无额外换行输出），严格保留文件原始内容，不注入额外空行。
- **理由**：
  保证文件查看命令对原始文档内容的保真度，避免给下游管道处理或查看引入虚假空行。

---

## 6. 命令行解析与退出码规范

- **Python 版行为**：
  参数缺失或不合法时退出码为 1 或 argparse 默认退出码 2，业务逻辑错误退出码多为 1 或未捕获异常退出。
- **Rust 版行为**：
  基于 clap 4 标准库，参数或用法错误（如缺少必需参数、未知子命令）统一返回退出码 `2`；业务逻辑错误（找不到条目、缺少必须文件等）统一输出中文错误到 stderr 并返回退出码 `1`；正常完成或 lint 报告统一返回退出码 `0`。
- **理由**：
  符合 POSIX CLI 工业标准（语法/参数错误退出码 2，运行期业务错误退出码 1），便于上层脚本与调度器精准区分错误类型。

---

## 7. 管道提前关闭与 SIGPIPE 处理

- **Python 版行为**：
  Python 默认在管道下游提前关闭时，写入 stdout 会抛出 `BrokenPipeError: [Errno 32] Broken pipe` 并向 stderr 打印异常回溯（除非显式捕获或将 SIGPIPE 重设为默认行为）。
- **Rust 版行为**：
  Rust 运行时启动时默认将 `SIGPIPE` 设为忽略（`SIG_IGN`），导致管道下游提前关闭（如 `riko-memory ... | head -1`）后写标准输出遇到 `EPIPE` 时触发 `std::io` panic（`failed printing to stdout: Broken pipe (os error 32)`，退出码 101）。
  Rust 版在程序入口最开头通过 `libc::signal(libc::SIGPIPE, libc::SIG_DFL)` 恢复 Unix 默认的 SIGPIPE 处理机制。在管道中断时直接由操作系统内核终止进程（退出码为 141 或 0），stderr 保持完全为空，彻底消除 panic 与异常堆栈输出。
- **理由**：
  命令行工具常与 `head`、`less` 等管道下游配合使用。恢复系统默认信号处置行为符合 POSIX 哲学，避免了 Broken pipe panic 噪音并防止退出码退化为 101。

---

## 8. dream 子命令（Rust 版专属功能，Python 版无此命令）

- **Python 版行为**：
  原 Python 版工具库中无 `dream` 子命令。
- **Rust 版行为**：
  Rust 版新增后台记忆整理子命令 `dream`（v0.2.0 起），用于异步扫描解压 DSH 会话记录（`session.v4.jsonl.zstd` 多帧流），增量切块后调用 LLM 提取叙事记忆草稿并写入 `memory/inbox/YYYY-MM-DD.md` 收集箱，同时维护状态、文件锁与运行日志。
- **理由**：
  将长程记忆提炼工作卸载至后台独立运行，严格遵循「只写 inbox、不写正文记忆」的安全边界，供人工审核与归档。
