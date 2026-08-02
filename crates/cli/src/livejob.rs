//! Never lose a job the server already accepted.
//!
//! This module exists because of one incident (NOL-344): a generation was
//! submitted, the client-side pipe died, the CLI printed a transport error and
//! **no job id**, and the operator — with no evidence that a job existed at
//! all — re-ran the command and paid for the render twice.
//!
//! The server-side half of that fix has since landed: an identical
//! re-submission is refused with a `409` naming the first job, so the second
//! charge is now structurally prevented. This module is the other half. A
//! guard that makes the duplicate free does not make it *unnecessary* — the
//! user still has to be given a reason not to re-run, and that reason is
//! always the same reason: **a job is live under this id.**
//!
//! Four different endings deliver that same fact:
//!
//! - the server's long-poll expired (`408`) while the job kept running,
//! - the user pressed Ctrl-C after the submission went out,
//! - the connection dropped, or anything else failed, after the submission
//!   went out,
//! - the submission was refused as a duplicate (`409`) of a job that already
//!   exists.
//!
//! From where the user sits these are one situation — *work is live, follow
//! it, do not re-submit* — so they share one rendering and one exit code.
//! Treating them as four unrelated errors is precisely the mistake that cost
//! 84 credits.

use std::fmt;

use uuid::Uuid;

use crate::output::OutputFormat;

/// Exit status for "a job is live; re-running is the wrong move".
///
/// Deliberately not `1`. A caller has to be able to tell "this has not
/// finished yet" from "this failed", and until now it could not: the CLI
/// collapsed a `408` long-poll expiry into the same exit `1` as a genuine
/// error. That is not only a human problem — it is why nolgia-agent's CLI
/// backend cannot implement the `_wait_once() -> None` contract that its SDK
/// backend has had all along, so on the CLI transport any render outlasting a
/// single long-poll chunk aborts the whole wait.
///
/// `75` is sysexits.h `EX_TEMPFAIL` — "temporary failure, the request should
/// be retried" — which is exactly this situation. It stays non-zero, so no
/// existing caller mistakes it for success, and it avoids the shell's
/// 126/127 and the 128+n signal range.
pub const EXIT_LIVE_JOB: u8 = 75;

/// A generation job that the server has accepted and is still responsible
/// for, surfaced at the moment this command stopped being able to follow it.
#[derive(Debug)]
pub enum LiveJob {
    /// `GET /jobs/{id}/wait` returned `408`. Nothing failed: the long-poll
    /// window closed while the job was still running.
    StillRunning { job_id: Uuid, waited_seconds: u64 },
    /// Ctrl-C arrived after the submission had already gone out.
    Interrupted { job_id: Uuid },
    /// The submission succeeded and something afterwards did not.
    Detached { job_id: Uuid, cause: String },
    /// `409` — this exact request is already a job.
    Duplicate { job_id: Uuid, detail: String },
}

impl LiveJob {
    pub fn job_id(&self) -> Uuid {
        match self {
            Self::StillRunning { job_id, .. }
            | Self::Interrupted { job_id }
            | Self::Detached { job_id, .. }
            | Self::Duplicate { job_id, .. } => *job_id,
        }
    }

    /// Stable machine token for `--json` consumers.
    pub fn outcome(&self) -> &'static str {
        match self {
            Self::StillRunning { .. } => "still_running",
            Self::Interrupted { .. } => "interrupted",
            Self::Detached { .. } => "detached",
            Self::Duplicate { .. } => "duplicate",
        }
    }

    /// The first line: what happened, and the job id — in that order, because
    /// the id is the part the reader has to leave with.
    fn headline(&self) -> String {
        let id = self.job_id();
        match self {
            Self::StillRunning { waited_seconds, .. } => {
                format!("still running after {waited_seconds}s — job {id}")
            }
            Self::Interrupted { .. } => format!("interrupted — job {id} is still running"),
            Self::Detached { .. } => {
                format!("submitted job {id} — this command could not finish following it")
            }
            Self::Duplicate { .. } => format!("already submitted — job {id}"),
        }
    }

    /// Why this is not a reason to re-run. Every branch has to say two things:
    /// the job still exists, and a re-run would be a *second* job.
    fn explanation(&self) -> Vec<String> {
        match self {
            Self::StillRunning { .. } => vec![
                "Nothing failed. The server's long-poll window closed while the job was \
                 still running — the job was not cancelled and is still being worked on."
                    .into(),
                "It will be billed once, whether or not you keep waiting. Re-running this \
                 command would start a second job."
                    .into(),
            ],
            Self::Interrupted { .. } => vec![
                "The submission had already gone out when the interrupt arrived, so the \
                 job was not cancelled and is still being worked on."
                    .into(),
                "It will be billed once, whether or not you wait for it. Re-running this \
                 command would start a second job."
                    .into(),
            ],
            Self::Detached { cause, .. } => vec![
                "The submission itself succeeded, so the job exists and is unaffected by \
                 whatever went wrong here."
                    .into(),
                "It will be billed once. Re-running this command would start a second job.".into(),
                format!("Cause: {cause}"),
            ],
            Self::Duplicate { detail, .. } => vec![
                // The server's own wording is authoritative on the window and
                // on the fact that nothing was double-charged, so it is
                // reproduced rather than paraphrased.
                detail.clone(),
            ],
        }
    }

    /// Commands the reader can actually run. The API's `409` says to
    /// "check it with GET /jobs/{id}", which is true and useless at a shell
    /// prompt; this is the same advice in the vocabulary of this program.
    fn follow_ups(&self) -> Vec<(String, &'static str)> {
        let id = self.job_id();
        let mut steps = vec![
            (format!("nolgia wait {id}"), "keep waiting for it"),
            (format!("nolgia status {id}"), "check it once"),
        ];
        if matches!(self, Self::Duplicate { .. }) {
            steps.reverse();
            steps.push((
                "nolgia gen ... --idempotency-key <new-value>".to_string(),
                "deliberately run it again as a separate job",
            ));
        }
        steps
    }

    /// The human rendering. No `Error:` prefix anywhere in here — that word is
    /// what made a 408 read as a failure and sent people back to re-run.
    pub fn render_text(&self) -> String {
        let mut out = self.headline();
        for line in self.explanation() {
            out.push_str("\n  ");
            out.push_str(&line);
        }
        let steps = self.follow_ups();
        let width = steps.iter().map(|(c, _)| c.len()).max().unwrap_or(0);
        for (command, note) in steps {
            out.push_str(&format!("\n    {command:width$}  # {note}"));
        }
        out
    }

    fn render_json(&self) -> serde_json::Value {
        serde_json::json!({
            "job_id": self.job_id().to_string(),
            "outcome": self.outcome(),
            "billed_twice": false,
            "message": self.headline(),
            "follow_up": self.follow_ups()
                .into_iter()
                .map(|(command, _)| command)
                .collect::<Vec<_>>(),
        })
    }

    /// Report to the user. The human block always goes to stderr — including
    /// under `--json`, so that a program's stdout stays a clean JSON document
    /// while a human reading the same terminal still learns the job id.
    pub fn report(&self, format: OutputFormat) {
        eprintln!("{}", self.render_text());
        if format == OutputFormat::Json {
            println!(
                "{}",
                serde_json::to_string_pretty(&self.render_json())
                    .unwrap_or_else(|_| self.job_id().to_string())
            );
        }
    }
}

impl fmt::Display for LiveJob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render_text())
    }
}

impl std::error::Error for LiveJob {}

/// Print the job id the moment the server accepts it, before any waiting
/// starts.
///
/// This is the cheapest and most robust half of the whole fix: an error path
/// can only speak if the process survives long enough to reach it, and in the
/// incident it did not. A line written at submission time is already in the
/// operator's scrollback even if the CLI is later killed outright, the pipe is
/// torn down, or the terminal goes away.
///
/// stderr, so `--json` stdout stays a single parseable document.
pub fn announce(job_id: Uuid, timeout_seconds: u64) {
    eprintln!(
        "submitted job {job_id} — waiting up to {timeout_seconds}s (Ctrl-C is safe: it does not cancel the job)"
    );
}

/// Run the post-submission phase of a `gen` command so that no ending can lose
/// the job id.
///
/// Two things happen here that cannot happen at the call sites: Ctrl-C is
/// raced against the work (otherwise the process dies silently holding the
/// only copy of the id), and any error that is not already a [`LiveJob`] is
/// re-labelled as one, because *every* failure after a successful submission
/// leaves a job running.
pub async fn guard<T>(
    job_id: Uuid,
    work: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    let result = tokio::select! {
        biased;
        result = work => result,
        _ = tokio::signal::ctrl_c() => Err(LiveJob::Interrupted { job_id }.into()),
    };
    result.map_err(|err| match err.downcast::<LiveJob>() {
        // Already carries the job id and the right explanation.
        Ok(live) => live.into(),
        Err(err) => LiveJob::Detached {
            job_id,
            // `{err:#}` flattens the anyhow chain onto one line, keeping the
            // recovery block readable while preserving the diagnosis.
            cause: format!("{err:#}"),
        }
        .into(),
    })
}

/// Pull the job id out of an RFC 7807 `detail`.
///
/// The API names the existing job in prose only — both `409` and `408` are
/// typed in the spec as bare problem objects (`additionalProperties: false`),
/// so there is no structured field to read and no way to add one without an
/// API change. Scanning the sentence is therefore the only way to promote the
/// id to the top of our own message; when it fails the caller still prints the
/// server's text verbatim, which is exactly today's behavior.
pub fn find_job_id(detail: &str) -> Option<Uuid> {
    const UUID_LEN: usize = 36;
    let bytes = detail.as_bytes();
    (0..bytes.len().saturating_sub(UUID_LEN - 1))
        .filter(|start| detail.is_char_boundary(*start))
        .find_map(|start| {
            let end = start + UUID_LEN;
            detail
                .is_char_boundary(end)
                .then(|| Uuid::parse_str(&detail[start..end]).ok())
                .flatten()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact body prod returns on a duplicate submission, captured from
    /// `POST /v1/generate/audio` on 2026-08-02. The id appears twice and the
    /// second occurrence is glued to a path and a full stop, so a naive
    /// whitespace split would find nothing.
    const REAL_409_DETAIL: &str = "this exact request was already submitted as job \
        184166c4-0ecd-453c-b907-66cf511ae241 less than 5m0s ago and has not been billed \
        twice — check it with GET /jobs/184166c4-0ecd-453c-b907-66cf511ae241. To run it \
        again anyway, resubmit with a different Idempotency-Key header.";

    #[test]
    fn finds_the_job_id_in_the_real_409_detail() {
        assert_eq!(
            find_job_id(REAL_409_DETAIL),
            Some(Uuid::parse_str("184166c4-0ecd-453c-b907-66cf511ae241").expect("valid uuid"))
        );
    }

    #[test]
    fn finds_a_job_id_glued_to_punctuation() {
        let id = Uuid::parse_str("233d0d6f-859a-4aa2-8b33-3cc420ca3932").expect("valid uuid");
        for detail in [
            "GET /jobs/233d0d6f-859a-4aa2-8b33-3cc420ca3932.",
            "(233d0d6f-859a-4aa2-8b33-3cc420ca3932)",
            "233d0d6f-859a-4aa2-8b33-3cc420ca3932",
        ] {
            assert_eq!(find_job_id(detail), Some(id), "failed on {detail:?}");
        }
    }

    #[test]
    fn finds_nothing_when_there_is_no_job_id() {
        assert_eq!(find_job_id("job did not finish before timeout"), None);
        assert_eq!(find_job_id(""), None);
        assert_eq!(find_job_id("184166c4-0ecd-453c-b907"), None);
    }

    /// Multi-byte text must not panic the scanner.
    #[test]
    fn tolerates_non_ascii_detail() {
        assert_eq!(find_job_id("déjà vu — 🎬 no id here"), None);
        assert!(find_job_id("看 184166c4-0ecd-453c-b907-66cf511ae241 好").is_some());
    }

    /// A 408 is not a failure and must not read like one. The bar is not
    /// "avoids the word failure" — the copy denies failure outright, which is
    /// the point — but that the reader is never *told* something went wrong.
    #[test]
    fn a_wait_timeout_never_reads_as_a_failure() {
        let text = LiveJob::StillRunning {
            job_id: Uuid::nil(),
            waited_seconds: 300,
        }
        .render_text();

        // The exact prefix that made a 408 look like a failure. `Error:` is
        // what `main` prints for a genuine error, and printing it here is the
        // whole defect: it is the strongest possible prompt to re-run.
        assert!(!text.contains("Error:"), "{text}");

        // The headline is the line people actually read, so it carries the
        // strict rule.
        let headline = text.lines().next().expect("a headline");
        let lowered = headline.to_lowercase();
        assert!(
            !lowered.contains("error") && !lowered.contains("fail"),
            "the headline must not suggest failure: {headline}"
        );
        assert!(
            headline.starts_with("still running after 300s — job "),
            "{headline}"
        );

        // ...and the body has to say so in as many words.
        assert!(text.contains("Nothing failed."), "{text}");
        assert!(text.contains("would start a second job"), "{text}");
    }

    /// Whatever the ending, the id and both recovery commands must be present
    /// — that is the entire point of the module.
    #[test]
    fn every_ending_names_the_job_and_offers_a_way_to_follow_it() {
        let job_id = Uuid::parse_str("184166c4-0ecd-453c-b907-66cf511ae241").expect("valid uuid");
        for live in [
            LiveJob::StillRunning {
                job_id,
                waited_seconds: 300,
            },
            LiveJob::Interrupted { job_id },
            LiveJob::Detached {
                job_id,
                cause: "connection closed before message completed".into(),
            },
            LiveJob::Duplicate {
                job_id,
                detail: REAL_409_DETAIL.into(),
            },
        ] {
            let text = live.render_text();
            assert!(text.contains(&job_id.to_string()), "{text}");
            assert!(text.contains(&format!("nolgia status {job_id}")), "{text}");
            assert!(text.contains(&format!("nolgia wait {job_id}")), "{text}");
            assert_eq!(live.render_json()["job_id"], job_id.to_string());
            assert_eq!(live.render_json()["billed_twice"], false);
        }
    }

    /// A duplicate is the one ending where the user may genuinely have meant
    /// it, so the escape hatch has to be reachable from the CLI.
    #[test]
    fn a_duplicate_offers_the_deliberate_second_take() {
        let text = LiveJob::Duplicate {
            job_id: Uuid::nil(),
            detail: REAL_409_DETAIL.into(),
        }
        .render_text();
        assert!(text.contains("--idempotency-key"), "{text}");
        assert!(text.contains("has not been billed twice"), "{text}");
    }
}
