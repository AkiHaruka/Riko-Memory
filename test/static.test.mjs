import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
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

test("P12: 静态检查 - dist/index.js import 的模块只有参考实现中出现的 DSH 包与 node 内置模块", () => {
    // 1. 读取参考实现 dist/index.js 中出现的外部包
    const refIndexPath = "/work/research/hymem-dsh-reference/dist/index.js";
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

    // 4. 检查 package.json 零 dependencies 网络包
    const pkgJson = JSON.parse(readFileSync(join(rootDir, "package.json"), "utf8"));
    assert.equal(pkgJson.dependencies, undefined, "package.json 不得声明 dependencies 网络包");
    assert.ok(pkgJson.peerDependencies, "package.json 必须保留 peerDependencies");
});
