package muxlane.protocol

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.encodeToJsonElement
import java.util.Base64

object Methods {
    const val STATE_LIST = "state.list"
    const val SYSTEM_HELLO = "system.hello"
    const val TERM_SUBSCRIBE = "term.subscribe"
    const val TERM_UNSUBSCRIBE = "term.unsubscribe"
    const val TERM_INPUT = "term.input"
    const val TERM_RESIZE = "term.resize"
    const val EVENTS_SUBSCRIBE = "events.subscribe"
    const val AGENT_SPAWN = "agent.spawn"
    const val AGENT_DELETE = "agent.delete"
    const val AGENT_MARK_SEEN = "agent.mark_seen"
    const val PAIR_BEGIN = "pair.begin"
    const val PRESET_LIST = "preset.list"
}

object Events {
    const val STATE_CHANGED = "state.changed"
    const val AGENT_STATUS = "agent.status_changed"
    const val TERM_DATA = "term.data"
    const val TERM_RESYNC = "term.resync"
    const val TERM_REPLAY_CHUNK = "term.replay_chunk"
    const val TERM_EXIT = "term.exit"
}

val muxlaneJson = Json {
    ignoreUnknownKeys = true
    encodeDefaults = true
    explicitNulls = false
}

@Serializable
data class Request(
    val id: Long,
    val method: String,
    val params: JsonElement = JsonNull,
)

@Serializable
data class RpcError(
    val code: String,
    val message: String,
    val method: String? = null,
)

@Serializable
data class Response(
    val id: Long,
    val result: JsonElement? = null,
    val error: RpcError? = null,
)

@Serializable
data class EventMsg(
    val event: String,
    val params: JsonElement = JsonNull,
)

@Serializable
data class PairBeginParams(
    @SerialName("host_id") val hostId: String? = null,
    val token: String? = null,
    val device: String? = null,
)

@Serializable
data class PairBeginResult(
    val token: String,
    val machine: MachineInfo,
)

@Serializable
data class MachineInfo(
    @SerialName("machine_id") val machineId: String,
    val name: String,
    val os: String,
    val version: String,
)

@Serializable
data class Project(
    val id: String,
    val name: String,
    val path: String,
    val branch: String? = null,
    val agents: List<String> = emptyList(),
)

@Serializable
enum class AgentType {
    @SerialName("claude") CLAUDE,
    @SerialName("codex") CODEX,
    @SerialName("opencode") OPENCODE,
    @SerialName("pi") PI,
    @SerialName("gemini") GEMINI,
    @SerialName("agy") AGY,
    @SerialName("qwen") QWEN,
    @SerialName("kimi") KIMI,
    @SerialName("shell") SHELL,
    @SerialName("unknown") UNKNOWN,
}

@Serializable
enum class AgentStatus {
    @SerialName("working") WORKING,
    @SerialName("blocked") BLOCKED,
    @SerialName("idle") IDLE,
    @SerialName("done") DONE,
    @SerialName("failed") FAILED,
    @SerialName("unknown") UNKNOWN,
}

fun AgentStatus.isAlert(): Boolean = this == AgentStatus.BLOCKED ||
    this == AgentStatus.DONE ||
    this == AgentStatus.FAILED

@Serializable
data class AgentInstance(
    val id: String,
    val project: String,
    @SerialName("agent_type") val agentType: AgentType = AgentType.UNKNOWN,
    val title: String,
    val status: AgentStatus = AgentStatus.IDLE,
    @SerialName("status_since") val statusSince: Long = 0,
    val seen: Boolean = true,
    @SerialName("tmux_session") val tmuxSession: String? = null,
)

@Serializable
data class Snapshot(
    val machine: MachineInfo? = null,
    val projects: List<Project> = emptyList(),
    val agents: List<AgentInstance> = emptyList(),
)

@Serializable
data class AgentPreset(
    val id: String,
    val label: String,
    @SerialName("agent_type") val agentType: AgentType,
    val program: String,
    val args: List<String> = emptyList(),
)

@Serializable
data class TermSubscribeParams(
    val agent: String,
    @SerialName("accept_replay_chunks") val acceptReplayChunks: Boolean = true,
)

@Serializable
data class TermSubscribeResult(
    @SerialName("sub_id") val subId: String,
    @SerialName("replay_b64") val replayB64: String = "",
)

@Serializable
data class TermInputParams(
    val agent: String,
    @SerialName("data_b64") val dataB64: String,
)

@Serializable
data class TermResizeParams(
    val agent: String,
    val cols: Int,
    val rows: Int,
)

@Serializable
data class TermDataEvent(
    val agent: String,
    @SerialName("data_b64") val dataB64: String,
)

@Serializable
data class TermReplayChunkEvent(
    val agent: String,
    @SerialName("sub_id") val subId: String,
    @SerialName("replay_id") val replayId: Long,
    @SerialName("chunk_index") val chunkIndex: Int,
    @SerialName("data_b64") val dataB64: String,
)

@Serializable
data class AgentStatusEvent(
    val agent: String,
    @SerialName("agent_type") val agentType: AgentType? = null,
    val from: AgentStatus,
    val to: AgentStatus,
    val message: String? = null,
)

@Serializable
data class AgentSpawnParams(
    val project: String,
    @SerialName("agent_type") val agentType: AgentType? = null,
    val program: String? = null,
    val args: List<String>? = null,
    @SerialName("preset_name") val presetName: String? = null,
)

fun encodeFrame(value: Any): String = when (value) {
    is Request -> muxlaneJson.encodeToString(Request.serializer(), value)
    is Response -> muxlaneJson.encodeToString(Response.serializer(), value)
    is EventMsg -> muxlaneJson.encodeToString(EventMsg.serializer(), value)
    else -> error("unsupported frame")
}

fun decodeIncoming(line: String): IncomingFrame {
    val trimmed = line.trim()
    val element = muxlaneJson.parseToJsonElement(trimmed)
    val obj = element as? kotlinx.serialization.json.JsonObject
        ?: error("frame is not an object")
    return if (obj.containsKey("event")) {
        IncomingFrame.Event(muxlaneJson.decodeFromJsonElement(EventMsg.serializer(), element))
    } else {
        IncomingFrame.Response(muxlaneJson.decodeFromJsonElement(Response.serializer(), element))
    }
}

sealed class IncomingFrame {
    data class Response(val value: muxlane.protocol.Response) : IncomingFrame()
    data class Event(val value: EventMsg) : IncomingFrame()
}

fun b64Encode(bytes: ByteArray): String = Base64.getEncoder().encodeToString(bytes)

fun b64Decode(value: String): ByteArray = if (value.isEmpty()) ByteArray(0) else Base64.getDecoder().decode(value)

fun <T> encodeParams(serializer: kotlinx.serialization.SerializationStrategy<T>, value: T): JsonElement =
    muxlaneJson.encodeToJsonElement(serializer, value)
