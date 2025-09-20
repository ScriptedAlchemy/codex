#!/usr/bin/env node

import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(here, "..", "");
const distIndex = path.resolve(projectRoot, "dist", "index.js");

const server = spawn("node", [distIndex], {
  cwd: projectRoot,
  stdio: ["pipe", "pipe", "pipe"],
  env: { ...process.env },
});

const rl = createInterface({
  input: server.stdout,
  output: process.stdout,
  terminal: false,
});

rl.on("line", (line) => {
  console.log("Server output:", line);
});

server.stderr.on("data", (data) => {
  console.error("Server error:", data.toString());
});

const sendJsonRpc = (message: unknown) => {
  server.stdin.write(`${JSON.stringify(message)}\n`);
};

setTimeout(() => {
  console.log("Sending initialize request...");
  sendJsonRpc({
    jsonrpc: "2.0",
    method: "initialize",
    params: {
      protocolVersion: "2024-11-05",
      capabilities: {},
      clientInfo: { name: "test-client", version: "1.0.0" },
    },
    id: 1,
  });
}, 100);

setTimeout(() => {
  console.log("Sending list tools request...");
  sendJsonRpc({
    jsonrpc: "2.0",
    method: "tools/list",
    params: {},
    id: 2,
  });
}, 500);

const shutdown = () => {
  rl.close();
  server.kill();
  process.exit(0);
};

setTimeout(shutdown, 2000);

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
