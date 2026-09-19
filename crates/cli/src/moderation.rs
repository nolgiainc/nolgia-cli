use std::fmt;

use nolgia_client::types::Job;

use crate::output::{OutputFormat, print_json};

/// sysexits.h EX_DATAERR ("the input data was incorrect"): the provider's
/// content filter refused the prompt or references. Edit them or switch models
/// to fix the request. Distinct from 1 (a genuine error) and 75 (a live job).
pub const EXIT_MODERATED: u8 = 65;

#[derive(Debug)]
pub struct Moderated {
    job: Job,
}

impl Moderated {
    pub fn from_job(job: &Job) -> Option<Self> {
        (job.status == "failed"
            && job
                .failure
                .as_ref()
                .is_some_and(|failure| failure.kind == "moderated"))
        .then(|| Self { job: job.clone() })
    }

    pub fn render_text(&self) -> String {
        let failure = self.job.failure.as_ref();
        let message = failure.map_or("", |failure| failure.message.as_str());
        let refund = match failure.and_then(|failure| failure.credits_refunded) {
            Some(true) => "refunded, the credit hold was released.",
            Some(false) => {
                "charged for this attempt, because the provider processed the request before its filter blocked it."
            }
            None => {
                "no refund outcome was recorded for this job. Check your usage before assuming either way."
            }
        };
        format!(
            "Blocked by the content filter: job {} was not generated.\n  {}\n  Credits: {}\n  Edit the prompt or the reference media, or switch models, and run the command again.",
            self.job.id, message, refund
        )
    }

    pub fn report(&self, format: OutputFormat) {
        eprintln!("{}", self.render_text());
        if format == OutputFormat::Json
            && let Err(err) = print_json(&self.job)
        {
            eprintln!("Could not print the job as JSON: {err}");
        }
    }
}

impl fmt::Display for Moderated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render_text())
    }
}

impl std::error::Error for Moderated {}

pub fn ensure_not_moderated(job: Job) -> anyhow::Result<Job> {
    match Moderated::from_job(&job) {
        Some(moderated) => Err(moderated.into()),
        None => Ok(job),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn job(status: &str, failure: Option<serde_json::Value>) -> Job {
        serde_json::from_value(json!({
            "id": "184166c4-0ecd-453c-b907-66cf511ae241",
            "user_id": "184166c4-0ecd-453c-b907-66cf511ae242",
            "modality": "video", "model": "test-model", "status": status,
            "created_at": "2026-06-13T00:00:00Z", "updated_at": "2026-06-13T00:00:00Z",
            "failure": failure
        }))
        .expect("valid job")
    }

    #[test]
    fn only_failed_moderated_jobs_are_classified_as_moderated() {
        for (status, kind) in [
            ("succeeded", Some("moderated")),
            ("running", Some("moderated")),
            ("failed", Some("error")),
            ("failed", Some("future_kind")),
            ("failed", None),
        ] {
            let job = job(
                status,
                kind.map(|kind| json!({"kind": kind, "message": "reason"})),
            );
            assert!(Moderated::from_job(&job).is_none());
            assert!(ensure_not_moderated(job).is_ok());
        }
    }

    #[test]
    fn rendering_reports_the_exact_refund_truth_without_em_dashes() {
        for (refund, truth) in [
            (Some(json!(true)), "refunded, the credit hold was released."),
            (
                Some(json!(false)),
                "charged for this attempt, because the provider processed the request before its filter blocked it.",
            ),
            (
                None,
                "no refund outcome was recorded for this job. Check your usage before assuming either way.",
            ),
            (
                Some(json!(null)),
                "no refund outcome was recorded for this job. Check your usage before assuming either way.",
            ),
        ] {
            let mut failure =
                json!({"kind": "moderated", "message": "Provider blocked reference media"});
            if let Some(refund) = refund {
                failure["credits_refunded"] = refund;
            }
            let job = job("failed", Some(failure));
            let moderated = Moderated::from_job(&job).expect("moderated job");
            let text = moderated.render_text();
            assert_eq!(
                text,
                format!(
                    "Blocked by the content filter: job {} was not generated.\n  Provider blocked reference media\n  Credits: {truth}\n  Edit the prompt or the reference media, or switch models, and run the command again.",
                    job.id
                )
            );
            assert!(!text.contains('\u{2014}'));
            assert_eq!(moderated.to_string(), text);
            assert!(ensure_not_moderated(job).unwrap_err().is::<Moderated>());
        }
    }
}
