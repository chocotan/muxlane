import net from "node:net"
import { execFileSync } from "node:child_process"

function muxlaneEnv(name = "") {
  if (process.env[name]) return process.env[name]
  if (!process.env.TMUX || !process.env.TMUX_PANE) return undefined
  try {
    const line = execFileSync("tmux", ["show-environment", "-t", process.env.TMUX_PANE, name], { encoding: "utf8" }).trim()
    return line.startsWith(name + "=") ? line.slice(name.length + 1) : undefined
  } catch { return undefined }
}

function report(event = "", message = "", meta = null) {
  if ((typeof isLegacySubagentProcess !== "undefined" && isLegacySubagentProcess)
    || (typeof isSubagentProcess !== "undefined" && isSubagentProcess)) return Promise.resolve()
  const socket = muxlaneEnv("MUXLANE_SOCKET")
  const agent = muxlaneEnv("MUXLANE_AGENT_ID")
  const token = muxlaneEnv("MUXLANE_HOOK_TOKEN")
  if (!socket || !agent || !token) return Promise.resolve()
  const params = { token, agent, event, message }
  if (meta && typeof meta === "object") {
    if (typeof meta.session_id === "string" && meta.session_id) params.session_id = meta.session_id
    if (typeof meta.session_file === "string" && meta.session_file) params.session_file = meta.session_file
    if (typeof meta.model === "string" && meta.model) params.model = meta.model
    if (typeof meta.streaming_preview === "string" && meta.streaming_preview) {
      params.streaming_preview = meta.streaming_preview
    }
  }
  return new Promise((resolve) => {
    const client = net.createConnection(socket)
    client.setTimeout(1200)
    client.on("connect", () => client.end(JSON.stringify({
      id: Date.now(), method: "agent.report",
      params
    }) + "\n"))
    client.on("error", () => resolve(undefined))
    client.on("timeout", () => { client.destroy(); resolve(undefined) })
    client.on("close", () => resolve(undefined))
  })
}
