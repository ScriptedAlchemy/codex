# Codex CLI (Rust Implementation)

We provide Codex CLI as a standalone, native executable to ensure a zero-dependency install.

## Installing Codex

Today, the easiest way to install Codex is via `npm`, though we plan to publish Codex to other package managers soon.

```shell
npm i -g @openai/codex@native
codex
```

You can also download a platform-specific release directly from our [GitHub Releases](https://github.com/openai/codex/releases).

## What's new in the Rust CLI

While we are [working to close the gap between the TypeScript and Rust implementations of Codex CLI](https://github.com/openai/codex/issues/1262), note that the Rust CLI has a number of features that the TypeScript CLI does not!

### Config

Codex supports a rich set of configuration options. Note that the Rust CLI uses `config.toml` instead of `config.json`. See [`docs/config.md`](../docs/config.md) for details.

### Model Context Protocol Support

Codex CLI functions as an MCP client that can connect to MCP servers on startup. See the [`mcp_servers`](../docs/config.md#mcp_servers) section in the configuration documentation for details.

It is still experimental, but you can also launch Codex as an MCP _server_ by running `codex mcp`. Use the [`@modelcontextprotocol/inspector`](https://github.com/modelcontextprotocol/inspector) to try it out:

```shell
npx @modelcontextprotocol/inspector codex mcp
```

### Notifications

You can enable notifications by configuring a script that is run whenever the agent finishes a turn. The [notify documentation](../docs/config.md#notify) includes a detailed example that explains how to get desktop notifications via [terminal-notifier](https://github.com/julienXX/terminal-notifier) on macOS.

### `codex exec` to run Codex programmatically/non-interactively

To run Codex non-interactively, run `codex exec PROMPT` (you can also pass the prompt via `stdin`) and Codex will work on your task until it decides that it is done and exits. Output is printed to the terminal directly. You can set the `RUST_LOG` environment variable to see more about what's going on.

### Use `@` for file search

Typing `@` triggers a fuzzy-filename search over the workspace root. Use up/down to select among the results and Tab or Enter to replace the `@` with the selected path. You can use Esc to cancel the search.

### Esc–Esc to edit a previous message

When the chat composer is empty, press Esc to prime “backtrack” mode. Press Esc again to open a transcript preview highlighting the last user message; press Esc repeatedly to step to older user messages. Press Enter to confirm and Codex will fork the conversation from that point, trim the visible transcript accordingly, and pre‑fill the composer with the selected user message so you can edit and resubmit it.

In the transcript preview, the footer shows an `Esc edit prev` hint while editing is active.

### `--cd`/`-C` flag

Sometimes it is not convenient to `cd` to the directory you want Codex to use as the "working root" before running Codex. Fortunately, `codex` supports a `--cd` option so you can specify whatever folder you want. You can confirm that Codex is honoring `--cd` by double-checking the **workdir** it reports in the TUI at the start of a new session.

### Shell completions

Generate shell completion scripts via:

```shell
codex completion bash
codex completion zsh
codex completion fish
```

### Experimenting with the Codex Sandbox

To test to see what happens when a command is run under the sandbox provided by Codex, we provide the following subcommands in Codex CLI:

```
# macOS
codex debug seatbelt [--full-auto] [COMMAND]...

# Linux
codex debug landlock [--full-auto] [COMMAND]...
```

### Selecting a sandbox policy via `--sandbox`

The Rust CLI exposes a dedicated `--sandbox` (`-s`) flag that lets you pick the sandbox policy **without** having to reach for the generic `-c/--config` option:

```shell
# Run Codex with the default, read-only sandbox
codex --sandbox read-only

# Allow the agent to write within the current workspace while still blocking network access
codex --sandbox workspace-write

# Danger! Disable sandboxing entirely (only do this if you are already running in a container or other isolated env)
codex --sandbox danger-full-access
```

The same setting can be persisted in `~/.codex/config.toml` via the top-level `sandbox_mode = "MODE"` key, e.g. `sandbox_mode = "workspace-write"`.

## Code Organization

This folder is the root of a Cargo workspace. It contains quite a bit of experimental code, but here are the key crates:

- [`core/`](./core) contains the business logic for Codex. Ultimately, we hope this to be a library crate that is generally useful for building other Rust/native applications that use Codex.
- [`exec/`](./exec) "headless" CLI for use in automation.
- [`tui/`](./tui) CLI that launches a fullscreen TUI built with [Ratatui](https://ratatui.rs/).
- [`cli/`](./cli) CLI multitool that provides the aforementioned CLIs via subcommands.

### OpenAI‑Compatible Passthrough (codex‑proxy)

Run a local HTTP server that forwards any `/v1/*` request to your configured provider. Useful for pointing OpenAI SDKs at `localhost` or for tooling that expects an OpenAI‑compatible API.

Start the proxy

```bash
cargo run -p codex-proxy -- --bind 127.0.0.1:11435
# Optional: enable permissive CORS for browser apps
# cargo run -p codex-proxy -- --bind 127.0.0.1:11435 --allow-cors-any
```

Provider selection

- By default, the proxy uses the active `model_provider` from `~/.codex/config.toml`.
- You can override via `-c` flags (same as Codex CLI). Example: forward to a mock server (or Ollama/OpenAI‑compatible gateway) at `http://localhost:11434/v1`:

```bash
cargo run -p codex-proxy -- \
  -c 'model_providers.mock={ name = "mock", base_url = "http://localhost:11434/v1", wire_api = "chat" }' \
  -c 'model_provider="mock"'
```

Auth behavior

- If the incoming request includes `Authorization: Bearer …`, the header is forwarded unchanged.
- Otherwise the proxy injects auth using (in order):
  - Provider `env_key` (e.g., `OPENAI_API_KEY`) from your environment, if configured in the provider.
  - Your ChatGPT token from `~/.codex/auth.json` when available.

Health

- `GET /health` returns `{ "status": "ok" }`.

Examples

Chat Completions via curl

```bash
curl -sS http://127.0.0.1:11435/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer $OPENAI_API_KEY' \
  -d '{
        "model": "gpt-4o-mini",
        "messages": [ {"role":"user","content":"Hello"} ]
      }'
```

Streaming (SSE) via curl

```bash
curl -N http://127.0.0.1:11435/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer $OPENAI_API_KEY' \
  -d '{
        "model": "gpt-4o-mini",
        "stream": true,
        "messages": [ {"role":"user","content":"Hello"} ]
      }'
```

File/image upload (multipart)

```bash
curl -sS http://127.0.0.1:11435/v1/files \
  -H 'Authorization: Bearer $OPENAI_API_KEY' \
  -F purpose=assistants \
  -F file=@image.jpg
```

SDK setup (Node & Python)

```ts
// Node
import OpenAI from "openai";
const client = new OpenAI({ apiKey: process.env.OPENAI_API_KEY, baseURL: "http://127.0.0.1:11435/v1" });
const chat = await client.chat.completions.create({
  model: "gpt-4o-mini",
  messages: [{ role: "user", content: "Hello" }],
});
```

```py
# Python
from openai import OpenAI
client = OpenAI(api_key="YOUR_KEY", base_url="http://127.0.0.1:11435/v1")
resp = client.chat.completions.create(
    model="gpt-4o-mini",
    messages=[{"role": "user", "content": "Hello"}],
)
```

Notes

- The proxy streams request and response bodies end‑to‑end, so large multipart uploads and SSE streams are supported.
- Hop‑by‑hop headers (Connection, Transfer-Encoding, etc.) are stripped; all other headers are forwarded.
- For browser apps, pass `--allow-cors-any` during local development.
