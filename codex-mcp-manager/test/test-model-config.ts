#!/usr/bin/env node

// @ts-ignore - Node.js global process
declare const process: {
  exit(code?: number): never;
};

import { MODEL_CANDIDATES, selectPreferredModel } from "../src/lib/config.js";

// Test that model candidates are properly configured
console.log("Testing model configuration...");

// Verify expected models are present
const expectedModels = [
  "gpt-5-codex-high",
  "gpt-5-codex",
  "gpt-5-high",
  "gpt-5"
];

console.log("Available model candidates:", MODEL_CANDIDATES);

// Check that all expected models are present
for (const model of expectedModels) {
  if (!MODEL_CANDIDATES.includes(model)) {
    console.error(`❌ Expected model '${model}' not found in MODEL_CANDIDATES`);
    process.exit(1);
  } else {
    console.log(`✅ Model '${model}' found`);
  }
}

// Check that unwanted models are not present
const unwantedModels = [
  "codex-mini-latest",
  "gpt-5-medium",
  "gpt-5-low",
  "gpt-5-minimal",
  "o3",
  "o4-mini",
  "gpt-4.1",
  "gpt-4o",
];

for (const model of unwantedModels) {
  if (MODEL_CANDIDATES.includes(model)) {
    console.error(`❌ Unwanted model '${model}' found in MODEL_CANDIDATES`);
    process.exit(1);
  } else {
    console.log(`✅ Model '${model}' correctly excluded`);
  }
}

// Test selectPreferredModel function
const defaultModel = selectPreferredModel();
console.log(`Default model: ${defaultModel}`);

if (defaultModel !== "gpt-5-codex-high") {
  console.error(`❌ Expected default model 'gpt-5-codex-high', got '${defaultModel}'`);
  process.exit(1);
} else {
  console.log(`✅ Default model is correctly set to '${defaultModel}'`);
}

// Test with explicit model selection
const explicitModel = selectPreferredModel("gpt-5-codex-high");
if (explicitModel !== "gpt-5-codex-high") {
  console.error(`❌ Expected explicit model 'gpt-5-codex-high', got '${explicitModel}'`);
  process.exit(1);
} else {
  console.log(`✅ Explicit model selection works: '${explicitModel}'`);
}

// Test with null/undefined
const nullModel = selectPreferredModel(null);
if (nullModel !== "gpt-5-codex-high") {
  console.error(`❌ Expected fallback to 'gpt-5-codex-high' for null, got '${nullModel}'`);
  process.exit(1);
} else {
  console.log(`✅ Null model selection falls back correctly: '${nullModel}'`);
}

// Test that explicit model selection works for any valid model
const explicitNonCandidateModel = selectPreferredModel("gpt-4o");
if (explicitNonCandidateModel !== "gpt-4o") {
  console.error(
    `❌ Expected explicit model 'gpt-4o', got '${explicitNonCandidateModel}'`,
  );
  process.exit(1);
} else {
  console.log(`✅ Explicit non-candidate model selection works: '${explicitNonCandidateModel}'`);
}

console.log("\n🎉 All model configuration tests passed!");
process.exit(0);
