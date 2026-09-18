//! PLAN.md export/import (AgentTrail-compatible conventions).

use std::fmt::Write as _;
use std::path::Path;

use crate::{ExecutionPlan, PlanStep, VerificationStrategy};

/// Relative path used under a project root.
pub const PLAN_REL_PATH: &str = ".raya/PLAN.md";

/// Render an execution plan as Markdown.
pub fn plan_to_markdown(plan: &ExecutionPlan, project_name: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# {project_name}");
    let _ = writeln!(out);
    if let Some(summary) = &plan.summary {
        let _ = writeln!(out, "{summary}");
        let _ = writeln!(out);
    }
    let _ = writeln!(out, "## Execution {{#execution}}");
    let _ = writeln!(
        out,
        "verification: {}",
        verification_label(&plan.verification)
    );
    let _ = writeln!(out);

    for step in &plan.steps {
        let tools = if step.expected_tools.is_empty() {
            String::new()
        } else {
            format!("\n  tools: [{}]", step.expected_tools.join(", "))
        };
        let _ = writeln!(
            out,
            "- [ ] {} {{#{}}}{tools}",
            step.description.trim(),
            sanitize_id(&step.id)
        );
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "## decisions");
    let _ = writeln!(
        out,
        "- {}: plan exported by RAYA Agent",
        chrono::Utc::now().date_naive()
    );
    out
}

/// Parse a minimal PLAN.md back into an [`ExecutionPlan`].
///
/// Recognizes checkbox lines: `- [ ] desc {#id}` / `- [x]` / `- [~]` / `- [!]`.
pub fn plan_from_markdown(text: &str) -> ExecutionPlan {
    let mut steps = Vec::new();
    let mut summary = None;
    let mut verification = VerificationStrategy::default();

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("verification:") {
            verification = parse_verification(rest.trim());
            continue;
        }
        if let Some((status, rest)) = parse_checkbox(trimmed) {
            let (desc, id) = split_id(rest);
            let _ = status; // status reserved for future sync of step state
            steps.push(PlanStep {
                id,
                description: desc,
                expected_tools: Vec::new(),
            });
            continue;
        }
        // First non-heading paragraph as summary
        if summary.is_none()
            && !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && !trimmed.starts_with("tools:")
            && !trimmed.starts_with('-')
        {
            summary = Some(trimmed.to_string());
        }
    }

    ExecutionPlan {
        steps,
        verification,
        summary,
        nodes: Vec::new(),
    }
}

/// Write plan markdown under `project_root/.raya/PLAN.md`.
pub fn write_plan_file(
    project_root: &Path,
    plan: &ExecutionPlan,
) -> std::io::Result<std::path::PathBuf> {
    let dir = project_root.join(".raya");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("PLAN.md");
    let name = project_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project");
    std::fs::write(&path, plan_to_markdown(plan, name))?;
    Ok(path)
}

/// Read `.raya/PLAN.md` if present.
pub fn read_plan_file(project_root: &Path) -> std::io::Result<Option<ExecutionPlan>> {
    let path = project_root.join(PLAN_REL_PATH);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)?;
    Ok(Some(plan_from_markdown(&text)))
}

fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn verification_label(v: &VerificationStrategy) -> &'static str {
    match v {
        VerificationStrategy::None => "none",
        VerificationStrategy::Test => "test",
        VerificationStrategy::Build => "build",
        VerificationStrategy::TestAndBuild => "test_and_build",
        VerificationStrategy::Custom { .. } => "custom",
    }
}

fn parse_verification(s: &str) -> VerificationStrategy {
    match s {
        "none" => VerificationStrategy::None,
        "build" => VerificationStrategy::Build,
        "test_and_build" => VerificationStrategy::TestAndBuild,
        "custom" => VerificationStrategy::Custom {
            command: String::new(),
        },
        _ => VerificationStrategy::Test,
    }
}

fn parse_checkbox(line: &str) -> Option<(char, &str)> {
    let rest = line.strip_prefix("- [")?;
    let status = rest.chars().next()?;
    let after = rest.get(1..)?.strip_prefix(']')?.trim_start();
    Some((status, after))
}

fn split_id(rest: &str) -> (String, String) {
    if let Some(start) = rest.rfind("{#")
        && let Some(end) = rest[start..].find('}')
    {
        let id = rest[start + 2..start + end].to_string();
        let desc = rest[..start].trim().to_string();
        return (desc, id);
    }
    (rest.trim().to_string(), format!("step-{}", rest.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_checkbox() {
        let plan = ExecutionPlan::new(
            vec![PlanStep {
                id: "1".into(),
                description: "Write hello file".into(),
                expected_tools: vec!["filesystem.write".into()],
            }],
            VerificationStrategy::None,
        );
        let md = plan_to_markdown(&plan, "demo");
        assert!(md.contains("- [ ] Write hello file {#1}"));
        let parsed = plan_from_markdown(&md);
        assert_eq!(parsed.steps.len(), 1);
        assert_eq!(parsed.steps[0].id, "1");
        assert!(parsed.steps[0].description.contains("Write hello"));
    }
}
