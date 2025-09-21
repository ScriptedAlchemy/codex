You are reviewing batch {batch_index}/{batch_total} for the branch diff '{base}...HEAD'.
Batch size: {size_hint}

Only review the files listed below for this batch. Do not review any other files.

Files:
{file_list}

Instructions:
- For any file you inspect, fetch minimal hunks only: `git diff --no-color -U0 {base}...HEAD -- -- <path>`.
- Scope findings ONLY to code overlapping the branch diff.
- Follow the existing review schema from the system prompt; output ONLY the JSON object.
- Be concise; do not paste full diffs; cite exact `file:line` ranges.
- If two candidate issues are effectively the same, prefer the higher-confidence one.

Static checks for this batch (run, don’t auto‑fix):
- You MUST attempt to discover and run configured project linters/type checkers for the files in this batch. Begin by briefly listing which checks you will run (or why none are applicable), then run them.
- Identify affected subprojects from the files in this batch and run the project’s checker/linter only for those parts.
  - Rust: `cargo clippy -p <crate> --tests --all-features` (no `--fix`) and/or `cargo check -p <crate>`.
  - JS/TS: `npm run -w <pkg> lint` and `npm run -w <pkg> typecheck`.
  - Python: `ruff`/`flake8` and `mypy` for the batch’s modules.
- Record any errors/warnings overlapping this batch; convert substantive ones into findings (cite rule/lint), otherwise summarize under `overall_explanation`.
- If a checker cannot run, state exactly what you tried and why it could not run, then proceed.

Skip low-value files unless there is a direct, non-speculative impact — and do not fetch diffs for them:
- Lockfiles (e.g., `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `Cargo.lock`).
- Generated/vendored or minified assets; images and other binaries.
- Doc-only changes that do not affect correctness/security.
If such files appear in the batch list, omit them and continue; only fetch a minimal hunk for them if you have a concrete, high-confidence reason that they affect correctness/security.

Context exploration (allowed, but keep it tight):
- If a finding depends on behavior in a related file not listed above (e.g., a called function or config include), you MAY fetch a minimal hunk for that related file to confirm the conclusion.
- Prefer targeted commands (e.g., `git diff -U0 {base}...HEAD -- -- related/path`, `git show {base}:related/path` for a few lines) over broad scans.
- Do not expand scope beyond what is necessary to verify the specific finding.
