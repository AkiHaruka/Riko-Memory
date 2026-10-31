# riko-memory-dsh

**璃的 P2 记忆系统（EverOS 事实轨）DSH 插件** —— 将 DSH 记忆底座由 Hy-Memory 切换为 EverOS。

本插件为 DeepSeek Harness（DSH）提供三项核心能力：
1. **记忆工具（5 个）**：`memory_search`、`memory_remember`、`memory_profile`、`memory_list`、`memory_status`，通过官方 `defineTool` 规范直连 EverOS HTTP 服务。
2. **SOUL 注入**：会话启动/compaction 后注入 `soul.md`，维持 prompt cache 前缀静止。
3. **常驻记忆注入**：仅在 `compaction/end` 后的第一个 `agent/pre-step` 注入一次 `resident.md` 状态板（字符安全截断，不拉动态画像）。

---

## 架构与设计原则

- **纯 JS ESM**：零构建步骤，零网络 dependencies，依赖 Node.js 内置模块与 DSH 运行时 peerDependencies。
- **官方 defineTool 包装**：工具定义走 `@deepseek-ai/dsh-tools` 的 `defineTool`，自动编译合法 JSON Schema 并执行参数校验。
- **按需检索**：不做每轮自动 prefetch 检索，由 Agent 主动调用工具触发，避免闲聊上下文污染。
- **文件为真相与纠错路径**：
  - 不实现 `memory_correct` / `memory_forget` / `memory_evolve` / `memory_finalize` 等旧 Hy-Memory 专有工具。
  - **纠错路径**：直接修改 EverOS 的 Markdown 文件（如 `episodes/` 或 `profiles/`），EverOS 的 cascade 机制会自动检测外部修改并重新构建 SQLite 与向量索引。`/data/everos` 为 EverOS 容器内路径，宿主对应 `/data/home/riko/ws/everos`。

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
    mdRoot: '/data/everos'
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
| `mdRoot` | `/data/everos` | EverOS 容器内 Markdown 根目录（宿主对应 `/data/home/riko/ws/everos`） |
| `workspaceAllow` | `["/root/riko"]` | 工作区路径白名单，空数组表示不限 |

---

## 工具列表

1. `memory_search`: 支持单查询（`query`）或多查询（`queries`），向 EverOS 发起 hybrid 检索，多路结果按 id 去重并按最高 score 排序输出。
2. `memory_remember`: 接收 `text` 或 `messages`，合成结构化消息写入 EverOS `/add` 并在同 session 下立即执行 `/flush`。
3. `memory_profile`: 读取长期用户画像（`POST /api/v2/memory/get`，`memory_type=profile`，`page=1, page_size=1`），格式化为人类可读文本（摘要、explicit_info 最多 50 条截断、implicit_traits 列表）。
4. `memory_list`: 分页列出记忆事件列表（`POST /api/v2/memory/get`，`memory_type=episode`），将客户端入参 `limit` 与 `offset` 映射为 `page` 与 `page_size`，未对齐时追加对齐说明，按实际返回页展示范围。
5. `memory_status`: 读取 `GET /health`，展示 EverOS 版本、status、embed、cascade 状态与 Markdown 根目录（优先使用 health 返回值，缺省回退至 `cfg.mdRoot` 即 `/data/everos`）。

---

## 部署步骤（由璃在 DSH 容器/宿主执行）

> [!IMPORTANT]
> **重启注意**：宿主重启 `riko-dsh` 容器会导致约 30–90 秒 QQ 离线，**必须先征得爹地同意**后方可执行。

### 1. 建立 profile 插件软链
在 DSH 容器内执行：
```bash
ln -sfn /root/riko/tools/riko-memory-dsh /root/.dsh/profiles/web/node_modules/riko-memory-dsh
```

### 2. 建立 peerDependencies 软链
在插件目录下建立指向 DSH 运行环境 `@deepseek-ai/dsh-llm` 与 `@deepseek-ai/dsh-tools` 的软链（在 DSH 容器内执行）：
```bash
bash /root/riko/tools/riko-memory-dsh/scripts/link-peers.sh
```

### 3. 备份现役配置文件
在 DSH 容器内执行：
```bash
cp -a /root/.dsh/profiles/web/cordis.patch.yml{,.bak-20261101}
```

### 4. 修改 cordis.patch.yml
- 从 `/root/.dsh/profiles/web/cordis.patch.yml` 中删除旧的 `- id: riko-hymem ...`（`@aki_riko/hymem-dsh`）整段配置。
- 插入 `riko-memory-dsh` 配置块：
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
    mdRoot: '/data/everos'
    workspaceAllow:
      - '/root/riko'
```

### 5. 重启 DSH 容器
在宿主（Host）执行（需征得爹地同意）：
```bash
docker restart riko-dsh
```

### 6. 冒烟检查
1. 查看 DSH 容器启动日志中插件加载情况：
```bash
docker logs --tail 100 riko-dsh | grep riko-memory-dsh
```
2. 验证 SOUL 与常驻状态板正常注入，且 5 个工具均可正常调用（如调用 `memory_status` 返回 EverOS 健康状态）。

---

## 回滚步骤（15 分钟内完成）

若新插件或 EverOS 服务出现异常需紧急回滚：

1. **恢复配置文件**（DSH 容器内）：
```bash
cp -a /root/.dsh/profiles/web/cordis.patch.yml.bak-20261101 /root/.dsh/profiles/web/cordis.patch.yml
```

2. **移除插件软链**（DSH 容器内）：
```bash
rm -f /root/.dsh/profiles/web/node_modules/riko-memory-dsh
```
*(注：`/root/riko/tools/riko-memory-dsh` 插件源码目录与内建 node_modules 软链可保留，无需删除)*

3. **宿主启动旧记忆容器**（宿主执行）：
```bash
docker start hy-memory
```

4. **宿主重启 DSH 容器**（宿主执行）：
```bash
docker restart riko-dsh
```

5. **验证旧工具恢复**：在对话中调用 `memory_status` 确认旧 Hy-Memory 恢复正常。
