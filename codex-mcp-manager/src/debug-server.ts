#!/usr/bin/env node

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  startArgsSchema,
  replyArgsSchema,
  endArgsSchema,
  type StartArgs,
} from "./lib/schemas.js";

const server = new McpServer({
  name: "debug-server",
  version: "0.1.0",
});

console.error("Schemas loaded:");
console.error(
  "startArgsSchema:",
  startArgsSchema ? "defined" : "undefined",
  startArgsSchema?._def ? "has _def" : "no _def",
);
console.error(
  "replyArgsSchema:",
  replyArgsSchema ? "defined" : "undefined",
  replyArgsSchema?._def ? "has _def" : "no _def",
);
console.error(
  "endArgsSchema:",
  endArgsSchema ? "defined" : "undefined",
  endArgsSchema?._def ? "has _def" : "no _def",
);

try {
  (server.registerTool as unknown as any)(
    "test-start",
    {
      title: "Test Start",
      description: "Test with start schema",
      inputSchema: startArgsSchema as never,
    },
    async (args: StartArgs) => {
      return {
        content: [{ type: "text", text: `Got prompt: ${args.prompt}` }],
      };
    },
  );
  console.error("Successfully registered test-start");
} catch (err) {
  console.error(
    "Error registering test-start:",
    err instanceof Error ? err : String(err),
  );
}

try {
  server.registerTool(
    "test-no-schema",
    {
      title: "Test No Schema",
      description: "Test without schema",
    },
    async () => {
      return {
        content: [{ type: "text", text: "No args" }],
      };
    },
  );
  console.error("Successfully registered test-no-schema");
} catch (err) {
  console.error(
    "Error registering test-no-schema:",
    err instanceof Error ? err : String(err),
  );
}

const registeredTools = (server as unknown as {
  _registeredTools?: Record<string, { inputSchema?: unknown }>;
})._registeredTools ?? {};

console.error("Registered tools:", Object.keys(registeredTools));
console.error("Tool details:");
for (const [name, tool] of Object.entries(registeredTools)) {
  const hasDef = Boolean(
    (tool.inputSchema as { _def?: unknown } | undefined)?._def,
  );
  console.error(`  ${name}:`, {
    hasInputSchema: !!tool.inputSchema,
    inputSchemaType: tool.inputSchema ? typeof tool.inputSchema : "N/A",
    hasDef,
  });
}

const transport = new StdioServerTransport();
await server.connect(transport);
console.error("Server connected");

process.stdin.resume();
