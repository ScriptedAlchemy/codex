#!/usr/bin/env node

import { applyEffortIntoConfig, type Effort } from "../src/lib/config.js";
import { startArgsSchema } from "../src/lib/schemas.js";

// Basic schema acceptance for allowed efforts
for (const eff of ["minimal", "low", "medium", "high"] as Effort[]) {
  const parsed = startArgsSchema.parse({ prompt: "x", effort: eff });
  if (parsed.effort !== eff) {
    console.error(`❌ Effort '${eff}' did not roundtrip in schema parse`);
    process.exit(1);
  }
}

// Mapping helper merges correctly
const base = { some_other_key: true } as const;
const merged = applyEffortIntoConfig(base as any, "high");
if (merged.model_reasoning_effort !== "high") {
  console.error("❌ Expected model_reasoning_effort to be 'high'");
  process.exit(1);
}
if ((merged as any).some_other_key !== true) {
  console.error("❌ applyEffortIntoConfig should preserve existing keys");
  process.exit(1);
}

console.log("✅ Effort schema and mapping look good");
process.exit(0);

