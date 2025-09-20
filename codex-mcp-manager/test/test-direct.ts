#!/usr/bin/env node

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  startArgsShape,
  replyArgsShape,
  endArgsShape,
} from "../src/lib/schemas.js";

const server = new McpServer({ name: "test-direct", version: "0.1.0" });

(server.registerTool as unknown as any)(
  "direct-start",
  {
    title: "Direct Start",
    description: "Smoke test registerTool with Zod shapes",
    inputSchema: startArgsShape as never,
  },
  async () => ({ content: [{ type: "text", text: "ok" }] }),
);

(server.registerTool as unknown as any)(
  "direct-reply",
  {
    title: "Direct Reply",
    description: "Ensure reply shape works",
    inputSchema: replyArgsShape as never,
  },
  async () => ({ content: [{ type: "text", text: "ok" }] }),
);

(server.registerTool as unknown as any)(
  "direct-end",
  {
    title: "Direct End",
    description: "Ensure end shape works",
    inputSchema: endArgsShape as never,
  },
  async () => ({ content: [{ type: "text", text: "ok" }] }),
);

const transport = new StdioServerTransport();
await server.connect(transport);
console.log("Direct test server started successfully");
console.log(JSON.stringify({ jsonrpc: "2.0", method: "tools/list", id: 1 }));
process.exit(0);
