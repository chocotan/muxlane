package muxlane.protocol

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class ProtocolTest {
    @Test
    fun pairParamsEncodeHost() {
        val json = encodeFrame(
            Request(
                7,
                Methods.PAIR_BEGIN,
                encodeParams(PairBeginParams.serializer(), PairBeginParams(hostId = "machine_a")),
            ),
        )
        assertTrue(json.contains("\"method\":\"pair.begin\""))
        assertTrue(json.contains("machine_a"))
    }

    @Test
    fun eventAndResponseDecode() {
        val event = decodeIncoming(
            """{"event":"agent.status_changed","params":{"agent":"a","from":"working","to":"done"}}""",
        )
        val ev = (event as IncomingFrame.Event).value
        assertEquals(Events.AGENT_STATUS, ev.event)
        val status = muxlaneJson.decodeFromJsonElement(AgentStatusEvent.serializer(), ev.params)
        assertEquals(AgentStatus.DONE, status.to)
        assertTrue(status.to.isAlert())

        val response = decodeIncoming("""{"id":1,"result":{"ok":true}}""")
        assertEquals(1, (response as IncomingFrame.Response).value.id)
    }

    @Test
    fun b64RoundTrip() {
        val src = "你好".toByteArray(Charsets.UTF_8)
        assertTrue(b64Decode(b64Encode(src)).contentEquals(src))
    }
}
