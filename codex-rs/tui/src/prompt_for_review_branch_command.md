Review all changes on the current branch relative to its base branch '{base}'.
Diff range: {base}...HEAD.
{size_hint_line}

Scope findings ONLY to code overlapping the branch diff. Follow the existing review schema and constraints. If the branch is large, prioritize highest‑risk areas first and summarize what remains in the overall_explanation.

Procedure:
- Enumerate changed files: `git diff --name-only --diff-filter=ACDMRTUXB --no-color {base}...HEAD`.
- For any file you inspect, fetch minimal hunks: `git diff --no-color -U0 {base}...HEAD -- -- <path>`.
- Prioritize in this order: security‑sensitive I/O/auth/crypto; public APIs and error handling; core logic; migrations/scripts/build logic.
- If you cannot cover everything in one pass, review in batches and present the highest‑impact findings first.

Scope filters (skip "junk" files unless there is a direct, non‑speculative impact):
- Package manager lockfiles (e.g., `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `Cargo.lock`).
- Generated code, vendored bundles, and minified assets.
- Large binary assets (images, fonts, media) and formatting‑only changes.
- Pure docs/changelogs unless they create a correctness or security risk.

Constraints:
- Do not paste full diffs; cite exact `file:line` ranges.
- Keep comments brief and concrete; follow the JSON schema from the system prompt (no extra prose).
