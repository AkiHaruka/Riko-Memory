#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLUGIN_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
TARGET_DIR="${PLUGIN_DIR}/node_modules/@deepseek-ai"

mkdir -p "${TARGET_DIR}"

PACKAGES=("dsh-llm" "dsh-tools")

GLOBAL_DSH_BASE="${GLOBAL_DSH_BASE:-/usr/local/lib/node_modules/@deepseek-ai/dsh/node_modules/@deepseek-ai}"
PROFILE_PNPM_BASE="${PROFILE_PNPM_BASE:-/root/.dsh/profiles/web/node_modules/.pnpm}"

for pkg in "${PACKAGES[@]}"; do
    SRC=""
    if [ -d "${GLOBAL_DSH_BASE}/${pkg}" ]; then
        SRC="${GLOBAL_DSH_BASE}/${pkg}"
    elif [ -d "${PROFILE_PNPM_BASE}" ]; then
        FOUND="$(find "${PROFILE_PNPM_BASE}" -maxdepth 4 -type d -path "*/node_modules/@deepseek-ai/${pkg}" | head -n 1 || true)"
        if [ -n "${FOUND}" ] && [ -d "${FOUND}" ]; then
            SRC="$(readlink -f "${FOUND}")"
        fi
    fi

    if [ -n "${SRC}" ] && [ -d "${SRC}" ]; then
        ln -sfn "${SRC}" "${TARGET_DIR}/${pkg}"
        echo "Linked @deepseek-ai/${pkg} -> ${SRC}"
    else
        echo "Warning: could not locate peer package @deepseek-ai/${pkg}" >&2
    fi
done

for pkg in "${PACKAGES[@]}"; do
    if [ ! -f "${TARGET_DIR}/${pkg}/package.json" ]; then
        echo "Error: peer package @deepseek-ai/${pkg} is missing (${TARGET_DIR}/${pkg}/package.json not found)" >&2
        exit 1
    fi
done

echo "Peer packages linked successfully."
echo "验证提示：可在插件目录执行以下命令验证导入："
echo "  node -e \"import('@deepseek-ai/dsh-llm').then(()=>import('@deepseek-ai/dsh-tools')).then(()=>console.log('peers ok'))\""
