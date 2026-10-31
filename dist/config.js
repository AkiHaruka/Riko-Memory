/**
 * 路径规范化：统一斜杠、去除结尾斜杠。
 * 兼容 Windows 与 POSIX 路径。
 */
export function normalizeWorkspacePath(p) {
    if (typeof p !== "string") {
        return "";
    }
    return p.replace(/[\\/]+/g, "/").replace(/\/+$/, "").toLowerCase();
}

/**
 * 判断会话的 cwd 是否在白名单内。
 * 白名单为空表示不限制。
 */
export function isWorkspaceAllowed(workspaceAllow, cwd) {
    if (!Array.isArray(workspaceAllow) || workspaceAllow.length === 0) {
        return true;
    }
    if (typeof cwd !== "string" || cwd.trim().length === 0) {
        return false;
    }
    const current = normalizeWorkspacePath(cwd);
    return workspaceAllow.some((allowed) => {
        const root = normalizeWorkspacePath(allowed);
        return current === root || current.startsWith(root + "/");
    });
}

/**
 * 顶层会话判定：排除子代理（subagent/fork），防止人格与常驻记忆污染。
 */
export function isTopLevelSession(header) {
    if (!header || typeof header !== "object") {
        return true;
    }
    if (header.parentSession) {
        return false;
    }
    if (typeof header.origin === "string" && header.origin.toLowerCase().includes("subagent")) {
        return false;
    }
    return true;
}

/**
 * 规范化并校验传入的配置。
 * 字段缺失使用默认值；字段类型非法或越界时抛出详细错误。
 */
export function resolveConfig(raw = {}) {
    if (raw === null || typeof raw !== "object" || Array.isArray(raw)) {
        throw new Error("配置必须为对象");
    }

    // everosUrl
    let everosUrl = "http://127.0.0.1:18100";
    if (raw.everosUrl !== undefined) {
        if (typeof raw.everosUrl !== "string" || !raw.everosUrl.trim()) {
            throw new Error("everosUrl 必须为非空字符串");
        }
        const trimmed = raw.everosUrl.trim();
        if (!/^https?:\/\//i.test(trimmed)) {
            throw new Error("everosUrl 必须以 http:// 或 https:// 开头");
        }
        everosUrl = trimmed.replace(/\/+$/, "");
    }

    // appId / projectId / userId / agentId
    const appId = typeof raw.appId === "string" && raw.appId.trim() ? raw.appId.trim() : "riko";
    const projectId = typeof raw.projectId === "string" && raw.projectId.trim() ? raw.projectId.trim() : "main";
    const userId = typeof raw.userId === "string" && raw.userId.trim() ? raw.userId.trim() : "aki";
    const agentId = typeof raw.agentId === "string" && raw.agentId.trim() ? raw.agentId.trim() : "riko";

    // timeoutMs
    let timeoutMs = 120000;
    if (raw.timeoutMs !== undefined) {
        if (typeof raw.timeoutMs !== "number" || !Number.isFinite(raw.timeoutMs) || raw.timeoutMs <= 0) {
            throw new Error("timeoutMs 必须为大于 0 的有效数字");
        }
        timeoutMs = raw.timeoutMs;
    }

    // soul
    const soulEnabled = raw.soulEnabled !== false;
    let soulFile = process.env.RIKO_SOUL_FILE ?? "/root/riko/soul.md";
    if (raw.soulFile !== undefined) {
        if (typeof raw.soulFile !== "string" || !raw.soulFile.trim()) {
            throw new Error("soulFile 必须为非空字符串");
        }
        soulFile = raw.soulFile.trim();
    }

    // resident
    const residentEnabled = raw.residentEnabled !== false;
    let residentFile = "/root/riko/memory/resident.md";
    if (raw.residentFile !== undefined) {
        if (typeof raw.residentFile !== "string" || !raw.residentFile.trim()) {
            throw new Error("residentFile 必须为非空字符串");
        }
        residentFile = raw.residentFile.trim();
    }

    let residentBudgetChars = 6000;
    if (raw.residentBudgetChars !== undefined) {
        if (typeof raw.residentBudgetChars !== "number" || !Number.isFinite(raw.residentBudgetChars) || raw.residentBudgetChars <= 0) {
            throw new Error("residentBudgetChars 必须为大于 0 的有效数字");
        }
        residentBudgetChars = raw.residentBudgetChars;
    }

    // tools
    const toolsEnabled = raw.toolsEnabled !== false;

    // workspaceAllow
    let workspaceAllow = ["/root/riko"];
    if (raw.workspaceAllow !== undefined) {
        if (!Array.isArray(raw.workspaceAllow)) {
            throw new Error("workspaceAllow 必须为数组");
        }
        workspaceAllow = raw.workspaceAllow.filter((w) => typeof w === "string" && w.trim().length > 0);
    }

    return {
        everosUrl,
        appId,
        projectId,
        userId,
        agentId,
        timeoutMs,
        soulEnabled,
        soulFile,
        residentEnabled,
        residentFile,
        residentBudgetChars,
        toolsEnabled,
        workspaceAllow,
    };
}
