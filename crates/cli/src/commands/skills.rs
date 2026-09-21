//! Bundled agent skills: SKILL.md packs that teach AI agents (Claude Code,
//! hermes, Cursor, ...) how to generate on the NOLGIA platform. Embedded in
//! the binary so `brew install` + `nolgia skills install` is the whole story.

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::output::{OutputContext, OutputFormat, print_json};

pub struct BundledSkill {
    pub name: &'static str,
    pub content: &'static str,
}

pub const SKILLS: &[BundledSkill] = &[
    BundledSkill {
        name: "nolgia-platform",
        content: include_str!("../../skills/nolgia-platform/SKILL.md"),
    },
    BundledSkill {
        name: "nolgia-video-prompting",
        content: include_str!("../../skills/nolgia-video-prompting/SKILL.md"),
    },
    BundledSkill {
        name: "nolgia-ugc-ads",
        content: include_str!("../../skills/nolgia-ugc-ads/SKILL.md"),
    },
];

#[derive(Subcommand, Debug)]
pub enum SkillsCommand {
    /// List the skills bundled with this binary
    List,
    /// Print a bundled skill to stdout
    Show(ShowArgs),
    /// Install bundled skills for an AI agent to use
    Install(InstallArgs),
}

#[derive(Args, Debug)]
pub struct ShowArgs {
    pub name: String,
}

#[derive(Args, Debug)]
pub struct InstallArgs {
    /// Skills to install (default: all)
    pub names: Vec<String>,
    /// Where to install
    #[arg(long, value_enum, default_value_t = Target::ClaudeUser)]
    pub target: Target,
    /// Custom directory (implies --target dir)
    #[arg(long)]
    pub dir: Option<PathBuf>,
    /// Overwrite existing skill files
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// ~/.claude/skills/<name>/SKILL.md (Claude Code, user-wide)
    ClaudeUser,
    /// ./.claude/skills/<name>/SKILL.md (current project)
    ClaudeProject,
    /// $HERMES_HOME/skills/<name>/SKILL.md (hermes-agent)
    Hermes,
    /// --dir <path>/<name>/SKILL.md
    Dir,
}

#[derive(Serialize)]
struct SkillInfo {
    name: &'static str,
    description: String,
}

#[derive(Serialize)]
struct Installed {
    name: &'static str,
    path: String,
    status: InstallStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum InstallStatus {
    Installed,
    Unchanged,
    Skipped,
    Overwritten,
}

/// Pull the (possibly folded multi-line) `description:` value out of the
/// YAML frontmatter without a YAML dependency.
fn description_of(content: &str) -> String {
    let mut in_frontmatter = false;
    for line in content.lines() {
        if line.trim_end() == "---" {
            if in_frontmatter {
                break;
            }
            in_frontmatter = true;
            continue;
        }
        if in_frontmatter && let Some(rest) = line.strip_prefix("description:") {
            let d = rest.trim().trim_matches('"');
            let first_sentence = d.split(". ").next().unwrap_or(d);
            return first_sentence.trim_end_matches('.').to_string();
        }
    }
    String::new()
}

fn find(name: &str) -> Result<&'static BundledSkill> {
    SKILLS.iter().find(|s| s.name == name).with_context(|| {
        let names: Vec<_> = SKILLS.iter().map(|s| s.name).collect();
        format!(
            "unknown skill {name:?} — bundled skills: {}",
            names.join(", ")
        )
    })
}

fn target_root(target: Target, dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(d) = dir {
        return Ok(d.to_path_buf());
    }
    match target {
        Target::Dir => bail!("--target dir requires --dir <path>"),
        Target::ClaudeProject => Ok(PathBuf::from(".claude/skills")),
        Target::ClaudeUser => {
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .context("cannot resolve home directory")?;
            Ok(PathBuf::from(home).join(".claude/skills"))
        }
        Target::Hermes => {
            let home = std::env::var("HERMES_HOME").unwrap_or_else(|_| "/opt/data".into());
            Ok(PathBuf::from(home).join("skills"))
        }
    }
}

pub fn run(command: SkillsCommand, output: &OutputContext) -> Result<()> {
    match command {
        SkillsCommand::List => list(output),
        SkillsCommand::Show(args) => show(args),
        SkillsCommand::Install(args) => install(args, output),
    }
}

fn list(output: &OutputContext) -> Result<()> {
    let infos: Vec<SkillInfo> = SKILLS
        .iter()
        .map(|s| SkillInfo {
            name: s.name,
            description: description_of(s.content),
        })
        .collect();
    match output.format() {
        OutputFormat::Json => print_json(output, &infos),
        OutputFormat::Text => {
            for info in infos {
                println!("{:24} {}", info.name, info.description);
            }
            Ok(())
        }
    }
}

fn show(args: ShowArgs) -> Result<()> {
    print!("{}", find(&args.name)?.content);
    Ok(())
}

fn install(args: InstallArgs, output: &OutputContext) -> Result<()> {
    let root = target_root(args.target, args.dir.as_deref())?;
    let selected: Vec<&BundledSkill> = if args.names.is_empty() {
        SKILLS.iter().collect()
    } else {
        args.names
            .iter()
            .map(|n| find(n))
            .collect::<Result<Vec<_>>>()?
    };

    let mut installed = Vec::new();
    for skill in selected {
        let dir = root.join(skill.name);
        let path = dir.join("SKILL.md");
        let status = match fs::read(&path) {
            Ok(content) if content == skill.content.as_bytes() => InstallStatus::Unchanged,
            Ok(_) if args.force => InstallStatus::Overwritten,
            Ok(_) => InstallStatus::Skipped,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => InstallStatus::Installed,
            Err(_) if args.force => InstallStatus::Overwritten,
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", path.display()));
            }
        };
        if matches!(
            status,
            InstallStatus::Installed | InstallStatus::Overwritten
        ) {
            fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
            fs::write(&path, skill.content)
                .with_context(|| format!("writing {}", path.display()))?;
        }
        installed.push(Installed {
            name: skill.name,
            path: path.display().to_string(),
            status,
        });
    }

    match output.format() {
        OutputFormat::Json => print_json(output, &installed),
        OutputFormat::Text => {
            let mut counts = [0; 4];
            for i in &installed {
                match i.status {
                    InstallStatus::Installed => {
                        counts[0] += 1;
                        println!("installed {} -> {}", i.name, i.path);
                    }
                    InstallStatus::Unchanged => {
                        counts[1] += 1;
                        println!("unchanged {} (already at {})", i.name, i.path);
                    }
                    InstallStatus::Skipped => {
                        counts[2] += 1;
                        println!(
                            "skipped {} (differs from the bundled copy; pass --force to update it) -> {}",
                            i.name, i.path
                        );
                    }
                    InstallStatus::Overwritten => {
                        counts[3] += 1;
                        println!("overwritten {} -> {}", i.name, i.path);
                    }
                }
            }
            println!(
                "{} installed, {} unchanged, {} skipped, {} overwritten. Agents pick them up on their next session.",
                counts[0], counts[1], counts[2], counts[3]
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_skills_have_valid_frontmatter() {
        for skill in SKILLS {
            assert!(
                skill.content.starts_with("---"),
                "{} missing frontmatter",
                skill.name
            );
            assert!(
                skill.content.contains(&format!("name: {}", skill.name)),
                "{} frontmatter name mismatch",
                skill.name
            );
            assert!(
                !description_of(skill.content).is_empty(),
                "{} missing description",
                skill.name
            );
        }
    }

    /// The frontmatter line `key: value` inside the leading `---` block.
    fn frontmatter_value<'a>(content: &'a str, key: &str) -> Option<&'a str> {
        let block = content.strip_prefix("---\n")?.split("\n---\n").next()?;
        block.lines().find_map(|line| {
            line.trim_start()
                .strip_prefix(key)
                .and_then(|rest| rest.strip_prefix(':'))
                .map(str::trim)
        })
    }

    /// The same conventions nolgiainc/nolgia-skills lints in CI (NOL-884,
    /// NOL-885): these files are copied there verbatim, so a pack that would
    /// fail the public repo's check fails here first. A description is the
    /// routing signal an agent reads every turn: it must say when to use the
    /// skill and when not to, within the Agent Skills 1,024-character limit.
    #[test]
    fn bundled_skills_follow_the_published_skill_conventions() {
        let names: Vec<&str> = SKILLS.iter().map(|s| s.name).collect();
        let mut versions = Vec::new();
        for skill in SKILLS {
            let description = frontmatter_value(skill.content, "description")
                .unwrap_or_else(|| panic!("{} has no description line", skill.name))
                .trim_matches('"');
            assert!(
                description.chars().count() <= 1024,
                "{} description is {} chars (max 1024)",
                skill.name,
                description.chars().count()
            );
            for marker in ["Use when", "NOT for"] {
                assert!(
                    description.contains(marker),
                    "{} description needs a {marker:?} clause",
                    skill.name
                );
            }
            let version = frontmatter_value(skill.content, "version")
                .unwrap_or_else(|| panic!("{} has no version", skill.name));
            versions.push(version);
            if let Some(related) = frontmatter_value(skill.content, "related_skills") {
                for other in related.trim_matches(['[', ']']).split(',').map(str::trim) {
                    assert!(
                        names.contains(&other),
                        "{} lists related skill {other:?}, which is not bundled",
                        skill.name
                    );
                }
            }
        }
        assert!(
            versions.windows(2).all(|pair| pair[0] == pair[1]),
            "bundled skills must share one version (the skills repo's VERSION): {versions:?}"
        );
    }

    #[test]
    fn install_writes_files_and_respects_force() {
        let tmp = tempfile::tempdir().unwrap();
        let args = |force| InstallArgs {
            names: vec!["nolgia-platform".into()],
            target: Target::Dir,
            dir: Some(tmp.path().to_path_buf()),
            force,
        };
        install(args(false), &OutputFormat::Text.into()).unwrap();
        let path = tmp.path().join("nolgia-platform/SKILL.md");
        assert_eq!(fs::read_to_string(&path).unwrap(), SKILLS[0].content);

        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        install(args(false), &OutputFormat::Text.into()).unwrap();
        install(args(true), &OutputFormat::Text.into()).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);

        fs::write(&path, "customized skill").unwrap();
        install(args(false), &OutputFormat::Text.into()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "customized skill");

        install(args(true), &OutputFormat::Text.into()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), SKILLS[0].content);
    }
}
