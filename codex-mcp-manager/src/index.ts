#!/usr/bin/env node

import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import process from "node:process";

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  MODEL_CANDIDATES,
  selectPreferredModel,
  applyEffortIntoConfig,
  type Effort,
} from "./lib/config.js";
import {
  startArgsShape,
  replyArgsShape,
  endArgsShape,
  startArgsSchema,
  replyArgsSchema,
  endArgsSchema,
  type StartArgs,
  type ReplyArgs,
  type EndArgs,
} from "./lib/schemas.js";
import type {
  CallToolResult,
  Notification,
} from "@modelcontextprotocol/sdk/types.js";

const DEFAULT_CALL_TIMEOUT_MS = 10 * 60 * 1000; // 10 minutes per request
const DEFAULT_MAX_TIMEOUT_MS = 60 * 60 * 1000; // 60 minutes total guard

const STATE_DIR = path.join(os.homedir(), ".codex", "mcp-manager");
const STATE_FILE = path.join(STATE_DIR, "state.json");

const DEFAULT_COMMAND = process.env.CODEX_MCP_MANAGER_COMMAND || "npx";
const DEFAULT_ARGS = process.env.CODEX_MCP_MANAGER_ARGS
  ? splitArgs(process.env.CODEX_MCP_MANAGER_ARGS)
  : ["--yes", "codex", "mcp", "serve"];

const START_TOOL_NAME = "codex-manager-start";
const REPLY_TOOL_NAME = "codex-manager-reply";
const LIST_TOOL_NAME = "codex-manager-list";
const END_TOOL_NAME = "codex-manager-end";
const INFO_RESOURCE_URI = "codex-manager://overview";
const DEFAULT_MODEL = selectPreferredModel();

type ConversationEntry = {
  numericId: number;
  conversationId: string;
  label?: string;
  model?: string;
  rolloutPath?: string;
  createdAt?: string;
  lastUsedAt?: string;
};

type ConversationState = {
  nextId: number;
  conversations: ConversationEntry[];
};

type ManagerStructuredContent = {
  tool?: string;
  infoResource?: string;
  [key: string]: unknown;
};

interface ManagerCallToolResult
  extends Omit<CallToolResult, "structuredContent"> {
  structuredContent?: ManagerStructuredContent;
}

type PendingRequestEntry = {
  events: Notification[];
  outputChunks: string[];
  forwardNotification?: (notification: Notification) => void;
  promptSummary: string;
  sessionResolved: boolean;
  conversationEntry?: ConversationEntry;
  error: unknown;
};

type CodexNotification = Notification & {
  method?: string;
  params?: {
    _meta?: { requestId?: unknown };
    msg?: {
      type?: string;
      session_id?: string;
      model?: string;
      rollout_path?: string;
      message?: string;
      last_agent_message?: string;
    };
  };
};

type ToolHandlerExtra = {
  sendNotification?: (notification: Notification) => void;
};

type CodexStartArguments = {
  prompt: string;
  model?: string;
  sandbox?: StartArgs["sandbox"];
  includePlanTool?: boolean;
  includeApplyPatchTool?: boolean;
  baseInstructions?: string;
  config?: Record<string, unknown>;
  "approval-policy"?: StartArgs["approvalPolicy"];
};

type ListStructuredContent = ManagerStructuredContent & {
  tool: typeof LIST_TOOL_NAME;
  infoResource: typeof INFO_RESOURCE_URI;
  sessions: Array<{
    numericId: number;
    conversationId: string;
    label: string | null;
    model: string | null;
    rolloutPath: string | null;
    createdAt: string | null;
    lastUsedAt: string | null;
  }>;
};

type BuildResultOptions = {
  textSummary?: string;
  isError?: boolean;
  structuredContent?: ManagerStructuredContent;
};

const CONVERSATION_EXPIRATION_MS = 5 * 60 * 60 * 1000; // 5 hours

function conversationReferenceTimeMs(entry: ConversationEntry): number | null {
  const reference = entry.lastUsedAt ?? entry.createdAt;
  if (!reference) {
    return null;
  }
  const ms = new Date(reference).getTime();
  return Number.isNaN(ms) ? null : ms;
}

function isConversationExpired(
  entry: ConversationEntry,
  nowMs = Date.now(),
): boolean {
  const referenceMs = conversationReferenceTimeMs(entry);
  if (referenceMs === null) {
    return false;
  }
  return nowMs - referenceMs > CONVERSATION_EXPIRATION_MS;
}

function splitArgs(input: string): string[] {
  return (input.match(/(?:"[^"]*"|'[^']*'|[^\s]+)/g) || []).map((token) =>
    token.replace(/^"(.*)"$/, "$1").replace(/^'(.*)'$/, "$1"),
  );
}

async function ensureStateDir(): Promise<void> {
  await fs.mkdir(STATE_DIR, { recursive: true });
}

function cloneEnv(): Record<string, string> {
  return Object.fromEntries(
    Object.entries(process.env).filter(
      (entry): entry is [string, string] => typeof entry[1] === "string",
    ),
  );
}

// Queue of callbacks that capture the actual JSON‑RPC id for the next
// outgoing `tools/call` request to the "codex" tool. This avoids relying on
// private SDK internals for correlating notifications to requests.
const requestIdCaptureQueue: Array<(id: string) => void> = [];

function scheduleNextCodexRequestIdCapture(assign: (id: string) => void): void {
  requestIdCaptureQueue.push(assign);
}

async function loadState(): Promise<ConversationState> {
  try {
    const raw = await fs.readFile(STATE_FILE, "utf8");
    const parsed = JSON.parse(raw);
    return normalizeState(parsed);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") {
      return { nextId: 1, conversations: [] };
    }
    throw error;
  }
}

async function saveState(state: ConversationState): Promise<ConversationState> {
  await ensureStateDir();
  const normalized = normalizeState(state);
  const data = JSON.stringify(normalized, null, 2);
  const tmpPath = `${STATE_FILE}.tmp`;
  await fs.writeFile(tmpPath, data, "utf8");
  await fs.rename(tmpPath, STATE_FILE);
  return normalized;
}

function normalizeState(state: unknown): ConversationState {
  const base =
    typeof state === "object" && state !== null
      ? (state as Record<string, unknown>)
      : {};

  const rawConversations = Array.isArray(base.conversations)
    ? (base.conversations as unknown[])
    : [];

  const conversations = rawConversations
    .map((entry) => normalizeConversation(entry))
    .filter((entry): entry is ConversationEntry => entry !== null);

  conversations.sort((a, b) => a.numericId - b.numericId);

  const nextIdCandidate = Number(base.nextId);
  const nextId =
    Number.isFinite(nextIdCandidate) && nextIdCandidate > 0
      ? Math.floor(nextIdCandidate)
      : conversations.reduce(
          (max, entry) => Math.max(max, entry.numericId + 1),
          1,
        );

  return { nextId, conversations };
}

function normalizeConversation(entry: unknown): ConversationEntry | null {
  if (!entry) {
    return null;
  }
  const typed = entry as Record<string, unknown>;
  const numericId = Number(typed.numericId);
  const conversationId =
    typeof typed.conversationId === "string"
      ? typed.conversationId.trim()
      : "";
  if (!Number.isFinite(numericId) || numericId <= 0 || !conversationId) {
    return null;
  }
  return {
    numericId: Math.floor(numericId),
    conversationId,
    label: typeof typed.label === "string" ? typed.label : undefined,
    model: typeof typed.model === "string" ? typed.model : undefined,
    rolloutPath:
      typeof typed.rolloutPath === "string" ? typed.rolloutPath : undefined,
    createdAt: coerceIsoTimestamp(typed.createdAt, true),
    lastUsedAt: coerceIsoTimestamp(typed.lastUsedAt, false),
  };
}

function coerceIsoTimestamp(
  value: unknown,
  fallbackNow: boolean,
): string | undefined {
  if (typeof value === "string") {
    const date = new Date(value);
    if (!Number.isNaN(date.getTime())) {
      return date.toISOString();
    }
  }
  if (fallbackNow) {
    return new Date().toISOString();
  }
  return undefined;
}

function summarizePrompt(prompt: string): string {
  const trimmed = prompt.replace(/\s+/g, " ").trim();
  if (trimmed.length <= 80) {
    return trimmed;
  }
  return `${trimmed.slice(0, 77)}...`;
}

function formatTimestamp(iso: string | null | undefined): string {
  if (!iso) {
    return "";
  }
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return iso;
  }
  return date.toISOString();
}

function decodeRequestId(value: unknown): string | null {
  if (value === undefined || value === null) {
    return null;
  }
  if (typeof value === "number") {
    return value.toString();
  }
  if (typeof value === "string") {
    return value;
  }
  return null;
}

function extractTextContent(result: ManagerCallToolResult | null | undefined) {
  if (!result || !Array.isArray(result.content)) {
    return "";
  }
  const lines = result.content
    .filter((block) => block?.type === "text" && typeof block.text === "string")
    .map((block) => block.text.trim())
    .filter(Boolean);
  return lines.join("\n\n");
}

function mapCodexArgumentsFromStart(
  args: StartArgs,
): { mapped: CodexStartArguments; fallbackNotice?: string } {
  const mapped: CodexStartArguments = { prompt: args.prompt };

  const requestedModel =
    typeof args.model === "string" && args.model.trim().length > 0
      ? args.model.trim()
      : null;
  const model = selectPreferredModel(args.model);
  let fallbackNotice: string | undefined;

  if (model) {
    mapped.model = model;
  }
  if (requestedModel && model && requestedModel !== model) {
    fallbackNotice = `Requested model ${requestedModel} is not supported. Using ${model} instead.`;
  }
  if (args.approvalPolicy) {
    mapped["approval-policy"] = args.approvalPolicy;
  }
  if (args.sandbox) {
    mapped.sandbox = args.sandbox;
  }
  if (typeof args.includePlanTool === "boolean") {
    mapped.includePlanTool = args.includePlanTool;
  }
  if (typeof args.includeApplyPatchTool === "boolean") {
    mapped.includeApplyPatchTool = args.includeApplyPatchTool;
  }
  if (args.baseInstructions) {
    mapped.baseInstructions = args.baseInstructions;
  }
  // Merge user-provided config, then overlay effort if supplied.
  mapped.config = args.config ? { ...args.config } : undefined;
  if (args.effort) {
    mapped.config = applyEffortIntoConfig(
      mapped.config,
      args.effort as Effort,
    );
  }
  return { mapped, fallbackNotice };
}

function makeListStructuredContent(
  conversations: ConversationEntry[],
): ListStructuredContent {
  return {
    tool: LIST_TOOL_NAME,
    infoResource: INFO_RESOURCE_URI,
    sessions: conversations.map((entry) => ({
      numericId: entry.numericId,
      conversationId: entry.conversationId,
      label: entry.label ?? null,
      model: entry.model ?? null,
      rolloutPath: entry.rolloutPath ?? null,
      createdAt: entry.createdAt ?? null,
      lastUsedAt: entry.lastUsedAt ?? null,
    })),
  };
}

function buildConversationTableLines(
  conversations: ConversationEntry[],
): string[] {
  return [
    "Managed Codex sessions (reuse the numeric id with codex-manager-reply or codex-manager-end).",
    "",
    "ID | Model      | Last Used            | Conversation Context",
    "---+------------+----------------------+--------------------------------------------------------",
    ...conversations.map((entry) => {
      const paddedId = entry.numericId.toString().padStart(2, " ");
      const model = (entry.model || "unknown").padEnd(10, " ").slice(0, 10);
      const lastUsed = formatTimestamp(entry.lastUsedAt)
        .slice(0, 20)
        .padEnd(20, " ");
      const context = (entry.label || "No description available").slice(0, 56);
      return `${paddedId} | ${model} | ${lastUsed} | ${context}`;
    }),
    "",
    "💡 Tip: Use 'codex-manager-reply <ID> <prompt>' to continue a conversation",
    "🗑️  Tip: Use 'codex-manager-end <ID>' to clean up finished sessions",
  ];
}

function formatConversationTable(conversations: ConversationEntry[]): string {
  if (conversations.length === 0) {
    return "";
  }
  return buildConversationTableLines(conversations).join("\n");
}

function buildResult({
  textSummary,
  isError = false,
  structuredContent,
}: BuildResultOptions): ManagerCallToolResult {
  return {
    content: textSummary ? [{ type: "text", text: textSummary }] : [],
    isError: isError || undefined,
    structuredContent,
  };
}

async function connectToCodex(): Promise<{
  client: Client;
  transport: StdioClientTransport;
}> {
  const transport = new StdioClientTransport({
    command: DEFAULT_COMMAND,
    args: DEFAULT_ARGS,
    env: cloneEnv(),
    stderr: "inherit",
  });

  const client = new Client(
    {
      name: "codex-mcp-manager",
      version: "0.1.0",
    },
    {
      capabilities: {
        tools: {},
        prompts: {},
        resources: {},
      },
    },
  );

  await client.connect(transport);

  // Wrap transport.send to capture the actual JSON-RPC id assigned by the SDK
  // for the next outgoing tools/call to the "codex" tool. This provides a
  // stable, public way to correlate notifications that include
  // params._meta.requestId without touching private fields.
  const anyTransport = transport as unknown as {
    send: (message: unknown, ...rest: unknown[]) => Promise<void>;
  };
  const originalSend = anyTransport.send.bind(transport);
  anyTransport.send = (message: unknown, ...rest: unknown[]) => {
    try {
      const m = message as { jsonrpc?: string; id?: unknown; method?: string; params?: any };
      if (
        m &&
        m.jsonrpc === "2.0" &&
        m.method === "tools/call" &&
        (m.params?.name === "codex" || m.params?.name === "codex-reply") &&
        (typeof m.id === "number" || typeof m.id === "string") &&
        requestIdCaptureQueue.length > 0
      ) {
        const capture = requestIdCaptureQueue.shift();
        capture?.(String(m.id));
      }
    } catch {
      // Never let correlation logic interfere with transport
    }
    return originalSend(message, ...rest);
  };

  return { client, transport };
}

const state = await loadState();
const conversationsByNumeric = new Map<number, ConversationEntry>(
  state.conversations.map((entry) => [entry.numericId, entry]),
);
const conversationsByUuid = new Map<string, number>(
  state.conversations.map((entry) => [entry.conversationId, entry.numericId]),
);
let nextNumericId = state.nextId;

function getSortedConversationEntries(): ConversationEntry[] {
  return Array.from(conversationsByNumeric.values()).sort(
    (a, b) => a.numericId - b.numericId,
  );
}

function snapshotState(): ConversationState {
  return {
    nextId: nextNumericId,
    conversations: getSortedConversationEntries(),
  };
}

async function purgeExpiredConversations(
  nowMs = Date.now(),
): Promise<number> {
  let removed = 0;
  for (const [numericId, entry] of conversationsByNumeric) {
    if (isConversationExpired(entry, nowMs)) {
      conversationsByNumeric.delete(numericId);
      conversationsByUuid.delete(entry.conversationId);
      removed += 1;
    }
  }
  if (removed > 0) {
    await saveState(snapshotState());
  }
  return removed;
}

await purgeExpiredConversations();

async function registerConversation({
  conversationId,
  label,
  model,
  rolloutPath,
}: {
  conversationId: string;
  label?: string;
  model?: string;
  rolloutPath?: string;
}): Promise<ConversationEntry> {
  const existingNumeric = conversationsByUuid.get(conversationId);
  if (existingNumeric) {
    const existingEntry = conversationsByNumeric.get(existingNumeric);
    const updated: ConversationEntry = {
      ...(existingEntry ?? { numericId: existingNumeric, conversationId }),
      label: existingEntry?.label ?? label,
      model: model ?? existingEntry?.model,
      rolloutPath: rolloutPath ?? existingEntry?.rolloutPath,
      lastUsedAt: new Date().toISOString(),
      createdAt: existingEntry?.createdAt,
    };
    conversationsByNumeric.set(existingNumeric, updated);
    conversationsByUuid.set(conversationId, existingNumeric);
    await saveState(snapshotState());
    return updated;
  }

  const numericId = nextNumericId++;
  const now = new Date().toISOString();
  const entry: ConversationEntry = {
    numericId,
    conversationId,
    label,
    model,
    rolloutPath,
    createdAt: now,
    lastUsedAt: now,
  };
  conversationsByNumeric.set(numericId, entry);
  conversationsByUuid.set(conversationId, numericId);
  await saveState(snapshotState());
  return entry;
}

async function markConversationUsed(
  numericId: number,
  updates: Partial<ConversationEntry> = {},
): Promise<ConversationEntry | null> {
  const entry = conversationsByNumeric.get(numericId);
  if (!entry) {
    return null;
  }
  const updated: ConversationEntry = {
    ...entry,
    ...Object.fromEntries(
      Object.entries(updates).filter(([, value]) => value !== undefined),
    ),
    lastUsedAt: new Date().toISOString(),
  };
  conversationsByNumeric.set(numericId, updated);
  conversationsByUuid.set(updated.conversationId, numericId);
  await saveState(snapshotState());
  return updated;
}

async function endConversation(
  numericId: number,
): Promise<ConversationEntry | null> {
  const entry = conversationsByNumeric.get(numericId);
  if (!entry) {
    return null;
  }
  conversationsByNumeric.delete(numericId);
  conversationsByUuid.delete(entry.conversationId);
  await saveState(snapshotState());
  return entry;
}

const { client: codexClient, transport: codexTransport } =
  await connectToCodex();

const pendingRequests = new Map<string, PendingRequestEntry>();

codexClient.fallbackNotificationHandler = async (
  notification: Notification,
) => {
  const codexNotification = notification as CodexNotification;
  const requestId = decodeRequestId(
    codexNotification.params?._meta?.requestId,
  );
  if (!requestId) {
    return;
  }
  const pending = pendingRequests.get(requestId);
  if (!pending) {
    return;
  }

  pending.events.push(notification);

  const { forwardNotification } = pending;
  if (forwardNotification) {
    forwardNotification(notification);
  }

  if (codexNotification.method !== "codex/event") {
    return;
  }

  const message = codexNotification.params?.msg;
  if (!message || typeof message.type !== "string") {
    return;
  }

  switch (message.type) {
    case "session_configured": {
      if (!pending.sessionResolved) {
        pending.sessionResolved = true;
        const label = pending.promptSummary;
        try {
          const sessionId =
            typeof message.session_id === "string"
              ? message.session_id
              : null;
          if (!sessionId) {
            throw new Error("Codex session_configured missing session_id");
          }
          const model =
            typeof message.model === "string" ? message.model : undefined;
          const rolloutPath =
            typeof message.rollout_path === "string"
              ? message.rollout_path
              : undefined;
          const entry = await registerConversation({
            conversationId: sessionId,
            label,
            model,
            rolloutPath,
          });
          pending.conversationEntry = entry;
        } catch (error) {
          pending.error = error;
        }
      }
      break;
    }
    case "agent_message": {
      if (message.message) {
        pending.outputChunks.push(`assistant: ${message.message}`);
      }
      break;
    }
    case "user_message": {
      if (message.message) {
        pending.outputChunks.push(`user: ${message.message}`);
      }
      break;
    }
    case "task_complete": {
      if (message.last_agent_message) {
        pending.outputChunks.push(`complete: ${message.last_agent_message}`);
      }
      break;
    }
    case "error": {
      if (message.message) {
        pending.outputChunks.push(`error: ${message.message}`);
      }
      break;
    }
    default:
      break;
  }
};

const server = new McpServer({
  name: "codex-manager",
  version: "0.1.0",
});

const originalRegisterTool = server.registerTool.bind(server);
(server as unknown as { registerTool: (...args: any[]) => unknown }).registerTool =
  function (...args: any[]) {
  const [name, config] = args as [string, { inputSchema?: unknown }, unknown];
  if (process.env.DEBUG_MCP) {
    console.error(`Registering tool: ${String(name)}`);
    const typedConfig = config as {
      inputSchema?: unknown;
    };
    console.error(`  inputSchema type: ${typeof typedConfig?.inputSchema}`);
    console.error(`  inputSchema is null: ${typedConfig?.inputSchema === null}`);
    console.error(
      `  inputSchema is undefined: ${typedConfig?.inputSchema === undefined}`,
    );
    if (typedConfig?.inputSchema && typeof typedConfig === "object") {
      console.error(
        `  inputSchema has _def: ${
          (typedConfig.inputSchema as { _def?: unknown })?._def ? "true" : "false"
        }`,
      );
    }
  }
  return (originalRegisterTool as (...inner: any[]) => unknown)(...args);
};

server.registerResource(
  "overview",
  INFO_RESOURCE_URI,
  {
    title: "Codex MCP Manager Overview",
    description:
      "Explains how to orchestrate Codex sessions via codex-manager tools.",
  },
  async () => ({
    contents: [
      {
        uri: INFO_RESOURCE_URI,
        text: `codex-manager bridges Codex's MCP interface with agent-friendly session management.\n\nCapabilities:\n- codex-manager-start: launch a new Codex session with optional overrides, returning a numeric id and streaming events (defaults to the first available preset in ${MODEL_CANDIDATES.join(", ")} unless you override).\n- codex-manager-reply: continue a tracked session by id, forwarding Codex notifications.\n- codex-manager-list: enumerate tracked sessions with metadata (model, rollout path, timestamps).\n- codex-manager-end: indicate work is complete for an id and remove it from tracking.\n\nModel selection and reasoning effort:\n- model: pass a slug like \"swiftfox-medium\" (recommended for coding) or \"gpt-5\". If omitted, defaults to ${MODEL_CANDIDATES[0]}.\n- effort: optional \"minimal|low|medium|high\". This maps to Codex's \`model_reasoning_effort\` and is the recommended way to control depth of reasoning without changing the model slug.\n  • minimal/low: fastest responses; great for straightforward tasks and quick edits.\n  • medium: balanced default for most coding flows.\n  • high: deeper reasoning for complex changes; expect higher latency and token use.\n\nAgents should store only the numeric session id; codex-manager maintains UUID mapping, event history, and lifecycle coordination on your behalf.`,
      },
    ],
  }),
);

(server.registerTool as unknown as any)(
  START_TOOL_NAME,
  {
    title: "Start Codex Session (Managed ID)",
    description: `Launch a Codex session using numeric ids that codex-manager keeps in sync with Codex conversation UUIDs. Defaults to model ${DEFAULT_MODEL}. Optional: set effort=minimal|low|medium|high to control reasoning depth (maps to model_reasoning_effort) without changing the model slug. Recommended models: swiftfox-* for coding tasks (aligns with main branch).`,
    inputSchema: startArgsShape as never,
  },
  async (args: StartArgs, extra?: ToolHandlerExtra) => {
    const expiredRemoved = await purgeExpiredConversations();
    const existingConversations = getSortedConversationEntries();
    const confirmNew = Boolean(args.confirm_new);

    const sessionsStructuredContent = makeListStructuredContent(
      existingConversations,
    ).sessions;

    if (!confirmNew) {
      const infoLines: string[] = [];
      if (expiredRemoved > 0) {
        infoLines.push(
          `Removed ${expiredRemoved} expired session${
            expiredRemoved === 1 ? "" : "s"
          } older than 5 hours.`,
        );
      }
      if (existingConversations.length === 0) {
        infoLines.push(
          "No managed sessions are currently tracked. Rerun with confirm_new=true to create a fresh conversation.",
        );
      } else {
        infoLines.push(
          "Existing managed sessions are listed below. Use codex-manager-reply with the numeric id to continue one, or rerun codex-manager-start with confirm_new=true to create a new conversation.",
        );
      }
      const table = formatConversationTable(existingConversations);
      if (table) {
        infoLines.push("", table);
      }

      const structuredContent: ManagerStructuredContent = {
        tool: START_TOOL_NAME,
        infoResource: INFO_RESOURCE_URI,
        requiresConfirmation: true,
        confirmArgument: "confirm_new",
      };
      if (sessionsStructuredContent.length > 0) {
        structuredContent.sessions = sessionsStructuredContent;
      }
      if (expiredRemoved > 0) {
        structuredContent.expiredSessionsRemoved = expiredRemoved;
      }

      return buildResult({
        textSummary: infoLines.filter(Boolean).join("\n\n"),
        structuredContent,
      });
    }

    const { mapped: mappedArgs, fallbackNotice } =
      mapCodexArgumentsFromStart(args);
    const promptSummary = summarizePrompt(args.prompt);

    // Generate a local token for forwarding; the actual JSON-RPC id will be
    // captured from the transport when the request is sent.
    const localRequestToken = `${Date.now()}-${Math.random()
      .toString(36)
      .slice(2, 10)}`;
    let actualRequestId: string | null = null;
    const events: Notification[] = [];
    const outputChunks: string[] = [];

    if (fallbackNotice) {
      outputChunks.push(fallbackNotice);
    }

    if (expiredRemoved > 0) {
      outputChunks.push(
        `Removed ${expiredRemoved} expired session${
          expiredRemoved === 1 ? "" : "s"
        } before creating a new one (sessions auto-expire after 5 hours).`,
      );
    }

    if (existingConversations.length > 0) {
      outputChunks.push(
        "Existing managed sessions detected. Review them before launching a new one:",
      );
      const table = formatConversationTable(existingConversations);
      if (table) {
        outputChunks.push(table);
      }
    }

    let forwardNotification: PendingRequestEntry["forwardNotification"];
    const sendNotification = extra?.sendNotification;
    if (sendNotification) {
      forwardNotification = (notification) => {
        sendNotification({
          method: "codex-manager-event",
          params: {
            requestId: actualRequestId ?? localRequestToken,
            notification,
          },
        });
      };
    }

    const pendingEntry: PendingRequestEntry = {
      events,
      outputChunks,
      forwardNotification,
      promptSummary,
      sessionResolved: false,
      conversationEntry: undefined,
      error: null,
    };
    // Capture the JSON-RPC id assigned to the following call and register the
    // pending entry only once we know the real id.
    scheduleNextCodexRequestIdCapture((rid) => {
      actualRequestId = rid;
      pendingRequests.set(rid, pendingEntry);
    });

    let result: ManagerCallToolResult | undefined;
    try {
      result = (await codexClient.callTool(
        {
          name: "codex",
          arguments: mappedArgs,
        },
        undefined,
        {
          timeout: DEFAULT_CALL_TIMEOUT_MS,
          maxTotalTimeout: DEFAULT_MAX_TIMEOUT_MS,
          resetTimeoutOnProgress: true,
        },
      )) as ManagerCallToolResult;
    } catch (error) {
      if (actualRequestId) {
        pendingRequests.delete(actualRequestId);
      }
      throw error;
    }

    if (actualRequestId) {
      pendingRequests.delete(actualRequestId);
    }

    if (pendingEntry.error) {
      throw pendingEntry.error;
    }

    if (result?.isError) {
      const message = extractTextContent(result) || "Codex reported an error.";
      return buildResult({
        textSummary: message,
        isError: true,
        structuredContent: { events: pendingEntry.events },
      });
    }

    const conversationEntry = pendingEntry.conversationEntry;
    if (!conversationEntry) {
      const message =
        extractTextContent(result) ||
        "Codex did not provide a conversation id.";
      return buildResult({
        textSummary: message,
        isError: true,
      });
    }

    await markConversationUsed(conversationEntry.numericId, {
      label: conversationEntry.label ?? promptSummary,
      model: conversationEntry.model,
    });

    const activeConversations = getSortedConversationEntries();

    const text = [
      ...pendingEntry.outputChunks,
      extractTextContent(result),
      `session ${conversationEntry.numericId} (${conversationEntry.conversationId}) ready`,
      `See ${INFO_RESOURCE_URI} for a quick reference to all codex-manager tools.`,
    ]
      .filter(Boolean)
      .join("\n\n");

    const structuredContent: ManagerStructuredContent = {
      tool: START_TOOL_NAME,
      infoResource: INFO_RESOURCE_URI,
      numericId: conversationEntry.numericId,
      conversationId: conversationEntry.conversationId,
      model: conversationEntry.model,
      rolloutPath: conversationEntry.rolloutPath,
      events: pendingEntry.events,
    };

    if (existingConversations.length > 0) {
      structuredContent.sessionsBeforeStart = sessionsStructuredContent;
    }
    if (activeConversations.length > 0) {
      structuredContent.activeSessions =
        makeListStructuredContent(activeConversations).sessions;
    }
    if (expiredRemoved > 0) {
      structuredContent.expiredSessionsRemoved = expiredRemoved;
    }

    return buildResult({
      textSummary: text,
      structuredContent,
    });
  },
);

(server.registerTool as unknown as any)(
  REPLY_TOOL_NAME,
  {
    title: "Reply to Managed Session",
    description:
      "Continue a tracked Codex session by numeric id while streaming Codex notifications through this manager.",
    inputSchema: replyArgsShape as never,
  },
  async (args: ReplyArgs, extra?: ToolHandlerExtra) => {
    await purgeExpiredConversations();
    const entry = conversationsByNumeric.get(args.id);
    if (!entry) {
      return buildResult({
        textSummary: `No tracked session with id ${args.id}. It may have expired after 5 hours of inactivity.`,
        isError: true,
      });
    }

    const localRequestToken = `${Date.now()}-${Math.random()
      .toString(36)
      .slice(2, 10)}`;
    let actualRequestId: string | null = null;
    const events: Notification[] = [];
    const outputChunks: string[] = [];

    let forwardNotification: PendingRequestEntry["forwardNotification"];
    const sendNotification = extra?.sendNotification;
    if (sendNotification) {
      forwardNotification = (notification) => {
        sendNotification({
          method: "codex-manager-event",
          params: {
            requestId: actualRequestId ?? localRequestToken,
            notification,
          },
        });
      };
    }

    const pendingEntry: PendingRequestEntry = {
      events,
      outputChunks,
      forwardNotification,
      promptSummary: summarizePrompt(args.prompt),
      sessionResolved: true,
      conversationEntry: entry,
      error: null,
    };
    scheduleNextCodexRequestIdCapture((rid) => {
      actualRequestId = rid;
      pendingRequests.set(rid, pendingEntry);
    });

    let result: ManagerCallToolResult | undefined;
    try {
      result = (await codexClient.callTool(
        {
          name: "codex-reply",
          arguments: {
            conversationId: entry.conversationId,
            prompt: args.prompt,
          },
        },
        undefined,
        {
          timeout: DEFAULT_CALL_TIMEOUT_MS,
          maxTotalTimeout: DEFAULT_MAX_TIMEOUT_MS,
          resetTimeoutOnProgress: true,
        },
      )) as ManagerCallToolResult;
    } finally {
      if (actualRequestId) {
        pendingRequests.delete(actualRequestId);
      }
    }

    if (result?.isError) {
      return buildResult({
        textSummary: extractTextContent(result) || "Codex reported an error.",
        isError: true,
      });
    }

    await markConversationUsed(entry.numericId, {
      label: entry.label ?? summarizePrompt(args.prompt),
    });

    const text = [
      ...events
        .filter((notification) => notification.method === "codex/event")
        .map((notification) => {
          const msg = (notification as CodexNotification).params?.msg;
          if (!msg || typeof msg.type !== "string") {
            return null;
          }
          switch (msg.type) {
            case "agent_message":
              return msg.message ? `assistant: ${msg.message}` : null;
            case "task_complete":
              return msg.last_agent_message
                ? `complete: ${msg.last_agent_message}`
                : null;
            case "error":
              return msg.message ? `error: ${msg.message}` : null;
            default:
              return null;
          }
        })
        .filter(Boolean),
      extractTextContent(result),
      `Tip: ${INFO_RESOURCE_URI} lists every codex-manager tool and expected workflow.`,
    ]
      .filter(Boolean)
      .join("\n\n");

    return buildResult({
      textSummary: text,
      structuredContent: {
        tool: REPLY_TOOL_NAME,
        infoResource: INFO_RESOURCE_URI,
        numericId: entry.numericId,
        conversationId: entry.conversationId,
        events,
      },
    });
  },
);

(server.registerTool as unknown as any)(
  LIST_TOOL_NAME,
  {
    title: "List Managed Sessions",
    description:
      "Report the numeric ids, models, rollout paths, and timestamps for sessions tracked by codex-manager.",
  },
  async () => {
    const expiredRemoved = await purgeExpiredConversations();
    const conversations = getSortedConversationEntries();
    if (conversations.length === 0) {
      const textSummary =
        expiredRemoved > 0
          ? `No tracked conversations. Removed ${expiredRemoved} expired session${
              expiredRemoved === 1 ? "" : "s"
            } older than 5 hours.`
          : "No tracked conversations.";
      return buildResult({ textSummary });
    }

    const lines = buildConversationTableLines(conversations);
    const textSummary =
      expiredRemoved > 0
        ? [`Removed ${expiredRemoved} expired session${expiredRemoved === 1 ? "" : "s"} older than 5 hours.`, "", ...lines].join("\n")
        : lines.join("\n");

    return buildResult({
      textSummary,
      structuredContent: makeListStructuredContent(conversations),
    });
  },
);

(server.registerTool as unknown as any)(
  END_TOOL_NAME,
  {
    title: "End Managed Session",
    description:
      "Mark a tracked session as finished so the numeric id is retired and the UUID mapping is cleared.",
    inputSchema: endArgsShape as never,
  },
  async (args: EndArgs) => {
    await purgeExpiredConversations();
    const removed = await endConversation(args.id);
    if (!removed) {
      return buildResult({
        textSummary: `No active conversation found for id ${args.id}. It may have expired after 5 hours of inactivity.`,
        isError: true,
      });
    }
    return buildResult({
      textSummary: `Ended session ${args.id} (${removed.conversationId}).`,
      structuredContent: {
        tool: END_TOOL_NAME,
        infoResource: INFO_RESOURCE_URI,
        numericId: removed.numericId,
        conversationId: removed.conversationId,
      },
    });
  },
);

const serverTransport = new StdioServerTransport();
await server.connect(serverTransport);

function shutdown() {
  Promise.allSettled([
    server.close(),
    codexClient.close(),
    codexTransport.close(),
  ]).finally(() => process.exit(0));
}

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);

// Keep the process alive when running under stdio transport.
process.stdin.resume();
