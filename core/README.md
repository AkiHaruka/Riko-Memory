# riko-memory (Rust 版)

璃的叙事记忆工具，基于 Rust 从零重写的单二进制命令行工具，命令与输出格式对齐原 Python 版，已知差异见 CLI-DIFFERENCES.md。

## 1. 构建与测试

### 环境依赖
- Rust 1.80+ (推荐 1.99.1 stable)
- 工具链环境变量设置：
  - 在 riko-agy 容器里：
    ```bash
    export CARGO_HOME=/work/.cargo RUSTUP_HOME=/work/.rustup PATH=/work/.cargo/bin:$PATH
    ```
  - 在宿主 / 璃的容器里：
    ```bash
    export CARGO_HOME=/root/riko/.cargo RUSTUP_HOME=/root/riko/.rustup PATH=/root/riko/.cargo/bin:$PATH
    ```

### 构建命令
```bash
# 调试构建
cargo build

# 发布构建（自动 strip 与 LTO 优化，体积 < 3MB，零 warning）
cargo build --release

# 运行全量测试（包含单元测试与集成测试）
cargo test
```

## 2. 环境变量

- `RIKO_MEM_DIR`: 记忆库存储根目录。默认路径为 `/root/riko/memory`。
- `RIKO_AGENTS_PATH`: AGENTS.md 状态板文件绝对路径。默认路径为 `/root/riko/AGENTS.md`。

## 3. 命令清单 (14 个)

### 3.1 add
添加一条记忆条目。
```bash
riko-memory add --text "正文" [--type moment|person|self] [--quote "原话"] [--source 渠道] [--tags 标签1,标签2] [--time ISO时间] [--title 标题]
```
- 默认 type 为 `moment`。
- title 安全处理：去除 `\r\n`、`\r`、`\n` 为空格，双引号转为单引号，trim，按字符截断至 30 字符；缺省时从正文首句提取。
- quote 安全处理：去除换行转为空格，双引号转义为 `\"`，trim 为空则整行不写入。
- 正文（body）保留多行换行。
- 打印生成的新文件绝对路径。

### 3.2 inbox
追加待办内容到收件箱。
```bash
riko-memory inbox "候选内容" [--source 渠道]
```
- 追加至 `inbox/YYYY-MM-DD.md`。打印文件路径。

### 3.3 search
搜索记忆库条目。
```bash
riko-memory search 词1 [词2 ...] [--type T] [--tag G] [--days N] [--limit 12]
```
- 输出格式：`相对路径<TAB>时间<TAB>标题<TAB>(hit N)`。
- 主查询与回退逻辑统一受 `--type`、`--tag`、`--days` 约束。
- 若无命中条目，在过滤后的集合中回退为 OR 匹配，按 mtime 降序，第 4 列为空。过滤后无匹配则无输出（exit 0）。

### 3.4 recent
查看近期修改过的条目。
```bash
riko-memory recent [--days 7] [--type T]
```
- 输出格式：`时间(前16位空格替换T)<TAB>相对路径<TAB>标题`。

### 3.5 reveal
查看条目全文。
```bash
riko-memory reveal KEY
```
- 按优先级匹配：文件名去 `.md` 等于 KEY、完整文件名等于 KEY、路径包含 KEY。
- 找不到时 stderr 报错并退出 1。

### 3.6 open
查看未闭环事件。
```bash
riko-memory open
```
- 打印 `open-loops.md` 全文，不存在时输出 `（没有 open-loops.md）`。

### 3.7 resident
查看状态板。
```bash
riko-memory resident
```
- 打印 `resident.md` 全文，不存在时报错退出 1。

### 3.8 summary
查看记忆索引摘要。
```bash
riko-memory summary [--lines 60]
```
- 打印 `MEMORY.md` 前 N 行。

### 3.9 lint
检查记忆库条目格式规范。
```bash
riko-memory lint
```
- 自动跳过 `inbox/` 及 `RULES.md`、`CHANGELOG.md`、`MEMORY.md`、`open-loops.md`、`resident.md`、`README.md`、`AGENTS.md`。
- front-matter 行检查：任何非 `key: value` 格式的行报 `front-matter 行无法解析: <path> (<行内容>)`。
- 缺 time 单条报 `缺 time: <path>`，不追报 `time 不合法`。
- time 校验优先检查是否以 Unknown 结尾（报 `time 为 Unknown: <path>`），否则再检查 RFC3339 解析合法性。
- 末尾输出 `lint: N 处问题`（无论 N 多少退出码均为 0）。

### 3.10 sync-agents
同步 resident.md 状态板到 AGENTS.md。
```bash
riko-memory sync-agents
```
- 过滤 resident.md 里的空行、一级标题行和引用行，生成指定模板块。
- AGENTS.md 缺失或内容为空时，直接写入生成块加末尾换行，无前导空行；非空且有标记时非贪婪替换；非空无标记时追加。

### 3.11 index
列出所有条目与分类统计。
```bash
riko-memory index
```
- 打印全部文件列表及 `moments: N｜people: N｜self: N｜inbox: N`。

### 3.12 changelog
追加变更日志。
```bash
riko-memory changelog "日志内容"
```
- 追加到 `CHANGELOG.md`，每行格式：`- YYYY-MM-DD HH:MM 内容`。

### 3.13 dream
后台记忆整理。详见下文专用章节。

### 3.14 facts
会话事实捕获进 EverOS。详见下文专用章节。

## dream（后台候选整理）

读取 DSH 会话记录，将值得记入叙事记忆的草稿整理写入收集箱。

```bash
riko-memory dream [--config <PATH>] [--max-calls <N>]
riko-memory dream --dry-run
```

- **配置路径**：默认 `/root/riko/var/dream.json`，可通过环境变量 `RIKO_DREAM_CONFIG` 或 `--config` 覆盖。
- **边界保证**：**只写 inbox（`memory/inbox/*.md`），绝不写正文记忆**（如 `memory/moments/`、`memory/people/`、`memory/self/`、`MEMORY.md` 等），所有条目由璃人工审阅后才可归档。

## facts（会话事实捕获）

读取 DSH 会话记录，将爹地（user 消息）的事实提取并发送至 EverOS。

```bash
riko-memory facts [--config <PATH>] [--max-calls <N>] [--backfill]
riko-memory facts --dry-run
```

- **用途**：捕获会话事实，增量同步至 EverOS 事实库。
- **用法**：定期通过 cron 调度执行，支持 `--dry-run` 预览与 `--backfill` 全量回填。
- **配置路径**：默认 `/root/riko/var/facts.json`，可通过 `--config` 指定。
- **只发 user 消息**：仅发送 `source.kind=user` 的爹地消息，绝不发送 assistant/工具事件。
- **状态与日志**：状态记录于 `var/facts.state.json`，运行日志记录于 `var/facts.log`，并发由 `var/facts.lock` 保护。

## 4. 退出码规范

- 参数或用法错误：退出码 `2`（clap 自动处理）。
- 业务错误（找不到条目、缺少必须文件等）：stderr 报错，退出码 `1`。
- 正常执行成功（含 lint 检查完成）：退出码 `0`。

## 5. 与 Python 原版的行为差异

详见根目录文档 [CLI-DIFFERENCES.md](CLI-DIFFERENCES.md)。
