import test from "node:test";
import assert from "node:assert/strict";
import { resolveConfig } from "../dist/config.js";
import { buildMemoryHandlers, RESULT_SCHEMA } from "../dist/tools.js";

test("P2: 工具 schema - 5 个工具名与 schema 结构正确", () => {
    const cfg = resolveConfig({});
    const tools = buildMemoryHandlers(cfg, console);

    assert.equal(tools.length, 5);

    const names = tools.map((t) => t.name);
    assert.deepEqual(names, [
        "memory_search",
        "memory_remember",
        "memory_profile",
        "memory_list",
        "memory_status",
    ]);

    for (const tool of tools) {
        assert.ok(tool.name, "工具必须有 name");
        assert.ok(tool.description, "工具必须有 description");
        assert.ok(tool.parameters, "工具必须有 parameters");
        assert.equal(typeof tool.execute, "function", "工具必须有 execute 函数");
        assert.deepEqual(tool.output.schema, RESULT_SCHEMA, "output schema 必须匹配规范");
    }

    // 1. memory_search 参数 schema
    const searchTool = tools.find((t) => t.name === "memory_search");
    assert.ok(searchTool.parameters.query, "memory_search 必须支持 query");
    assert.ok(searchTool.parameters.queries, "memory_search 必须支持 queries");
    assert.ok(searchTool.parameters.limit, "memory_search 必须支持 limit");
    assert.ok(searchTool.parameters.include_profile, "memory_search 必须支持 include_profile");

    // 2. memory_remember 参数 schema
    const rememberTool = tools.find((t) => t.name === "memory_remember");
    assert.ok(rememberTool.parameters.text, "memory_remember 必须支持 text");
    assert.ok(rememberTool.parameters.messages, "memory_remember 必须支持 messages");

    // 3. memory_profile 参数 schema
    const profileTool = tools.find((t) => t.name === "memory_profile");
    assert.ok(profileTool.parameters.user_id, "memory_profile 必须支持 user_id");

    // 4. memory_list 参数 schema
    const listTool = tools.find((t) => t.name === "memory_list");
    assert.ok(listTool.parameters.limit, "memory_list 必须支持 limit");
    assert.ok(listTool.parameters.offset, "memory_list 必须支持 offset");

    // 5. memory_status 参数 schema
    const statusTool = tools.find((t) => t.name === "memory_status");
    assert.deepEqual(Object.keys(statusTool.parameters), [], "memory_status 无必填入参");
});
