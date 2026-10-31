/**
 * 统一的工具返回结构模式。
 * 兼容 DSH 标准工具返回规范。
 */
export const RESULT_SCHEMA = {
    type: "object",
    additionalProperties: false,
    properties: {
        ok: { type: "boolean", required: true },
        data: { type: "json" },
        error: {
            type: "object",
            additionalProperties: false,
            properties: {
                code: { type: "string", required: true },
                message: { type: "string", required: true },
            },
        },
    },
};

export function renderResult(_args, value) {
    if (typeof value === "string") {
        return [{ type: "text", text: value }];
    }
    if (value && typeof value === "object") {
        if (value.ok && typeof value.data === "string") {
            return [{ type: "text", text: value.data }];
        }
        if (!value.ok && value.error && typeof value.error.message === "string") {
            return [{ type: "text", text: value.error.message }];
        }
    }
    return [{ type: "text", text: JSON.stringify(value) }];
}

export function toToolResult(r) {
    if (r.ok) {
        return { ok: true, data: r.data ?? null };
    }
    return { ok: false, error: r.error };
}

export function bad(code, message) {
    return { ok: false, error: { code, message } };
}

function str(v) {
    return typeof v === "string" && v.trim() ? v.trim() : undefined;
}

function arr(v) {
    if (!Array.isArray(v)) return undefined;
    const out = v.filter((x) => typeof x === "string" && x.trim().length > 0);
    return out.length > 0 ? out : undefined;
}

/**
 * 当配置非法时，构建返回清晰错误信息的假工具集，防止 DSH 崩溃同时提示错误。
 */
export function buildDisabledHandlers(configErrorMsg) {
    const errorResult = (name) => {
        if (name === "memory_status") {
            return bad("EVEROS_UNAVAILABLE", `EverOS 不可用：配置错误: ${configErrorMsg}`);
        }
        return bad("CONFIG_ERROR", `记忆服务不可用：配置错误: ${configErrorMsg}`);
    };

    return [
        mk("memory_search", "搜索长期记忆。", {}, async () => errorResult("memory_search")),
        mk("memory_remember", "写入长期记忆。", {}, async () => errorResult("memory_remember")),
        mk("memory_profile", "读取长期用户画像。", {}, async () => errorResult("memory_profile")),
        mk("memory_list", "分页列出记忆事件。", {}, async () => errorResult("memory_list")),
        mk("memory_status", "查看 EverOS 状态。", {}, async () => errorResult("memory_status")),
    ];
}
export const buildDisabledTools = buildDisabledHandlers;

/**
 * 统一工具包装函数，不依赖 @deepseek-ai/dsh-tools，纯 JS 对象构造。
 */
export function mk(name, description, parameters, run) {
    return {
        name,
        description,
        parameters,
        output: { schema: RESULT_SCHEMA, render: renderResult },
        async execute(args, exec) {
            return run(args ?? {}, exec);
        },
    };
}

const S = (description, required = false) =>
    required ? { type: "string", required, description } : { type: "string", description };
const N = (description) => ({ type: "number", description });
const B = (description, required = false) =>
    required ? { type: "boolean", required, description } : { type: "boolean", description };
const J = (description) => ({ type: "json", description });

/**
 * 格式化 Asia/Singapore 日期字符串。
 */
export function formatSingaporeDate(dateInput, withTime = true) {
    if (!dateInput) return "时间未知";
    const d = new Date(dateInput);
    if (Number.isNaN(d.getTime())) return String(dateInput);

    const formatter = new Intl.DateTimeFormat("en-CA", {
        timeZone: "Asia/Singapore",
        year: "numeric",
        month: "2-digit",
        day: "2-digit",
        hour: withTime ? "2-digit" : undefined,
        minute: withTime ? "2-digit" : undefined,
        hour12: false,
    });
    const parts = formatter.formatToParts(d);
    const getPart = (type) => parts.find((p) => p.type === type)?.value ?? "";
    const y = getPart("year");
    const m = getPart("month");
    const day = getPart("day");
    if (!withTime) {
        return `${y}-${m}-${day}`;
    }
    const h = getPart("hour");
    const min = getPart("minute");
    return `${y}-${m}-${day} ${h}:${min}`;
}

/**
 * 获取 Asia/Singapore 当天 YYYYMMDD 格式字符串。
 */
export function getSingaporeYMD(dateInput = new Date()) {
    const d = new Date(dateInput);
    const formatter = new Intl.DateTimeFormat("en-CA", {
        timeZone: "Asia/Singapore",
        year: "numeric",
        month: "2-digit",
        day: "2-digit",
    });
    const parts = formatter.formatToParts(d);
    const y = parts.find((p) => p.type === "year")?.value ?? "";
    const m = parts.find((p) => p.type === "month")?.value ?? "";
    const day = parts.find((p) => p.type === "day")?.value ?? "";
    return `${y}${m}${day}`;
}

/**
 * 带超时与外部 signal 监听的 fetch 包装。
 */
export async function doFetch(url, options = {}, timeoutMs = 120000, externalSignal) {
    const controller = new AbortController();
    const timeoutId = setTimeout(() => {
        controller.abort(new Error(`请求超时（超 ${timeoutMs}ms 未响应）`));
    }, timeoutMs);

    let onExternalAbort = null;
    if (externalSignal) {
        if (externalSignal.aborted) {
            clearTimeout(timeoutId);
            controller.abort(externalSignal.reason);
        } else {
            onExternalAbort = () => {
                controller.abort(externalSignal.reason);
            };
            externalSignal.addEventListener("abort", onExternalAbort, { once: true });
        }
    }

    try {
        const res = await fetch(url, {
            ...options,
            signal: controller.signal,
        });
        return res;
    } finally {
        clearTimeout(timeoutId);
        if (externalSignal && onExternalAbort) {
            externalSignal.removeEventListener("abort", onExternalAbort);
        }
    }
}

/**
 * 格式化 profile_data 为人类可读文本。
 */
export function formatProfileData(data) {
    if (!data || typeof data !== "object") {
        return null;
    }
    const lines = [];
    if (typeof data.summary === "string" && data.summary.trim()) {
        lines.push(`摘要：${data.summary.trim()}`);
    }

    if (Array.isArray(data.explicit_info) && data.explicit_info.length > 0) {
        const total = data.explicit_info.length;
        const slice = data.explicit_info.slice(0, 50);
        for (const item of slice) {
            const cat = item?.category ? String(item.category).trim() : "未分类";
            const desc = item?.description
                ? String(item.description).trim()
                : (item?.content ? String(item.content).trim() : JSON.stringify(item));
            lines.push(`- [${cat}] ${desc}`);
        }
        if (total > 50) {
            lines.push(`- …（共 ${total} 条，已显示 50 条）`);
        }
    }

    if (Array.isArray(data.implicit_traits) && data.implicit_traits.length > 0) {
        lines.push("隐性特质：");
        for (const trait of data.implicit_traits) {
            const t = typeof trait === "string" ? trait.trim() : (trait?.description ?? trait?.content ?? JSON.stringify(trait));
            lines.push(`- ${t}`);
        }
    }

    if (lines.length > 0) {
        return lines.join("\n");
    }
    return null;
}

/**
 * 构建 5 个与 EverOS 后端对接的记忆工具处理函数（handlers）。
 */
export function buildMemoryHandlers(cfg, logger) {
    // 1. memory_search
    const memorySearch = mk(
        "memory_search",
        "搜索长期记忆。支持单查询或多查询数组混合检索，自动去重合并并格式化输出。",
        {
            query: S("要查找的记忆内容（与 queries 二选一）"),
            queries: J("多查询数组，多路检索后自动融合去重"),
            limit: N("返回数量上限，默认 5（1-20）"),
            include_profile: B("是否同时召回用户画像，默认 false"),
        },
        async (args, exec) => {
            const singleQuery = str(args.query);
            const multiQueries = arr(args.queries);
            if (!singleQuery && !multiQueries) {
                return bad("INVALID_FIELD", "query 或 queries 必须提供一个");
            }

            let limit = 5;
            if (typeof args.limit === "number" && Number.isFinite(args.limit)) {
                limit = Math.max(1, Math.min(20, Math.floor(args.limit)));
            }
            const includeProfile = Boolean(args.include_profile);
            const queryList = multiQueries ?? [singleQuery];

            try {
                const results = await Promise.all(
                    queryList.map(async (q) => {
                        const payload = {
                            user_id: cfg.userId,
                            app_id: cfg.appId,
                            project_id: cfg.projectId,
                            query: q,
                            method: "hybrid",
                            top_k: limit,
                            include_profile: includeProfile,
                        };
                        const res = await doFetch(
                            `${cfg.everosUrl}/api/v2/memory/search`,
                            {
                                method: "POST",
                                headers: { "Content-Type": "application/json" },
                                body: JSON.stringify(payload),
                            },
                            cfg.timeoutMs,
                            exec?.signal
                        );
                        if (!res.ok) {
                            const errBody = await res.text().catch(() => "");
                            throw new Error(`HTTP ${res.status} ${errBody}`.trim());
                        }
                        return res.json();
                    })
                );

                // 多 query 去重合并：按 episode id 去重，保留最高 score
                const episodeMap = new Map();
                for (const r of results) {
                    const episodes = Array.isArray(r?.data?.episodes) ? r.data.episodes : [];
                    for (const ep of episodes) {
                        const id = ep.id;
                        if (!id) continue;
                        const score = typeof ep.score === "number" ? ep.score : 0;
                        if (!episodeMap.has(id) || episodeMap.get(id).score < score) {
                            episodeMap.set(id, { ...ep, score });
                        }
                    }
                }

                const mergedEpisodes = Array.from(episodeMap.values())
                    .sort((a, b) => b.score - a.score)
                    .slice(0, limit);

                if (mergedEpisodes.length === 0) {
                    return { ok: true, data: "没有找到相关记忆。" };
                }

                // 格式化输出
                const lines = mergedEpisodes.map((ep, idx) => {
                    const dateStr = formatSingaporeDate(ep.timestamp, true);
                    const subject = ep.subject ? ep.subject.trim() : (ep.summary ? ep.summary.slice(0, 20) : "未命名记忆");
                    const scoreStr = (ep.score ?? 0).toFixed(2);
                    const header = `${idx + 1}. [episode] ${dateStr} · ${subject}（score ${scoreStr}）`;

                    const summaryText = typeof ep.summary === "string" ? ep.summary.trim().slice(0, 300) : "";
                    const summaryLine = `   摘要：${summaryText}`;

                    const facts = Array.isArray(ep.atomic_facts) ? ep.atomic_facts.slice(0, 5) : [];
                    if (facts.length > 0) {
                        const factLines = facts.map((f, fIdx) => {
                            const c = typeof f === "string" ? f.trim() : (f?.content ? String(f.content).trim() : "");
                            return fIdx === 0 ? `   事实：- ${c}` : `         - ${c}`;
                        }).join("\n");
                        return `${header}\n${summaryLine}\n${factLines}`;
                    }

                    return `${header}\n${summaryLine}`;
                });

                return { ok: true, data: lines.join("\n\n") };
            } catch (err) {
                const msg = err instanceof Error ? err.message : String(err);
                logger?.warn?.(`riko-memory-dsh: memory_search 失败: ${msg}`);
                return bad("SERVICE_UNAVAILABLE", `记忆服务不可用：${msg}`);
            }
        }
    );

    // 2. memory_remember
    const memoryRemember = mk(
        "memory_remember",
        "主动写入记忆。支持 text 单句快速记录或 messages 完整对话数组，写入后自动触发 flush 抽取与索引。",
        {
            text: S("要保存的事实正文（与 messages 二选一）"),
            messages: J("对话消息数组 [{role, content}]，与 text 二选一"),
        },
        async (args, exec) => {
            const textVal = str(args.text);
            const hasMessages = Array.isArray(args.messages) && args.messages.length > 0;
            if (!textVal && !hasMessages) {
                return bad("INVALID_FIELD", "text 或 messages 必须提供一个");
            }

            const nowMs = Date.now();
            let messagesList = [];
            if (textVal) {
                messagesList = [
                    {
                        sender_id: cfg.userId,
                        sender_name: "爹地",
                        role: "user",
                        timestamp: nowMs,
                        content: textVal,
                    },
                ];
            } else {
                messagesList = args.messages.map((m, idx) => {
                    const isUser = m.role === "user";
                    return {
                        sender_id: isUser ? cfg.userId : cfg.agentId,
                        sender_name: isUser ? "爹地" : "璃",
                        role: isUser ? "user" : "assistant",
                        timestamp: typeof m.timestamp === "number" ? m.timestamp : nowMs + idx,
                        content: typeof m.content === "string" ? m.content : JSON.stringify(m.content ?? ""),
                    };
                });
            }

            const sessionId = `riko-manual-${getSingaporeYMD(new Date())}`;

            // Step 1: POST /api/v2/memory/add
            try {
                const addPayload = {
                    session_id: sessionId,
                    app_id: cfg.appId,
                    project_id: cfg.projectId,
                    defer_extraction: false,
                    messages: messagesList,
                };

                const addRes = await doFetch(
                    `${cfg.everosUrl}/api/v2/memory/add`,
                    {
                        method: "POST",
                        headers: { "Content-Type": "application/json" },
                        body: JSON.stringify(addPayload),
                    },
                    cfg.timeoutMs,
                    exec?.signal
                );

                if (!addRes.ok) {
                    const errText = await addRes.text().catch(() => "");
                    throw new Error(`HTTP ${addRes.status} ${errText}`.trim());
                }
            } catch (addErr) {
                const msg = addErr instanceof Error ? addErr.message : String(addErr);
                logger?.warn?.(`riko-memory-dsh: memory_remember add 失败: ${msg}`);
                return bad("SERVICE_UNAVAILABLE", `记忆服务不可用：${msg}`);
            }

            // Step 2: POST /api/v2/memory/flush
            try {
                const flushPayload = {
                    session_id: sessionId,
                    app_id: cfg.appId,
                    project_id: cfg.projectId,
                };

                const flushRes = await doFetch(
                    `${cfg.everosUrl}/api/v2/memory/flush`,
                    {
                        method: "POST",
                        headers: { "Content-Type": "application/json" },
                        body: JSON.stringify(flushPayload),
                    },
                    cfg.timeoutMs,
                    exec?.signal
                );

                if (!flushRes.ok) {
                    const flushErrText = await flushRes.text().catch(() => "");
                    throw new Error(`HTTP ${flushRes.status} ${flushErrText}`.trim());
                }
            } catch (flushErr) {
                const msg = flushErr instanceof Error ? flushErr.message : String(flushErr);
                logger?.warn?.(`riko-memory-dsh: memory_remember flush 失败: ${msg}`);
                return {
                    ok: true,
                    data: `已写入缓冲区，索引稍后重试：${msg}`,
                };
            }

            return { ok: true, data: "已写入，等待索引（md 已落盘）。" };
        }
    );

    // 3. memory_profile
    const memoryProfile = mk(
        "memory_profile",
        "读取长期用户画像（可溯源事实）。",
        {
            user_id: S("用户 ID，默认当前配置"),
        },
        async (args, exec) => {
            const targetUser = str(args.user_id) ?? cfg.userId;
            const payload = {
                user_id: targetUser,
                app_id: cfg.appId,
                project_id: cfg.projectId,
                memory_type: "profile",
                page: 1,
                page_size: 1,
            };

            try {
                const res = await doFetch(
                    `${cfg.everosUrl}/api/v2/memory/get`,
                    {
                        method: "POST",
                        headers: { "Content-Type": "application/json" },
                        body: JSON.stringify(payload),
                    },
                    cfg.timeoutMs,
                    exec?.signal
                );

                if (!res.ok) {
                    const errText = await res.text().catch(() => "");
                    throw new Error(`HTTP ${res.status} ${errText}`.trim());
                }

                const json = await res.json();
                const profiles = Array.isArray(json?.data?.profiles) ? json.data.profiles : [];
                if (profiles.length === 0) {
                    return { ok: true, data: "暂无画像。" };
                }

                const first = profiles[0];
                let outText = "";
                const formatted = formatProfileData(first.profile_data);
                if (formatted !== null) {
                    outText = formatted;
                } else if (first.profile_data && typeof first.profile_data === "object") {
                    outText = Object.entries(first.profile_data)
                        .map(([k, v]) => `${k}：${typeof v === "object" ? JSON.stringify(v) : v}`)
                        .join("\n");
                } else if (typeof first.content === "string" && first.content.trim()) {
                    outText = first.content.trim();
                } else {
                    outText = JSON.stringify(first);
                }

                return { ok: true, data: outText };
            } catch (err) {
                const msg = err instanceof Error ? err.message : String(err);
                logger?.warn?.(`riko-memory-dsh: memory_profile 失败: ${msg}`);
                return bad("SERVICE_UNAVAILABLE", `记忆服务不可用：${msg}`);
            }
        }
    );

    // 4. memory_list
    const memoryList = mk(
        "memory_list",
        "分页列出记忆事件（episodes）。",
        {
            limit: N("每页条数，默认 10（1-50）"),
            offset: N("分页偏移量，默认 0"),
        },
        async (args, exec) => {
            let limit = 10;
            if (typeof args.limit === "number" && Number.isFinite(args.limit)) {
                limit = Math.max(1, Math.min(50, Math.floor(args.limit)));
            }
            let offset = 0;
            if (typeof args.offset === "number" && Number.isFinite(args.offset)) {
                offset = Math.max(0, Math.floor(args.offset));
            }

            const page = Math.floor(offset / limit) + 1;
            const pageSize = limit;
            const alignedOffset = (page - 1) * pageSize;
            const isUnaligned = offset % limit !== 0;

            const payload = {
                user_id: cfg.userId,
                app_id: cfg.appId,
                project_id: cfg.projectId,
                memory_type: "episode",
                page,
                page_size: pageSize,
            };

            try {
                const res = await doFetch(
                    `${cfg.everosUrl}/api/v2/memory/get`,
                    {
                        method: "POST",
                        headers: { "Content-Type": "application/json" },
                        body: JSON.stringify(payload),
                    },
                    cfg.timeoutMs,
                    exec?.signal
                );

                if (!res.ok) {
                    const errText = await res.text().catch(() => "");
                    throw new Error(`HTTP ${res.status} ${errText}`.trim());
                }

                const json = await res.json();
                const episodes = Array.isArray(json?.data?.episodes) ? json.data.episodes : [];
                const totalCount = typeof json?.data?.total_count === "number"
                    ? json.data.total_count
                    : (typeof json?.total_count === "number" ? json.total_count : episodes.length);

                const itemLines = episodes.map((ep) => {
                    const dateStr = formatSingaporeDate(ep.timestamp, false);
                    const summary = typeof ep.summary === "string" ? ep.summary.trim().slice(0, 120) : "";
                    return `- ${dateStr} ${summary}`;
                });

                const countShown = episodes.length;
                const start = (page - 1) * pageSize + 1;
                const end = start + countShown - 1;
                const rangeStr = countShown > 0 ? `${start}-${end}` : "0-0";
                let footer = `共 ${totalCount} 条，已显示 ${rangeStr}`;
                if (isUnaligned) {
                    footer += `\n（offset 按页对齐到 ${alignedOffset}）`;
                }

                const resultText = itemLines.length > 0 ? `${itemLines.join("\n")}\n${footer}` : footer;
                return { ok: true, data: resultText };
            } catch (err) {
                const msg = err instanceof Error ? err.message : String(err);
                logger?.warn?.(`riko-memory-dsh: memory_list 失败: ${msg}`);
                return bad("SERVICE_UNAVAILABLE", `记忆服务不可用：${msg}`);
            }
        }
    );

    // 5. memory_status
    const memoryStatus = mk(
        "memory_status",
        "查看 EverOS 记忆服务状态与健康检查结果。",
        {},
        async (_args, exec) => {
            try {
                const res = await doFetch(
                    `${cfg.everosUrl}/health`,
                    { method: "GET" },
                    cfg.timeoutMs,
                    exec?.signal
                );

                if (!res.ok) {
                    const errText = await res.text().catch(() => "");
                    throw new Error(`HTTP ${res.status} ${errText}`.trim());
                }

                const json = await res.json();
                const version = json.version ?? "1.6.0";
                const status = json.status ?? "ok";
                const embed = json.capabilities?.embed ?? json.embed ?? false;
                const cascade = typeof json.cascade?.healthy === "boolean"
                    ? json.cascade.healthy
                    : (typeof json.cascade === "boolean" ? json.cascade : false);
                const root = json.root ?? json.md_root ?? cfg.mdRoot ?? "/data/everos";

                return {
                    ok: true,
                    data: `EverOS ${version} status=${status} embed=${embed} cascade=${cascade} · md 根：${root}`,
                };
            } catch (err) {
                const msg = err instanceof Error ? err.message : String(err);
                logger?.warn?.(`riko-memory-dsh: memory_status 失败: ${msg}`);
                return bad("EVEROS_UNAVAILABLE", `EverOS 不可用：${msg}`);
            }
        }
    );

    return [
        memorySearch,
        memoryRemember,
        memoryProfile,
        memoryList,
        memoryStatus,
    ];
}

export const buildMemoryTools = buildMemoryHandlers;
