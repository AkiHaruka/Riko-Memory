# riko-memory-dsh

**璃的 P2 记忆系统（EverOS 事实轨）DSH 插件** —— 将 DSH 记忆底座由 Hy-Memory 切换为 EverOS。

本插件为 DeepSeek Harness（DSH）提供三项核心能力：
1. **记忆工具（5 个）**：`memory_search`、`memory_remember`、`memory_profile`、`memory_list`、`memory_status`，直连 EverOS HTTP 服务。
2. **SOUL 注入**：会话启动/compaction 后注入 `soul.md`，维持 prompt cache 前缀静止。
3. **常驻记忆注入**：仅在 `compaction/end` 后的第一个 `agent/pre-step` 注入一次 `resident.md` 状态板（字符安全截断，不拉动态画像）。

---

## 架构与设计原则

- **纯 JS ESM**：零构建步骤，零网络 dependencies，依赖 Node.js 内置模块与 DSH 运行时 peerDependencies。
- **按需检索**：不做每轮自动 prefetch 检索，由 Agent 主动调用工具触发，避免闲聊上下文污染。
- **文件为真相与纠错路径**：
  - 不实现 `memory_correct` / `memory_forget` / `memory_evolve` / `memory_finalize` 等旧 Hy-Memory 专有工具。
  - **纠错路径**：璃可直接修改 `/work/everos` 下的 Markdown 文件（如 `episodes/` 或 `profiles/`），EverOS 的 cascade 机制会自动检测外部修改并重新构建 SQLite 与向量索引。

---

## 配置项（cordis.patch.yml）

在 DSH profile 的 `cordis.patch.yml` 中插入如下实例配置：

```yaml
- id: riko-memory-dsh
  name: 'riko-memory-dsh'
  config:
    everosUrl: 'http://127.0.0.1:18100'
    appId: riko
    projectId: main
    userId: aki
    agentId: riko
    timeoutMs: 120000
    soulEnabled: true
    soulFile: '/root/riko/soul.md'
    residentEnabled: true
    residentFile: '/root/riko/memory/resident.md'
    residentBudgetChars: 6000
    toolsEnabled: true
    workspaceAllow:
      - '/root/riko'
```

### 配置字段说明

| 字段 | 默认值 | 说明 |
|---|---|---|
| `everosUrl` | `http://127.0.0.1:18100` | EverOS 服务 HTTP 地址 |
| `appId` | `riko` | 应用空间 ID |
| `projectId` | `main` | 项目空间 ID |
| `userId` | `aki` | 人类用户 ID（爹地） |
| `agentId` | `riko` | AI 自身 ID（璃） |
| `timeoutMs` | `120000` | HTTP 请求超时时间（毫秒） |
| `soulEnabled` | `true` | 是否启用 SOUL prompt 注入 |
| `soulFile` | `/root/riko/soul.md` | SOUL 文件路径 |
| `residentEnabled` | `true` | 是否启用常驻记忆注入 |
| `residentFile` | `/root/riko/memory/resident.md` | 常驻状态板文件路径 |
| `residentBudgetChars` | `6000` | 常驻记忆字符预算上限 |
| `toolsEnabled` | `true` | 是否向 DSH 注册记忆工具 |
| `workspaceAllow` | `["/root/riko"]` | 工作区路径白名单，空数组表示不限 |

---

## 工具列表

1. `memory_search`: 支持单查询（`query`）或多查询（`queries`），向 EverOS 发起 hybrid 检索，多路结果按 id 去重并按最高 score 排序输出。
2. `memory_remember`: 接收 `text` 或 `messages`，合成结构化消息写入 EverOS `/add` 并在同 session 下立即执行 `/flush`。
3. `memory_profile`: 读取长期用户画像事实（`POST /api/v2/memory/get`，`memory_type=profile`）。
4. `memory_list`: 分页列出记忆事件列表（`POST /api/v2/memory/get`，`memory_type=episode`）。
5. `memory_status`: 读取 `GET /health`，展示 EverOS 版本、status、embed、cascade 状态与 Markdown 根目录。

---

## 部署步骤（璃执行）

1. **放置插件目录**：
   将 `tools/riko-memory-dsh` 软链或复制到 DSH profile 的插件目录：
   ```bash
   ln -s /root/riko/tools/riko-memory-dsh /root/.dsh/profiles/web/node_modules/riko-memory-dsh
   ```
2. **备份配置文件**：
   ```bash
   cp /root/.dsh/profiles/web/cordis.patch.yml /root/.dsh/profiles/web/cordis.patch.yml.bak-$(date +%Y%m%d%H%M)
   ```
3. **编辑 cordis.patch.yml**：
   - 移除旧的 `id: riko-hymem` 及其配置块。
   - 插入上述 `riko-memory-dsh` 配置块。
4. **重启 DSH**：
   ```bash
   systemctl restart riko-dsh   # 或对应的宿主/容器重启命令
   ```
5. **验证检查**：
   - 查看启动日志无报错：`journalctl -u riko-dsh -n 50 --no-pager`。
   - 验证工具正常：对话中调用 `memory_status` 返回 EverOS 健康状态。

---

## 回滚步骤

若新插件或 EverOS 服务出现不可逆异常，按以下步骤回滚：
1. **恢复配置文件**：
   ```bash
   cp /root/.dsh/profiles/web/cordis.patch.yml.bak-* /root/.dsh/profiles/web/cordis.patch.yml
   ```
2. **移除软链**：
   ```bash
   rm -f /root/.dsh/profiles/web/node_modules/riko-memory-dsh
   ```
3. **确保 hy-memory 容器处于运行状态**：
   ```bash
   docker start hy-memory
   ```
4. **重启 DSH**：
   ```bash
   systemctl restart riko-dsh
   ```
5. **验证旧工具恢复**：调用 `memory_status` 确认旧 hy-memory 连接正常。
