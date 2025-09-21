import { z } from "zod";

export const startArgsShape = {
  prompt: z.string().min(1, "prompt is required"),
  approvalPolicy: z.enum(["untrusted", "on-failure", "never"]).optional(),
  model: z.string().optional(),
  // Reasoning effort level for GPT‑5 families. This is mapped to
  // Codex's `model_reasoning_effort` config override under the hood.
  effort: z.enum(["minimal", "low", "medium", "high"]).optional(),
  sandbox: z
    .enum(["read-only", "workspace-write", "danger-full-access"])
    .optional(),
  includePlanTool: z.boolean().optional(),
  includeApplyPatchTool: z.boolean().optional(),
  baseInstructions: z.string().optional(),
  config: z.record(z.unknown()).optional(),
  confirm_new: z.boolean().optional(),
} satisfies Record<string, z.ZodTypeAny>;

export const replyArgsShape = {
  id: z.number().int().positive(),
  prompt: z.string().min(1, "prompt is required"),
} satisfies Record<string, z.ZodTypeAny>;

export const endArgsShape = {
  id: z.number().int().positive(),
} satisfies Record<string, z.ZodTypeAny>;

export const startArgsSchema = z.object(startArgsShape);
export const replyArgsSchema = z.object(replyArgsShape);
export const endArgsSchema = z.object(endArgsShape);

export type StartArgs = z.infer<typeof startArgsSchema>;
export type ReplyArgs = z.infer<typeof replyArgsSchema>;
export type EndArgs = z.infer<typeof endArgsSchema>;
