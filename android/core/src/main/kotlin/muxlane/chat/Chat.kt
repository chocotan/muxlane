@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package muxlane.chat

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonClassDiscriminator
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject

@Serializable
enum class SessionDriverMode {
    @SerialName("terminal_pty") TERMINAL_PTY,
    @SerialName("pi_rpc") PI_RPC,
    @SerialName("acp") ACP,
}

@Serializable
enum class SessionViewMode {
    @SerialName("terminal") TERMINAL,
    @SerialName("chat") CHAT,
    @SerialName("split") SPLIT,
}

@Serializable
enum class ChatDriverKind {
    @SerialName("pi_rpc") PI_RPC,
    @SerialName("acp") ACP,
}

@Serializable
enum class StreamingBehavior {
    @SerialName("steer") STEER,
    @SerialName("follow_up") FOLLOW_UP,
}

@Serializable
data class ChatAttachment(
    val name: String,
    val path: String? = null,
    @SerialName("mime_type") val mimeType: String,
    @SerialName("data_b64") val dataB64: String? = null,
)

@Serializable
enum class ToolCallKind {
    @SerialName("read") READ,
    @SerialName("edit") EDIT,
    @SerialName("write") WRITE,
    @SerialName("bash") BASH,
    @SerialName("search") SEARCH,
    @SerialName("fetch") FETCH,
    @SerialName("think") THINK,
    @SerialName("other") OTHER,
}

@Serializable
enum class ToolCallState {
    @SerialName("pending") PENDING,
    @SerialName("running") RUNNING,
    @SerialName("completed") COMPLETED,
    @SerialName("failed") FAILED,
}

@Serializable
data class ToolDiff(
    val path: String? = null,
    @SerialName("old_text") val oldText: String = "",
    @SerialName("new_text") val newText: String = "",
)

@Serializable
enum class NoticeLevel {
    @SerialName("info") INFO,
    @SerialName("warning") WARNING,
    @SerialName("error") ERROR,
    @SerialName("compaction") COMPACTION,
}

@Serializable
enum class PlanEntryStatus {
    @SerialName("pending") PENDING,
    @SerialName("in_progress") IN_PROGRESS,
    @SerialName("completed") COMPLETED,
}

@Serializable
data class PlanEntry(
    val content: String,
    val status: PlanEntryStatus = PlanEntryStatus.PENDING,
    val priority: String? = null,
)

@Serializable
@JsonClassDiscriminator("type")
sealed class TimelineEntry {
    abstract val id: String

    @Serializable
    @SerialName("user_message")
    data class UserMessage(
        override val id: String,
        @SerialName("turn_id") val turnId: String? = null,
        val text: String,
        val attachments: List<ChatAttachment> = emptyList(),
        @SerialName("created_at_ms") val createdAtMs: Long = 0,
    ) : TimelineEntry()

    @Serializable
    @SerialName("thinking")
    data class Thinking(
        override val id: String,
        @SerialName("turn_id") val turnId: String? = null,
        val text: String,
        val streaming: Boolean = false,
        @SerialName("duration_ms") val durationMs: Long? = null,
        @SerialName("created_at_ms") val createdAtMs: Long = 0,
    ) : TimelineEntry()

    @Serializable
    @SerialName("assistant_text")
    data class AssistantText(
        override val id: String,
        @SerialName("turn_id") val turnId: String? = null,
        val text: String,
        val streaming: Boolean = false,
        val model: String? = null,
        @SerialName("created_at_ms") val createdAtMs: Long = 0,
    ) : TimelineEntry()

    @Serializable
    @SerialName("tool_call")
    data class ToolCall(
        override val id: String,
        @SerialName("turn_id") val turnId: String? = null,
        @SerialName("call_id") val callId: String,
        val name: String,
        val kind: ToolCallKind = ToolCallKind.OTHER,
        val title: String? = null,
        val input: JsonElement = JsonObject(emptyMap()),
        val output: String? = null,
        val diff: ToolDiff? = null,
        @SerialName("exit_code") val exitCode: Int? = null,
        val locations: List<String> = emptyList(),
        val state: ToolCallState = ToolCallState.PENDING,
        @SerialName("is_error") val isError: Boolean = false,
        @SerialName("duration_ms") val durationMs: Long? = null,
        @SerialName("created_at_ms") val createdAtMs: Long = 0,
    ) : TimelineEntry()

    @Serializable
    @SerialName("plan")
    data class Plan(
        override val id: String,
        @SerialName("turn_id") val turnId: String? = null,
        val entries: List<PlanEntry> = emptyList(),
        @SerialName("created_at_ms") val createdAtMs: Long = 0,
    ) : TimelineEntry()

    @Serializable
    @SerialName("notice")
    data class Notice(
        override val id: String,
        @SerialName("turn_id") val turnId: String? = null,
        val level: NoticeLevel,
        val text: String,
        @SerialName("created_at_ms") val createdAtMs: Long = 0,
    ) : TimelineEntry()
}

@Serializable
enum class ApprovalSource {
    @SerialName("acp_permission") ACP_PERMISSION,
    @SerialName("pi_extension_ui") PI_EXTENSION_UI,
}

@Serializable
data class ApprovalOption(
    @SerialName("option_id") val optionId: String,
    val label: String,
    val kind: String? = null,
)

@Serializable
data class PendingApproval(
    val id: String,
    val agent: String,
    val source: ApprovalSource,
    val method: String,
    val title: String,
    val message: String? = null,
    @SerialName("tool_name") val toolName: String? = null,
    @SerialName("command_or_input") val commandOrInput: String? = null,
    val options: List<ApprovalOption> = emptyList(),
    val prefill: String? = null,
    @SerialName("created_at_ms") val createdAtMs: Long = 0,
)

@Serializable
@JsonClassDiscriminator("kind")
sealed class ApprovalReply {
    @Serializable
    @SerialName("selected_option")
    data class SelectedOption(@SerialName("option_id") val optionId: String) : ApprovalReply()

    @Serializable
    @SerialName("confirm")
    data class Confirm(val confirmed: Boolean) : ApprovalReply()

    @Serializable
    @SerialName("text")
    data class Text(val value: String) : ApprovalReply()

    @Serializable
    @SerialName("cancel")
    data object Cancel : ApprovalReply()
}

@Serializable
data class ChatModelInfo(
    val provider: String = "",
    val id: String,
    val name: String,
)

@Serializable
data class ChatSessionMeta(
    val driver: SessionDriverMode = SessionDriverMode.TERMINAL_PTY,
    @SerialName("supports_chat_input") val supportsChatInput: Boolean = false,
    @SerialName("supports_terminal_switch") val supportsTerminalSwitch: Boolean = false,
    @SerialName("has_pty") val hasPty: Boolean = false,
    @SerialName("provider_session_id") val providerSessionId: String? = null,
    @SerialName("provider_session_file") val providerSessionFile: String? = null,
    @SerialName("current_model") val currentModel: ChatModelInfo? = null,
    @SerialName("available_models") val availableModels: List<ChatModelInfo> = emptyList(),
    @SerialName("thinking_level") val thinkingLevel: String? = null,
    @SerialName("available_thinking_levels") val availableThinkingLevels: List<String> = emptyList(),
    @SerialName("is_streaming") val isStreaming: Boolean = false,
    @SerialName("context_used_tokens") val contextUsedTokens: Long? = null,
    @SerialName("context_window_tokens") val contextWindowTokens: Long? = null,
    @SerialName("total_cost_usd") val totalCostUsd: Double? = null,
    @SerialName("last_error") val lastError: String? = null,
)

@Serializable
enum class TextDeltaKind {
    @SerialName("assistant") ASSISTANT,
    @SerialName("thinking") THINKING,
    @SerialName("tool_output") TOOL_OUTPUT,
}

@Serializable
@JsonClassDiscriminator("op")
sealed class ChatDeltaOp {
    @Serializable
    @SerialName("upsert_entry")
    data class UpsertEntry(val entry: TimelineEntry) : ChatDeltaOp()

    @Serializable
    @SerialName("append_text")
    data class AppendText(
        @SerialName("entry_id") val entryId: String,
        val delta: String,
        val kind: TextDeltaKind,
    ) : ChatDeltaOp()

    @Serializable
    @SerialName("replace_entry_text")
    data class ReplaceEntryText(
        @SerialName("entry_id") val entryId: String,
        val text: String,
        val kind: TextDeltaKind,
        val finish: Boolean = false,
    ) : ChatDeltaOp()

    @Serializable
    @SerialName("finish_entry")
    data class FinishEntry(
        @SerialName("entry_id") val entryId: String,
        @SerialName("duration_ms") val durationMs: Long? = null,
    ) : ChatDeltaOp()

    @Serializable
    @SerialName("meta_updated")
    data class MetaUpdated(val meta: ChatSessionMeta) : ChatDeltaOp()

    @Serializable
    @SerialName("approval_requested")
    data class ApprovalRequested(val approval: PendingApproval) : ChatDeltaOp()

    @Serializable
    @SerialName("approval_resolved")
    data class ApprovalResolved(@SerialName("approval_id") val approvalId: String) : ChatDeltaOp()

    @Serializable
    @SerialName("turn_finished")
    data class TurnFinished(
        @SerialName("turn_id") val turnId: String? = null,
        val error: String? = null,
    ) : ChatDeltaOp()

    @Serializable
    @SerialName("reset")
    data object Reset : ChatDeltaOp()
}

@Serializable
data class ChatDeltaEvent(
    val agent: String,
    val seq: Long,
    val delta: ChatDeltaOp,
)

@Serializable
data class ChatHistoryResult(
    val agent: String,
    val seq: Long,
    val meta: ChatSessionMeta = ChatSessionMeta(),
    val entries: List<TimelineEntry> = emptyList(),
    @SerialName("pending_approvals") val pendingApprovals: List<PendingApproval> = emptyList(),
)

@Serializable
data class ChatSubscribeParams(
    val agent: String,
    @SerialName("from_seq") val fromSeq: Long? = null,
)

@Serializable
data class ChatSubscribeResult(
    @SerialName("sub_id") val subId: String,
    val history: ChatHistoryResult,
)

@Serializable
data class ChatSpawnParams(
    val project: String,
    @SerialName("agent_type") val agentType: muxlane.protocol.AgentType? = null,
    val program: String? = null,
    val args: List<String>? = null,
    @SerialName("preset_name") val presetName: String? = null,
    val driver: ChatDriverKind? = null,
    @SerialName("provider_session") val providerSession: String? = null,
    @SerialName("initial_prompt") val initialPrompt: String? = null,
    val model: String? = null,
    @SerialName("thinking_level") val thinkingLevel: String? = null,
)

@Serializable
data class ChatSendParams(
    val agent: String,
    val text: String,
    val attachments: List<ChatAttachment> = emptyList(),
    @SerialName("streaming_behavior") val streamingBehavior: StreamingBehavior? = null,
    @SerialName("client_msg_id") val clientMsgId: String? = null,
)

@Serializable
data class ChatApproveParams(
    val agent: String,
    @SerialName("approval_id") val approvalId: String,
    val reply: ApprovalReply,
)

@Serializable
data class ChatConfigureParams(
    val agent: String,
    val provider: String? = null,
    @SerialName("model_id") val modelId: String? = null,
    @SerialName("thinking_level") val thinkingLevel: String? = null,
)

@Serializable
data class ChatSwitchModeParams(
    val agent: String,
    val target: SessionDriverMode,
)

@Serializable
data class ChatSessionSummary(
    val agent: String,
    val meta: ChatSessionMeta,
    @SerialName("pending_approvals") val pendingApprovals: Int = 0,
    val seq: Long = 0,
)

@Serializable
data class ChatListResult(
    val sessions: List<ChatSessionSummary> = emptyList(),
)

@Serializable
data class ChatApprovalNoticeEvent(
    val agent: String,
    val approval: PendingApproval? = null,
    @SerialName("resolved_id") val resolvedId: String? = null,
)

data class ChatTimelineSnapshot(
    val agent: String = "",
    val seq: Long = 0,
    val meta: ChatSessionMeta = ChatSessionMeta(),
    val entries: List<TimelineEntry> = emptyList(),
    val pendingApprovals: List<PendingApproval> = emptyList(),
) {
    companion object {
        fun fromHistory(history: ChatHistoryResult): ChatTimelineSnapshot = ChatTimelineSnapshot(
            agent = history.agent,
            seq = history.seq,
            meta = history.meta,
            entries = history.entries,
            pendingApprovals = history.pendingApprovals,
        )
    }

    fun applyDelta(event: ChatDeltaEvent): ChatTimelineSnapshot {
        if (event.agent != agent && agent.isNotEmpty()) return this
        if (event.seq <= seq) return this
        return applyOp(event.delta).copy(seq = event.seq)
    }

    fun applyOp(op: ChatDeltaOp): ChatTimelineSnapshot = when (op) {
        is ChatDeltaOp.UpsertEntry -> {
            val idx = entries.indexOfFirst { it.id == op.entry.id }
            val next = if (idx >= 0) {
                entries.toMutableList().apply { set(idx, op.entry) }
            } else {
                entries + op.entry
            }
            copy(entries = next)
        }
        is ChatDeltaOp.AppendText -> mutateEntry(op.entryId) { e ->
            when {
                e is TimelineEntry.AssistantText && op.kind == TextDeltaKind.ASSISTANT -> e.copy(text = e.text + op.delta, streaming = true)
                e is TimelineEntry.Thinking && op.kind == TextDeltaKind.THINKING -> e.copy(text = e.text + op.delta, streaming = true)
                e is TimelineEntry.ToolCall && op.kind == TextDeltaKind.TOOL_OUTPUT -> e.copy(output = (e.output ?: "") + op.delta)
                else -> e
            }
        }
        is ChatDeltaOp.ReplaceEntryText -> mutateEntry(op.entryId) { e ->
            when {
                e is TimelineEntry.AssistantText && op.kind == TextDeltaKind.ASSISTANT -> e.copy(text = op.text, streaming = !op.finish)
                e is TimelineEntry.Thinking && op.kind == TextDeltaKind.THINKING -> e.copy(text = op.text, streaming = !op.finish)
                e is TimelineEntry.ToolCall && op.kind == TextDeltaKind.TOOL_OUTPUT -> e.copy(output = op.text)
                else -> e
            }
        }
        is ChatDeltaOp.FinishEntry -> mutateEntry(op.entryId) { e ->
            when (e) {
                is TimelineEntry.AssistantText -> e.copy(streaming = false)
                is TimelineEntry.Thinking -> e.copy(streaming = false, durationMs = op.durationMs ?: e.durationMs)
                else -> e
            }
        }
        is ChatDeltaOp.MetaUpdated -> copy(meta = op.meta)
        is ChatDeltaOp.ApprovalRequested -> {
            val next = pendingApprovals.filterNot { it.id == op.approval.id } + op.approval
            copy(pendingApprovals = next)
        }
        is ChatDeltaOp.ApprovalResolved -> copy(
            pendingApprovals = pendingApprovals.filterNot { it.id == op.approvalId },
        )
        is ChatDeltaOp.TurnFinished -> {
            val nextMeta = meta.copy(isStreaming = false, lastError = op.error ?: meta.lastError)
            val nextEntries = entries.map { entry ->
                when {
                    entry is TimelineEntry.Thinking && entry.streaming -> entry.copy(streaming = false)
                    entry is TimelineEntry.AssistantText && entry.streaming -> entry.copy(streaming = false)
                    else -> entry
                }
            }
            copy(meta = nextMeta, entries = nextEntries)
        }
        ChatDeltaOp.Reset -> copy(entries = emptyList(), pendingApprovals = emptyList())
    }

    private fun mutateEntry(id: String, transform: (TimelineEntry) -> TimelineEntry): ChatTimelineSnapshot {
        val idx = entries.indexOfFirst { it.id == id }
        if (idx < 0) return this
        val next = entries.toMutableList()
        next[idx] = transform(next[idx])
        return copy(entries = next)
    }
}

enum class DiffOp {
    CONTEXT, ADD, DEL
}

data class MobileDiffLine(
    val op: DiffOp,
    val text: String,
    val lineNum: Int? = null,
)

fun computeUnifiedDiffLines(oldText: String, newText: String, maxLines: Int = 80): List<MobileDiffLine> {
    val oldLines = if (oldText.isEmpty()) emptyList() else oldText.lines()
    val newLines = if (newText.isEmpty()) emptyList() else newText.lines()
    // Myers/DP LCS on bounded lines so mobile UI renders fast inline diffs.
    val n = oldLines.size
    val m = newLines.size
    if (n + m > 400) {
        return (oldLines.take(20).mapIndexed { i, l -> MobileDiffLine(DiffOp.DEL, l, i + 1) } +
            newLines.take(20).mapIndexed { i, l -> MobileDiffLine(DiffOp.ADD, l, i + 1) }).take(maxLines)
    }
    val dp = Array(n + 1) { IntArray(m + 1) }
    for (i in n - 1 downTo 0) {
        for (j in m - 1 downTo 0) {
            dp[i][j] = if (oldLines[i] == newLines[j]) dp[i + 1][j + 1] + 1 else maxOf(dp[i + 1][j], dp[i][j + 1])
        }
    }
    val out = ArrayList<MobileDiffLine>()
    var i = 0
    var j = 0
    while (i < n || j < m) {
        if (out.size >= maxLines) break
        when {
            i < n && j < m && oldLines[i] == newLines[j] -> {
                out.add(MobileDiffLine(DiffOp.CONTEXT, oldLines[i], j + 1))
                i++; j++
            }
            j < m && (i == n || dp[i][j + 1] >= dp[i + 1][j]) -> {
                out.add(MobileDiffLine(DiffOp.ADD, newLines[j], j + 1))
                j++
            }
            i < n -> {
                out.add(MobileDiffLine(DiffOp.DEL, oldLines[i], i + 1))
                i++
            }
        }
    }
    return out
}
