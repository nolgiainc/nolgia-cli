//! A canceled job is a finished job, not a broken one (NOL-1025).
//!
//! `canceled` is its own terminal status: the owner asked the server to stop
//! the job (`nolgia jobs cancel`, or the same action on the web), so it will
//! never be delivered. Rendering it as a failure would be wrong twice over: it
//! reads as "something broke, run it again", and it hides the one thing the
//! reader needs, which is what happened to the credits.
//!
//! The server states that in `cancellation.message`, a sentence written to be
//! shown to the customer as-is, so every rendering here prints it verbatim and
//! adds only the credit figures and, while the provider has not answered yet
//! (`settlement: pending`), the command that shows how it settled.

use std::fmt;

use nolgia_client::types::Job;

use crate::output::{OutputFormat, print_json_unselected};

/// Wire value of a canceled job's `status`.
pub const STATUS: &str = "canceled";

/// What canceling did, in lines a person can read: the server's own sentence,
/// then the credits. `None` for a job that is not canceled.
///
/// A `pending` settlement ends with the command that shows the final answer:
/// the job is already terminal, so `nolgia wait` would return at once without
/// it, and only a later read of the job can.
pub fn describe(job: &Job) -> Option<String> {
    if job.status != STATUS {
        return None;
    }
    let Some(cancellation) = job.cancellation.as_ref() else {
        // A job canceled before the API recorded cancellations carries no
        // settlement at all; say what we know and nothing we do not.
        return Some(
            job.status_message
                .clone()
                .unwrap_or_else(|| "The job was canceled.".to_string()),
        );
    };
    let mut out = cancellation.message.clone();
    out.push_str("\nCredits: ");
    if cancellation.settlement == "pending" {
        out.push_str(&format!(
            "pending. The model provider has not given its final answer yet, so nothing is \
             refunded or charged so far.\nSee how it settles with: nolgia jobs get {}",
            job.id
        ));
        return Some(out);
    }
    let mut figures = Vec::new();
    if let Some(refunded) = cancellation.credits_refunded {
        figures.push(format!("{refunded} refunded"));
    }
    if let Some(charged) = cancellation.credits_charged {
        figures.push(format!("{charged} charged"));
    }
    if figures.is_empty() {
        // The settlement word alone (`refunded`, `charged`, or one this build
        // does not know yet) is still the truth, just without the amount.
        out.push_str(&cancellation.settlement.replace('_', " "));
    } else {
        out.push_str(&figures.join(", "));
    }
    out.push('.');
    Some(out)
}

/// A job that was canceled while a `gen`/`restore` command was waiting for its
/// result. The command cannot produce the asset it promised, so it exits
/// non-zero, but the job is finished rather than live: it must not be
/// reported as [`crate::livejob::LiveJob`] (which tells the reader the job is
/// still being worked on and will be billed) nor as a bare `Error:`.
#[derive(Debug)]
pub struct Canceled {
    job: Job,
}

impl Canceled {
    pub fn render_text(&self) -> String {
        let mut out = format!(
            "canceled: job {} was canceled, so there is no result to download.",
            self.job.id
        );
        if let Some(details) = describe(&self.job) {
            for line in details.lines() {
                out.push_str("\n  ");
                out.push_str(line);
            }
        }
        out
    }

    /// Human text on stderr; under `--json` the whole canceled job on stdout,
    /// unselected, so a program sees `status` and `cancellation` intact.
    pub fn report(&self, format: OutputFormat) {
        eprintln!("{}", self.render_text());
        if format == OutputFormat::Json
            && let Err(err) = print_json_unselected(&self.job)
        {
            eprintln!("Could not print the job as JSON: {err}");
        }
    }
}

impl fmt::Display for Canceled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render_text())
    }
}

impl std::error::Error for Canceled {}

/// Pass a job through unless it was canceled.
pub fn ensure_not_canceled(job: Job) -> anyhow::Result<Job> {
    if job.status == STATUS {
        return Err(Canceled { job }.into());
    }
    Ok(job)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    const ID: &str = "184166c4-0ecd-453c-b907-66cf511ae241";

    fn job(status: &str, cancellation: Value) -> Job {
        serde_json::from_value(json!({
            "id": ID,
            "user_id": "184166c4-0ecd-453c-b907-66cf511ae242",
            "modality": "video", "model": "test-model", "status": status,
            "created_at": "2026-06-13T00:00:00Z", "updated_at": "2026-06-13T00:00:00Z",
            "cancellation": cancellation
        }))
        .expect("valid job")
    }

    fn cancellation(settlement: &str, refunded: Value, charged: Value) -> Value {
        json!({
            "canceled_at": "2026-06-13T00:00:01Z", "stage": "in_progress",
            "provider_cancel": "cancelled", "settlement": settlement,
            "credits_refunded": refunded, "credits_charged": charged,
            "message": "The provider stopped the render."
        })
    }

    #[test]
    fn only_canceled_jobs_are_described_or_refused() {
        for status in ["queued", "running", "succeeded", "failed"] {
            let job = job(status, Value::Null);
            assert!(describe(&job).is_none());
            assert!(ensure_not_canceled(job).is_ok());
        }
        let canceled = job("canceled", cancellation("refunded", json!(30), Value::Null));
        assert!(ensure_not_canceled(canceled).unwrap_err().is::<Canceled>());
    }

    #[test]
    fn settled_credits_print_the_server_sentence_then_the_figures() {
        for (settlement, refunded, charged, credits) in [
            ("refunded", json!(30), Value::Null, "Credits: 30 refunded."),
            (
                "partially_refunded",
                json!(20),
                json!(10),
                "Credits: 20 refunded, 10 charged.",
            ),
            ("charged", Value::Null, json!(30), "Credits: 30 charged."),
            // No figures: the settlement word is still printed, not invented.
            ("refunded", Value::Null, Value::Null, "Credits: refunded."),
            (
                "future_settlement",
                Value::Null,
                Value::Null,
                "Credits: future settlement.",
            ),
        ] {
            let text = describe(&job(
                "canceled",
                cancellation(settlement, refunded, charged),
            ))
            .expect("canceled job is described");
            assert_eq!(text, format!("The provider stopped the render.\n{credits}"));
        }
    }

    #[test]
    fn a_pending_settlement_says_nothing_moved_yet_and_how_to_see_it_land() {
        let text = describe(&job(
            "canceled",
            cancellation("pending", Value::Null, Value::Null),
        ))
        .expect("canceled job is described");
        assert!(text.starts_with("The provider stopped the render.\nCredits: pending."));
        assert!(text.contains("nothing is refunded or charged so far"));
        assert!(text.ends_with(&format!("nolgia jobs get {ID}")));
        // The job is terminal: `wait` would return at once and show nothing new.
        assert!(!text.contains("nolgia wait"));
    }

    #[test]
    fn a_canceled_job_without_a_cancellation_record_claims_no_settlement() {
        let text = describe(&job("canceled", Value::Null)).expect("described");
        assert_eq!(text, "The job was canceled.");
        assert!(!text.contains("Credits"));
    }

    #[test]
    fn a_canceled_wait_never_reads_as_a_failure_or_a_live_job() {
        let text = Canceled {
            job: job("canceled", cancellation("refunded", json!(30), Value::Null)),
        }
        .render_text();
        assert!(text.starts_with(&format!("canceled: job {ID} was canceled")));
        for forbidden in ["Error:", "fail", "still running", "billed once", "\u{2014}"] {
            assert!(!text.contains(forbidden), "{forbidden:?} in {text}");
        }
        assert!(text.contains("\n  The provider stopped the render.\n  Credits: 30 refunded."));
    }
}
