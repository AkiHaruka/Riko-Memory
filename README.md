# Riko-Memory

璃的记忆系统。Rust 主体 + DeepSeek Harness 插件 + EverOS 后端。

## 结构

```
core/        Rust CLI `riko-memory`（约 8000 行，69 个测试）
dsh-plugin/  DSH 插件 `riko-memory-dsh`（纯 JS ESM，16 个测试）
```

- **core/**：叙事记忆（`memory/` 下的 md 是真相）、`dream` 后台候选、`facts` 会话捕捉、`lint`、`sync-agents`、`search`/`reveal`/`open`。
- **dsh-plugin/**：DSH 插件。注入 SOUL 与常驻状态板，提供 5 个记忆工具（`memory_search` / `memory_remember` / `memory_profile` / `memory_list` / `memory_status`），转调 EverOS HTTP 服务。
- **EverOS**（外部依赖，锁定 1.6.0）：事实轨后端，不在本仓库。

## 测试

```bash
cd core && cargo test          # 69 passed
cd dsh-plugin && npm test      # 16 passed
```

## 构建与安装

```bash
cd core
cargo build --release
cp target/release/riko-memory /usr/local/bin/riko-memory
```

插件安装（peer 链接、cordis.patch.yml 配置、验收与回滚）见 [dsh-plugin/README.md](dsh-plugin/README.md)。

## 设计原则

- 写权在人：`moments/`、`people/`、`self/` 只有作者本人能改；后台模型只往 `inbox/` 提候选，审过才进正文。
- 文件为真相：记忆条目是 Markdown，索引和检索是派生品，不是另一份数据。
- 插件层只做注入与工具壳；真正的逻辑在 `core/` 和 EverOS 里。
