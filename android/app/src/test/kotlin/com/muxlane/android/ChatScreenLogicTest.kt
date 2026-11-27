package com.muxlane.android

import muxlane.chat.TimelineEntry
import muxlane.chat.ToolCallKind
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatScreenLogicTest {
    @Test
    fun buildMobileTimelineItemsClustersConsecutiveExplorationTools() {
        val entries = listOf(
            TimelineEntry.UserMessage(id = "u1", text = "check repo"),
            TimelineEntry.ToolCall(id = "t1", callId = "c1", name = "read", kind = ToolCallKind.READ),
            TimelineEntry.ToolCall(id = "t2", callId = "c2", name = "search", kind = ToolCallKind.SEARCH),
            TimelineEntry.ToolCall(id = "t3", callId = "c3", name = "bash", kind = ToolCallKind.BASH),
            TimelineEntry.ToolCall(id = "t4", callId = "c4", name = "fetch", kind = ToolCallKind.FETCH),
        )
        val items = buildMobileTimelineItems(entries)
        assertEquals(4, items.size)
        assertTrue(items[0] is MobileTimelineItem.Single)
        assertTrue(items[1] is MobileTimelineItem.ExplorationCluster)
        assertEquals(2, (items[1] as MobileTimelineItem.ExplorationCluster).tools.size)
        assertTrue(items[2] is MobileTimelineItem.Single)
        assertTrue(items[3] is MobileTimelineItem.Single)
    }

    @Test
    fun parseMarkdownBlocksSeparatesHeadingsCodeQuotesAndLists() {
        val md = """
            ## Overview
            First paragraph line.

            > quoted note

            - first item
            - second item

            ```rust
            fn main() {}
            ```
        """.trimIndent()
        val blocks = parseMarkdownBlocks(md)
        assertEquals(6, blocks.size)
        assertEquals(MarkdownBlock.Heading(2, "Overview"), blocks[0])
        assertEquals(MarkdownBlock.Paragraph("First paragraph line."), blocks[1])
        assertEquals(MarkdownBlock.Quote("quoted note"), blocks[2])
        assertEquals(MarkdownBlock.BulletItem("•", "first item"), blocks[3])
        assertEquals(MarkdownBlock.BulletItem("•", "second item"), blocks[4])
        assertEquals(MarkdownBlock.CodeBlock("rust", "fn main() {}"), blocks[5])
    }

    @Test
    fun buildMobileTimelineItemsDoesNotClusterToolCallWithImageAttachments() {
        val entries = listOf(
            TimelineEntry.ToolCall(
                id = "t1",
                callId = "c1",
                name = "read",
                kind = ToolCallKind.READ,
                attachments = listOf(
                    muxlane.chat.ChatAttachment(
                        name = "photo.png",
                        mimeType = "image/png",
                        dataB64 = "iVBORw0KGgo=",
                    ),
                ),
            ),
            TimelineEntry.ToolCall(id = "t2", callId = "c2", name = "search", kind = ToolCallKind.SEARCH),
        )
        val items = buildMobileTimelineItems(entries)
        assertEquals(2, items.size)
        assertTrue(items[0] is MobileTimelineItem.Single)
        assertTrue(items[1] is MobileTimelineItem.Single)
    }

    @Test
    fun filterMobileSlashCommandsMergesServerCommandsAndFiltersByPrefix() {
        val serverCmds = listOf(
            muxlane.chat.ChatSlashCommand(name = "plan", description = "Step-by-step plan", kind = "skill"),
            muxlane.chat.ChatSlashCommand(name = "review", description = "Code review", kind = "command"),
        )
        val filtered = filterMobileSlashCommands(serverCmds, "pl")
        assertEquals(1, filtered.size)
        assertEquals("plan", filtered[0].name)

        val all = filterMobileSlashCommands(serverCmds, "")
        assertTrue(all.any { it.name == "plan" })
        assertTrue(all.none { it.name == "terminal" })
        assertTrue(all.any { it.name == "abort" })
    }

    @Test
    fun presetChatDriverMatchesDesktopRouting() {
        assertEquals(muxlane.chat.ChatDriverKind.PI_RPC, presetChatDriver(muxlane.protocol.AgentType.PIE))
        assertEquals(muxlane.chat.ChatDriverKind.PI_RPC, presetChatDriver(muxlane.protocol.AgentType.PI))
        assertEquals(muxlane.chat.ChatDriverKind.ACP, presetChatDriver(muxlane.protocol.AgentType.AGY))
        assertEquals(null, presetChatDriver(muxlane.protocol.AgentType.CLAUDE))
        assertEquals(null, presetChatDriver(muxlane.protocol.AgentType.CODEX))
        assertEquals(null, presetChatDriver(muxlane.protocol.AgentType.SHELL))
    }

    @Test
    fun formatMessageUsageProducesCompactSummary() {
        val summary = formatMessageUsage(
            muxlane.chat.MessageUsage(
                inputTokens = 1500,
                outputTokens = 400,
                totalTokens = 1900,
                durationMs = 2300,
                costUsd = 0.015,
            ),
        )
        assertTrue(summary.contains("1.5k↑"))
        assertTrue(summary.contains("400↓"))
        assertTrue(summary.contains("2.3s"))
    }

    @Test
    fun parseApprovalPromptStructuresPiSelectQuestionAndMultiSelectInput() {
        val select = parseApprovalPrompt(
            muxlane.chat.PendingApproval(
                id = "q1",
                agent = "pie_1",
                source = muxlane.chat.ApprovalSource.PI_EXTENSION_UI,
                method = "select",
                title = "[新闻偏好] 你最想了解马斯克哪方面的新闻？",
                options = listOf(
                    muxlane.chat.ApprovalOption(
                        optionId = "1. 特斯拉与机器人 — 关注电动车、自动驾驶和机器人进展。",
                        label = "1. 特斯拉与机器人 — 关注电动车、自动驾驶和机器人进展。",
                        kind = "select",
                    ),
                    muxlane.chat.ApprovalOption(
                        optionId = "2. Type something.",
                        label = "2. Type something.",
                        kind = "select",
                    ),
                ),
            ),
        )
        assertTrue(select.isQuestion)
        assertTrue(select.useListLayout)
        assertEquals("新闻偏好", select.badge)
        assertEquals("你最想了解马斯克哪方面的新闻？", select.questionTitle)
        assertEquals(2, select.options.size)
        assertEquals("1", select.options[0].indexLabel)
        assertEquals("特斯拉与机器人", select.options[0].headline)
        assertEquals("关注电动车、自动驾驶和机器人进展。", select.options[0].description)
        assertTrue(select.options[1].isCustom)
        assertEquals("2. Type something.", select.options[1].optionId)

        val multi = parseApprovalPrompt(
            muxlane.chat.PendingApproval(
                id = "q2",
                agent = "pie_1",
                source = muxlane.chat.ApprovalSource.PI_EXTENSION_UI,
                method = "input",
                title = "[模块] 请选择要改动的模块\n\n1. Desktop — 桌面端\n2. Android — 移动端\n\nEnter the numbers of all that apply, comma-separated (e.g. \"1,3\"), or type a custom answer as plain text.",
            ),
        )
        assertTrue(multi.isInputOnly)
        assertTrue(multi.isMultiSelectInput)
        assertEquals("模块", multi.badge)
        assertEquals(2, multi.embeddedOptions.size)
        assertEquals("Desktop", multi.embeddedOptions[0].headline)
        assertEquals("桌面端", multi.embeddedOptions[0].description)
        assertEquals("2", toggleMultiSelectNumber("", "2"))
        assertEquals("1,2", toggleMultiSelectNumber("2", "1"))
        assertEquals("1", toggleMultiSelectNumber("1,2", "2"))
    }
}
