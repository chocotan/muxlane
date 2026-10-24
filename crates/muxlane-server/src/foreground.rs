//! Identify an Agent in a tmux pane's foreground process group, never from screen text.
use muxlane_core::model::AgentType;
use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::time::Duration;

#[derive(Debug)]
struct Process {
    pid: u32,
    parent: u32,
    group: i64,
    foreground: i64,
    command: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ShellForeground {
    pub shell: String,
    pub agent_type: AgentType,
}

/// Two batched queries cover all panes; no per-pane forks and no shell evaluation.
/// Missing/failed samples are omitted rather than treated as an Agent exit.
pub(crate) async fn sample() -> HashMap<String, ShellForeground> {
    let (panes, processes) = tokio::join!(
        output(
            "tmux",
            &[
                "-L",
                "muxlane",
                "list-panes",
                "-a",
                "-F",
                "#{session_name} #{pane_pid}"
            ]
        ),
        output("ps", &["-axo", "pid=,ppid=,pgid=,tpgid=,args="])
    );
    match (panes, processes) {
        (Some(panes), Some(processes)) => identify_panes(&panes, &processes),
        _ => HashMap::new(),
    }
}

async fn output(program: &str, args: &[&str]) -> Option<String> {
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    result
        .status
        .success()
        .then(|| String::from_utf8_lossy(&result.stdout).into_owned())
}

fn identify_panes(panes: &str, process_list: &str) -> HashMap<String, ShellForeground> {
    let processes: HashMap<_, _> = process_list
        .lines()
        .filter_map(parse_process)
        .map(|p| (p.pid, p))
        .collect();
    panes
        .lines()
        .filter_map(|line| {
            // tmux replaces literal tabs with underscores in a non-UTF-8 locale.
            let (name, pid) = line.rsplit_once(' ')?;
            let root = processes.get(&pid.trim().parse::<u32>().ok()?)?;
            let shell = executable(&root.command);
            if !matches!(
                shell,
                "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" | "nu"
            ) || root.foreground <= 0
            {
                return None;
            }
            let mut candidates: Vec<_> = processes
                .values()
                .filter(|process| process.group == root.foreground)
                .filter_map(|process| {
                    let mut current = process;
                    let mut seen = HashSet::new();
                    let mut recognized = None;
                    loop {
                        if current.pid == root.pid {
                            return Some(recognized);
                        }
                        if !seen.insert(current.pid) {
                            return None;
                        }
                        // Inspect the foreground process and its ancestors up to the pane shell.
                        // A foreground tool launched by an Agent still belongs to that Agent.
                        if let Some(kind) = command_agent(&current.command) {
                            recognized = Some((seen.len(), kind));
                        }
                        current = processes.get(&current.parent)?;
                    }
                })
                .collect();
            if candidates.is_empty() {
                return None; // A transient/incomplete ps snapshot must not reset identity.
            }
            candidates
                .sort_by_key(|item| item.as_ref().map(|(depth, kind)| (*depth, kind.as_str())));
            let agent_type = candidates
                .into_iter()
                .flatten()
                .last()
                .map(|(_, kind)| kind)
                .unwrap_or(AgentType::Shell);
            Some((
                name.to_string(),
                ShellForeground {
                    shell: shell.into(),
                    agent_type,
                },
            ))
        })
        .collect()
}

fn parse_process(line: &str) -> Option<Process> {
    // BusyBox and BSD ps can glue right-aligned columns together. Take four
    // numeric fields, then the command, instead of requiring a space between them.
    let rest = line.trim_start();
    let mut values = [0i64; 4];
    let mut cursor = 0;
    for value in &mut values {
        let bytes = rest[cursor..].as_bytes();
        let start = bytes.iter().position(|byte| !byte.is_ascii_whitespace())?;
        cursor += start;
        let token = rest[cursor..].split_whitespace().next()?;
        let number: String = token
            .chars()
            .take_while(|ch| ch.is_ascii_digit() || *ch == '-')
            .collect();
        if number.is_empty() || number == "-" {
            return None;
        }
        *value = number.parse().ok()?;
        cursor += number.len();
    }
    let command = rest[cursor..].trim().to_string();
    (!command.is_empty()).then_some(Process {
        pid: values[0] as u32,
        parent: values[1] as u32,
        group: values[2],
        foreground: values[3],
        command,
    })
}

fn executable(command: &str) -> &str {
    command
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .trim_start_matches('-')
}

fn command_agent(command: &str) -> Option<AgentType> {
    let direct = |name| match name {
        "claude" => Some(AgentType::Claude),
        "codex" => Some(AgentType::Codex),
        "opencode" => Some(AgentType::Opencode),
        "pi" => Some(AgentType::Pi),
        "pie" => Some(AgentType::Pie),
        "agy" => Some(AgentType::Agy),
        "qwen" => Some(AgentType::Qwen),
        "kimi" => Some(AgentType::Kimi),
        _ => None,
    };
    let program = executable(command);
    if let Some(kind) = direct(program) {
        return Some(kind);
    }
    if !matches!(program, "node" | "nodejs" | "bun" | "python" | "python3") {
        return None;
    }
    let mut args = command.split_whitespace().skip(1);
    let script = args.next()?;
    // Do not mistake `node -e '...pi...'`, echo, grep, or script arguments for an Agent.
    if script.starts_with('-') {
        return None;
    }
    if let Some(kind) = direct(script.rsplit('/').next()?) {
        return Some(kind);
    }
    for (package, kind) in [
        ("/@mariozechner/pi-coding-agent/", AgentType::Pi),
        ("/@earendil-works/pi-coding-agent/", AgentType::Pi),
        ("/pi-coding-agent/", AgentType::Pi),
        ("/.pie/", AgentType::Pie),
        ("/@anthropic-ai/claude-code/", AgentType::Claude),
        ("/@openai/codex/", AgentType::Codex),
        ("/opencode-ai/", AgentType::Opencode),
        ("/@qwen-code/qwen-code/", AgentType::Qwen),
    ] {
        if script.contains(package) {
            return Some(kind);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreground_agent_switches_back_to_shell_and_ignores_background_jobs() {
        let panes = "muxlane-shell_test 100\n";
        let running = "100 1 100 200 /bin/zsh\n200 100 200 200 node /opt/lib/node_modules/@mariozechner/pi-coding-agent/dist/cli.js\n300 100 300 200 codex\n";
        assert_eq!(
            identify_panes(panes, running)["muxlane-shell_test"].agent_type,
            AgentType::Pi
        );
        let shell = running.replace("100 1 100 200", "100 1 100 100");
        assert_eq!(
            identify_panes(panes, &shell)["muxlane-shell_test"].agent_type,
            AgentType::Shell
        );
        let tool = format!(
            "{}400 200 400 400 /bin/git status\n",
            running.replace("100 1 100 200", "100 1 100 400")
        );
        assert_eq!(
            identify_panes(panes, &tool)["muxlane-shell_test"].agent_type,
            AgentType::Pi
        );
    }

    #[test]
    fn recognizes_pie_in_sessions_with_spaces() {
        let panes = "muxlane-shell test 2430944\n";
        let processes =
            "2430944 860104 2430944 2431410 /usr/bin/zsh\n2431410 2430944 2431410 2431410 pie\n";
        assert_eq!(
            identify_panes(panes, processes)["muxlane-shell test"].agent_type,
            AgentType::Pie
        );
    }

    #[test]
    fn incomplete_samples_and_non_shell_presets_are_not_reset() {
        let panes = "test 100\n";
        for ps in [
            "",
            "100 1 100 -1 /bin/zsh",
            "100 1 100 200 /bin/zsh",
            "100 1 100 100 codex",
        ] {
            assert!(identify_panes(panes, ps).is_empty());
        }
    }

    #[test]
    fn recognizes_executables_and_npm_entrypoints_not_mentions() {
        for command in ["pi", "/usr/local/bin/pi --help", "node /home/test/.nvm/versions/node/v22/lib/node_modules/@mariozechner/pi-coding-agent/dist/cli.js", "node /opt/lib/node_modules/@earendil-works/pi-coding-agent/dist/cli.js", "node /opt/bin/pi", "node /opt/pi-coding-agent/dist/cli.js"] {
            assert_eq!(command_agent(command), Some(AgentType::Pi), "{command}");
        }
        assert_eq!(
            command_agent("node /home/choco/.pie/runtime/cli.js"),
            Some(AgentType::Pie)
        );
        assert_eq!(command_agent("pie"), Some(AgentType::Pie));
        let panes = "muxlane-shell_test 100\n";
        let busybox = "100 1 100 200/bin/bash\n200 100 200 200/usr/bin/node /opt/pi-coding-agent/dist/cli.js\n";
        assert_eq!(
            identify_panes(panes, busybox)["muxlane-shell_test"].agent_type,
            AgentType::Pi
        );
        for command in [
            "echo pi",
            "grep codex log",
            "node app.js pi",
            "node -e 'pi()'",
            "python3 my-pi.py",
            "gemini",
            "pi-helper",
        ] {
            assert_eq!(command_agent(command), None, "{command}");
        }
    }
}
