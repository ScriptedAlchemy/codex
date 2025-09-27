export const MODEL_CANDIDATES: readonly string[] = [
  // Codex variants (preferred for coding tasks)
  "gpt-5-codex-high",
  "gpt-5-codex",
  
  // GPT-5 variants (fallback options)
  "gpt-5-high",
  "gpt-5",
];

export function selectPreferredModel(
  preferred?: string | null,
): string | null {
  if (typeof preferred === "string") {
    const trimmed = preferred.trim();
    if (trimmed.length > 0) {
      return trimmed;
    }
  }
  return MODEL_CANDIDATES.find(Boolean) ?? null;
}

export type Effort = "minimal" | "low" | "medium" | "high";

/**
 * Returns a new config object with `model_reasoning_effort` set when `effort` is provided.
 * Does not mutate the input; preserves existing keys.
 */
export function applyEffortIntoConfig(
  base: Record<string, unknown> | undefined,
  effort?: Effort,
): Record<string, unknown> {
  const out: Record<string, unknown> = { ...(base ?? {}) };
  if (effort) {
    out.model_reasoning_effort = effort;
  }
  return out;
}
