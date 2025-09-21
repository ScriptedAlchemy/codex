use std::io;
use std::process::Stdio;

use tokio::process::Command;

#[derive(Clone, Debug)]
pub(crate) struct NumstatRow {
    pub path: String,
    pub added: usize,
    pub deleted: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct Batch {
    pub files: Vec<NumstatRow>,
    pub total_added: usize,
    pub total_deleted: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct ChunkLimits {
    pub small_files_cap: usize,
    pub large_files_cap: usize,
    pub large_file_threshold_lines: usize,
    pub max_lines: usize,
}

pub(crate) async fn collect_branch_numstat(base: &str) -> io::Result<Vec<NumstatRow>> {
    // git diff --numstat base...HEAD
    let output = Command::new("git")
        .args(["diff", "--numstat", &format!("{base}...HEAD")])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await?;
    if !(output.status.success() || output.status.code() == Some(1)) {
        return Ok(Vec::new());
    }
    let mut rows = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        // Format: <added> <deleted> <path>
        let mut parts = line.split_whitespace();
        let a = parts.next();
        let d = parts.next();
        let path = parts.collect::<Vec<_>>().join(" ");
        if let (Some(a), Some(d)) = (a, d) {
            // Binary files show '-' in numstat; treat as 0
            let added = a.parse::<usize>().unwrap_or(0);
            let deleted = d.parse::<usize>().unwrap_or(0);
            if !path.is_empty() {
                rows.push(NumstatRow {
                    path,
                    added,
                    deleted,
                });
            }
        }
    }
    // Filter out low-value/junk paths to avoid reviewing lockfiles, docs-only, binaries, etc.
    rows.retain(|r| !is_junk_path(&r.path));
    // Keep zero-line rows (submodule pointers, binaries) — they still matter for review context.
    Ok(rows)
}

pub(crate) fn score_and_chunk(mut rows: Vec<NumstatRow>, limits: ChunkLimits) -> Vec<Batch> {
    // Simple scoring: lines changed descending, then path
    rows.sort_by(|a, b| {
        (b.added + b.deleted)
            .cmp(&(a.added + a.deleted))
            .then(a.path.cmp(&b.path))
    });

    let mut out = Vec::new();
    let mut cur = Batch {
        files: Vec::new(),
        total_added: 0,
        total_deleted: 0,
    };
    let mut cur_contains_large = false;

    for row in rows.into_iter() {
        let projected_files = cur.files.len() + 1;
        let projected_lines = cur.total_added + cur.total_deleted + row.added + row.deleted;
        let row_lines = row.added + row.deleted;
        let effective_cap = if cur_contains_large || row_lines > limits.large_file_threshold_lines {
            limits.large_files_cap
        } else {
            limits.small_files_cap
        };
        if !cur.files.is_empty()
            && (projected_files > effective_cap || projected_lines > limits.max_lines)
        {
            out.push(cur);
            cur = Batch {
                files: Vec::new(),
                total_added: 0,
                total_deleted: 0,
            };
            cur_contains_large = false;
        }
        cur.total_added += row.added;
        cur.total_deleted += row.deleted;
        cur.files.push(row);
        if row_lines > limits.large_file_threshold_lines {
            cur_contains_large = true;
        }
    }
    if !cur.files.is_empty() {
        out.push(cur);
    }
    out
}

fn is_junk_path(path: &str) -> bool {
    let p = path.replace('\\', "/").to_lowercase();

    // Exact lockfiles
    const LOCKFILES: &[&str] = &[
        "package-lock.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "cargo.lock",
        "gemfile.lock",
        "pipfile.lock",
        "poetry.lock",
        "composer.lock",
        "podfile.lock",
    ];
    if LOCKFILES.iter().any(|&f| p.ends_with(f)) {
        return true;
    }

    // Common vendor/generated/binary directories
    const JUNK_DIRS: &[&str] = &[
        "node_modules/",
        "vendor/",
        "dist/",
        "build/",
        "target/",
        ".next/",
        ".cache/",
        "out/",
        "coverage/",
    ];
    if JUNK_DIRS
        .iter()
        .any(|d| p.starts_with(d) || p.contains(&format!("/{d}")))
    {
        return true;
    }

    // Minified bundles and source maps
    if p.ends_with(".min.js") || p.ends_with(".map") {
        return true;
    }

    // Common binary/media formats
    const BIN_EXTS: &[&str] = &[
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".bmp", ".svg", ".pdf", ".mp4", ".mov",
        ".zip", ".tar", ".gz", ".tgz", ".7z", ".woff", ".woff2", ".ttf",
    ];
    if BIN_EXTS.iter().any(|ext| p.ends_with(ext)) {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junk_filters_lockfiles_and_vendor_but_not_docs() {
        assert!(is_junk_path("package-lock.json"));
        assert!(is_junk_path("pnpm-lock.yaml"));
        assert!(is_junk_path("vendor/lib.rs"));
        assert!(is_junk_path("node_modules/react/index.js"));
        assert!(is_junk_path("dist/app.min.js"));
        assert!(is_junk_path("assets/logo.svg"));

        // Docs should not be hard-filtered
        assert!(!is_junk_path("README.md"));
        assert!(!is_junk_path("docs/guide.mdx"));
    }

    #[test]
    fn chunk_small_files_into_25_then_remainder() {
        let mut rows = Vec::new();
        for i in 0..30 {
            rows.push(NumstatRow {
                path: format!("src/file_{i}.rs"),
                added: 5,
                deleted: 5,
            });
        }
        let limits = ChunkLimits {
            small_files_cap: 25,
            large_files_cap: 5,
            large_file_threshold_lines: 400,
            max_lines: 5000,
        };
        let batches = score_and_chunk(rows, limits);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].files.len(), 25);
        assert_eq!(batches[1].files.len(), 5);
    }

    #[test]
    fn chunk_with_large_file_caps_batch_at_5() {
        let mut rows = Vec::new();
        // One large file (1000 lines changed)
        rows.push(NumstatRow {
            path: "src/huge.rs".to_string(),
            added: 800,
            deleted: 200,
        });
        // Ten small files
        for i in 0..10 {
            rows.push(NumstatRow {
                path: format!("src/small_{i}.rs"),
                added: 10,
                deleted: 5,
            });
        }
        let limits = ChunkLimits {
            small_files_cap: 25,
            large_files_cap: 5,
            large_file_threshold_lines: 400,
            max_lines: 5000,
        };
        let batches = score_and_chunk(rows, limits);
        assert!(batches.len() >= 2);
        // First batch must be capped at 5 files due to the large file presence.
        assert_eq!(batches[0].files.len(), 5);
    }
    #[test]
    fn retain_zero_change_rows_for_submodules_and_binaries() {
        let rows = vec![
            NumstatRow {
                path: "submodules/libA".into(),
                added: 0,
                deleted: 0,
            },
            NumstatRow {
                path: "src/real_change.ts".into(),
                added: 3,
                deleted: 2,
            },
        ];
        let limits = ChunkLimits {
            small_files_cap: 25,
            large_files_cap: 5,
            large_file_threshold_lines: 400,
            max_lines: 5000,
        };
        let batches = score_and_chunk(rows, limits);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].files.len(), 2);
    }
}
