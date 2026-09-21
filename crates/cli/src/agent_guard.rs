use std::{ffi::OsString, fmt};

use crate::output::{OutputFormat, print_json_unselected};

/// sysexits.h EX_NOPERM: an agent cannot change the owner's workspace.
pub const EXIT_AGENT_REFUSED: u8 = 77;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentMarker {
    TurnCredential,
    AgentPod,
    DeclaredAgentSurface,
}

impl fmt::Display for AgentMarker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TurnCredential => "agent turn credential",
            Self::AgentPod => "agent pod",
            Self::DeclaredAgentSurface => "declared agent surface",
        })
    }
}

pub fn detect(token: &str, env: impl Fn(&str) -> Option<OsString>) -> Option<AgentMarker> {
    if token.starts_with("nolt_") {
        return Some(AgentMarker::TurnCredential);
    }
    if env("HERMES_HOME").is_some() && env("HERMES_DASHBOARD").is_some() {
        return Some(AgentMarker::AgentPod);
    }
    if env("NOLGIA_SURFACE")
        .and_then(|value| value.into_string().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("hermes"))
    {
        return Some(AgentMarker::DeclaredAgentSurface);
    }
    None
}

#[derive(Debug)]
pub enum AgentRefused {
    Switch,
    Create,
}

impl AgentRefused {
    const fn command(&self) -> &'static str {
        match self {
            Self::Switch => "org switch",
            Self::Create => "org create",
        }
    }

    const fn message(&self) -> &'static str {
        match self {
            Self::Switch => "Refused: an agent cannot switch the owner's workspace.",
            Self::Create => "Refused: an agent cannot create an organization for the owner.",
        }
    }

    const fn explanation(&self) -> &'static str {
        match self {
            Self::Switch => {
                "Switching moves the owner's workspace everywhere at once: the web app, every chat session and every token."
            }
            Self::Create => {
                "A new organization becomes the owner's active workspace everywhere at once."
            }
        }
    }

    const fn owner_action(&self) -> &'static str {
        match self {
            Self::Switch => {
                "The owner switches from the workspace switcher in the account menu on nolgia.ai."
            }
            Self::Create => "The owner creates one from the account menu on nolgia.ai.",
        }
    }

    pub fn report(&self, format: OutputFormat) {
        eprintln!("{self}");
        if format == OutputFormat::Json
            && let Err(err) = print_json_unselected(&serde_json::json!({
                "error": "agent_refused",
                "command": self.command(),
                "message": self.message(),
                "owner_action": self.owner_action(),
            }))
        {
            eprintln!("Could not print the refusal as JSON: {err}");
        }
    }
}

impl fmt::Display for AgentRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}\n{}\n{}",
            self.message(),
            self.explanation(),
            self.owner_action()
        )
    }
}

impl std::error::Error for AgentRefused {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_local_agent_markers() {
        for (token, env, expected) in [
            (
                "nolt_0123456789abcdef",
                vec![],
                Some(AgentMarker::TurnCredential),
            ),
            (
                "",
                vec![("HERMES_HOME", ""), ("HERMES_DASHBOARD", "")],
                Some(AgentMarker::AgentPod),
            ),
            (
                "",
                vec![("NOLGIA_SURFACE", "hermes")],
                Some(AgentMarker::DeclaredAgentSurface),
            ),
            (
                "",
                vec![("NOLGIA_SURFACE", " Hermes ")],
                Some(AgentMarker::DeclaredAgentSurface),
            ),
            ("", vec![("HERMES_HOME", "home")], None),
            ("", vec![("HERMES_DASHBOARD", "dashboard")], None),
            (
                "",
                vec![
                    ("NOLGIA_SURFACE", "cli"),
                    ("HERMES_HOME", "home"),
                    ("HERMES_DASHBOARD", "dashboard"),
                ],
                Some(AgentMarker::AgentPod),
            ),
            ("nol_0123456789abcdef", vec![], None),
            ("", vec![], None),
            (
                "nolt_test",
                vec![
                    ("HERMES_HOME", "home"),
                    ("HERMES_DASHBOARD", "dashboard"),
                    ("NOLGIA_SURFACE", "hermes"),
                ],
                Some(AgentMarker::TurnCredential),
            ),
            ("", vec![("NOLGIA_SURFACE", "hermes-other")], None),
        ] {
            let marker = detect(token, |key| {
                env.iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| OsString::from(value))
            });
            assert_eq!(marker, expected);
        }
    }
}
