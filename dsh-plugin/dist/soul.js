import { readFileSync } from "node:fs";
import { resolve } from "node:path";

export const SOUL_SECTION_NAME = "riko-soul";
export const SOUL_SECTION_ORDER = 100;

/**
 * 注入进 prompt 的文本进行 XML 转义，避免被当成 XML 指令或标签解析。
 */
export function escapeXmlData(text) {
    if (typeof text !== "string") {
        return "";
    }
    return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

/**
 * 创建 SOUL 管理器。
 * 负责文件读取、缓存维护与 prompt section 构造。
 * 读取失败只 warn，保留旧缓存，绝不抛出异常。
 */
export function createSoulManager({ soulFile, logger } = {}) {
    const filePath = soulFile ? resolve(soulFile) : "/root/riko/soul.md";
    let soulText = "";
    const soulVersion = "local";

    const loadSoul = (reason = "startup") => {
        try {
            const content = readFileSync(filePath, "utf8");
            soulText = content;
            logger?.info?.(`riko-memory-dsh: SOUL 读取成功 (${reason}, ${soulText.length} 字符)`);
            return soulText;
        } catch (error) {
            const msg = error instanceof Error ? error.message : String(error);
            logger?.warn?.(`riko-memory-dsh: SOUL 读取失败 (${reason})，保留旧缓存: ${filePath} (${msg})`);
            return soulText;
        }
    };

    const renderSoulSection = () => {
        if (!soulText || !soulText.trim()) {
            return null;
        }
        return {
            name: SOUL_SECTION_NAME,
            order: SOUL_SECTION_ORDER,
            text: `<agent_memory:soul version="${soulVersion}">\n${escapeXmlData(soulText)}\n</agent_memory:soul>`,
            interpolate: false,
        };
    };

    return {
        loadSoul,
        renderSoulSection,
        getSoulText: () => soulText,
        getSoulVersion: () => soulVersion,
        filePath,
    };
}
