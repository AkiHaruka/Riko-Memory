import { defineTool } from "@deepseek-ai/dsh-tools";
import { buildMemoryHandlers, buildDisabledHandlers } from "./tools.js";

export function buildMemoryTools(cfg, logger) {
    return buildMemoryHandlers(cfg, logger).map((h) => defineTool(h));
}

export function buildDisabledTools(configErrorMsg) {
    return buildDisabledHandlers(configErrorMsg).map((h) => defineTool(h));
}
