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
}
