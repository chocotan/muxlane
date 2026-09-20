package com.muxlane.android

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
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
    private var socket: WebSocket? = null
    private var opened: CompletableDeferred<Unit>? = null
    private val nextId = AtomicLong(1)
    private val pending = ConcurrentHashMap<Long, CompletableDeferred<Response>>()
    private val _events = MutableSharedFlow<EventMsg>(extraBufferCapacity = 64)
    val events: SharedFlow<EventMsg> = _events

    @Volatile
    var lastError: String? = null
        private set

    fun connect(url: String) {
        close()
        val ready = CompletableDeferred<Unit>()
        opened = ready
        socket = client.newWebSocket(
            HttpRequest.Builder().url(url).build(),
            object : WebSocketListener() {
                override fun onOpen(webSocket: WebSocket, response: HttpResponse) {
                    ready.complete(Unit)
                }

                override fun onMessage(webSocket: WebSocket, text: String) {
                    handleLine(text)
                }

                override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                    webSocket.close(code, reason)
                }

                override fun onFailure(webSocket: WebSocket, t: Throwable, response: HttpResponse?) {
                    lastError = t.message
                    ready.completeExceptionally(t)
                    failAll(t)
                }
            },
        )
    }

    fun close() {
        socket?.close(1000, "bye")
        socket = null
        opened = null
        failAll(IllegalStateException("disconnected"))
    }

    suspend fun call(method: String, params: kotlinx.serialization.json.JsonElement): Response {
        val ready = opened ?: error("not connected")
        ready.await()
        val id = nextId.getAndIncrement()
        val deferred = CompletableDeferred<Response>()
        pending[id] = deferred
        val sent = socket?.send(encodeFrame(Request(id, method, params))) == true
        if (!sent) {
            pending.remove(id)
            error("not connected")
        }
        return deferred.await()
    }

    private fun handleLine(text: String) {
        when (val frame = decodeIncoming(text)) {
            is IncomingFrame.Response -> pending.remove(frame.value.id)?.complete(frame.value)
            is IncomingFrame.Event -> _events.tryEmit(frame.value)
        }
    }

    private fun failAll(error: Throwable) {
        pending.values.forEach { it.completeExceptionally(error) }
        pending.clear()
    }
}

fun joinRelay(base: String, path: String): String {
    val trimmed = base.trim().trimEnd('/')
    return "$trimmed/$path"
}
