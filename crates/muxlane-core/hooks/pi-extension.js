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

// agent_end 之后可能紧跟重试/恢复/压缩/排队的新 run，不能立刻报完成；
// 但长驻后台任务（dev server 等）挂着时 agent_settled 永远不来，working 会悬死。
// 折中：agent_end 后短防抖，没有新 run 跟上就按完成上报。
const SETTLE_DEBOUNCE_MS = 2500

export function isSubagentSession(ctx) {
  const header = ctx?.sessionManager?.getHeader?.()
  const name = ctx?.sessionManager?.getSessionName?.() || ""
  return Boolean(header?.parentSession) && /#[0-9a-f]{8}$/i.test(name)
}

export function shouldIgnoreRun(ctx) {
  return isLegacySubagentProcess || isSubagentSession(ctx)
}

export default function (pi) {
  if (process.argv.includes("--mode") && process.argv.includes("rpc")) return
  registerInlineImages(pi)
  let latestAssistant = ""
  // 最近一条 assistant 错误消息（stopReason === "error"）。agent_end/agent_settled
  // 事件本身不带 error 字段，只能靠消息流判断最终是不是失败。
  let lastError = ""
  const seenAskUserCalls = new Set()
  let doneReported = false
  let settleTimer = null
  // ask_user 待回答期间 run 会挂起：此时 agent_end 不应被防抖翻成 done。
  let blockedPending = false
  let lastKnownMeta = {}

  const collectSessionMeta = (ctx) => {
    const sm = ctx?.sessionManager
    const header = sm?.getHeader?.()
    const sessionId = (typeof sm?.getSessionId === "function" ? sm.getSessionId() : "")
      || (typeof header?.id === "string" ? header.id : "")
      || lastKnownMeta.session_id
      || ""
    const sessionFile = (typeof sm?.getSessionFile === "function" ? sm.getSessionFile() : "")
      || lastKnownMeta.session_file
      || ""
    const modelObj = ctx?.model
    const model = (typeof modelObj === "string" ? modelObj : (modelObj?.id || modelObj?.name || ""))
      || lastKnownMeta.model
      || ""
    const next = {}
    if (sessionId) next.session_id = sessionId
    if (sessionFile) next.session_file = sessionFile
    if (model) next.model = model
    if (sessionId || sessionFile || model) lastKnownMeta = next
    return next
  }

  const cancelSettle = () => {
    if (settleTimer) {
      clearTimeout(settleTimer)
      settleTimer = null
    }
  }

  const finishRun = async (event, ctx) => {
    doneReported = true
    const meta = collectSessionMeta(ctx)
    if (event?.error || ctx?.error) {
      const err = shortText(event?.error || ctx?.error || "执行出错")
      await report("failed", `任务异常: ${err}`, meta)
    } else if (lastError) {
      await report("failed", `任务异常: ${lastError}`, meta)
      lastError = ""
    } else {
      await report("done", latestAssistant || "任务已完成", meta)
    }
    latestAssistant = ""
  }

  pi.on("session_start", async (_event, ctx) => {
    if (shouldIgnoreRun(ctx)) return
    latestAssistant = ""
    lastError = ""
    doneReported = false
    seenAskUserCalls.clear()
    cancelSettle()
    blockedPending = false
    lastKnownMeta = {}
    const meta = collectSessionMeta(ctx)
    if (meta.session_file || meta.session_id) {
      await report("meta", "", meta)
    }
  })
  pi.on("turn_start", async (_event, ctx) => {
    if (shouldIgnoreRun(ctx)) return
    cancelSettle()
    doneReported = false
    await report("working", "", collectSessionMeta(ctx))
  })
  pi.on("agent_start", async (_event, ctx) => {
    if (shouldIgnoreRun(ctx)) return
    cancelSettle()
    doneReported = false
    blockedPending = false
    await report("working", "", collectSessionMeta(ctx))
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
      blockedPending = true
      await report("blocked", question, collectSessionMeta(ctx))
    }
  })

  pi.on("message_end", async (event, ctx) => {
    if (shouldIgnoreRun(ctx)) return
    const message = event?.message
    if (message?.role === "assistant") {
      if (message.stopReason === "error") {
        lastError = shortText(message.errorMessage || "执行出错")
      } else {
        lastError = ""
      }
      const text = typeof message.content === "string" ? message.content : assistantText([message])
      if (text) {
        latestAssistant = shortText(text, 180)
        const meta = collectSessionMeta(ctx)
        if (meta.session_file || meta.session_id) {
          await report("preview", "", { ...meta, streaming_preview: text.slice(0, 4000) })
        }
      }
    }
  })

  pi.on("agent_end", async (event, ctx) => {
    if (shouldIgnoreRun(ctx)) return
    const text = assistantText(event?.messages || [])
    if (text) latestAssistant = shortText(text, 180)
    if (doneReported) return
    // 旧版 Pi 用 willRetry===false 标记最终结束。
    if (event?.willRetry === false) {
      cancelSettle()
      await finishRun(event, ctx)
      return
    }
    // Pie 的 agent_end 不带 willRetry：短防抖，重试/压缩/排队的新 run 会取消它。
    cancelSettle()
    settleTimer = setTimeout(() => {
      settleTimer = null
      // 待用户回答时保持 blocked；其余情况视为完成（后台长驻任务不阻塞状态）。
      if (!doneReported && !blockedPending) finishRun({}, null)
    }, SETTLE_DEBOUNCE_MS)
  })

  // 新版 Pi 在所有重试、压缩和排队消息结束后触发；旧版没有此事件。
  pi.on("agent_settled", async (event, ctx) => {
    if (shouldIgnoreRun(ctx)) return
    cancelSettle()
    if (doneReported) return
    doneReported = true
    // 用户取消也要收掉 working，但不能生成完成/失败通知。
    if (event?.aborted) {
      latestAssistant = ""
      lastError = ""
      await report("idle", "", collectSessionMeta(ctx))
      return
    }
    await finishRun(event, ctx)
  })
}
