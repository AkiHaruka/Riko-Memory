# riko-memory-dsh

**璃的记忆系统（EverOS 事实轨）DSH 插件**（包名 riko-memory-dsh） —— 将 DSH 记忆底座由 Hy-Memory 切换为 EverOS。

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

在 DSH profile 的 `cordis.patch.yml` 中配置如下实例（必须放在 `- insert:` 列表中，不能写成顶层 `- id:` 条目）：

```yaml
- insert:
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

### 0. 安装方式

发布版从 npm 安装：

```bash
npm install riko-memory-dsh
```

在 DSH profile 里安装：

```bash
npm install --prefix /root/.dsh/profiles/web riko-memory-dsh
```

peerDependencies 由 DSH 运行时提供；如果 npm 解析不到，执行 `scripts/link-peers.sh` 补软链（脚本已随包发布）。本仓库开发版跳过这步，直接用下面第 1 步的软链。

包内 `cordis.patch.yml` 是空组合包声明：DSH 要求每个 profile 组合包必须有可解析的 patch 文件，否则会在插件页标成「没有声明组合包」。实例配置不写在这个文件里，统一放 profile 的 `cordis.patch.yml`（entry id: `riko-memory-dsh`）。在 DSH 插件页把这个包加入 profile 组合包后，插件页会显示它。

### 1. 建立 profile 插件软链
在 DSH 容器内执行：
```bash
ln -sfn /root/riko/tools/riko-memory-dsh/dsh-plugin /root/.dsh/profiles/web/node_modules/riko-memory-dsh
```

### 2. 建立 peerDependencies 软链
在插件目录下建立指向 DSH 运行环境 `@deepseek-ai/dsh-llm` 与 `@deepseek-ai/dsh-tools` 的软链。在 DSH 容器内执行：
```bash
bash /root/riko/tools/riko-memory-dsh/dsh-plugin/scripts/link-peers.sh
```

### 3. 备份现役配置文件
在 DSH 容器内执行：
```bash
cp -a /root/.dsh/profiles/web/cordis.patch.yml{,.bak-20261101}
```
（注：只需要备份 `cordis.patch.yml`；hymem 的 bundle patch 是空列表，切换时不用动 `profile/package.json`，也不要去改 pnpm 依赖，无需执行 `pnpm install`。）

### 4. 修改 cordis.patch.yml
在 DSH 容器内编辑 `/root/.dsh/profiles/web/cordis.patch.yml`：

用下面这一整段替换文件里原来的 `- insert:` 块（该块里目前只有 `- id: riko-hymem` 一项）。不要留下空的 `insert`，也不要把新条目写成顶层 `- id:`（顶层条目只能 patch 已存在的 entry，DSH 会报 `patch: entry "riko-memory-dsh" not found` 并且不加载插件）：

```yaml
- insert:
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

### 5. 部署前组合预检
在 DSH 容器内执行：
```bash
dsh --profile web --dump-config > /tmp/dump-memory.yml 2>/tmp/dump-memory.err; echo "EXIT=$?"; cat /tmp/dump-memory.err; grep -n "riko-memory-dsh\|riko-hymem" /tmp/dump-memory.yml
```

`--dump-config` 只做组合不挂载插件，安全，所以可以重启前先验。

期望结果：
1. `EXIT=0`；
2. stderr 为空（出现 `patch: entry ... not found` 或 `Cannot find package` 就停下来修，不要重启）；
3. `/tmp/dump-memory.yml` 里有 `- id: riko-memory-dsh` 一整段（含 `everosUrl` 等 config）；
4. 不再有 `- id: riko-hymem`。

### 6. 重启 DSH 容器
在宿主执行（需征得爹地同意，DSH 容器内无 docker CLI）：
```bash
docker restart riko-dsh
```

### 7. 冒烟检查
1. 在**宿主**执行：查看容器启动日志，确认插件加载成功：
```bash
docker logs --tail 300 riko-dsh 2>&1 | grep "riko-memory-dsh"
```
应看到两行：
- `riko-memory-dsh: 已注册 5 个记忆工具 (user_id=aki, url=http://127.0.0.1:18100)`
- `riko-memory-dsh: 插件加载完成 tools=true soul=true resident=true url=http://127.0.0.1:18100`
（注：参数按实际配置略有不同，比如 `user_id` 和 `url`）。
2. 在 **DSH 会话里**执行：调用 `memory_status`，期望返回 `EverOS 1.6.0 status=ok … md 根：/data/everos`（版本号按实际）。
3. 在 **DSH 会话里**执行：调用检索与列表：
   - 调用 `memory_search`（如 `query="记忆系统"`）应有结果；
   - 调用 `memory_list` 第一页 footer 形如 `共 N 条，已显示 1-10`。
4. 在 **DSH 会话里**人工确认：SOUL 注入。在新会话里观察 Agent 的自我认知/说话风格是否符合 soul；脚本无法断言，需人工检查。
5. 在 **DSH 会话里**人工确认：常驻记忆注入。发生一次 compact 后，下一轮 pre-step 会出现 `<agent_memory:resident>` 包裹的注入消息；等自然 compact 或手动触发压缩后观察会话确认。
6. 排错指南（若工具未出现）：
   - 在**宿主**查看日志，若搜到 `Cannot find package`：说明 peerDependencies 缺失，在 **DSH 容器内**重跑 `scripts/link-peers.sh`；
   - 在**宿主**查看日志，若搜到 `riko-memory-dsh` 名字解析错误：检查 **DSH 容器内** profile 软链是否建立（`/root/.dsh/profiles/web/node_modules/riko-memory-dsh`）。

---

## 回滚步骤（15 分钟内完成）

若新插件或 EverOS 服务出现异常需紧急回滚：

1. **恢复配置文件**（在 DSH 容器内执行）：
```bash
cp -a /root/.dsh/profiles/web/cordis.patch.yml.bak-20261101 /root/.dsh/profiles/web/cordis.patch.yml
```

2. **移除插件软链**（在 DSH 容器内执行）：
```bash
rm -f /root/.dsh/profiles/web/node_modules/riko-memory-dsh
```
*(注：`/root/riko/tools/riko-memory-dsh` 插件源码目录与内建 node_modules 软链可保留，无需删除)*

3. **启动旧记忆容器**（在宿主执行）：
```bash
docker start hy-memory
```

4. **重启 DSH 容器**（在宿主执行）：
```bash
docker restart riko-dsh
```

5. **验证旧工具恢复**（在 DSH 会话里执行）：
在 DSH 会话里调用 `memory_status` 确认旧 Hy-Memory 恢复正常。
