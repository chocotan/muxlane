function assistantText(messages) {
  for (let index = (messages || []).length - 1; index >= 0; index--) {
    const message = messages[index]
    if (message?.role !== "assistant") continue
    if (typeof message.content === "string" && message.content.trim()) return message.content.trim()
    if (Array.isArray(message.content)) {
      const text = message.content
        .filter((part) => part?.type === "text" && typeof part.text === "string")
        .map((part) => part.text)
        .join("\n")
        .trim()
      if (text) return text
    }
  }
  return ""
}

function shortText(value, max = 160) {
  if (typeof value !== "string") return ""
  const t = value.replace(/\s+/g, " ").trim()
  return t.length <= max ? t : t.slice(0, max - 1) + "…"
}

// Legacy subprocess agents use JSON mode. Native Agent tasks use a named child session.
const isLegacySubagentProcess = process.argv.includes("--mode")
  && process.argv.includes("json")
  && process.argv.includes("--no-session")

export function isSubagentSession(ctx) {
  const header = ctx?.sessionManager?.getHeader?.()
  const name = ctx?.sessionManager?.getSessionName?.() || ""
  return Boolean(header?.parentSession) && /#[0-9a-f]{8}$/i.test(name)
}

export function shouldIgnoreRun(ctx) {
  return isLegacySubagentProcess || isSubagentSession(ctx)
}

export default function (pi) {
  registerInlineImages(pi)
  let latestAssistant = ""
  const seenAskUserCalls = new Set()
  let doneReported = false

  pi.on("session_start", async (_event, ctx) => {
    if (shouldIgnoreRun(ctx)) return
    latestAssistant = ""
    doneReported = false
    seenAskUserCalls.clear()
  })
  pi.on("turn_start", async (_event, ctx) => {
    if (shouldIgnoreRun(ctx)) return
    doneReported = false
    await report("working", "")
  })
  pi.on("agent_start", async (_event, ctx) => {
    if (shouldIgnoreRun(ctx)) return
    doneReported = false
    await report("working", "")
  })

  // Only the parent Pi run owns muxlane state. Subagent lifecycle events stay local.
  pi.on("tool_execution_start", async (event, ctx) => {
    if (shouldIgnoreRun(ctx)) return
    const toolName = event?.toolName || ""
    if (toolName === "ask_user" || toolName === "confirm" || toolName === "prompt_user" || toolName === "user_input") {
      const callId = event?.toolCallId || String(Date.now())
      if (seenAskUserCalls.has(callId)) return
      seenAskUserCalls.add(callId)
      const args = event?.args || {}
      const question = shortText(args.question || args.message || args.prompt || "等待用户确认")
      await report("blocked", question)
    }
  })

  pi.on("message_end", (event, ctx) => {
    if (shouldIgnoreRun(ctx)) return
    const message = event?.message
    if (message?.role === "assistant") {
      const text = typeof message.content === "string" ? message.content : assistantText([message])
      if (text) latestAssistant = shortText(text, 180)
    }
  })

  pi.on("agent_end", async (event, ctx) => {
    if (shouldIgnoreRun(ctx)) return
    const text = assistantText(event?.messages || [])
    if (text) latestAssistant = shortText(text, 180)
    // Pi < 0.80.4 没有 agent_settled；旧版在 agent_end 且没有重试时完成上报。
    if (doneReported || (ctx?.isIdle && !ctx.isIdle())) return
    doneReported = true
    let msg = latestAssistant
    if (event?.error || ctx?.error) {
      const err = shortText(event?.error || ctx?.error || "执行出错")
      await report("failed", `任务异常: ${err}`)
    } else {
      await report("done", msg || "任务已完成")
    }
    latestAssistant = ""
  })

  // 新版 Pi 在所有重试、压缩和排队消息结束后触发；旧版没有此事件。
  pi.on("agent_settled", async (event, ctx) => {
    if (shouldIgnoreRun(ctx)) return
    if (doneReported || (ctx?.isIdle && !ctx.isIdle())) return
    doneReported = true
    let msg = latestAssistant
    if (event?.error || ctx?.error) {
      const err = shortText(event?.error || ctx?.error || "执行出错")
      await report("failed", `任务异常: ${err}`)
    } else {
      await report("done", msg || "任务已完成")
    }
    latestAssistant = ""
  })
}
