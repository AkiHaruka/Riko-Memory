import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { resolveConfig } from "../dist/config.js";
import { buildMemoryHandlers } from "../dist/tools.js";

// 启动端口 0 的假 EverOS 服务器辅助工具
async function withMockEverOS(handler, fn) {
    const sockets = new Set();
    const server = createServer(async (req, res) => {
        res.setHeader("Connection", "close");
        let body = "";
        req.on("data", (chunk) => {
            body += chunk;
        });
        req.on("end", async () => {
            let parsedBody = null;
            if (body && req.headers["content-type"]?.includes("application/json")) {
                try {
                    parsedBody = JSON.parse(body);
                } catch {
                    parsedBody = body;
                }
            }
            await handler(req, res, parsedBody);
        });
    });

    server.on("connection", (socket) => {
        sockets.add(socket);
        socket.on("close", () => sockets.delete(socket));
    });

    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
    const port = server.address().port;
    const everosUrl = `http://127.0.0.1:${port}`;

    try {
        await fn(everosUrl);
    } finally {
        for (const s of sockets) {
            s.destroy();
        }
        await new Promise((resolve) => server.close(resolve));
    }
}

test("P3: memory_search - 请求体校验、多 query 去重合并与格式化输出", async () => {
    const requests = [];

    await withMockEverOS(
        (req, res, body) => {
            requests.push({ url: req.url, method: req.method, body });
            if (req.url === "/api/v2/memory/search") {
                if (body.query === "吵架") {
                    res.writeHead(200, { "Content-Type": "application/json" });
                    res.end(
                        JSON.stringify({
                            request_id: "r1",
                            data: {
                                episodes: [
                                    {
                                        id: "ep_1",
                                        timestamp: "2026-10-29T21:42:00+08:00",
                                        subject: "第一次吵架",
                                        summary: "讨论记忆系统分歧，达成保留状态板的共识。",
                                        score: 0.8712,
                                        atomic_facts: [
                                            { id: "af_1", content: "爹地把记忆怎么记交给我自己决定" },
                                            { id: "af_2", content: "常驻状态板保留 40 行内" },
                                        ],
                                    },
                                ],
                            },
                        })
                    );
                    return;
                }
                if (body.query === "q1") {
                    res.writeHead(200, { "Content-Type": "application/json" });
                    res.end(
                        JSON.stringify({
                            data: {
                                episodes: [
                                    { id: "ep_1", timestamp: "2026-10-29T21:42:00+08:00", subject: "吵架", summary: "摘要1", score: 0.80 },
                                    { id: "ep_2", timestamp: "2026-10-30T10:00:00+08:00", subject: "和好", summary: "摘要2", score: 0.65 },
                                ],
                            },
                        })
                    );
                    return;
                }
                if (body.query === "q2") {
                    res.writeHead(200, { "Content-Type": "application/json" });
                    res.end(
                        JSON.stringify({
                            data: {
                                episodes: [
                                    { id: "ep_1", timestamp: "2026-10-29T21:42:00+08:00", subject: "吵架", summary: "摘要1", score: 0.93 },
                                    { id: "ep_3", timestamp: "2026-10-31T15:00:00+08:00", subject: "定稿", summary: "摘要3", score: 0.77 },
                                ],
                            },
                        })
                    );
                    return;
                }
            }
            res.writeHead(404);
            res.end();
        },
        async (everosUrl) => {
            const cfg = resolveConfig({ everosUrl });
            const tools = buildMemoryHandlers(cfg);
            const searchTool = tools.find((t) => t.name === "memory_search");

            // 1. 单 query 检验请求体与格式化输出
            const res1 = await searchTool.execute({ query: "吵架", limit: 5 });
            assert.equal(res1.ok, true);

            const lastReq = requests[0];
            assert.equal(lastReq.method, "POST");
            assert.equal(lastReq.url, "/api/v2/memory/search");
            assert.equal(lastReq.body.user_id, "aki");
            assert.equal(lastReq.body.app_id, "riko");
            assert.equal(lastReq.body.project_id, "main");
            assert.equal(lastReq.body.query, "吵架");
            assert.equal(lastReq.body.method, "hybrid");
            assert.equal(lastReq.body.top_k, 5);
            assert.equal(lastReq.body.include_profile, false);

            assert.match(res1.data, /1\. \[episode\] 2026-10-29 21:42 · 第一次吵架（score 0\.87）/);
            assert.match(res1.data, /摘要：讨论记忆系统分歧，达成保留状态板的共识。/);
            assert.match(res1.data, /事实：- 爹地把记忆怎么记交给我自己决定/);
            assert.match(res1.data, /- 常驻状态板保留 40 行内/);

            // 2. 多 query 去重合并
            const resMulti = await searchTool.execute({ queries: ["q1", "q2"], limit: 5 });
            assert.equal(resMulti.ok, true);
            // ep_1 应合并并保留更高的 0.93
            assert.match(resMulti.data, /1\. \[episode\] .* · 吵架（score 0\.93）/);
            assert.match(resMulti.data, /2\. \[episode\] .* · 定稿（score 0\.77）/);
            assert.match(resMulti.data, /3\. \[episode\] .* · 和好（score 0\.65）/);
        }
    );
});

test("P4: memory_search - 无结果与服务不可用两种返回", async () => {
    let mode = "empty";

    await withMockEverOS(
        (req, res) => {
            if (mode === "empty") {
                res.writeHead(200, { "Content-Type": "application/json" });
                res.end(JSON.stringify({ data: { episodes: [] } }));
            } else {
                res.writeHead(500, { "Content-Type": "application/json" });
                res.end(JSON.stringify({ error: "Internal Server Error" }));
            }
        },
        async (everosUrl) => {
            const cfg = resolveConfig({ everosUrl });
            const tools = buildMemoryHandlers(cfg);
            const searchTool = tools.find((t) => t.name === "memory_search");

            // 1. 无结果
            mode = "empty";
            const resEmpty = await searchTool.execute({ query: "不存在的事情" });
            assert.equal(resEmpty.ok, true);
            assert.equal(resEmpty.data, "没有找到相关记忆。");

            // 2. 服务不可用（不崩溃，返回错误文本）
            mode = "error";
            const resError = await searchTool.execute({ query: "任何内容" });
            assert.equal(resError.ok, false);
            assert.match(resError.error.message, /^记忆服务不可用：/);
        }
    );
});

test("P5: memory_remember - text/messages 映射与 add->flush 调用序列", async () => {
    const callLog = [];

    await withMockEverOS(
        (req, res, body) => {
            callLog.push({ url: req.url, method: req.method, body });
            res.writeHead(200, { "Content-Type": "application/json" });
            res.end(JSON.stringify({ status: "ok" }));
        },
        async (everosUrl) => {
            const cfg = resolveConfig({ everosUrl });
            const tools = buildMemoryHandlers(cfg);
            const rememberTool = tools.find((t) => t.name === "memory_remember");

            // 1. text 输入
            const resText = await rememberTool.execute({ text: "爹地今天吃了番茄炒蛋" });
            assert.equal(resText.ok, true);
            assert.equal(resText.data, "已写入，等待索引（md 已落盘）。");

            assert.equal(callLog.length, 2);
            assert.equal(callLog[0].url, "/api/v2/memory/add");
            assert.equal(callLog[0].body.messages[0].sender_id, "aki");
            assert.equal(callLog[0].body.messages[0].sender_name, "爹地");
            assert.equal(callLog[0].body.messages[0].role, "user");
            assert.equal(callLog[0].body.messages[0].content, "爹地今天吃了番茄炒蛋");
            assert.match(callLog[0].body.session_id, /^riko-manual-\d{8}$/);

            assert.equal(callLog[1].url, "/api/v2/memory/flush");
            assert.equal(callLog[1].body.session_id, callLog[0].body.session_id);

            callLog.length = 0;

            // 2. messages 数组输入（测试 user 与 assistant 的 sender 映射）
            const resMsgs = await rememberTool.execute({
                messages: [
                    { role: "user", content: "今天吃什么？" },
                    { role: "assistant", content: "吃甜口辣子鸡。" },
                ],
            });
            assert.equal(resMsgs.ok, true);
            assert.equal(callLog[0].body.messages[0].sender_id, "aki");
            assert.equal(callLog[0].body.messages[0].role, "user");
            assert.equal(callLog[0].body.messages[1].sender_id, "riko");
            assert.equal(callLog[0].body.messages[1].role, "assistant");
        }
    );

    // 3. flush 失败时的降级返回
    await withMockEverOS(
        (req, res) => {
            if (req.url === "/api/v2/memory/add") {
                res.writeHead(200, { "Content-Type": "application/json" });
                res.end(JSON.stringify({ status: "ok" }));
            } else {
                res.writeHead(500, { "Content-Type": "application/json" });
                res.end(JSON.stringify({ error: "flush busy" }));
            }
        },
        async (everosUrl) => {
            const cfg = resolveConfig({ everosUrl });
            const tools = buildMemoryHandlers(cfg);
            const rememberTool = tools.find((t) => t.name === "memory_remember");

            const resFlushFail = await rememberTool.execute({ text: "临时测试" });
            assert.equal(resFlushFail.ok, true);
            assert.match(resFlushFail.data, /^已写入缓冲区，索引稍后重试：/);
        }
    );
});

test("P6: memory_profile - 请求体与格式化，空画像返回，超过50条截断", async () => {
    let mode = "has_profile";
    let recordedReq = null;

    await withMockEverOS(
        (req, res, body) => {
            recordedReq = { url: req.url, body };
            if (mode === "has_profile") {
                res.writeHead(200, { "Content-Type": "application/json" });
                res.end(
                    JSON.stringify({
                        data: {
                            profiles: [
                                {
                                    id: "prof_1",
                                    profile_data: {
                                        summary: "关于爹地的核心事实与日常偏好",
                                        explicit_info: [
                                            {
                                                category: "称呼",
                                                description: "爹地是人类开发者",
                                                evidence: "日常对话",
                                                observed_at: "2026-10-30",
                                            },
                                            {
                                                category: "偏好",
                                                description: "甜口辣子鸡",
                                                evidence: "夜宵点单记录",
                                                observed_at: "2026-10-31",
                                            },
                                        ],
                                        implicit_traits: ["对妹妹很温柔", "喜欢技术细节清晰"],
                                        profile_timestamp_ms: 1730371200000,
                                    },
                                },
                            ],
                        },
                    })
                );
            } else if (mode === "overflow") {
                const manyInfo = Array.from({ length: 55 }, (_, i) => ({
                    category: `分类${i + 1}`,
                    description: `事实条目${i + 1}`,
                    evidence: `证据${i + 1}`,
                }));
                res.writeHead(200, { "Content-Type": "application/json" });
                res.end(
                    JSON.stringify({
                        data: {
                            profiles: [
                                {
                                    id: "prof_many",
                                    profile_data: {
                                        summary: "超长事实画像",
                                        explicit_info: manyInfo,
                                    },
                                },
                            ],
                        },
                    })
                );
            } else {
                res.writeHead(200, { "Content-Type": "application/json" });
                res.end(JSON.stringify({ data: { profiles: [] } }));
            }
        },
        async (everosUrl) => {
            const cfg = resolveConfig({ everosUrl });
            const tools = buildMemoryHandlers(cfg);
            const profileTool = tools.find((t) => t.name === "memory_profile");

            // 1. 有真实形状画像：断言请求体无 limit/offset 且含 page=1, page_size=1
            const res = await profileTool.execute({});
            assert.equal(res.ok, true);
            assert.equal(recordedReq.url, "/api/v2/memory/get");
            assert.equal(recordedReq.body.user_id, "aki");
            assert.equal(recordedReq.body.app_id, "riko");
            assert.equal(recordedReq.body.project_id, "main");
            assert.equal(recordedReq.body.memory_type, "profile");
            assert.equal(recordedReq.body.page, 1);
            assert.equal(recordedReq.body.page_size, 1);
            assert.equal(recordedReq.body.limit, undefined);
            assert.equal(recordedReq.body.offset, undefined);

            assert.match(res.data, /^摘要：关于爹地的核心事实与日常偏好/);
            assert.match(res.data, /- \[称呼\] 爹地是人类开发者/);
            assert.match(res.data, /- \[偏好\] 甜口辣子鸡/);
            assert.match(res.data, /隐性特质：\n- 对妹妹很温柔\n- 喜欢技术细节清晰/);

            // 2. 超出 50 条 explicit_info 截断断言
            mode = "overflow";
            const resOverflow = await profileTool.execute({});
            assert.equal(resOverflow.ok, true);
            assert.match(resOverflow.data, /- \[分类1\] 事实条目1/);
            assert.match(resOverflow.data, /- \[分类50\] 事实条目50/);
            assert.match(resOverflow.data, /- …（共 55 条，已显示 50 条）/);
            assert.equal(resOverflow.data.includes("分类51"), false);

            // 3. 空画像
            mode = "empty";
            const resEmpty = await profileTool.execute({});
            assert.equal(resEmpty.ok, true);
            assert.equal(resEmpty.data, "暂无画像。");
        }
    );
});

test("P7: memory_list - 分页参数与输出行格式", async () => {
    let recordedReq = null;
    let mode = "normal";

    await withMockEverOS(
        (req, res, body) => {
            recordedReq = { url: req.url, body };
            if (mode === "normal") {
                res.writeHead(200, { "Content-Type": "application/json" });
                res.end(
                    JSON.stringify({
                        data: {
                            total_count: 15,
                            episodes: [
                                {
                                    id: "ep_1",
                                    timestamp: "2026-10-31T12:00:00+08:00",
                                    summary: "今天完成了 Phase C 的插件开发。",
                                },
                                {
                                    id: "ep_2",
                                    timestamp: "2026-10-30T10:00:00+08:00",
                                    summary: "爹地拍板记忆系统方案。",
                                },
                            ],
                        },
                    })
                );
            } else if (mode === "empty_page") {
                res.writeHead(200, { "Content-Type": "application/json" });
                res.end(
                    JSON.stringify({
                        data: {
                            total_count: 15,
                            episodes: [],
                        },
                    })
                );
            }
        },
        async (everosUrl) => {
            const cfg = resolveConfig({ everosUrl });
            const tools = buildMemoryHandlers(cfg);
            const listTool = tools.find((t) => t.name === "memory_list");

            // 1. offset=0, limit=10 -> page=1, page_size=10
            const res1 = await listTool.execute({ limit: 10, offset: 0 });
            assert.equal(res1.ok, true);
            assert.equal(recordedReq.body.page, 1);
            assert.equal(recordedReq.body.page_size, 10);
            assert.equal(recordedReq.body.limit, undefined);
            assert.equal(recordedReq.body.offset, undefined);
            assert.equal(recordedReq.body.memory_type, "episode");

            assert.match(res1.data, /- 2026-10-31 今天完成了 Phase C 的插件开发。/);
            assert.match(res1.data, /- 2026-10-30 爹地拍板记忆系统方案。/);
            assert.match(res1.data, /共 15 条，已显示 1-2$/);

            // 2. offset=10, limit=10 -> page=2, page_size=10
            const res2 = await listTool.execute({ limit: 10, offset: 10 });
            assert.equal(res2.ok, true);
            assert.equal(recordedReq.body.page, 2);
            assert.equal(recordedReq.body.page_size, 10);
            assert.equal(recordedReq.body.limit, undefined);
            assert.equal(recordedReq.body.offset, undefined);
            assert.match(res2.data, /共 15 条，已显示 11-12$/);

            // 3. offset=15, limit=10 -> page=2, page_size=10, 对齐说明
            const res3 = await listTool.execute({ limit: 10, offset: 15 });
            assert.equal(res3.ok, true);
            assert.equal(recordedReq.body.page, 2);
            assert.equal(recordedReq.body.page_size, 10);
            assert.match(res3.data, /共 15 条，已显示 11-12/);
            assert.match(res3.data, /（offset 按页对齐到 10）/);

            // 4. 空页时：共 15 条，已显示 0-0
            mode = "empty_page";
            const resEmpty = await listTool.execute({ limit: 10, offset: 50 });
            assert.equal(resEmpty.ok, true);
            assert.equal(resEmpty.data, "共 15 条，已显示 0-0");
        }
    );
});

test("P8: memory_status - 健康与不可用状态检查", async () => {
    let mode = "no_root";

    await withMockEverOS(
        (req, res) => {
            if (mode === "no_root") {
                // 真实 EverOS 1.6.0 GET /health 无 root 字段
                res.writeHead(200, { "Content-Type": "application/json" });
                res.end(
                    JSON.stringify({
                        version: "1.6.0",
                        status: "ok",
                        capabilities: { embed: true },
                        cascade: { healthy: true },
                    })
                );
            } else if (mode === "has_root") {
                res.writeHead(200, { "Content-Type": "application/json" });
                res.end(
                    JSON.stringify({
                        version: "1.6.0",
                        status: "ok",
                        capabilities: { embed: true },
                        cascade: { healthy: true },
                        root: "/custom/everos-root",
                    })
                );
            } else {
                res.writeHead(503, { "Content-Type": "application/json" });
                res.end(JSON.stringify({ status: "degraded", error: "database locked" }));
            }
        },
        async (everosUrl) => {
            const cfg = resolveConfig({ everosUrl });
            const tools = buildMemoryHandlers(cfg);
            const statusTool = tools.find((t) => t.name === "memory_status");

            // 1. 真实情况：health 无 root，回退到 cfg.mdRoot（默认 /data/everos）
            const resDefaultRoot = await statusTool.execute({});
            assert.equal(resDefaultRoot.ok, true);
            assert.equal(
                resDefaultRoot.data,
                "EverOS 1.6.0 status=ok embed=true cascade=true · md 根：/data/everos"
            );

            // 2. health 返回 root 时优先使用
            mode = "has_root";
            const resExplicitRoot = await statusTool.execute({});
            assert.equal(resExplicitRoot.ok, true);
            assert.equal(
                resExplicitRoot.data,
                "EverOS 1.6.0 status=ok embed=true cascade=true · md 根：/custom/everos-root"
            );

            // 3. 失败
            mode = "fail";
            const resFail = await statusTool.execute({});
            assert.equal(resFail.ok, false);
            assert.match(resFail.error.message, /^EverOS 不可用：/);
        }
    );
});

test("P11: 超时 - 假服务挂起时在 timeoutMs 内返回错误且不崩溃", async () => {
    await withMockEverOS(
        (_req, _res) => {
            // 故意不响应，模拟挂起
        },
        async (everosUrl) => {
            const cfg = resolveConfig({
                everosUrl,
                timeoutMs: 50, // 50ms 超时
            });
            const tools = buildMemoryHandlers(cfg);
            const searchTool = tools.find((t) => t.name === "memory_search");

            const start = Date.now();
            const res = await searchTool.execute({ query: "超时测试" });
            const duration = Date.now() - start;

            assert.equal(res.ok, false);
            assert.match(res.error.message, /记忆服务不可用：.*超时/);
            assert.ok(duration < 1500, `应在短时间内超时，实际耗时 ${duration}ms`);
        }
    );
});
