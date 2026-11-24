//! SOUL.md & MEMORY.md prompt composition and post-run memory distillation.
use muxlane_core::bot::{BotDefinition, BotPermissionProfile};

const MAX_MEMORY_BYTES: usize = 8 * 1024;

pub fn compose_bot_prompt(
    bot: &BotDefinition,
    trigger_source: &str,
    payload: Option<&str>,
) -> String {
    let payload_text = payload
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("");
    let task_body = if bot.prompt_template.contains("{{payload}}") {
        bot.prompt_template.replace("{{payload}}", payload_text)
    } else if payload_text.is_empty() {
        bot.prompt_template.clone()
    } else if bot.prompt_template.trim().is_empty() {
        payload_text.to_string()
    } else {
        format!(
            "{}\n\nTrigger payload:\n{}",
            bot.prompt_template.trim(),
            payload_text
        )
    };

    let policy_note = match bot.permission {
        BotPermissionProfile::ReadOnly => {
            "Execution policy: READ-ONLY. Never modify files or execute state-mutating shell commands."
        }
        BotPermissionProfile::WorkspaceSafe => {
            "Execution policy: WORKSPACE-SAFE. Avoid destructive external commands."
        }
        BotPermissionProfile::AskFirst => {
            "Execution policy: ASK-FIRST. Dangerous/mutating operations require explicit human approval."
        }
        BotPermissionProfile::FullAuto => {
            "Execution policy: FULL-AUTO within your isolated workspace."
        }
    };

    let mut sections = Vec::new();
    if !bot.soul_markdown.trim().is_empty() {
        sections.push(format!("<soul>\n{}\n</soul>", bot.soul_markdown.trim()));
    }
    if !bot.memory_markdown.trim().is_empty() {
        sections.push(format!(
            "<memory>\n{}\n</memory>",
            bot.memory_markdown.trim()
        ));
    }
    sections.push(format!(
        "<bot_context>\nBot: {}\nTrigger: {}\n{}\n</bot_context>",
        bot.name, trigger_source, policy_note
    ));
    sections.push(task_body.trim().to_string());
    sections.join("\n\n")
}

pub fn distill_memory(existing: &str, trigger_source: &str, summary: &str) -> String {
    let clean_summary: String = summary
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(360)
        .collect();
    if clean_summary.is_empty() {
        return existing.to_string();
    }
    let ts = muxlane_core::model::now_secs();
    let entry = format!("- [{ts}] ({trigger_source}) {clean_summary}");
    let mut lines: Vec<&str> = existing.lines().filter(|l| !l.trim().is_empty()).collect();
    lines.push(&entry);

    let mut joined = lines.join("\n");
    while joined.len() > MAX_MEMORY_BYTES && lines.len() > 1 {
        lines.remove(0);
        joined = lines.join("\n");
    }
    if !joined.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxlane_core::bot::BotDefinition;

    #[test]
    fn compose_and_distill_memory_bounds_output() {
        let mut bot =
            BotDefinition::new("bot_1".into(), "Nightly Reviewer".into(), "proj_1".into());
        bot.soul_markdown = "Check test regressions.".into();
        bot.memory_markdown = "- [1] (cron) baseline ok".into();
        let prompt = compose_bot_prompt(&bot, "cron", Some("PR #42"));
        assert!(prompt.contains("<soul>"));
        assert!(prompt.contains("<memory>"));
        assert!(prompt.contains("PR #42"));

        let next = distill_memory(
            &bot.memory_markdown,
            "manual",
            "Found 1 clippy issue\nand fixed",
        );
        assert!(next.contains("(cron) baseline ok"));
        assert!(next.contains("Found 1 clippy issue and fixed"));
    }
}
