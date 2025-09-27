#!/usr/bin/env node

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { zodToJsonSchema } from "zod-to-json-schema";

const server = new McpServer({
  name: "investigate",
  version: "0.1.0",
});

// Register a tool without schema
server.registerTool(
  "no-schema",
  {
    title: "No Schema",
    description: "Test",
  },
  async () => ({ content: [] }),
);

console.error("Registered tool:");
const registeredTools = (server as unknown as {
  _registeredTools: Record<string, { inputSchema?: unknown }>;
})._registeredTools;

const tool = registeredTools["no-schema"];
if (!tool) {
  throw new Error("Tool no-schema not registered");
}
console.error("inputSchema:", tool.inputSchema);
console.error("inputSchema === null:", tool.inputSchema === null);
console.error("inputSchema === undefined:", tool.inputSchema === undefined);
console.error("typeof inputSchema:", typeof tool.inputSchema);

// Now manually test what the SDK does
console.error("\nTesting SDK logic:");
const EMPTY_OBJECT_JSON_SCHEMA = {
  type: "object",
  properties: {},
  additionalProperties: false,
};

try {
  const result = tool.inputSchema
    ? zodToJsonSchema(tool.inputSchema as any, { strictUnions: true })
    : EMPTY_OBJECT_JSON_SCHEMA;
  console.error("Result would be:", result);
} catch (err) {
  console.error(
    "Error in SDK logic:",
    err instanceof Error ? err.message : String(err),
  );
}

// Connect and handle list request
const transport = new StdioServerTransport();
await server.connect(transport);

// Monkey-patch the server's list handler to debug
const originalHandler = (server.server as unknown as {
  _requestHandlers: Map<
    string,
    (request: unknown, extra: unknown) => Promise<unknown>
  >;
})._requestHandlers.get("tools/list");
if (originalHandler) {
  (server.server as unknown as {
    _requestHandlers: Map<
      string,
      (request: unknown, extra: unknown) => Promise<unknown>
    >;
  })._requestHandlers.set("tools/list", async (request, extra) => {
    console.error("\nIn tools/list handler");
    console.error("Registered tools:", Object.keys(registeredTools));

    try {
      for (const [toolName, toolEntry] of Object.entries(registeredTools)) {
        console.error(`Processing tool ${toolName}:`);
        console.error("  inputSchema:", toolEntry.inputSchema);
        console.error("  inputSchema type:", typeof toolEntry.inputSchema);
        console.error("  inputSchema === null:", toolEntry.inputSchema === null);
        console.error(
          "  inputSchema === undefined:",
          toolEntry.inputSchema === undefined,
        );

        if (toolEntry.inputSchema) {
          console.error("  Attempting to convert to JSON Schema...");
          try {
            const jsonSchema = zodToJsonSchema(toolEntry.inputSchema as any, {
              strictUnions: true,
            });
            console.error("  Success!");
          } catch (err) {
            console.error(
              "  Error:",
              err instanceof Error ? err.message : String(err),
            );
          }
        }
      }
    } catch (err) {
      console.error(
        "Outer error:",
        err instanceof Error ? err.message : String(err),
      );
    }

    return originalHandler(request, extra);
  });
}

process.stdin.resume();
