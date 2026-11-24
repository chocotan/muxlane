package com.muxlane.android

import androidx.annotation.StringRes
import androidx.compose.ui.graphics.Color
import muxlane.chat.ChatSessionSummary
import muxlane.chat.PendingApproval
import muxlane.chat.SessionDriverMode
import muxlane.protocol.AgentInstance
import muxlane.protocol.AgentPreset
import muxlane.protocol.AgentStatus
import muxlane.protocol.AgentType
import muxlane.protocol.Project
import muxlane.protocol.Snapshot

enum class HubTab {
    FEED,
    WORKSPACES,
    AGENTS,
    BOTS,
}

enum class AgentGroupBy {
    STATUS,
    PROJECT,
    DRIVER,
}

enum class NewTabSurface {
    CHAT,
    CLI,
}

enum class TabPriorityGroup {
    NEEDS_YOU,
    WORKING,
    OTHER,
}

data class SessionTabSummary(
    val agentId: String,
    val title: String,
    val projectId: String,
    val projectName: String,
    val agentType: AgentType,
    val status: AgentStatus,
    val isChatCapable: Boolean,
    val driverMode: SessionDriverMode,
    val pendingApprovalsCount: Int,
    val isStreaming: Boolean,
    val statusSince: Long,
    val seen: Boolean,
    val group: TabPriorityGroup,
)

data class AgentListRowModel(
    val agent: AgentInstance,
    val projectName: String,
    val projectBranch: String?,
    val isChatCapable: Boolean,
    val driverLabel: String,
    val pendingApprovalsCount: Int,
    val isStreaming: Boolean,
    val needsYou: Boolean,
)

data class AgentListSectionModel(
    val key: String,
    val title: String,
    val rows: List<AgentListRowModel>,
)

data class AgentListProjection(
    val sections: List<AgentListSectionModel>,
    val idleRows: List<AgentListRowModel>,
    val totalCount: Int,
    val needsYouCount: Int,
    val workingCount: Int,
)

data class QuickCommandItem(
    val id: String,
    val label: String,
    val command: String,
    val forChat: Boolean,
    val forTerminal: Boolean,
)

internal fun defaultQuickCommands(): List<QuickCommandItem> = listOf(
    QuickCommandItem("status", "Git Status", "git status -sb\r", forChat = false, forTerminal = true),
    QuickCommandItem("diff", "Git Diff", "git diff --stat\r", forChat = false, forTerminal = true),
    QuickCommandItem("test", "运行测试", "cargo test --workspace\r", forChat = false, forTerminal = true),
    QuickCommandItem("compact", "压缩上下文", "/compact", forChat = true, forTerminal = false),
    QuickCommandItem("clear", "清空对话", "/clear", forChat = true, forTerminal = false),
    QuickCommandItem("plan", "列出执行计划", "请先输出下一步最小验证计划，确认后再修改文件。", forChat = true, forTerminal = false),
)

internal fun appendOrFocusTab(openTabs: List<String>, agentId: String): List<String> {
    if (agentId in openTabs) return openTabs
    return openTabs + agentId
}

internal fun removeTabAndNextSelection(
    openTabs: List<String>,
    closingId: String,
    currentSelected: String?,
): Pair<List<String>, String?> {
    val index = openTabs.indexOf(closingId)
    val nextTabs = openTabs.filterNot { it == closingId }
    val nextSelected = when {
        currentSelected != closingId -> currentSelected?.takeIf { it in nextTabs }
        nextTabs.isEmpty() -> null
        index < 0 -> nextTabs.lastOrNull()
        index < nextTabs.size -> nextTabs[index]
        else -> nextTabs.lastOrNull()
    }
    return nextTabs to nextSelected
}

internal fun stepTabInOrder(openTabs: List<String>, currentSelected: String?, delta: Int): String? {
    if (openTabs.size <= 1) return currentSelected
    val currentIdx = openTabs.indexOf(currentSelected).takeIf { it >= 0 } ?: 0
    val targetIdx = currentIdx + delta
    if (targetIdx !in openTabs.indices) return null
    return openTabs[targetIdx]
}

internal fun pruneStaleTabs(openTabs: List<String>, agents: List<AgentInstance>): List<String> {
    val validIds = agents.map { it.id }.toSet()
    return openTabs.filter { it in validIds }
}

internal fun buildSessionTabSummaries(
    openTabs: List<String>,
    snapshot: Snapshot?,
    chatSessions: Map<String, ChatSessionSummary>,
    globalApprovals: List<PendingApproval>,
): List<SessionTabSummary> {
    val agentsById = snapshot?.agents?.associateBy { it.id }.orEmpty()
    val projectsById = snapshot?.projects?.associateBy { it.id }.orEmpty()
    return openTabs.mapNotNull { id ->
        val agent = agentsById[id] ?: return@mapNotNull null
        val summary = chatSessions[id]
        val approvalsForAgent = maxOf(
            summary?.pendingApprovals ?: 0,
            globalApprovals.count { it.agent == id },
        )
        val project = projectsById[agent.project]
        val isStreaming = summary?.meta?.isStreaming == true
        val needsYou = agent.status == AgentStatus.BLOCKED || approvalsForAgent > 0
        val working = agent.status == AgentStatus.WORKING || isStreaming
        val group = when {
            needsYou || agent.status == AgentStatus.FAILED -> TabPriorityGroup.NEEDS_YOU
            working -> TabPriorityGroup.WORKING
            else -> TabPriorityGroup.OTHER
        }
        SessionTabSummary(
            agentId = id,
            title = agent.title,
            projectId = agent.project,
            projectName = project?.displayName() ?: agent.project,
            agentType = agent.agentType,
            status = agent.status,
            isChatCapable = summary != null,
            driverMode = summary?.meta?.driver ?: SessionDriverMode.TERMINAL_PTY,
            pendingApprovalsCount = approvalsForAgent,
            isStreaming = isStreaming,
            statusSince = agent.statusSince,
            seen = agent.seen,
            group = group,
        )
    }
}

internal fun buildAgentListProjection(
    snapshot: Snapshot?,
    chatSessions: Map<String, ChatSessionSummary>,
    globalApprovals: List<PendingApproval>,
    groupBy: AgentGroupBy,
    query: String,
    needsYouOnly: Boolean,
): AgentListProjection {
    val projectsById = snapshot?.projects?.associateBy { it.id }.orEmpty()
    val needle = query.trim().lowercase()
    val allRows = snapshot?.agents.orEmpty().map { agent ->
        val project = projectsById[agent.project]
        val chat = chatSessions[agent.id]
        val approvalsCount = maxOf(
            chat?.pendingApprovals ?: 0,
            globalApprovals.count { it.agent == agent.id },
        )
        val needsYou = agent.status == AgentStatus.BLOCKED || approvalsCount > 0 || agent.status == AgentStatus.FAILED
        AgentListRowModel(
            agent = agent,
            projectName = project?.displayName() ?: agent.project,
            projectBranch = project?.branch,
            isChatCapable = chat != null,
            driverLabel = driverBadgeLabel(agent.agentType, chat?.meta?.driver),
            pendingApprovalsCount = approvalsCount,
            isStreaming = chat?.meta?.isStreaming == true,
            needsYou = needsYou,
        )
    }
    val needsYouTotal = allRows.count { it.needsYou }
    val workingTotal = allRows.count { !it.needsYou && (it.agent.status == AgentStatus.WORKING || it.isStreaming) }

    val filtered = allRows.filter { row ->
        val matchesNeeds = !needsYouOnly || row.needsYou || !row.agent.seen
        val matchesQuery = needle.isEmpty() ||
            row.agent.title.lowercase().contains(needle) ||
            row.projectName.lowercase().contains(needle) ||
            row.driverLabel.lowercase().contains(needle)
        matchesNeeds && matchesQuery
    }.sortedWith(
        compareBy<AgentListRowModel>(
            { if (it.needsYou) 0 else 1 },
            { statusPriority(it.agent.status) },
            { -it.agent.statusSince },
        ),
    )

    if (groupBy == AgentGroupBy.STATUS && !needsYouOnly && needle.isEmpty()) {
        val activeNeedYou = filtered.filter { it.needsYou }
        val activeWorking = filtered.filter { !it.needsYou && (it.agent.status == AgentStatus.WORKING || it.isStreaming) }
        val activeDone = filtered.filter { !it.needsYou && it.agent.status == AgentStatus.DONE && !it.isStreaming }
        val idle = filtered.filter {
            !it.needsYou &&
                it.agent.status != AgentStatus.WORKING &&
                !it.isStreaming &&
                it.agent.status != AgentStatus.DONE
        }
        val sections = buildList {
            if (activeNeedYou.isNotEmpty()) add(AgentListSectionModel("needs_you", "需要你确认", activeNeedYou))
            if (activeWorking.isNotEmpty()) add(AgentListSectionModel("working", "进行中", activeWorking))
            if (activeDone.isNotEmpty()) add(AgentListSectionModel("done", "最近完成", activeDone))
        }
        return AgentListProjection(
            sections = sections,
            idleRows = idle,
            totalCount = filtered.size,
            needsYouCount = needsYouTotal,
            workingCount = workingTotal,
        )
    }

    val grouped = when (groupBy) {
        AgentGroupBy.STATUS -> filtered.groupBy {
            when {
                it.needsYou -> "needs_you" to "需要你确认"
                it.agent.status == AgentStatus.WORKING || it.isStreaming -> "working" to "进行中"
                it.agent.status == AgentStatus.DONE -> "done" to "已完成"
                else -> "idle" to "空闲"
            }
        }
        AgentGroupBy.PROJECT -> filtered.groupBy { row ->
            row.agent.project to row.projectName
        }
        AgentGroupBy.DRIVER -> filtered.groupBy { row ->
            row.driverLabel to row.driverLabel
        }
    }
    val sections = grouped.entries.map { (keyPair, rows) ->
        AgentListSectionModel(key = keyPair.first, title = keyPair.second, rows = rows)
    }
    return AgentListProjection(
        sections = sections,
        idleRows = emptyList(),
        totalCount = filtered.size,
        needsYouCount = needsYouTotal,
        workingCount = workingTotal,
    )
}

internal fun filterProjectsAndAgents(
    projects: List<Project>,
    agents: List<AgentInstance>,
    query: String,
    needsYouOnly: Boolean,
    approvals: List<PendingApproval>,
): Pair<List<Project>, Map<String, List<AgentInstance>>> {
    val needle = query.trim().lowercase()
    val grouped = agents.groupBy { it.project }
    val resultProjects = ArrayList<Project>()
    val resultAgents = LinkedHashMap<String, List<AgentInstance>>()

    for (project in projects) {
        val projectMatches = needle.isEmpty() ||
            project.displayName().lowercase().contains(needle) ||
            project.path.lowercase().contains(needle) ||
            (project.branch?.lowercase()?.contains(needle) == true)
        val candidates = grouped[project.id].orEmpty()
        val matchingAgents = candidates.filter { agent ->
            val hasApproval = approvals.any { it.agent == agent.id }
            val matchesNeeds = !needsYouOnly ||
                agent.status == AgentStatus.BLOCKED ||
                agent.status == AgentStatus.FAILED ||
                hasApproval ||
                !agent.seen
            val matchesText = projectMatches ||
                agent.title.lowercase().contains(needle) ||
                agent.agentType.name.lowercase().contains(needle)
            matchesNeeds && matchesText
        }.sortedWith(compareBy({ statusPriority(it.status) }, { -it.statusSince }))

        if (matchingAgents.isNotEmpty() || (!needsYouOnly && projectMatches)) {
            resultProjects.add(project)
            resultAgents[project.id] = matchingAgents
        }
    }
    return resultProjects to resultAgents
}

internal fun orderAgentPresetsPiFirst(presets: List<AgentPreset>): Pair<List<AgentPreset>, AgentPreset?> {
    val shell = presets.firstOrNull { it.agentType == AgentType.SHELL || it.id == "shell" }
    val agents = presets
        .filter { it.agentType != AgentType.SHELL && it.id != "shell" }
        .sortedWith(
            compareBy<AgentPreset> { presetPriority(it.agentType) }
                .thenBy { it.label.lowercase() },
        )
    return agents to shell
}

private fun presetPriority(type: AgentType): Int = when (type) {
    AgentType.PI, AgentType.PIE -> 0
    AgentType.OPENCODE -> 1
    AgentType.QWEN -> 2
    AgentType.KIMI -> 3
    AgentType.AGY -> 4
    AgentType.GEMINI -> 5
    AgentType.CODEX -> 6
    AgentType.CLAUDE -> 8
    else -> 7
}

internal fun driverBadgeLabel(agentType: AgentType, driver: SessionDriverMode?): String {
    val base = when (agentType) {
        AgentType.PI, AgentType.PIE -> "Pi"
        AgentType.OPENCODE -> "OpenCode"
        AgentType.QWEN -> "Qwen"
        AgentType.KIMI -> "Kimi"
        AgentType.AGY -> "AGY"
        AgentType.GEMINI -> "Gemini"
        AgentType.CODEX -> "Codex"
        AgentType.CLAUDE -> "Claude"
        AgentType.SHELL -> "Shell"
        AgentType.UNKNOWN -> "Agent"
    }
    return when (driver) {
        SessionDriverMode.PI_RPC -> "$base · RPC"
        SessionDriverMode.ACP -> "$base · ACP"
        SessionDriverMode.TERMINAL_PTY, null -> "$base · CLI"
    }
}

internal fun Project.displayName(): String = name.ifBlank { path.substringAfterLast('/') }

internal fun formatRelativeAge(timestampSecs: Long, nowMs: Long): String {
    if (timestampSecs <= 0L) return ""
    val normalizedMs = if (timestampSecs > 10_000_000_000L) timestampSecs else timestampSecs * 1000L
    val diffSec = ((nowMs - normalizedMs) / 1000L).coerceAtLeast(0L)
    return when {
        diffSec < 60L -> "刚刚"
        diffSec < 3600L -> "${diffSec / 60L}m"
        diffSec < 86400L -> "${diffSec / 3600L}h"
        else -> "${diffSec / 86400L}d"
    }
}

@StringRes
internal fun statusLabelRes(status: AgentStatus): Int = when (status) {
    AgentStatus.WORKING -> R.string.status_working
    AgentStatus.BLOCKED -> R.string.status_blocked
    AgentStatus.DONE -> R.string.status_done
    AgentStatus.FAILED -> R.string.status_failed
    else -> R.string.status_idle
}

internal fun statusPriority(status: AgentStatus): Int = when (status) {
    AgentStatus.BLOCKED -> 0
    AgentStatus.FAILED -> 1
    AgentStatus.WORKING -> 2
    AgentStatus.DONE -> 3
    else -> 4
}

internal fun statusColor(status: AgentStatus): Color = when (status) {
    AgentStatus.WORKING -> MuxAccent
    AgentStatus.BLOCKED -> MuxYellow
    AgentStatus.DONE -> MuxGreen
    AgentStatus.FAILED -> MuxRed
    else -> MuxFg2
}
