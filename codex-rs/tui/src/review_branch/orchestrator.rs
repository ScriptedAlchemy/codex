use codex_core::protocol::ReviewFinding;
use codex_core::protocol::ReviewOutputEvent;
use codex_core::protocol::ReviewRequest;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::history_cell;
use crate::review_branch::chunker::Batch;
use crate::review_branch::chunker::ChunkLimits;
use crate::review_branch::chunker::collect_branch_numstat;
use crate::review_branch::chunker::score_and_chunk;

#[derive(Clone, Debug, PartialEq)]
enum Stage {
    Batching,
    Consolidation,
    Done,
}

#[derive(Debug)]
pub(crate) struct Orchestrator {
    pub base: String,
    pub reason: String,
    pub batches: Vec<Batch>,
    pub idx: usize,
    pub acc: Vec<ReviewFinding>,
    stage: Stage,
    tx: AppEventSender,
    batch_prompt_tmpl: &'static str,
    consolidation_prompt_tmpl: &'static str,
}

impl Orchestrator {
    pub async fn new(
        tx: AppEventSender,
        base: String,
        reason: String,
        small_files_cap: usize,
        large_files_cap: usize,
        large_file_threshold_lines: usize,
        max_lines: usize,
        batch_prompt_tmpl: &'static str,
        consolidation_prompt_tmpl: &'static str,
    ) -> anyhow::Result<Self> {
        let rows = collect_branch_numstat(&base).await.unwrap_or_default();
        let limits = ChunkLimits {
            small_files_cap,
            large_files_cap,
            large_file_threshold_lines,
            max_lines,
        };
        let batches = score_and_chunk(rows, limits);
        Ok(Self {
            tx,
            base,
            reason,
            batches,
            idx: 0,
            acc: Vec::new(),
            stage: Stage::Batching,
            batch_prompt_tmpl,
            consolidation_prompt_tmpl,
        })
    }

    pub fn is_running(&self) -> bool {
        self.stage != Stage::Done
    }

    pub fn has_batches(&self) -> bool {
        !self.batches.is_empty()
    }

    pub fn start(&mut self) {
        if self.batches.is_empty() {
            self.stage = Stage::Done;
            return;
        }
        self.send_batch_prompt();
    }

    pub fn on_batch_result(&mut self, output: &ReviewOutputEvent) {
        self.acc.extend(output.findings.clone());
        self.idx += 1;
        if self.idx < self.batches.len() {
            self.send_batch_prompt();
        } else {
            // Move to consolidation stage
            self.stage = Stage::Consolidation;
            self.send_consolidation_prompt();
        }
    }

    pub fn on_consolidation_result(&mut self, _output: &ReviewOutputEvent) {
        // Final pass already returns a single consolidated ReviewOutputEvent that UI will render.
        self.stage = Stage::Done;
    }

    fn send_batch_prompt(&self) {
        let k = self.idx + 1;
        let n = self.batches.len();
        let batch = &self.batches[self.idx];
        let file_list = batch
            .files
            .iter()
            .map(|r| r.path.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let size_hint = format!(
            "~{} files, +{}/-{} lines",
            batch.files.len(),
            batch.total_added,
            batch.total_deleted
        );

        let prompt = self
            .batch_prompt_tmpl
            .replace("{base}", &self.base)
            .replace("{batch_index}", &k.to_string())
            .replace("{batch_total}", &n.to_string())
            .replace("{size_hint}", &size_hint)
            .replace("{file_list}", &file_list);

        let hint = format!("batch {k}/{n} vs {} ({})", self.base, self.reason);
        self.tx.send(AppEvent::InsertHistoryCell(Box::new(
            history_cell::new_review_status_line(format!(">> Batch {k}/{n}: {size_hint} <<")),
        )));
        self.tx
            .send(AppEvent::CodexOp(codex_core::protocol::Op::Review {
                review_request: ReviewRequest {
                    prompt,
                    user_facing_hint: hint,
                },
            }));
    }

    fn send_consolidation_prompt(&self) {
        let (clusters_text, stats_text) = build_consolidation_package(&self.acc);
        let prompt = self
            .consolidation_prompt_tmpl
            .replace("{base}", &self.base)
            .replace("{stats}", &stats_text)
            .replace("{clusters}", &clusters_text);
        let hint = format!("consolidation vs {}", self.base);
        self.tx.send(AppEvent::InsertHistoryCell(Box::new(
            history_cell::new_review_status_line(
                ">> Consolidating batch findings (final pass)… <<".to_string(),
            ),
        )));
        self.tx
            .send(AppEvent::CodexOp(codex_core::protocol::Op::Review {
                review_request: ReviewRequest {
                    prompt,
                    user_facing_hint: hint,
                },
            }));
    }
}

/// Build a compact consolidation package to keep token size low.
fn build_consolidation_package(findings: &[ReviewFinding]) -> (String, String) {
    // Very light clustering: group by file and overlapping ranges (<= 5 lines apart), similar titles (case-insensitive prefix match).
    #[derive(Clone)]
    struct Key<'a> {
        path: &'a str,
        start: u32,
    }
    let mut items: Vec<&ReviewFinding> = findings.iter().collect();
    items.sort_by(|a, b| {
        a.code_location
            .absolute_file_path
            .cmp(&b.code_location.absolute_file_path)
            .then(
                a.code_location
                    .line_range
                    .start
                    .cmp(&b.code_location.line_range.start),
            )
    });

    let mut clusters: Vec<Vec<&ReviewFinding>> = Vec::new();
    for f in items {
        let mut placed = false;
        for c in clusters.iter_mut() {
            if let Some(head) = c.first() {
                let same_file =
                    head.code_location.absolute_file_path == f.code_location.absolute_file_path;
                let near = head
                    .code_location
                    .line_range
                    .start
                    .abs_diff(f.code_location.line_range.start)
                    <= 5;
                let title_similar = head.title.to_lowercase().split_whitespace().next()
                    == f.title.to_lowercase().split_whitespace().next();
                if same_file && near && title_similar {
                    c.push(f);
                    placed = true;
                    break;
                }
            }
        }
        if !placed {
            clusters.push(vec![f]);
        }
    }

    // Serialize minimal fields for each cluster
    let mut out = String::new();
    for (i, c) in clusters.iter().enumerate() {
        out.push_str(&format!("\n- cluster {i}:\n"));
        for f in c.iter() {
            let path = f.code_location.absolute_file_path.display();
            let lr = &f.code_location.line_range;
            out.push_str(&format!(
                "  - {title} | {path}:{start}-{end} | p={priority} | conf={conf:.2}\n",
                title = f.title,
                start = lr.start,
                end = lr.end,
                priority = f.priority,
                conf = f.confidence_score,
            ));
        }
    }
    let stats = format!(
        "total_findings: {} total_clusters: {}",
        findings.len(),
        clusters.len()
    );
    (out, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_event::AppEvent;
    use crate::app_event_sender::AppEventSender;
    use tokio::sync::mpsc::unbounded_channel;

    #[tokio::test(flavor = "current_thread")]
    async fn start_emits_hint_with_reason_and_batch_status() {
        // Prepare a single tiny batch
        let batch = Batch {
            files: vec![crate::review_branch::chunker::NumstatRow {
                path: "src/lib.rs".into(),
                added: 10,
                deleted: 0,
            }],
            total_added: 10,
            total_deleted: 0,
        };
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut orc = Orchestrator {
            base: "origin/main".to_string(),
            reason: "PR base: main".to_string(),
            batches: vec![batch],
            idx: 0,
            acc: Vec::new(),
            stage: Stage::Batching,
            tx,
            batch_prompt_tmpl: "{base} {batch_index}/{batch_total} {size_hint} {file_list}",
            consolidation_prompt_tmpl: "{base} {stats} {clusters}",
        };

        orc.start();

        // Expect a status InsertHistoryCell and a CodexOp Review with reason in hint
        let mut saw_status = false;
        let mut saw_review = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                AppEvent::InsertHistoryCell(_) => {
                    saw_status = true;
                }
                AppEvent::CodexOp(codex_core::protocol::Op::Review { review_request }) => {
                    assert!(review_request.user_facing_hint.contains("PR base: main"));
                    assert!(review_request.user_facing_hint.contains("batch 1/1"));
                    saw_review = true;
                }
                _ => {}
            }
        }
        assert!(saw_status && saw_review);
    }
}
