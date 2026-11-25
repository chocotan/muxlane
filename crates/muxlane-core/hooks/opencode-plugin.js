const isSubagentProcess = false

function openCodeAssistantText(messages: any[]) {
  for (let index = (messages || []).length - 1; index >= 0; index--) {
    const item = messages[index]
    const info = item?.info || item?.message || item
    if (info?.role !== "assistant") continue
    const parts = item?.parts || info?.parts || []
    const text = parts
      .filter((part: any) => part?.type === "text" && typeof part.text === "string")
      .map((part: any) => part.text)
      .join("\n")
      .trim()
    if (text) return text
  }
  return ""
}

export const Muxlane = async ({ client }: any) => ({
  event: async ({ event }: any) => {
    const sessionID = event?.properties?.sessionID || event?.sessionID
    const meta = sessionID ? { session_id: String(sessionID) } : null
    if (event.type === "session.idle") {
      let message = ""
      if (sessionID && client?.session?.messages) {
        try {
          const response = await client.session.messages({ path: { id: sessionID } })
          message = openCodeAssistantText(response?.data || response || [])
        } catch {}
      }
      await report("done", message || "任务已完成", message ? { ...(meta || {}), streaming_preview: message.slice(0, 4000) } : meta)
    } else if (event.type === "session.error") {
      const err = event?.error?.message || event?.error || "执行出错"
      await report("failed", `任务异常: ${err}`, meta)
    } else if (event.type === "permission.ask" || event.type === "tool.confirm" || event.type === "prompt.ask") {
      const question = event?.properties?.question || event?.properties?.message || "等待用户确认授权"
      await report("blocked", question, meta)
    } else if (event.type === "subagent.completed") {
      const summary = event?.properties?.summary || "Subagent 任务已完成"
      await report("working", summary, meta)
    } else if (event.type === "subagent.failed" || event.type === "subagent.error") {
      const err = event?.properties?.error || "Subagent 执行异常"
      await report("failed", `Subagent 异常: ${err}`, meta)
    } else if (event.type === "session.prompt" || event.type === "message.created" || event.type === "tool.call") {
      await report("working", "", meta)
    }
  }
})
