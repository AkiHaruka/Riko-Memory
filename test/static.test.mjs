import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const rootDir = join(__dirname, "..");

function extractImports(sourceCode) {
    const importRegex = /(?:import\s+[\s\S]*?from\s+["']([^"']+)["']|import\s+["']([^"']+)["'])/g;
    const imports = [];
    let match;
    while ((match = importRegex.exec(sourceCode)) !== null) {
        imports.push(match[1] || match[2]);
    }
    return imports;
}

test("P12: 静态检查 - dist/index.js 与 define.js import 的模块符合白名单约束", () => {
    // 1. 读取参考实现 dist/index.js 中出现的外部包（相对路径，不硬编码 /work）
    const refIndexPath = join(rootDir, "..", "..", "research", "hymem-dsh-reference", "dist", "index.js");
    if (!existsSync(refIndexPath)) {
        assert.fail(`参考实现 index.js 不存在，实际查找路径: ${refIndexPath}`);
    }
    const refCode = readFileSync(refIndexPath, "utf8");
    const refImports = extractImports(refCode);
    const refDshPackages = new Set(
        refImports.filter((pkg) => pkg.startsWith("@deepseek-ai/"))
    );

    // 参考实现中 dist/index.js 引入的 DSH 包应该只有 @deepseek-ai/dsh-llm
    assert.deepEqual(Array.from(refDshPackages), ["@deepseek-ai/dsh-llm"]);

    // 2. 检查新实现的 dist/index.js
    const targetIndexPath = join(rootDir, "dist", "index.js");
    const targetCode = readFileSync(targetIndexPath, "utf8");
    const targetImports = extractImports(targetCode);

    for (const mod of targetImports) {
        if (mod.startsWith("node:") || mod.startsWith("./") || mod.startsWith("../")) {
            // 合法：node 内置模块或本地相对路径模块
            continue;
        }
        if (mod.startsWith("@deepseek-ai/")) {
            assert.ok(
                refDshPackages.has(mod),
                `dist/index.js 不得引入参考实现未出现过的 DSH 包: ${mod}`
            );
        } else {
            assert.fail(`dist/index.js 不得引入第三方 npm 依赖: ${mod}`);
        }
    }

    // 3. 检查 dist/config.js、dist/soul.js、dist/resident.js、dist/tools.js 必须零 DSH 包 import
    const nonDshFiles = ["config.js", "soul.js", "resident.js", "tools.js"];
    for (const file of nonDshFiles) {
        const filePath = join(rootDir, "dist", file);
        const code = readFileSync(filePath, "utf8");
        const fileImports = extractImports(code);
        for (const imp of fileImports) {
            assert.ok(
                !imp.startsWith("@deepseek-ai/"),
                `${file} 不得 import 任何 DSH 包，实际引入了: ${imp}`
            );
            assert.ok(
                imp.startsWith("node:") || imp.startsWith("./") || imp.startsWith("../"),
                `${file} 只允许引入 node 内置模块或本地相对模块，实际引入了: ${imp}`
            );
        }
    }

    // 4. 检查 dist/define.js：仅允许引入 @deepseek-ai/dsh-tools 与本地相对模块
    const defineFilePath = join(rootDir, "dist", "define.js");
    assert.ok(existsSync(defineFilePath), "dist/define.js 必须存在");
    const defineCode = readFileSync(defineFilePath, "utf8");
    const defineImports = extractImports(defineCode);
    for (const imp of defineImports) {
        if (imp.startsWith("node:") || imp.startsWith("./") || imp.startsWith("../")) {
            continue;
        }
        assert.equal(
            imp,
            "@deepseek-ai/dsh-tools",
            `dist/define.js 外部 DSH 包只允许 @deepseek-ai/dsh-tools，实际引入了: ${imp}`
        );
    }

    // 5. 检查 package.json 零 dependencies 网络包
    const pkgJson = JSON.parse(readFileSync(join(rootDir, "package.json"), "utf8"));
    assert.equal(pkgJson.dependencies, undefined, "package.json 不得声明 dependencies 网络包");
    assert.ok(pkgJson.peerDependencies, "package.json 必须保留 peerDependencies");
});

test("P13: 静态检查 - define.js 走官方 defineTool 且 index.js 从 define.js 接入", () => {
    // 1. define.js 存在
    const definePath = join(rootDir, "dist", "define.js");
    assert.ok(existsSync(definePath), `dist/define.js 必须存在: ${definePath}`);
    const defineCode = readFileSync(definePath, "utf8");

    // 2. import 了 @deepseek-ai/dsh-tools 的 defineTool
    const defineImports = extractImports(defineCode);
    assert.ok(
        defineImports.includes("@deepseek-ai/dsh-tools"),
        "define.js 必须 import @deepseek-ai/dsh-tools"
    );
    assert.match(
        defineCode,
        /import\s+{[^}]*defineTool[^}]*}\s+from\s+["']@deepseek-ai\/dsh-tools["']/,
        "define.js 必须明确 import defineTool"
    );

    // 3. 对 buildMemoryHandlers 的每个 handler 调用 defineTool
    assert.match(
        defineCode,
        /buildMemoryHandlers\([^)]*\)\.map\(\s*\(?\w+\)?\s*=>\s*defineTool\(\w+\)\)/,
        "define.js 必须对 buildMemoryHandlers 返回的每个 handler 映射调用 defineTool"
    );

    // 4. index.js 从 ./define.js 引入 buildMemoryTools / buildDisabledTools，不再直接 import tools.js
    const indexPath = join(rootDir, "dist", "index.js");
    const indexCode = readFileSync(indexPath, "utf8");
    assert.match(
        indexCode,
        /import\s+{[^}]*buildMemoryTools[^}]*}\s+from\s+["']\.\/define\.js["']/,
        "dist/index.js 必须从 ./define.js 引入 buildMemoryTools"
    );
    assert.match(
        indexCode,
        /import\s+{[^}]*buildDisabledTools[^}]*}\s+from\s+["']\.\/define\.js["']/,
        "dist/index.js 必须从 ./define.js 引入 buildDisabledTools"
    );
    assert.doesNotMatch(
        indexCode,
        /from\s+["']\.\/tools\.js["']/,
        "dist/index.js 不得直接 import tools.js"
    );
});
