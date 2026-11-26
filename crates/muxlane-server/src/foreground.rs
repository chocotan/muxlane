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

#[derive(Debug, Default)]
pub(crate) struct ForegroundSample {
    pub shell_foregrounds: HashMap<String, ShellForeground>,
    pub agy_titles: HashMap<String, String>,
}

/// Two batched queries cover all panes; no per-pane forks and no shell evaluation.
/// Missing/failed samples are omitted rather than treated as an Agent exit.
pub(crate) async fn sample() -> HashMap<String, ShellForeground> {
    sample_all().await.shell_foregrounds
}

/// Batched sample of shell foreground identities AND live `agy` conversation titles.
pub(crate) async fn sample_all() -> ForegroundSample {
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
        (Some(panes), Some(processes)) => {
            let shell_foregrounds = identify_panes(&panes, &processes);
            let agy_pids = identify_agy_pids(&panes, &processes);
            let agy_titles = if agy_pids.is_empty() {
                HashMap::new()
            } else {
                tokio::task::spawn_blocking(move || resolve_agy_titles(&agy_pids))
                    .await
                    .unwrap_or_default()
            };
            ForegroundSample {
                shell_foregrounds,
                agy_titles,
            }
        }
        _ => ForegroundSample::default(),
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
        let bytes = &rest.as_bytes()[cursor..];
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
        ("/.pie/", AgentType::Pie),
        ("/@mariozechner/pi-coding-agent/", AgentType::Pi),
        ("/@earendil-works/pi-coding-agent/", AgentType::Pi),
        ("/pi-coding-agent/", AgentType::Pi),
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

#[derive(Debug, Clone)]
pub(crate) struct PaneRuntimeInfo {
    pub cwd: std::path::PathBuf,
    pub fg_proc_start_secs: Option<u64>,
}

/// Queries tmux + process table for a single pane's live working directory and foreground
/// agent process start timestamp (Unix seconds), used to accurately bind PTY bridges for
/// both new and pre-existing Pi/Pie sessions (even after `cd` inside a shell tab).
pub(crate) async fn sample_pane_runtime(tmux_session: &str) -> Option<PaneRuntimeInfo> {
    let target = format!("={tmux_session}");
    let tmux_args = [
        "-L",
        "muxlane",
        "list-panes",
        "-t",
        &target,
        "-F",
        "#{pane_pid}\t#{pane_current_path}",
    ];
    let (pane_out, ps_out) = tokio::join!(
        output("tmux", &tmux_args),
        output("ps", &["-axo", "pid=,ppid=,pgid=,tpgid=,args="])
    );
    let pane_line = pane_out?.lines().next()?.to_string();
    let (pid_raw, path_raw) = pane_line.split_once('\t')?;
    let pane_pid: u32 = pid_raw.trim().parse().ok()?;
    let pane_cwd = std::path::PathBuf::from(path_raw.trim());

    let fg_pid = ps_out
        .as_deref()
        .and_then(|procs| find_foreground_agent_pid(pane_pid, procs))
        .unwrap_or(pane_pid);

    let live_cwd = read_proc_cwd(fg_pid).unwrap_or(pane_cwd);
    let fg_proc_start_secs = read_proc_start_unix_secs(fg_pid);

    Some(PaneRuntimeInfo {
        cwd: live_cwd,
        fg_proc_start_secs,
    })
}

fn find_foreground_agent_pid(pane_pid: u32, process_list: &str) -> Option<u32> {
    let processes: HashMap<_, _> = process_list
        .lines()
        .filter_map(parse_process)
        .map(|p| (p.pid, p))
        .collect();
    let root = processes.get(&pane_pid)?;
    if command_agent(&root.command).is_some() {
        return Some(root.pid);
    }
    if root.foreground <= 0 {
        return None;
    }
    let mut best: Option<(usize, u32)> = None;
    for process in processes.values().filter(|p| p.group == root.foreground) {
        let mut current = process;
        let mut seen = HashSet::new();
        let mut recognized: Option<(usize, u32)> = None;
        loop {
            if current.pid == root.pid {
                if let Some(hit) = recognized {
                    if best.is_none_or(|prev| hit.0 < prev.0) {
                        best = Some(hit);
                    }
                }
                break;
            }
            if !seen.insert(current.pid) {
                break;
            }
            if command_agent(&current.command).is_some() {
                recognized = Some((seen.len(), current.pid));
            }
            let Some(parent) = processes.get(&current.parent) else {
                break;
            };
            current = parent;
        }
    }
    best.map(|(_, pid)| pid)
}

#[cfg(target_os = "linux")]
fn read_proc_cwd(pid: u32) -> Option<std::path::PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(not(target_os = "linux"))]
fn read_proc_cwd(_pid: u32) -> Option<std::path::PathBuf> {
    None
}

#[cfg(target_os = "linux")]
fn read_proc_start_unix_secs(pid: u32) -> Option<u64> {
    std::fs::metadata(format!("/proc/{pid}"))
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

#[cfg(not(target_os = "linux"))]
fn read_proc_start_unix_secs(_pid: u32) -> Option<u64> {
    None
}

/// Identifies the active `agy` process PID for each tmux pane (both direct `agy` panes
/// and shell-rooted panes currently running `agy` in the foreground).
fn identify_agy_pids(panes: &str, process_list: &str) -> HashMap<String, u32> {
    let processes: HashMap<_, _> = process_list
        .lines()
        .filter_map(parse_process)
        .map(|p| (p.pid, p))
        .collect();
    let mut out = HashMap::new();
    for line in panes.lines() {
        let Some((name, pid_raw)) = line.rsplit_once(' ') else {
            continue;
        };
        let Ok(pane_pid) = pid_raw.trim().parse::<u32>() else {
            continue;
        };
        let Some(root) = processes.get(&pane_pid) else {
            continue;
        };
        if command_agent(&root.command) == Some(AgentType::Agy) {
            out.insert(name.to_string(), root.pid);
            continue;
        }
        if root.foreground <= 0 {
            continue;
        }
        let mut best: Option<(usize, u32)> = None;
        for process in processes.values().filter(|p| p.group == root.foreground) {
            let mut current = process;
            let mut seen = HashSet::new();
            let mut recognized: Option<(usize, u32)> = None;
            loop {
                if current.pid == root.pid {
                    if let Some(hit) = recognized {
                        if best.is_none_or(|prev| hit.0 < prev.0) {
                            best = Some(hit);
                        }
                    }
                    break;
                }
                if !seen.insert(current.pid) {
                    break;
                }
                if command_agent(&current.command) == Some(AgentType::Agy) {
                    recognized = Some((seen.len(), current.pid));
                }
                let Some(parent) = processes.get(&current.parent) else {
                    break;
                };
                current = parent;
            }
        }
        if let Some((_, agy_pid)) = best {
            out.insert(name.to_string(), agy_pid);
        }
    }
    out
}

fn is_valid_uuid_like(s: &str) -> bool {
    s.len() >= 16
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b'-')
}

fn extract_agy_conversation_id_from_path(path: &std::path::Path) -> Option<String> {
    let raw = path.to_string_lossy();
    let marker = "/antigravity-cli/";
    let idx = raw.find(marker)?;
    let tail = &raw[idx + marker.len()..];
    let candidate = if let Some(rest) = tail.strip_prefix("presence/") {
        rest.strip_suffix(".lock")?
    } else if let Some(rest) = tail.strip_prefix("conversations/") {
        rest.strip_suffix(".db")
            .or_else(|| rest.strip_suffix(".db-wal"))
            .or_else(|| rest.strip_suffix(".db-shm"))?
    } else {
        return None;
    };
    is_valid_uuid_like(candidate).then(|| candidate.to_string())
}

#[cfg(target_os = "linux")]
fn read_agy_conversation_id(pid: u32) -> Option<String> {
    let entries = std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    let mut fallback_db: Option<String> = None;
    for entry in entries.flatten() {
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        let raw = target.to_string_lossy();
        if raw.contains("/antigravity-cli/presence/") {
            if let Some(id) = extract_agy_conversation_id_from_path(&target) {
                return Some(id);
            }
        } else if fallback_db.is_none() && raw.contains("/antigravity-cli/conversations/") {
            fallback_db = extract_agy_conversation_id_from_path(&target);
        }
    }
    fallback_db
}

#[cfg(not(target_os = "linux"))]
fn read_agy_conversation_id(_pid: u32) -> Option<String> {
    None
}

fn agy_app_data_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    Some(home.join(".gemini").join("antigravity-cli"))
}

fn file_mtime_nanos(path: &std::path::Path) -> Option<u128> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_nanos())
}

#[derive(Default)]
struct AgySummaryCache {
    db_mtime: Option<u128>,
    wal_mtime: Option<u128>,
    titles: HashMap<String, String>,
}

fn load_agy_summary_titles(app_dir: &std::path::Path) -> HashMap<String, String> {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<AgySummaryCache>> = OnceLock::new();

    let db_path = app_dir.join("conversation_summaries.db");
    let wal_path = app_dir.join("conversation_summaries.db-wal");
    let db_mtime = file_mtime_nanos(&db_path);
    let wal_mtime = file_mtime_nanos(&wal_path);
    if db_mtime.is_none() {
        return HashMap::new();
    }

    let mutex = CACHE.get_or_init(|| Mutex::new(AgySummaryCache::default()));
    if let Ok(guard) = mutex.lock() {
        if guard.db_mtime == db_mtime && guard.wal_mtime == wal_mtime {
            return guard.titles.clone();
        }
    }

    let mut titles = HashMap::new();
    if let Ok(output) = std::process::Command::new("sqlite3")
        .args([
            "-readonly",
            "-separator",
            "\x1f",
            &db_path.to_string_lossy(),
            "SELECT conversation_id, REPLACE(REPLACE(COALESCE(NULLIF(TRIM(title), ''), TRIM(preview), ''), CHAR(10), ' '), CHAR(13), ' ') FROM conversation_summaries WHERE COALESCE(NULLIF(TRIM(title), ''), TRIM(preview), '') != '';",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Some((cid, raw_title)) = line.split_once('\x1f') {
                    if let Some(clean) = sanitize_session_title(raw_title) {
                        titles.insert(cid.trim().to_string(), clean);
                    }
                }
            }
        }
    }

    if let Ok(mut guard) = mutex.lock() {
        guard.db_mtime = db_mtime;
        guard.wal_mtime = wal_mtime;
        guard.titles = titles.clone();
    }
    titles
}

fn sanitize_session_title(raw: &str) -> Option<String> {
    let first_line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let collapsed: String = first_line
        .chars()
        .filter(|ch| !ch.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.is_empty() {
        return None;
    }
    let mut chars = collapsed.chars();
    let head: String = chars.by_ref().take(60).collect();
    if chars.next().is_some() {
        Some(format!("{head}…"))
    } else {
        Some(head)
    }
}

fn read_agy_transcript_first_prompt(
    app_dir: &std::path::Path,
    conversation_id: &str,
) -> Option<String> {
    use std::io::Read;
    let transcript = app_dir
        .join("brain")
        .join(conversation_id)
        .join(".system_generated")
        .join("logs")
        .join("transcript.jsonl");
    let mut file = std::fs::File::open(transcript).ok()?;
    let mut buf = vec![0u8; 64 * 1024];
    let n = file.read(&mut buf).ok()?;
    buf.truncate(n);
    let text = String::from_utf8_lossy(&buf);
    for line in text.lines() {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if val.get("type").and_then(|v| v.as_str()) != Some("USER_INPUT") {
            continue;
        }
        let Some(content) = val.get("content").and_then(|v| v.as_str()) else {
            continue;
        };
        if let Some(title) = extract_agy_user_request_title(content) {
            return Some(title);
        }
    }
    None
}

fn extract_agy_user_request_title(content: &str) -> Option<String> {
    let inner = if let Some(start) = content.find("<USER_REQUEST>") {
        let after = &content[start + "<USER_REQUEST>".len()..];
        let end = after.find("</USER_REQUEST>").unwrap_or(after.len());
        &after[..end]
    } else {
        content
    };
    sanitize_session_title(inner)
}

fn resolve_agy_titles(agy_pids: &HashMap<String, u32>) -> HashMap<String, String> {
    let Some(app_dir) = agy_app_data_dir() else {
        return HashMap::new();
    };
    let mut pane_cids: Vec<(String, String)> = Vec::new();
    for (session_name, pid) in agy_pids {
        if let Some(cid) = read_agy_conversation_id(*pid) {
            pane_cids.push((session_name.clone(), cid));
        }
    }
    if pane_cids.is_empty() {
        return HashMap::new();
    }
    let summary_titles = load_agy_summary_titles(&app_dir);
    let mut resolved = HashMap::new();
    for (session_name, cid) in pane_cids {
        if let Some(title) = summary_titles.get(&cid) {
            resolved.insert(session_name, title.clone());
        } else if let Some(title) = read_agy_transcript_first_prompt(&app_dir, &cid) {
            resolved.insert(session_name, title);
        }
    }
    resolved
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

    #[test]
    fn identifies_agy_pids_for_both_direct_and_shell_panes() {
        let panes = "muxlane-agy_1 100\nmuxlane-shell_2 200\nmuxlane-shell_3 300\n";
        let ps = "\
100 1 100 100 agy
200 1 200 250 /usr/bin/zsh
250 200 250 250 /usr/local/bin/agy
300 1 300 300 /usr/bin/zsh
";
        let pids = identify_agy_pids(panes, ps);
        assert_eq!(pids.get("muxlane-agy_1"), Some(&100));
        assert_eq!(pids.get("muxlane-shell_2"), Some(&250));
        assert!(!pids.contains_key("muxlane-shell_3"));
    }

    #[test]
    fn extracts_agy_conversation_id_and_user_request_title() {
        assert_eq!(
            extract_agy_conversation_id_from_path(std::path::Path::new(
                "/home/choco/.gemini/antigravity-cli/presence/92f7980d-70f6-431b-8691-d8cb90fe1a74.lock"
            )),
            Some("92f7980d-70f6-431b-8691-d8cb90fe1a74".into())
        );
        assert_eq!(
            extract_agy_conversation_id_from_path(std::path::Path::new(
                "/home/choco/.gemini/antigravity-cli/conversations/5bda2a54-5b26-42fe-ac4d-78f9b0363cd1.db-wal"
            )),
            Some("5bda2a54-5b26-42fe-ac4d-78f9b0363cd1".into())
        );
        assert_eq!(
            extract_agy_user_request_title(
                "<USER_REQUEST>\n帮忙把当前目录整理为正确的文件名格式\n</USER_REQUEST>\n<ADDITIONAL_METADATA>..."
            ),
            Some("帮忙把当前目录整理为正确的文件名格式".into())
        );
    }
}
