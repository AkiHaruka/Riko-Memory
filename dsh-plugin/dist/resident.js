import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { escapeXmlData } from "./soul.js";

/**
 * 按 UTF-8 字符安全截断文本。
 * 使用 Unicode 码点安全拆分，绝不截断代理对（如 Emoji 或罕见汉字），保证字符完整性。
 * 若超出预算，截断并在末尾附加「…（超预算已截断）」。
 */
export function truncateResidentText(text, budgetChars) {
    if (typeof text !== "string") {
        return "";
    }
    if (!budgetChars || budgetChars <= 0) {
        return text;
    }
    const chars = Array.from(text);
    if (chars.length <= budgetChars) {
        return text;
    }
    const truncated = chars.slice(0, budgetChars).join("");
    return `${truncated}…（超预算已截断）`;
}

/**
 * 按 UTF-8 字节安全截断 Buffer 或字符串。
 * 确保多字节 UTF-8 序列（如汉字 3 字节）绝不在中间被截断。
 */
export function truncateResidentUtf8Bytes(str, maxBytes) {
    if (typeof str !== "string") {
        return "";
    }
    const buf = Buffer.from(str, "utf8");
    if (buf.length <= maxBytes) {
        return str;
    }
    // 寻找截断边界，避免拆散 UTF-8 多字节字符
    let end = maxBytes;
    while (end > 0 && (buf[end] & 0xc0) === 0x80) {
        end--;
    }
    // 如果 end 指向的是一个多字节前导字符，且其后续字节不足，则舍弃该前导字符
    if (end > 0) {
        const lead = buf[end - 1];
        let charLen = 1;
        if ((lead & 0xe0) === 0xc0) charLen = 2;
        else if ((lead & 0xf0) === 0xe0) charLen = 3;
        else if ((lead & 0xf8) === 0xf0) charLen = 4;

        if (charLen > 1 && (end - 1 + charLen) > maxBytes) {
            end = end - 1;
        }
    }
    const safeStr = buf.subarray(0, end).toString("utf8");
    return `${safeStr}…（超预算已截断）`;
}

/**
 * 读取常驻记忆文件。
 * 文件不存在或读取失败时，打 warn 日志并跳过（返回 null）。
 */
export function readResidentContent({ residentFile, budgetChars = 6000, logger } = {}) {
    const filePath = residentFile ? resolve(residentFile) : "/root/riko/memory/resident.md";
    let content = "";
    try {
        content = readFileSync(filePath, "utf8");
    } catch (error) {
        const msg = error instanceof Error ? error.message : String(error);
        logger?.warn?.(`riko-memory-dsh: 常驻记忆文件读取失败，跳过注入: ${filePath} (${msg})`);
        return null;
    }

    if (!content || !content.trim()) {
        return null;
    }

    return truncateResidentText(content.trim(), budgetChars);
}

/**
 * 将常驻记忆格式化为注入用的 XML 块。
 */
export function formatResidentXml(text) {
    if (!text || !text.trim()) {
        return null;
    }
    return `<agent_memory:resident>\n${escapeXmlData(text)}\n</agent_memory:resident>`;
}
