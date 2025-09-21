You are consolidating review findings from multiple batches into ONE final JSON `ReviewOutputEvent` for the branch diff '{base}...HEAD'.

Stats: {stats}

Candidate clusters (summaries):
{clusters}

Rules:
- Deduplicate near-duplicates within clusters: keep the higher-confidence (tie: higher priority), and adjust titles/line ranges if needed to be accurate.
- You MAY re-verify uncertain items by fetching minimal hunks: `git diff --no-color -U0 {base}...HEAD -- -- <path>`.
- Drop invalid or out-of-scope items.
- Drop "junk" findings that only concern lockfiles (package manager lock files), generated/minified/vendored assets, or binaries, unless there is a direct and material correctness/security impact.
- Output ONLY the final JSON object (no prose) following the existing review schema.
- Sort findings by priority (P0..P3) then confidence (desc) then file/line.
- If checker/linter diagnostics were collected during batches, fold them in here as findings when they materially impact correctness/maintainability (cite the rule/lint); otherwise, summarize them under `overall_explanation` as “Checker/Linter notes”.
