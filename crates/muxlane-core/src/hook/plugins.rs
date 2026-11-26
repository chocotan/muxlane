use super::write_file_if_changed;
use crate::Result;
use std::path::Path;

/// 上报脚本本体（node，零依赖：用 stdin 之外直接命令行参数 + socket 写 JSON）
pub const REPORT_SCRIPT: &str = concat!(
    "#!/usr/bin/env node\n",
    include_str!("../../hooks/shared-prelude.js"),
    include_str!("../../hooks/report.js")
);

/// OpenCode plugin：`session.idle` 权威上报 done。
pub const OPENCODE_PLUGIN: &str = concat!(
    include_str!("../../hooks/shared-prelude.js"),
    include_str!("../../hooks/opencode-plugin.js")
);

/// Pi extension：基于 pi-wechat-notifier 模式支持待确认(ask_user)、Subagent 状态区分及异常捕获。
pub const PI_EXTENSION: &str = concat!(
    include_str!("../../hooks/shared-prelude.js"),
    include_str!("../../hooks/pi-images.js"),
    include_str!("../../hooks/pi-extension.js")
);

/// 安装 OpenCode/Pi 插件（幂等，不覆盖其他插件）。
/// 同时清掉旧 lmux 时代的插件文件（改名的遗留），避免与新版并存重复上报。
pub fn install_agent_plugins(home: &Path) -> Result<()> {
    let opencode = home.join(".config/opencode/plugins/muxlane.ts");
    write_file_if_changed(&opencode, OPENCODE_PLUGIN)?;
    let pi = home.join(".pi/agent/extensions/muxlane.ts");
    write_file_if_changed(&pi, PI_EXTENSION)?;
    // Pie resolves getAgentDir() to ~/.pie (loading extensions from ~/.pie/extensions),
    // while older Pie builds looked under ~/.pie/agent/extensions. Install to both.
    let pie = home.join(".pie/extensions/muxlane.ts");
    write_file_if_changed(&pie, PI_EXTENSION)?;
    let pie_legacy_dir = home.join(".pie/agent/extensions/muxlane.ts");
    write_file_if_changed(&pie_legacy_dir, PI_EXTENSION)?;
    for legacy in [
        ".config/opencode/plugins/lmux.ts",
        ".pi/agent/extensions/lmux.ts",
        ".pie/extensions/lmux.ts",
        ".pie/agent/extensions/lmux.ts",
    ] {
        std::fs::remove_file(home.join(legacy)).ok();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_include_shared_prelude_once() {
        for script in [REPORT_SCRIPT, OPENCODE_PLUGIN, PI_EXTENSION] {
            assert_eq!(script.matches("function muxlaneEnv").count(), 1);
            assert_eq!(script.matches("function report").count(), 1);
        }
        assert!(REPORT_SCRIPT.starts_with("#!/usr/bin/env node\n"));
    }

    #[test]
    fn generated_pi_extension_reports_parent_lifecycle() {
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let extension = dir.path().join("muxlane.mjs");
        let runner = dir.path().join("runner.mjs");
        std::fs::write(&extension, PI_EXTENSION).unwrap();
        std::fs::write(
            &runner,
            r#"import assert from "node:assert/strict"
import net from "node:net"
import { pathToFileURL } from "node:url"

const reports = []
net.createConnection = () => {
  const handlers = {}
  const client = {
    setTimeout() {},
    end(data) { reports.push(JSON.parse(data).params) },
    destroy() {},
    on(name, callback) {
      handlers[name] = callback
      if (name === "close") queueMicrotask(() => {
        handlers.connect?.()
        handlers.close?.()
      })
      return client
    },
  }
  return client
}
process.env.MUXLANE_SOCKET = "/tmp/muxlane-hook-test.sock"
process.env.MUXLANE_AGENT_ID = "parent"
process.env.MUXLANE_HOOK_TOKEN = "token"

// 拦截防抖定时器：手动 flush，不让测试真等 2.5s。
const realSetTimeout = globalThis.setTimeout.bind(globalThis)
const pendingTimers = new Set()
globalThis.setTimeout = (cb) => { pendingTimers.add(cb); return cb }
globalThis.clearTimeout = (handle) => { pendingTimers.delete(handle) }
const flushTimers = async () => {
  const due = [...pendingTimers]
  pendingTimers.clear()
  for (const cb of due) cb()
  await new Promise((resolve) => realSetTimeout(resolve, 0))
}

const extension = await import(pathToFileURL(process.argv[2]).href)
const context = (parentSession, name) => ({
  sessionManager: {
    getHeader: () => ({ parentSession }),
    getSessionName: () => name,
  },
})
const child = context("parent.jsonl", "worker#76869bf6")
assert.equal(extension.isSubagentSession(child), true)
assert.equal(extension.isSubagentSession(context("parent.jsonl", "delegate#0ABEDA48")), true)
assert.equal(extension.isSubagentSession(context("parent.jsonl", "forked investigation")), false)
assert.equal(extension.isSubagentSession(context(undefined, "worker#76869bf6")), false)

const handlers = new Map()
extension.default({
  on: (name, callback) => handlers.set(name, callback),
  registerEntryRenderer: () => {},
})
assert.equal(handlers.size, 7)
const event = new Proxy({}, { get() { throw new Error("ignored event was inspected") } })
for (const callback of handlers.values()) await callback(event, child)
assert.equal(reports.length, 0)
const parent = context(undefined, "parent session")
const emit = (name, event = {}) => handlers.get(name)(event, parent)
const takeReports = () => reports.splice(0).map(({ event, message }) => ({ event, message }))

await emit("turn_start")
assert.deepEqual(takeReports(), [{ event: "working", message: "" }])
await emit("message_end", { message: { role: "assistant", content: "unfinished response" } })
// Pie's agent_end has no willRetry: it must wait for the final settle event.
await emit("agent_end", { messages: [] })
assert.deepEqual(takeReports(), [])
await emit("agent_settled", { aborted: true })
await emit("agent_settled", { aborted: true })
await emit("agent_settled", { aborted: false })
assert.deepEqual(takeReports(), [{ event: "idle", message: "" }])

// A cancelled retry may leave an error message. Cancellation wins and clears it.
await emit("agent_start")
await emit("message_end", { message: {
  role: "assistant", stopReason: "error", errorMessage: "retry failed", content: "partial answer",
} })
await emit("agent_settled", { aborted: true, error: "cancelled" })
assert.deepEqual(takeReports(), [
  { event: "working", message: "" }, { event: "idle", message: "" },
])

// The next run must not inherit the cancelled response or error.
await emit("agent_start")
await emit("agent_settled", { aborted: false })
await emit("agent_settled", { aborted: false })
assert.deepEqual(takeReports(), [
  { event: "working", message: "" }, { event: "done", message: "任务已完成" },
])
await emit("agent_start")
await emit("message_end", { message: { role: "assistant", content: "finished response" } })
await emit("agent_settled", { aborted: false })
assert.deepEqual(takeReports(), [
  { event: "working", message: "" }, { event: "done", message: "finished response" },
])
await emit("agent_start")
await emit("message_end", { message: {
  role: "assistant", stopReason: "error", errorMessage: "real error",
} })
await emit("agent_settled", { aborted: false })
assert.deepEqual(takeReports(), [
  { event: "working", message: "" }, { event: "failed", message: "任务异常: real error" },
])

// 长驻后台任务（dev server）：settled 永远不来，agent_end 防抖到期必须收掉 working。
await emit("agent_start")
await emit("message_end", { message: { role: "assistant", content: "server up" } })
await emit("agent_end", { messages: [] })
assert.deepEqual(takeReports(), [{ event: "working", message: "" }])
await flushTimers()
assert.deepEqual(takeReports(), [{ event: "done", message: "server up" }])

// 重试在防抖窗口内跟上：取消完成上报，最终由 settled 收尾。
await emit("agent_start")
await emit("agent_end", { messages: [] })
await emit("agent_start")
await flushTimers()
assert.deepEqual(takeReports(), [
  { event: "working", message: "" }, { event: "working", message: "" },
])
await emit("agent_settled", { aborted: false })
assert.deepEqual(takeReports(), [{ event: "done", message: "任务已完成" }])

// ask_user 待回答期间 agent_end：保持 blocked，不被防抖翻成 done。
await emit("agent_start")
await emit("tool_execution_start", {
  toolName: "ask_user", toolCallId: "q1", args: { question: "继续吗" },
})
await emit("agent_end", { messages: [] })
await flushTimers()
assert.deepEqual(takeReports().map((item) => item.event), ["working", "blocked"])
"#,
        )
        .unwrap();
        let output = std::process::Command::new("node")
            .arg(&runner)
            .arg(&extension)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "node behavior test failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn generated_pi_extension_reports_session_metadata_and_preview() {
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let extension = dir.path().join("muxlane.mjs");
        let runner = dir.path().join("runner.mjs");
        std::fs::write(&extension, PI_EXTENSION).unwrap();
        std::fs::write(
            &runner,
            r#"import assert from "node:assert/strict"
import net from "node:net"
import { pathToFileURL } from "node:url"

const reports = []
net.createConnection = () => {
  const handlers = {}
  const client = {
    setTimeout() {},
    end(data) { reports.push(JSON.parse(data).params) },
    destroy() {},
    on(name, callback) {
      handlers[name] = callback
      if (name === "close") queueMicrotask(() => {
        handlers.connect?.()
        handlers.close?.()
      })
      return client
    },
  }
  return client
}
process.env.MUXLANE_SOCKET = "/tmp/muxlane-hook-meta-test.sock"
process.env.MUXLANE_AGENT_ID = "pi_1"
process.env.MUXLANE_HOOK_TOKEN = "token"

const extension = await import(pathToFileURL(process.argv[2]).href)
const handlers = new Map()
extension.default({
  on: (name, callback) => handlers.set(name, callback),
  registerEntryRenderer: () => {},
})

const ctx = {
  model: { id: "claude-sonnet-4" },
  sessionManager: {
    getHeader: () => ({ id: "sess-123" }),
    getSessionId: () => "sess-123",
    getSessionFile: () => "/tmp/sessions/sess-123.jsonl",
    getSessionName: () => "main",
  },
}

await handlers.get("session_start")({}, ctx)
assert.equal(reports.length, 1)
assert.equal(reports[0].event, "meta")
assert.equal(reports[0].session_id, "sess-123")
assert.equal(reports[0].session_file, "/tmp/sessions/sess-123.jsonl")
assert.equal(reports[0].model, "claude-sonnet-4")

reports.length = 0
await handlers.get("message_end")({ message: { role: "assistant", content: "Streaming answer body" } }, ctx)
assert.equal(reports.length, 1)
assert.equal(reports[0].event, "preview")
assert.equal(reports[0].session_file, "/tmp/sessions/sess-123.jsonl")
assert.equal(reports[0].streaming_preview, "Streaming answer body")
"#,
        )
        .unwrap();
        let output = std::process::Command::new("node")
            .arg(&runner)
            .arg(&extension)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "node metadata test failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn installs_opencode_and_pi_plugins_idempotently() {
        let dir = tempfile::tempdir().unwrap();
        install_agent_plugins(dir.path()).unwrap();
        install_agent_plugins(dir.path()).unwrap();
        let opencode =
            std::fs::read_to_string(dir.path().join(".config/opencode/plugins/muxlane.ts"))
                .unwrap();
        let pi =
            std::fs::read_to_string(dir.path().join(".pi/agent/extensions/muxlane.ts")).unwrap();
        let pie = std::fs::read_to_string(dir.path().join(".pie/extensions/muxlane.ts")).unwrap();
        let pie_legacy =
            std::fs::read_to_string(dir.path().join(".pie/agent/extensions/muxlane.ts")).unwrap();
        assert!(opencode.contains("client.session.messages"));
        assert_eq!(pi, pie);
        assert_eq!(pi, pie_legacy);
        assert!(pi.contains("willRetry === false"));
        assert!(pi.contains("SETTLE_DEBOUNCE_MS"));
        assert!(pi.contains("agent_settled"));
        assert!(pi.contains("assistantText"));
        assert!(pi.contains("isLegacySubagentProcess"));
        assert!(pi.contains("--no-session"));
        assert!(pi.contains("isSubagentSession"));
        assert!(pi.contains("getHeader"));
        assert!(pi.contains("getSessionName"));
        assert!(pi.contains("header?.parentSession"));
        assert!(pi.contains("/#[0-9a-f]{8}$/i"));
        assert_eq!(pi.matches("if (shouldIgnoreRun(ctx)) return").count(), 7);
    }

    #[test]
    fn legacy_lmux_plugins_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let old_opencode = dir.path().join(".config/opencode/plugins/lmux.ts");
        let old_pi = dir.path().join(".pi/agent/extensions/lmux.ts");
        std::fs::create_dir_all(old_opencode.parent().unwrap()).unwrap();
        std::fs::create_dir_all(old_pi.parent().unwrap()).unwrap();
        std::fs::write(&old_opencode, "old").unwrap();
        std::fs::write(&old_pi, "old").unwrap();
        install_agent_plugins(dir.path()).unwrap();
        assert!(!old_opencode.exists());
        assert!(!old_pi.exists());
    }
}
