package muxlane.chat

import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import muxlane.crypto.E2eeKeyPair
import muxlane.crypto.E2eeSession
import muxlane.protocol.AgentType
import muxlane.protocol.muxlaneJson
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class ChatTest {
    @Test
    fun timelineReducerDecodesRustWireJsonAndAppliesDeltasIdempotently() {
        val rawHistory = """
            {
              "agent": "pi-1",
              "seq": 2,
              "meta": {
                "driver": "pi_rpc",
                "supports_chat_input": true,
                "supports_terminal_switch": true,
                "has_pty": false,
                "current_model": { "provider": "openai", "id": "gpt-5", "name": "gpt-5" },
                "thinking_level": "medium",
                "is_streaming": true
              },
              "entries": [
                {
                  "type": "user_message",
                  "id": "u1",
                  "text": "Fix bug",
                  "attachments": [],
                  "created_at_ms": 100
                },
                {
                  "type": "assistant_text",
                  "id": "a1",
                  "text": "Checking",
                  "streaming": true,
                  "created_at_ms": 101
                }
              ],
              "pending_approvals": []
            }
        """.trimIndent()
        val history = muxlaneJson.decodeFromString(ChatHistoryResult.serializer(), rawHistory)
        var snap = ChatTimelineSnapshot.fromHistory(history)
        assertEquals(2L, snap.seq)
        assertEquals(2, snap.entries.size)

        val deltaJson = """
            {
              "agent": "pi-1",
              "seq": 3,
              "delta": {
                "op": "append_text",
                "entry_id": "a1",
                "delta": " Cargo.toml",
                "kind": "assistant"
              }
            }
        """.trimIndent()
        val delta = muxlaneJson.decodeFromString(ChatDeltaEvent.serializer(), deltaJson)
        snap = snap.applyDelta(delta)
        // Duplicate seq must be ignored
        snap = snap.applyDelta(delta)
        assertEquals(3L, snap.seq)
        assertEquals("Checking Cargo.toml", (snap.entries[1] as TimelineEntry.AssistantText).text)

        val approvalJson = """
            {
              "agent": "pi-1",
              "seq": 4,
              "delta": {
                "op": "approval_requested",
                "approval": {
                  "id": "ap-1",
                  "agent": "pi-1",
                  "source": "acp_permission",
                  "method": "permission",
                  "title": "Run cargo test?",
                  "command_or_input": "cargo test --workspace",
                  "options": [
                    { "option_id": "allow", "label": "Allow", "kind": "allow_once" },
                    { "option_id": "deny", "label": "Deny", "kind": "reject_once" }
                  ],
                  "created_at_ms": 200
                }
              }
            }
        """.trimIndent()
        snap = snap.applyDelta(muxlaneJson.decodeFromString(ChatDeltaEvent.serializer(), approvalJson))
        assertEquals(1, snap.pendingApprovals.size)
        assertEquals("Run cargo test?", snap.pendingApprovals[0].title)

        val removeJson = """
            {
              "agent": "pi-1",
              "seq": 5,
              "delta": {
                "op": "remove_entry",
                "entry_id": "u1"
              }
            }
        """.trimIndent()
        snap = snap.applyDelta(muxlaneJson.decodeFromString(ChatDeltaEvent.serializer(), removeJson))
        assertEquals(1, snap.entries.size)
        assertEquals("a1", snap.entries.first().id)

        val cancelEncoded = muxlaneJson.encodeToString(ApprovalReply.serializer(), ApprovalReply.Cancel)
        assertTrue(cancelEncoded.contains("\"kind\":\"cancel\""), cancelEncoded)
    }

    @Test
    fun decodesModernChatMetaSetAssistantUsageAndTermResync() {
        val rawHistory = """
            {
              "agent": "pie_1",
              "seq": 10,
              "meta": {
                "driver": "pi_rpc",
                "supports_chat_input": true,
                "supports_terminal_switch": true,
                "has_pty": true,
                "available_commands": [
                  { "name": "plan", "description": "Create plan", "kind": "skill", "argument_hint": "<goal>" }
                ],
                "queued_messages": [
                  { "id": "q1", "text": "check tests too", "behavior": "steer", "attachments_count": 0 }
                ],
                "is_streaming": true,
                "stats": {
                  "turns": 3,
                  "steps": 7,
                  "tool_calls": 4,
                  "input_tokens": 12000,
                  "output_tokens": 1800,
                  "total_tokens": 13800,
                  "total_cost_usd": 0.042
                },
                "goal": { "objective": "Ship v0.0.36", "status": "active", "rounds": 1, "max_rounds": 5, "armed": true },
                "tasks": [
                  { "id": "t1", "subject": "Fix mobile chat", "status": "in_progress", "active_form": "Fixing mobile chat" }
                ],
                "subagents": [
                  { "id": "s1", "agent": "research", "description": "Audit protocol", "status": "running", "turns": 2 }
                ],
                "background_jobs": [
                  { "id": "j1", "name": "gradle", "description": "Run tests", "status": "completed", "exit_code": 0 }
                ]
              },
              "entries": [
                {
                  "type": "assistant_text",
                  "id": "a1",
                  "text": "Done.",
                  "streaming": false,
                  "created_at_ms": 500
                }
              ],
              "pending_approvals": []
            }
        """.trimIndent()
        val history = muxlaneJson.decodeFromString(ChatHistoryResult.serializer(), rawHistory)
        var snap = ChatTimelineSnapshot.fromHistory(history)
        assertEquals(1, snap.meta.availableCommands.size)
        assertEquals("plan", snap.meta.availableCommands[0].name)
        assertEquals(1, snap.meta.queuedMessages.size)
        assertEquals(StreamingBehavior.STEER, snap.meta.queuedMessages[0].behavior)
        assertEquals(3, snap.meta.stats?.turns)
        assertEquals("Ship v0.0.36", snap.meta.goal?.objective)
        assertEquals(1, snap.meta.tasks.size)
        assertEquals(1, snap.meta.subagents.size)
        assertEquals(1, snap.meta.backgroundJobs.size)

        val usageDeltaJson = """
            {
              "agent": "pie_1",
              "seq": 11,
              "delta": {
                "op": "set_assistant_usage",
                "entry_id": "a1",
                "usage": {
                  "input_tokens": 4200,
                  "output_tokens": 350,
                  "total_tokens": 4550,
                  "cost_usd": 0.0123,
                  "duration_ms": 1450
                }
              }
            }
        """.trimIndent()
        val usageDelta = muxlaneJson.decodeFromString(ChatDeltaEvent.serializer(), usageDeltaJson)
        snap = snap.applyDelta(usageDelta)
        val assistant = snap.entries[0] as TimelineEntry.AssistantText
        assertEquals(4550L, assistant.usage?.totalTokens)
        assertEquals(1450L, assistant.usage?.durationMs)

        val resyncJson = """{"agent":"shell_1","replay_b64":"aGVsbG8="}"""
        val resync = muxlaneJson.decodeFromString(muxlane.protocol.TermResyncEvent.serializer(), resyncJson)
        assertEquals("shell_1", resync.agent)
        assertEquals("aGVsbG8=", resync.replayB64)
    }

    @Test
    fun diffLinesDetectsAddedAndRemovedLines() {
        val diff = computeUnifiedDiffLines("line1\nold\nline3", "line1\nnew\nline3")
        assertTrue(diff.any { it.op == DiffOp.DEL && it.text == "old" })
        assertTrue(diff.any { it.op == DiffOp.ADD && it.text == "new" })
    }

    @Test
    fun e2eeP256EcdhAesGcmRoundtripsAndBlocksReplayAndWrongToken() {
        val clientKp = E2eeKeyPair.generate()
        val serverKp = E2eeKeyPair.generate()
        val cNonce = E2eeKeyPair.randomNonce16()
        val sNonce = E2eeKeyPair.randomNonce16()

        val client = E2eeSession.deriveClient(clientKp, serverKp.publicSec1, "tok-secret", cNonce, sNonce)
        val server = E2eeSession.deriveServer(serverKp, clientKp.publicSec1, "tok-secret", cNonce, sNonce)

        val env = client.seal("{\"method\":\"state.list\"}".toByteArray())
        val plain = server.open(env).toString(Charsets.UTF_8)
        assertEquals("{\"method\":\"state.list\"}", plain)

        assertFailsWith<IllegalArgumentException> {
            server.open(env)
        }

        val badServer = E2eeSession.deriveServer(
            E2eeKeyPair.generate(),
            clientKp.publicSec1,
            "wrong-token",
            cNonce,
            sNonce,
        )
        val env2 = client.seal("secret".toByteArray())
        assertFailsWith<Exception> {
            badServer.open(env2)
        }
        assertFalse(AgentType.PI.name.isBlank())
        assertTrue(buildJsonObject { put("k", JsonPrimitive("v")) }.containsKey("k"))
    }
}
