You are consolidating review findings from multiple batches into ONE final JSON `ReviewOutputEvent` for the branch diff '{base}...HEAD'.

CI status summary from `gh pr checks --watch`:
{ci_context}

Stats: {stats}

Candidate clusters (summaries):
{clusters}

Rules:
- Deduplicate near-duplicates within clusters: keep the higher-confidence (tie: higher priority), and adjust titles/line ranges if needed to be accurate.
- You MAY re-verify uncertain items by fetching minimal hunks: `git diff --no-color -U0 {base}...HEAD -- -- <path>`.
- Drop invalid or out-of-scope items.
- Drop "junk" findings that only concern lockfiles (package manager lock files), generated/minified/vendored assets, or binaries, unless there is a direct and material correctness/security impact.
- If CI checks failed, weave the relevant failure details above into the final findings (e.g., reference specific failing jobs/tests with actionable fixes).
- Output ONLY the final JSON object (no prose) following the existing review schema.
- Sort findings by priority (P0..P3) then confidence (desc) then file/line.
