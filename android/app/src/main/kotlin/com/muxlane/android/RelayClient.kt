package com.muxlane.android

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.withTimeout
import muxlane.protocol.EventMsg
import muxlane.protocol.IncomingFrame
import muxlane.protocol.Request
import muxlane.protocol.Response
import muxlane.protocol.decodeIncoming
import muxlane.protocol.encodeFrame
import okhttp3.OkHttpClient
import okhttp3.Request as HttpRequest
import okhttp3.Response as HttpResponse
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong

class RelayClient(
    private val client: OkHttpClient = OkHttpClient.Builder()
        .pingInterval(20, TimeUnit.SECONDS)
        .build(),
) {
    private data class Connection(
        val opened: CompletableDeferred<Unit> = CompletableDeferred(),
        val closed: CompletableDeferred<Throwable?> = CompletableDeferred(),
        var socket: WebSocket? = null,
    )

    @Volatile
    private var connection: Connection? = null
    @Volatile
    private var e2eeSession: muxlane.crypto.E2eeSession? = null
    private val txLock = Any()
    private val nextId = AtomicLong(1)
    private val pending = ConcurrentHashMap<Long, CompletableDeferred<Response>>()
    // WebSocket callbacks must not drop terminal chunks while the main dispatcher is busy.
    private val _events = Channel<EventMsg>(Channel.UNLIMITED)
    val events = _events.receiveAsFlow()

    @Volatile
    var lastError: String? = null
        private set

    val isE2eeActive: Boolean
        get() = e2eeSession != null

    fun setE2eeSession(session: muxlane.crypto.E2eeSession?) {
        synchronized(txLock) {
            e2eeSession = session
        }
    }

    suspend fun connect(url: String, timeoutMs: Long = CONNECT_TIMEOUT_MS): CompletableDeferred<Throwable?> {
        close()
        lastError = null
        e2eeSession = null
        val current = Connection()
        connection = current
        current.socket = client.newWebSocket(
            HttpRequest.Builder().url(url).build(),
            object : WebSocketListener() {
                override fun onOpen(webSocket: WebSocket, response: HttpResponse) {
                    if (connection === current) current.opened.complete(Unit)
                }

                override fun onMessage(webSocket: WebSocket, text: String) {
                    if (connection === current) {
                        runCatching { handleLine(text) }
                            .onFailure { finish(current, it) }
                    }
                }

                override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                    webSocket.close(code, reason)
                }

                override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                    finish(current, IllegalStateException(reason.ifBlank { "连接已关闭 ($code)" }))
                }

                override fun onFailure(webSocket: WebSocket, t: Throwable, response: HttpResponse?) {
                    finish(current, t)
                }
            },
        )
        try {
            withTimeout(timeoutMs) { current.opened.await() }
        } catch (error: Throwable) {
            if (connection === current) close()
            throw error
        }
        return current.closed
    }

    fun close() {
        val current = connection ?: run {
            e2eeSession = null
            return
        }
        connection = null
        e2eeSession = null
        current.socket?.close(1000, "bye")
        val error = IllegalStateException("连接已断开")
        current.opened.completeExceptionally(error)
        current.closed.complete(null)
        failAll(error)
    }

    suspend fun call(method: String, params: kotlinx.serialization.json.JsonElement): Response {
        val current = connection ?: error("尚未连接")
        withTimeout(CONNECT_TIMEOUT_MS) { current.opened.await() }
        val id = nextId.getAndIncrement()
        val deferred = CompletableDeferred<Response>()
        pending[id] = deferred
        val rawLine = encodeFrame(Request(id, method, params))
        val sent = synchronized(txLock) {
            val outgoing = e2eeSession?.let { session ->
                val envelope = session.seal(rawLine.toByteArray(Charsets.UTF_8))
                muxlane.protocol.muxlaneJson.encodeToString(muxlane.crypto.E2eeEnvelope.serializer(), envelope)
            } ?: rawLine
            current.socket?.send(outgoing) == true
        }
        if (!sent) {
            pending.remove(id)
            error("尚未连接")
        }
        return try {
            withTimeout(CALL_TIMEOUT_MS) { deferred.await() }
        } catch (timeout: TimeoutCancellationException) {
            finish(current, timeout)
            throw timeout
        } finally {
            pending.remove(id)
        }
    }

    private fun handleLine(text: String) {
        val plaintext = e2eeSession?.let { session ->
            val env = muxlane.protocol.muxlaneJson.decodeFromString(muxlane.crypto.E2eeEnvelope.serializer(), text.trim())
            session.open(env).toString(Charsets.UTF_8)
        } ?: text
        when (val frame = decodeIncoming(plaintext)) {
            is IncomingFrame.Response -> pending.remove(frame.value.id)?.complete(frame.value)
            is IncomingFrame.Event -> _events.trySend(frame.value)
        }
    }

    private fun failAll(error: Throwable) {
        pending.values.forEach { it.completeExceptionally(error) }
        pending.clear()
    }

    private fun finish(current: Connection, error: Throwable) {
        if (connection !== current) return
        connection = null
        e2eeSession = null
        current.socket?.cancel()
        lastError = error.message
        current.opened.completeExceptionally(error)
        current.closed.complete(error)
        failAll(error)
    }

    private companion object {
        const val CONNECT_TIMEOUT_MS = 15_000L
        const val CALL_TIMEOUT_MS = 30_000L
    }
}

fun joinRelay(base: String, path: String): String {
    val trimmed = base.trim().trimEnd('/')
    return "$trimmed/$path"
}
