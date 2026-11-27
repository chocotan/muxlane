package com.muxlane.android

import muxlane.chat.ApprovalSource
import muxlane.chat.ChatSessionMeta
import muxlane.chat.ChatSessionSummary
import muxlane.chat.PendingApproval
import muxlane.chat.SessionDriverMode
import muxlane.protocol.AgentInstance
import muxlane.protocol.AgentPreset
import muxlane.protocol.AgentStatus
import muxlane.protocol.AgentType
import muxlane.protocol.Project
import muxlane.protocol.Snapshot
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class MobileArchitectureTest {
    @Test
    fun tabHelpersManageOpenCloseAndSteppingCleanly() {
        val t1 = appendOrFocusTab(emptyList(), "a1")
        val t2 = appendOrFocusTab(t1, "a2")
        val t3 = appendOrFocusTab(t2, "a3")
        assertEquals(listOf("a1", "a2", "a3"), t3)
        assertEquals(t3, appendOrFocusTab(t3, "a2"))

        assertEquals("a2", stepTabInOrder(t3, "a1", 1))
        assertEquals("a1", stepTabInOrder(t3, "a2", -1))
        assertNull(stepTabInOrder(t3, "a1", -1))
        assertNull(stepTabInOrder(t3, "a3", 1))

        val (afterCloseMid, nextMid) = removeTabAndNextSelection(t3, "a2", "a2")
        assertEquals(listOf("a1", "a3"), afterCloseMid)
        assertEquals("a3", nextMid)

        val (afterCloseInactive, sameActive) = removeTabAndNextSelection(t3, "a1", "a3")
        assertEquals(listOf("a2", "a3"), afterCloseInactive)
        assertEquals("a3", sameActive)

        val (afterCloseLast, nextLast) = removeTabAndNextSelection(listOf("a1"), "a1", "a1")
        assertTrue(afterCloseLast.isEmpty())
        assertNull(nextLast)
    }

    @Test
    fun buildSessionTabSummariesGroupsByNeedsYouWorkingAndOther() {
        val snapshot = Snapshot(
            projects = listOf(Project(id = "p1", name = "muxlane", path = "/repo/muxlane", branch = "main")),
            agents = listOf(
                AgentInstance(id = "a1", project = "p1", agentType = AgentType.PI, title = "Pi Worker", status = AgentStatus.WORKING),
                AgentInstance(id = "a2", project = "p1", agentType = AgentType.OPENCODE, title = "OpenCode Fix", status = AgentStatus.IDLE),
                AgentInstance(id = "a3", project = "p1", agentType = AgentType.QWEN, title = "Qwen Docs", status = AgentStatus.DONE),
            ),
        )
        val chatSessions = mapOf(
            "a1" to ChatSessionSummary(agent = "a1", meta = ChatSessionMeta(driver = SessionDriverMode.PI_RPC, isStreaming = true)),
            "a2" to ChatSessionSummary(agent = "a2", meta = ChatSessionMeta(driver = SessionDriverMode.ACP), pendingApprovals = 1),
        )
        val approvals = listOf(
            PendingApproval(
                id = "apr1",
                agent = "a2",
                source = ApprovalSource.ACP_PERMISSION,
                method = "permission",
                title = "Allow bash",
            ),
        )

        val tabs = buildSessionTabSummaries(
            openTabs = listOf("a1", "a2", "a3", "missing"),
            snapshot = snapshot,
            chatSessions = chatSessions,
            globalApprovals = approvals,
        )
        assertEquals(3, tabs.size)
        assertEquals(TabPriorityGroup.WORKING, tabs[0].group)
        assertEquals(TabPriorityGroup.NEEDS_YOU, tabs[1].group)
        assertEquals(1, tabs[1].pendingApprovalsCount)
        assertEquals(TabPriorityGroup.OTHER, tabs[2].group)
    }

    @Test
    fun buildAgentListProjectionSeparatesActiveAndFoldableIdleAndGroupsByDriver() {
        val snapshot = Snapshot(
            projects = listOf(
                Project(id = "p1", name = "core", path = "/repo/core", branch = "main"),
                Project(id = "p2", name = "app", path = "/repo/app", branch = "feat/ui"),
            ),
            agents = listOf(
                AgentInstance(id = "a1", project = "p1", agentType = AgentType.PI, title = "Pi Session", status = AgentStatus.BLOCKED),
                AgentInstance(id = "a2", project = "p2", agentType = AgentType.OPENCODE, title = "OpenCode", status = AgentStatus.WORKING),
                AgentInstance(id = "a3", project = "p2", agentType = AgentType.SHELL, title = "bash", status = AgentStatus.IDLE),
            ),
        )
        val chats = mapOf(
            "a1" to ChatSessionSummary(agent = "a1", meta = ChatSessionMeta(driver = SessionDriverMode.PI_RPC)),
            "a2" to ChatSessionSummary(agent = "a2", meta = ChatSessionMeta(driver = SessionDriverMode.ACP)),
        )

        val statusProj = buildAgentListProjection(
            snapshot = snapshot,
            chatSessions = chats,
            globalApprovals = emptyList(),
            groupBy = AgentGroupBy.STATUS,
            query = "",
            needsYouOnly = false,
        )
        assertEquals(2, statusProj.sections.size)
        assertEquals("needs_you", statusProj.sections[0].key)
        assertEquals("working", statusProj.sections[1].key)
        assertEquals(1, statusProj.idleRows.size)
        assertEquals("a3", statusProj.idleRows.first().agent.id)

        val driverProj = buildAgentListProjection(
            snapshot = snapshot,
            chatSessions = chats,
            globalApprovals = emptyList(),
            groupBy = AgentGroupBy.DRIVER,
            query = "pi",
            needsYouOnly = false,
        )
        assertEquals(1, driverProj.totalCount)
        assertEquals("Pi · 对话", driverProj.sections.first().title)
    }

    @Test
    fun isChatOnlySessionMatchesDesktopTmuxRule() {
        val chatAgent = AgentInstance(id = "a1", project = "p1", agentType = AgentType.PIE, title = "Pie", tmuxSession = null)
        val ptyAgent = AgentInstance(id = "a2", project = "p1", agentType = AgentType.PIE, title = "Pie CLI", tmuxSession = "muxlane-a2")
        assertTrue(isChatOnlySession(chatAgent))
        assertTrue(!isChatOnlySession(ptyAgent))
    }

    @Test
    fun orderAgentPresetsPutsPiAndGenericAcpFirstAndSeparatesShell() {
        val presets = listOf(
            AgentPreset(id = "shell", label = "Shell", agentType = AgentType.SHELL, program = "/bin/zsh"),
            AgentPreset(id = "claude", label = "Claude", agentType = AgentType.CLAUDE, program = "claude"),
            AgentPreset(id = "opencode", label = "OpenCode", agentType = AgentType.OPENCODE, program = "opencode"),
            AgentPreset(id = "pi", label = "Pi", agentType = AgentType.PI, program = "pi"),
        )
        val (agents, shell) = orderAgentPresetsPiFirst(presets)
        assertEquals("shell", shell?.id)
        assertEquals(listOf("pi", "opencode", "claude"), agents.map { it.id })
    }
}
