Review all changes on the current branch relative to its base branch '{base}'.
Diff range: {base}...HEAD.
{size_hint_line}

Scope findings ONLY to code overlapping the branch diff. Follow the existing review schema and constraints. If the branch is large, prioritize highest‑risk areas first and summarize what remains in the overall_explanation.

Procedure:

- Enumerate changed files: `git diff --name-only --diff-filter=ACDMRTUXB --no-color {base}...HEAD`.
- For any file you inspect, fetch minimal hunks: `git diff --no-color -U0 {base}...HEAD -- -- <path>`.
- Prioritize in this order: security‑sensitive I/O/auth/crypto; public APIs and error handling; core logic; migrations/scripts/build logic.
- If you cannot cover everything in one pass, review in batches and present the highest‑impact findings first.

Static checks (run, don’t auto‑fix):

- Do NOT run build, serve, or long‑running dev commands. Only use lightweight linters/type‑checkers.
- Prefer running direct tool CLIs; avoid package scripts that may invoke builds.
- Determine affected subprojects from the changed paths and run the project’s checker/linter only for those parts.
  - Rust: `cargo clippy -p <crate> --tests --all-features` (no `--fix`) and/or `cargo check -p <crate>` if available.
  - JS/TS: `eslint` and `tsc --noEmit` if available; do not run `npm run build`, `vite dev`, `next dev`, etc.
  - Python: `ruff`/`flake8` and `mypy` scoped to changed modules.
- Record any errors or warnings that overlap the diff. Elevate substantive ones to findings (cite the rule/lint), otherwise summarize them under `overall_explanation` as “Checker/Linter notes”.
- If a checker is unavailable or cannot run, state exactly what you tried and why, then continue with manual review.

Scope filters (skip "junk" files unless there is a direct, non‑speculative impact — and avoid fetching diffs for them):

- Package manager lockfiles (e.g., `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `Cargo.lock`).
- Generated code, vendored bundles, and minified assets.
- Large binary assets (images, fonts, media) and formatting‑only changes.
- Pure docs/changelogs unless they create a correctness or security risk.
  If such files are present, omit them; only fetch a minimal hunk if you have a specific, high‑confidence reason they affect correctness/security.

Context exploration (allowed when needed):

- If assessing a change requires peeking at a related file (dependency, included config, or caller/callee), you MAY fetch a minimal hunk or a small slice from base vs HEAD to verify the conclusion.
- Keep it narrowly targeted (specific functions/lines) and avoid broad scans; only include enough context to confirm or fix the issue.

Constraints:

- Do not paste full diffs; cite exact `file:line` ranges.
- Keep comments brief and concrete; follow the JSON schema from the system prompt (no extra prose).
