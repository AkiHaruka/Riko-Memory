import { createUserMessage } from "@deepseek-ai/dsh-llm";
import { resolveConfig, isWorkspaceAllowed, isTopLevelSession } from "./config.js";
import { createSoulManager } from "./soul.js";
import { readResidentContent, formatResidentXml } from "./resident.js";
import { buildMemoryTools, buildDisabledTools } from "./define.js";

export const name = "riko-memory-dsh";
export const inject = ["tools"];

function makeLogger(ctx) {
    const anyCtx = ctx;
    const l = anyCtx?.logger;
    return {
        warn: (m) => l?.warn?.(m),
        error: (m) => l?.error?.(m),
        info: (m) => l?.info?.(m),
    };
}

/**
 * 在每个顶层 Agent 的 ctx 上挂 hook（DSH agent/created 模式）。
 */
function registerPerAgent(ctx, event, hook, workspaceAllow) {
    const host = ctx;
    host?.on?.("agent/created", (payload) => {
        const header = payload?.agent?.session?.header;
        if (!isTopLevelSession(header)) {
            return;
        }
        if (!isWorkspaceAllowed(workspaceAllow, header?.cwd)) {
            return;
        }
        const agentCtx = payload?.agent?.ctx;
        agentCtx?.on?.(event, hook);
    });
}

export async function apply(ctx, config = {}) {
    const logger = makeLogger(ctx);

    let cfg;
    let configError = null;
    try {
        cfg = resolveConfig(config);
    } catch (err) {
        configError = err;
        logger.error(`riko-memory-dsh: 配置解析失败: ${err.message}`);
    }

    // 如果配置非法：注册假工具返回错误信息，跳过注入，保证 DSH 不崩溃
    if (configError) {
        if (ctx?.tools?.register) {
            const disabledTools = buildDisabledTools(configError.message);
            for (const t of disabledTools) {
                ctx.tools.register(t);
            }
        }
        return;
    }

    // ---------- 1. 记忆工具 ----------
    if (cfg.toolsEnabled && ctx?.tools?.register) {
        const tools = buildMemoryTools(cfg, logger);
        for (const tool of tools) {
            ctx.tools.register(tool);
        }
        logger.info?.(`riko-memory-dsh: 已注册 5 个记忆工具 (user_id=${cfg.userId}, url=${cfg.everosUrl})`);
    }

    // ---------- 2. SOUL 注入 ----------
    if (cfg.soulEnabled) {
        const soulManager = createSoulManager({
            soulFile: cfg.soulFile,
            logger,
        });

        // 启动时读取一次
        soulManager.loadSoul("startup");

        // 仅在 compaction/end 事件重读 SOUL
        ctx?.on?.("session/event", async (session, event) => {
            if (event?.type !== "compaction/end") {
                return;
            }
            if (!isTopLevelSession(session?.header)) {
                return;
            }
            soulManager.loadSoul("compaction/end");
        });

        const soulHook = async (assembly, context, next) => {
            const result = await next();
            const header = context?.agent?.session?.header;
            if (!isTopLevelSession(header)) {
                return result;
            }
            if (!isWorkspaceAllowed(cfg.workspaceAllow, header?.cwd)) {
                return result;
            }
            if (result?.sections && result.sections.some((s) => s.name === "riko-soul")) {
                return result;
            }
            const section = soulManager.renderSoulSection();
            if (section && result?.sections) {
                result.sections.push(section);
            }
            return result;
        };

        registerPerAgent(ctx, "system-prompt/assemble", soulHook, cfg.workspaceAllow);
    }

    // ---------- 3. 常驻记忆注入 ----------
    if (cfg.residentEnabled) {
        let pending = false;

        // 仅在 compaction/end 之后标记 pending
        ctx?.on?.("session/event", (session, event) => {
            if (event?.type !== "compaction/end") {
                return;
            }
            if (!isTopLevelSession(session?.header)) {
                return;
            }
            pending = true;
        });

        const residentHook = async (payload, next) => {
            const decision = await next();
            if (!pending) {
                return decision;
            }
            pending = false;

            const p = payload;
            if (!isTopLevelSession(p?.agent?.session?.header)) {
                return decision;
            }
            if (!isWorkspaceAllowed(cfg.workspaceAllow, p?.agent?.session?.header?.cwd)) {
                return decision;
            }
            if (decision?.kind !== "enter" || p?.signal?.aborted) {
                return decision;
            }

            try {
                const text = readResidentContent({
                    residentFile: cfg.residentFile,
                    budgetChars: cfg.residentBudgetChars,
                    logger,
                });
                if (!text) {
                    return decision;
                }

                const xml = formatResidentXml(text);
                const message = createUserMessage({
                    content: [
                        {
                            type: "text",
                            text: xml,
                        },
                    ],
                    source: { kind: "system-prompt", form: "recall" },
                });

                return {
                    ...decision,
                    messages: [message, ...(decision.messages ?? [])],
                };
            } catch (err) {
                const msg = err instanceof Error ? err.message : String(err);
                logger.warn?.(`riko-memory-dsh: 常驻记忆注入异常: ${msg}`);
                return decision;
            }
        };

        registerPerAgent(ctx, "agent/pre-step", residentHook, cfg.workspaceAllow);
    }

    logger.info?.(
        `riko-memory-dsh: 插件加载完成 tools=${cfg.toolsEnabled} soul=${cfg.soulEnabled} resident=${cfg.residentEnabled} url=${cfg.everosUrl}`
    );
}
