import test from "node:test";
import assert from "node:assert/strict";
import { resolveConfig, isWorkspaceAllowed, isTopLevelSession } from "../dist/config.js";
import { buildDisabledTools } from "../dist/tools.js";

test("P1: 配置解析 - 缺省值填充与合法配置", () => {
    const cfg = resolveConfig({});
    assert.equal(cfg.everosUrl, "http://127.0.0.1:18100");
    assert.equal(cfg.appId, "riko");
    assert.equal(cfg.projectId, "main");
    assert.equal(cfg.userId, "aki");
    assert.equal(cfg.agentId, "riko");
    assert.equal(cfg.timeoutMs, 120000);
    assert.equal(cfg.soulEnabled, true);
    assert.equal(cfg.soulFile, "/root/riko/soul.md");
    assert.equal(cfg.residentEnabled, true);
    assert.equal(cfg.residentFile, "/root/riko/memory/resident.md");
    assert.equal(cfg.residentBudgetChars, 6000);
    assert.equal(cfg.toolsEnabled, true);
    assert.deepEqual(cfg.workspaceAllow, ["/root/riko"]);

    // 自定义合法配置覆盖
    const custom = resolveConfig({
        everosUrl: "https://everos.internal:18100/",
        timeoutMs: 30000,
        residentBudgetChars: 4000,
        workspaceAllow: ["/data/custom"],
    });
    assert.equal(custom.everosUrl, "https://everos.internal:18100");
    assert.equal(custom.timeoutMs, 30000);
    assert.equal(custom.residentBudgetChars, 4000);
    assert.deepEqual(custom.workspaceAllow, ["/data/custom"]);
});

test("P1: 配置解析 - 非法值报错且提供不崩溃的假工具保护", async () => {
    // 非法值校验抛出清晰错误
    assert.throws(() => resolveConfig({ everosUrl: "" }), /everosUrl/);
    assert.throws(() => resolveConfig({ everosUrl: "ftp://invalid" }), /http/);
    assert.throws(() => resolveConfig({ timeoutMs: -100 }), /timeoutMs/);
    assert.throws(() => resolveConfig({ residentBudgetChars: 0 }), /residentBudgetChars/);
    assert.throws(() => resolveConfig({ workspaceAllow: "not-an-array" }), /workspaceAllow/);

    // 验证配置非法时生成的假工具：返回清晰错误文本，不让 DSH 崩溃
    let caughtErr = null;
    try {
        resolveConfig({ residentBudgetChars: -500 });
    } catch (err) {
        caughtErr = err;
    }
    assert.ok(caughtErr);

    const fallbackTools = buildDisabledTools(caughtErr.message);
    assert.equal(fallbackTools.length, 5);

    const searchTool = fallbackTools.find((t) => t.name === "memory_search");
    assert.ok(searchTool);
    const res = await searchTool.execute({ query: "test" });
    assert.equal(res.ok, false);
    assert.match(res.error.message, /记忆服务不可用：配置错误/);

    const statusTool = fallbackTools.find((t) => t.name === "memory_status");
    assert.ok(statusTool);
    const statusRes = await statusTool.execute({});
    assert.equal(statusRes.ok, false);
    assert.match(statusRes.error.message, /EverOS 不可用：配置错误/);
});

test("P1 辅助: 工作区白名单与顶层会话判定", () => {
    assert.equal(isWorkspaceAllowed(["/root/riko"], "/root/riko"), true);
    assert.equal(isWorkspaceAllowed(["/root/riko"], "/root/riko/src"), true);
    assert.equal(isWorkspaceAllowed(["/root/riko"], "/other/path"), false);
    assert.equal(isWorkspaceAllowed([], "/any/path"), true);

    assert.equal(isTopLevelSession({}), true);
    assert.equal(isTopLevelSession({ parentSession: "parent-1" }), false);
    assert.equal(isTopLevelSession({ origin: "subagent-task" }), false);
});
