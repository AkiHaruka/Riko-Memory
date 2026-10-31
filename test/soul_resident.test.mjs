import test from "node:test";
import assert from "node:assert/strict";
import { writeFileSync, unlinkSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { createSoulManager, escapeXmlData } from "../dist/soul.js";
import {
    truncateResidentText,
    truncateResidentUtf8Bytes,
    readResidentContent,
    formatResidentXml,
} from "../dist/resident.js";

test("P9: resident 截断 - utf8/字符边界安全，超预算加提示", () => {
    // 1. 预算充足，原样返回
    const normal = "璃的当前状态板：一切正常。";
    assert.equal(truncateResidentText(normal, 100), normal);

    // 2. 超出预算，截断并加提示
    const longText = "一二三四五六七八九十";
    const truncated = truncateResidentText(longText, 5);
    assert.equal(truncated, "一二三四五…（超预算已截断）");

    // 3. 代理对/Unicode 边界安全：不截断半个 Emoji 或半个罕见汉字
    // 𠮷 (U+20BB7) 在 UTF-16 中占 2 个 code units (\uD842\uDFB7)
    const surrogateText = "你𠮷好世界";
    // 字符数为 5。如果预算为 2 个字符：应为 "你𠮷"
    const cutSurrogate = truncateResidentText(surrogateText, 2);
    assert.equal(cutSurrogate, "你𠮷…（超预算已截断）");
    // 验证没有孤立的高代理或低代理（孤立代理字符）
    const hasLoneSurrogate = /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/.test(cutSurrogate);
    assert.equal(hasLoneSurrogate, false);
    assert.equal(Array.from(cutSurrogate.replace("…（超预算已截断）", "")).length, 2);

    // 4. UTF-8 字节截断：切在汉字 3 字节中间时不产生无效半字节
    // "你好世界" 每个汉字 3 字节，共 12 字节
    // 如果预算为 7 字节，只能容纳 2 个汉字（6 字节），不能截出第 3 个汉字的第 1 个字节
    const byteCut = truncateResidentUtf8Bytes("你好世界", 7);
    assert.equal(byteCut, "你好…（超预算已截断）");
    assert.equal(byteCut.includes("\uFFFD"), false);
});

test("P9 辅助: readResidentContent 文件不存在 warn + 跳过，存在时正常截断与格式化", () => {
    const warns = [];
    const mockLogger = {
        warn(msg) {
            warns.push(msg);
        },
    };

    // 1. 文件不存在：warn 并返回 null
    const resNone = readResidentContent({
        residentFile: "/tmp/non-existent-resident-" + Date.now() + ".md",
        logger: mockLogger,
    });
    assert.equal(resNone, null);
    assert.equal(warns.length, 1);
    assert.match(warns[0], /常驻记忆文件读取失败，跳过注入/);

    // 2. 文件存在：正常截断与 XML 包装
    const tempFile = join(tmpdir(), `test-resident-${Date.now()}.md`);
    writeFileSync(tempFile, "状态板：没做完的事：P2 开发。\n", "utf8");

    try {
        const content = readResidentContent({
            residentFile: tempFile,
            budgetChars: 10,
            logger: mockLogger,
        });
        assert.ok(content);
        assert.match(content, /…（超预算已截断）$/);

        const xml = formatResidentXml(content);
        assert.match(xml, /^<agent_memory:resident>\n/);
        assert.match(xml, /\n<\/agent_memory:resident>$/);
    } finally {
        unlinkSync(tempFile);
    }
});

test("P10: soul 读取失败只 warn 不抛（注入函数返回 null），旧缓存保留", () => {
    const warns = [];
    const mockLogger = {
        warn(msg) {
            warns.push(msg);
        },
        info() {},
    };

    const tempSoulFile = join(tmpdir(), `test-soul-${Date.now()}.md`);

    // 1. 初始化时文件不存在：只 warn，不抛异常，renderSoulSection 返回 null
    const manager = createSoulManager({
        soulFile: tempSoulFile,
        logger: mockLogger,
    });

    assert.doesNotThrow(() => {
        manager.loadSoul("startup");
    });
    assert.equal(manager.renderSoulSection(), null);
    assert.equal(warns.length, 1);
    assert.match(warns[0], /SOUL 读取失败 \(startup\)，保留旧缓存/);

    // 2. 写入文件，读取成功
    writeFileSync(tempSoulFile, "我是璃，我认爹地。\n<test_xml_chars>&", "utf8");
    try {
        manager.loadSoul("after_write");
        const section = manager.renderSoulSection();
        assert.ok(section);
        assert.equal(section.name, "riko-soul");
        assert.equal(section.order, 100);
        assert.equal(section.interpolate, false);
        assert.match(section.text, /<agent_memory:soul version="local">/);
        // XML 转义验证
        assert.match(section.text, /&lt;test_xml_chars&gt;&amp;/);

        // 3. 删除文件模拟读取失败，再次重载：保留旧文本，不抛出异常
        unlinkSync(tempSoulFile);
        assert.doesNotThrow(() => {
            manager.loadSoul("compaction/end");
        });
        const cachedSection = manager.renderSoulSection();
        assert.ok(cachedSection);
        assert.match(cachedSection.text, /我是璃/);
        assert.equal(warns.length, 2);
    } finally {
        try {
            unlinkSync(tempSoulFile);
        } catch {}
    }
});
