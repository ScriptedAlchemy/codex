//! Helpers to determine the base branch to compare against for `/review-branch`.

use std::io;
use std::process::Stdio;
use tokio::process::Command;

/// Resolve the most appropriate base ref for the current branch.
///
/// Preference order:
/// 1. If a PR exists (GitHub CLI), use its base ref (remote or local).
/// 2. If upstream points to a different branch than the current local branch name, use it.
/// 3. The default branch of the primary remote (prefer `origin`), via `refs/remotes/<remote>/HEAD`.
/// 4. Remote common fallbacks: `origin/main`, `origin/master`, `origin/trunk`, `origin/develop`.
/// 5. Local common fallbacks: `main`, `master`, `trunk`, `develop`.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedBase {
    pub base: String,
    pub reason: String,
}

pub(crate) async fn resolve_base_with_hint() -> io::Result<ResolvedBase> {
    // Ensure we're inside a Git repo.
    if !inside_git_repo().await? {
        return Err(io::Error::other("not inside a git repository"));
    }

    // 0) PR base via GitHub CLI (optional).
    if let Some(base_ref) = gh_pr_base_ref().await? {
        if let Some(remote) = default_remote().await? {
            let remote_ref = format!("{remote}/{base_ref}");
            if rev_parse_verify(&remote_ref).await? {
                return Ok(ResolvedBase {
                    base: remote_ref,
                    reason: "PR base".to_string(),
                });
            }
        }
        if rev_parse_verify(&base_ref).await? {
            return Ok(ResolvedBase {
                base: base_ref,
                reason: "PR base".to_string(),
            });
        }
    }

    // 1) Upstream that is NOT just the remote-tracking copy of the same branch.
    let current = current_branch_name().await?;
    if let Some(up) = rev_parse_upstream().await? {
        if let Some(cur) = current.as_deref() {
            let tail = up.split('/').last().unwrap_or("");
            if tail != cur {
                return Ok(ResolvedBase {
                    base: up,
                    reason: "upstream".to_string(),
                });
            }
        } else {
            return Ok(ResolvedBase {
                base: up,
                reason: "upstream".to_string(),
            });
        }
    }

    // 2) Remote default HEAD, then common remote names.
    if let Some(remote) = default_remote().await? {
        if let Some(sym) = remote_head_symbolic_ref(&remote).await? {
            return Ok(ResolvedBase {
                base: sym,
                reason: "remote default".to_string(),
            });
        }
        for name in ["main", "master", "trunk", "develop"] {
            let candidate = format!("{remote}/{name}");
            if rev_parse_verify(&candidate).await? {
                return Ok(ResolvedBase {
                    base: candidate,
                    reason: "remote fallback".to_string(),
                });
            }
        }
    }

    // 3) Local common branch names (repos without remotes)
    for name in ["main", "master", "trunk", "develop"] {
        if rev_parse_verify(name).await? {
            return Ok(ResolvedBase {
                base: name.to_string(),
                reason: "local fallback".to_string(),
            });
        }
    }

    Err(io::Error::other(
        "could not determine base branch; set an upstream with `git push -u`, open a PR, or create a local `main`/`master` branch",
    ))
}

pub(crate) async fn resolve_base() -> io::Result<String> {
    Ok(resolve_base_with_hint().await?.base)
}

async fn inside_git_repo() -> io::Result<bool> {
    let status = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;

    match status {
        Ok(s) if s.success() => Ok(true),
        Ok(_) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Return `origin` if present, else the first remote if any.
async fn default_remote() -> io::Result<Option<String>> {
    let output = Command::new("git")
        .args(["remote"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut remotes: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if remotes.is_empty() {
        return Ok(None);
    }
    if remotes.contains(&"origin") {
        return Ok(Some("origin".to_string()));
    }
    Ok(Some(remotes.remove(0).to_string()))
}

/// Try to resolve the branch upstream, returning values like `origin/main`.
async fn rev_parse_upstream() -> io::Result<Option<String>> {
    maybe_capture_stdout(&[
        "rev-parse",
        "--abbrev-ref",
        "--symbolic-full-name",
        "@{upstream}",
    ])
    .await
}

/// Resolve `refs/remotes/<remote>/HEAD` to `<remote>/<default_branch>`.
async fn remote_head_symbolic_ref(remote: &str) -> io::Result<Option<String>> {
    if let Some(sym) = maybe_capture_stdout(&[
        "symbolic-ref",
        "--quiet",
        &format!("refs/remotes/{remote}/HEAD"),
    ])
    .await?
    {
        // Example: refs/remotes/origin/main -> origin/main
        let trimmed = sym.trim();
        let prefix = "refs/remotes/";
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return Ok(Some(rest.to_string()));
        }
        return Ok(Some(trimmed.to_string()));
    }
    Ok(None)
}

/// Return `true` if `rev-parse --verify --quiet <ref>` succeeds.
async fn rev_parse_verify(r: &str) -> io::Result<bool> {
    let status = Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", r])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?;
    Ok(status.success())
}

/// Capture stdout when the command succeeds; return Ok(None) when it fails.
async fn maybe_capture_stdout(args: &[&str]) -> io::Result<Option<String>> {
    let output = Command::new("git")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await?;

    if output.status.success() {
        Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ))
    } else {
        Ok(None)
    }
}

/// Current local branch name (None for detached HEAD).
async fn current_branch_name() -> io::Result<Option<String>> {
    let out = maybe_capture_stdout(&["rev-parse", "--abbrev-ref", "HEAD"]).await?;
    match out.as_deref() {
        Some("HEAD") => Ok(None),
        Some(name) if !name.is_empty() => Ok(Some(name.to_string())),
        _ => Ok(None),
    }
}

/// Optional: use `gh` to detect PR base ref for current branch.
async fn gh_pr_base_ref() -> io::Result<Option<String>> {
    // Is gh available?
    let gh_ok = Command::new("gh")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .ok()
        .map(|s| s.success())
        .unwrap_or(false);
    if !gh_ok {
        return Ok(None);
    }
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            "--json",
            "baseRefName",
            "--jq",
            ".baseRefName",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;
    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if text.is_empty() {
                Ok(None)
            } else {
                Ok(Some(text))
            }
        }
        _ => Ok(None),
    }
}
