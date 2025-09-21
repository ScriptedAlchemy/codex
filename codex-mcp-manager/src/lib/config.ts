export const MODEL_CANDIDATES: readonly string[] = [
  // Align with main branch: the Codex line uses swiftfox-* slugs.
  // Default to swiftfox-medium for balance, with explicit high/low as alternates.
  "swiftfox-medium",
  "swiftfox-high",
  "swiftfox-low",

  // Plain GPT‑5 as a fallback option.
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
