package com.muxlane.android

import android.app.Application
import android.content.Intent
import androidx.lifecycle.AndroidViewModel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import muxlane.protocol.AgentInstance
import muxlane.protocol.AgentPreset
import muxlane.protocol.AgentSpawnParams
import muxlane.protocol.AgentStatusEvent
import muxlane.protocol.Events
import muxlane.protocol.Methods
import muxlane.protocol.PairBeginParams
import muxlane.protocol.PairBeginResult
import muxlane.protocol.Project
import muxlane.protocol.Snapshot
import muxlane.protocol.TermDataEvent
import muxlane.protocol.TermInputParams
import muxlane.protocol.TermReplayChunkEvent
import muxlane.protocol.TermResizeParams
import muxlane.protocol.TermSubscribeParams
import muxlane.protocol.TermSubscribeResult
import muxlane.protocol.b64Decode
import muxlane.protocol.b64Encode
import muxlane.protocol.encodeParams
import muxlane.protocol.muxlaneJson
import muxlane.session.Pairing
import muxlane.term.VirtualTerminal
import kotlin.math.min

data class UiState(
    val relayUrl: String = "",
    val hostId: String = "",
    val connecting: Boolean = false,
    val connected: Boolean = false,
    val error: String? = null,
    val pairings: List<Pairing> = emptyList(),
    val pairing: Pairing? = null,
    val addingMachine: Boolean = false,
    val snapshot: Snapshot? = null,
    val selectedAgent: String? = null,
    val presets: List<AgentPreset> = emptyList(),
    val spawnProject: String? = null,
    val confirmDelete: String? = null,
    val confirmRemoveHost: String? = null,
)

class MuxlaneSession(private val app: Application) {
    private data class RemoteResize(val cols: Int, val rows: Int)

    private val store = PrefsStore(app)
    private val initialPairings = store.loadAll()
    private val initialPairing = store.activeHostId()?.let { id -> initialPairings.find { it.hostId == id } }
    private val client = RelayClient()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    val terminal = VirtualTerminal()
    private val _terminalRevision = MutableStateFlow(0L)
    val terminalRevision: StateFlow<Long> = _terminalRevision
    private val _state = MutableStateFlow(
        UiState(
            relayUrl = initialPairing?.relayUrl.orEmpty(),
            pairings = initialPairings,
            pairing = initialPairing,
        ),
    )
    val state: StateFlow<UiState> = _state
    private var connectionJob: Job? = null
    private var termSub: String? = null
    private var replayId: Long? = null
    private var nextChunk = 0
    private var lastRemoteResize: RemoteResize? = null

    init {
        scope.launch {
            client.events.collect { event ->
                runCatching {
                    when (event.event) {
                        Events.STATE_CHANGED -> refreshSnapshot()
                        Events.AGENT_STATUS -> {
                            val status = muxlaneJson.decodeFromJsonElement(AgentStatusEvent.serializer(), event.params)
                            _state.value.pairing?.let { pairing ->
                                RelayService.notifyStatus(app, pairing.hostId, snapshotAgent(status.agent), status)
                            }
                            refreshSnapshot()
                        }
                        Events.TERM_DATA -> {
                            val data = muxlaneJson.decodeFromJsonElement(TermDataEvent.serializer(), event.params)
                            if (data.agent == _state.value.selectedAgent) {
                                terminal.write(b64Decode(data.dataB64))
                                redrawTerminal()
                            }
                        }
                        Events.TERM_RESYNC -> {
                            val data = muxlaneJson.decodeFromJsonElement(TermDataEvent.serializer(), event.params)
                            if (data.agent == _state.value.selectedAgent) {
                                terminal.reset()
                                terminal.write(b64Decode(data.dataB64))
                                redrawTerminal()
                            }
                        }
                        Events.TERM_REPLAY_CHUNK -> {
                            val data = muxlaneJson.decodeFromJsonElement(TermReplayChunkEvent.serializer(), event.params)
                            if (data.agent == _state.value.selectedAgent && data.subId == termSub) {
                                if (replayId != data.replayId) {
                                    if (data.chunkIndex != 0) return@collect
                                    replayId = data.replayId
                                    nextChunk = 1
                                    terminal.reset()
                                    terminal.write(b64Decode(data.dataB64))
                                } else if (data.chunkIndex == nextChunk) {
                                    nextChunk += 1
                                    terminal.write(b64Decode(data.dataB64))
                                }
                                redrawTerminal()
                            }
                        }
                        Events.TERM_EXIT -> if (_state.value.selectedAgent != null) refreshSnapshot()
                    }
                }.onFailure { error ->
                    if (error !is CancellationException) setError(error)
                }
            }
        }
    }

    fun start() {
        if (_state.value.pairing == null) return
        app.startForegroundService(Intent(app, RelayService::class.java))
        ensureConnected()
    }

    fun ensureConnected() {
        if (_state.value.pairing == null || connectionJob?.isActive == true) return
        connectionJob = scope.launch { reconnectLoop() }
    }

    fun retryNow() {
        scope.launch {
            stopConnection()
            ensureConnected()
        }
    }

    fun beginAddMachine(relayUrl: String = "", hostId: String = "") {
        scope.launch {
            if (_state.value.pairing != null) stopConnection()
            store.setActiveHost(null)
            _state.value = _state.value.copy(
                relayUrl = relayUrl,
                hostId = hostId.trim(),
                pairing = null,
                addingMachine = true,
                snapshot = null,
                selectedAgent = null,
                error = null,
            )
            app.stopService(Intent(app, RelayService::class.java))
        }
    }

    fun cancelAddMachine() {
        _state.value = _state.value.copy(addingMachine = false, relayUrl = "", hostId = "", error = null)
    }

    fun selectMachine(hostId: String) {
        val target = _state.value.pairings.find { it.hostId == hostId } ?: return
        scope.launch {
            stopConnection()
            store.setActiveHost(hostId)
            _state.value = _state.value.copy(
                pairing = target,
                addingMachine = false,
                relayUrl = target.relayUrl,
                hostId = "",
                selectedAgent = null,
                error = null,
            )
            start()
        }
    }

    fun openNotification(hostId: String, agentId: String) {
        val target = _state.value.pairings.find { it.hostId == hostId } ?: return
        scope.launch {
            stopConnection()
            store.setActiveHost(hostId)
            _state.value = _state.value.copy(
                pairing = target,
                addingMachine = false,
                relayUrl = target.relayUrl,
                hostId = "",
                selectedAgent = agentId,
                error = null,
            )
            start()
        }
    }

    fun backToMachines() {
        scope.launch {
            stopConnection()
            store.setActiveHost(null)
            _state.value = _state.value.copy(
                pairing = null,
                addingMachine = false,
                snapshot = null,
                selectedAgent = null,
                presets = emptyList(),
                spawnProject = null,
                error = null,
            )
            app.stopService(Intent(app, RelayService::class.java))
        }
    }

    fun requestRemoveMachine(hostId: String) {
        _state.value = _state.value.copy(confirmRemoveHost = hostId)
    }

    fun cancelRemoveMachine() {
        _state.value = _state.value.copy(confirmRemoveHost = null)
    }

    fun confirmRemoveMachine() {
        val hostId = _state.value.confirmRemoveHost ?: return
        scope.launch {
            val removingActive = _state.value.pairing?.hostId == hostId
            if (removingActive) {
                stopConnection()
                terminal.reset()
                redrawTerminal()
            }
            store.remove(hostId)
            val pairings = store.loadAll()
            _state.value = _state.value.copy(
                pairings = pairings,
                pairing = if (removingActive) null else _state.value.pairing,
                snapshot = if (removingActive) null else _state.value.snapshot,
                selectedAgent = if (removingActive) null else _state.value.selectedAgent,
                confirmRemoveHost = null,
                error = null,
            )
            if (removingActive) app.stopService(Intent(app, RelayService::class.java))
        }
    }

    fun stop() {
        connectionJob?.cancel()
        connectionJob = null
        client.close()
        termSub = null
        lastRemoteResize = null
        _state.value = _state.value.copy(connecting = false, connected = false, snapshot = null)
    }

    fun setRelay(url: String) {
        _state.value = _state.value.copy(relayUrl = url, error = null)
    }

    fun setHostId(hostId: String) {
        _state.value = _state.value.copy(
            hostId = hostId.filter { it.isLetterOrDigit() || it == '-' || it == '_' || it == '.' }.take(64),
            error = null,
        )
    }

    fun pair() {
        val hostId = _state.value.hostId.trim()
        val relay = runCatching { normalizeRelayUrl(_state.value.relayUrl) }
            .getOrElse {
                _state.value = _state.value.copy(error = it.message)
                return
            }
        if (hostId.isEmpty()) {
            _state.value = _state.value.copy(error = "请填写电脑上的机器 ID")
            return
        }
        scope.launch {
            stopConnection()
            _state.value = _state.value.copy(connecting = true, connected = false, error = null, relayUrl = relay)
            runCatching {
                client.connect(joinRelay(relay, "phone/$hostId"))
                val device = android.os.Build.MODEL.replace(" ", "-").take(24).ifEmpty { "phone" }
                val response = client.call(
                    Methods.PAIR_BEGIN,
                    encodeParams(PairBeginParams.serializer(), PairBeginParams(hostId = hostId, device = device)),
                )
                val result = muxlaneJson.decodeFromJsonElement(
                    PairBeginResult.serializer(),
                    response.result ?: error(response.error?.message ?: "添加失败"),
                )
                val pairing = Pairing(relay, result.machine.machineId, result.token, result.machine.name)
                store.save(pairing)
                store.setActiveHost(pairing.hostId)
                client.close()
                _state.value = _state.value.copy(
                    pairings = store.loadAll(),
                    pairing = pairing,
                    addingMachine = false,
                    hostId = "",
                    connecting = false,
                    connected = false,
                    error = null,
                )
                start()
            }.onFailure {
                client.close()
                _state.value = _state.value.copy(connecting = false, connected = false, error = message(it))
            }
        }
    }

    fun unpair() {
        val hostId = _state.value.pairing?.hostId ?: return
        _state.value = _state.value.copy(confirmRemoveHost = hostId)
    }

    fun openAgent(id: String) {
        _state.value = _state.value.copy(selectedAgent = id, error = null)
        if (_state.value.connected) {
            scope.launch { runCatching { subscribeTerm(id) }.onFailure(::setError) }
        } else {
            start()
        }
    }

    fun closeAgent() {
        val sub = termSub
        termSub = null
        _state.value = _state.value.copy(selectedAgent = null)
        if (sub != null && _state.value.connected) {
            scope.launch { runCatching { client.call(Methods.TERM_UNSUBSCRIBE, obj("sub_id" to sub)) } }
        }
    }

    fun sendInput(text: String, onResult: (Boolean) -> Unit = {}): Boolean {
        return sendInputBytes(text.toByteArray(), onResult)
    }

    fun scrollTerminal(rows: Int) {
        val agent = _state.value.selectedAgent
        if (agent == null || !_state.value.connected || rows == 0) return
        val count = rows.coerceIn(-100, 100)
        val cursor = terminal.cursor
        sendInputBytes(
            buildString {
                repeat(kotlin.math.abs(count)) {
                    append(String(tmuxWheelReport(count > 0, cursor.col, cursor.row), Charsets.ISO_8859_1))
                }
            }.toByteArray(Charsets.ISO_8859_1),
        )
    }

    fun claimTerminalSize() {
        val agent = _state.value.selectedAgent ?: return
        if (!_state.value.connected) return
        scope.launch {
            runCatching { sendResizeIfNeeded(agent, terminal.cols, terminal.rows) }.onFailure(::setError)
        }
    }

    private fun sendInputBytes(data: ByteArray, onResult: (Boolean) -> Unit = {}): Boolean {
        val agent = _state.value.selectedAgent
        if (agent == null || !_state.value.connected) {
            onResult(false)
            return false
        }
        scope.launch {
            val result = runCatching {
                client.call(
                    Methods.TERM_INPUT,
                    encodeParams(TermInputParams.serializer(), TermInputParams(agent, b64Encode(data))),
                )
            }
            result.exceptionOrNull()?.let(::setError)
            onResult(result.isSuccess)
        }
        return true
    }

    fun resize(cols: Int, rows: Int) {
        if (cols != terminal.cols || rows != terminal.rows) {
            terminal.resize(cols, rows)
            redrawTerminal()
        }
        val agent = _state.value.selectedAgent ?: return
        if (!_state.value.connected) return
        scope.launch {
            runCatching { sendResizeIfNeeded(agent, cols, rows) }.onFailure(::setError)
        }
    }

    private suspend fun sendResizeIfNeeded(agent: String, cols: Int, rows: Int) {
        val size = RemoteResize(cols, rows)
        if (!_state.value.connected || _state.value.selectedAgent != agent || lastRemoteResize == size) return
        client.call(
            Methods.TERM_RESIZE,
            encodeParams(TermResizeParams.serializer(), TermResizeParams(agent, cols, rows)),
        ).error?.let { error(it.message) }
        if (_state.value.connected && _state.value.selectedAgent == agent) lastRemoteResize = size
    }

    fun clearTerminal() {
        terminal.reset()
        redrawTerminal()
    }

    fun loadPresets(project: String) {
        rpc {
            val response = client.call(Methods.PRESET_LIST, obj("project" to project))
            val presets = muxlaneJson.decodeFromJsonElement(
                kotlinx.serialization.builtins.ListSerializer(AgentPreset.serializer()),
                response.result ?: error(response.error?.message ?: "无法加载终端预设"),
            )
            _state.value = _state.value.copy(presets = presets, spawnProject = project, error = null)
        }
    }

    fun spawn(preset: AgentPreset) {
        val project = _state.value.spawnProject ?: return
        rpc {
            val response = client.call(
                Methods.AGENT_SPAWN,
                encodeParams(
                    AgentSpawnParams.serializer(),
                    AgentSpawnParams(
                        project = project,
                        agentType = preset.agentType,
                        program = preset.program.takeIf { preset.id != "shell" },
                        args = preset.args,
                        presetName = preset.label,
                    ),
                ),
            )
            response.error?.let { error(it.message) }
            _state.value = _state.value.copy(spawnProject = null, presets = emptyList(), error = null)
            refreshSnapshot()
        }
    }

    fun closeSpawn() {
        _state.value = _state.value.copy(spawnProject = null, presets = emptyList())
    }

    fun requestDelete(agent: String) {
        _state.value = _state.value.copy(confirmDelete = agent)
    }

    fun cancelDelete() {
        _state.value = _state.value.copy(confirmDelete = null)
    }

    fun confirmDelete() {
        val agent = _state.value.confirmDelete ?: return
        rpc {
            val response = client.call(Methods.AGENT_DELETE, obj("agent" to agent))
            response.error?.let { error(it.message) }
            _state.value = _state.value.copy(confirmDelete = null, error = null)
            if (_state.value.selectedAgent == agent) closeAgent()
            refreshSnapshot()
        }
    }

    fun markSeen(agent: String) {
        if (!_state.value.connected) return
        scope.launch { runCatching { client.call(Methods.AGENT_MARK_SEEN, obj("agent" to agent)) } }
    }

    private suspend fun reconnectLoop() {
        var waitMs = 1_000L
        while (scope.isActive) {
            val target = _state.value.pairing ?: break
            _state.value = _state.value.copy(connecting = true, connected = false, snapshot = null)
            try {
                val closed = client.connect(joinRelay(target.relayUrl, "phone/${target.hostId}"))
                val response = client.call(
                    Methods.PAIR_BEGIN,
                    encodeParams(PairBeginParams.serializer(), PairBeginParams(token = target.token)),
                )
                response.error?.let { error(it.message) }
                val result = muxlaneJson.decodeFromJsonElement(
                    PairBeginResult.serializer(),
                    response.result ?: error("认证失败"),
                )
                val updated = target.copy(
                    token = result.token,
                    machineName = result.machine.name,
                    hostId = result.machine.machineId,
                )
                store.save(updated)
                _state.value = _state.value.copy(
                    pairings = store.loadAll(),
                    pairing = updated,
                    relayUrl = updated.relayUrl,
                    connecting = false,
                    connected = true,
                    error = null,
                )
                client.call(Methods.EVENTS_SUBSCRIBE, JsonNull).error?.let { error(it.message) }
                refreshSnapshot()
                _state.value.selectedAgent?.let { subscribeTerm(it) }
                waitMs = 1_000L
                val cause = closed.await()
                if (cause != null) throw cause
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Throwable) {
                client.close()
                _state.value = _state.value.copy(
                    connecting = false,
                    connected = false,
                    snapshot = null,
                    error = message(error),
                )
            }
            if (_state.value.pairing == null) break
            delay(waitMs)
            waitMs = min(waitMs * 2, 30_000L)
        }
    }

    private suspend fun stopConnection() {
        val job = connectionJob
        connectionJob = null
        job?.cancelAndJoin()
        client.close()
        termSub = null
        _state.value = _state.value.copy(connecting = false, connected = false, snapshot = null)
    }

    private suspend fun refreshSnapshot() {
        val response = client.call(Methods.STATE_LIST, JsonNull)
        val snapshot = muxlaneJson.decodeFromJsonElement(
            Snapshot.serializer(),
            response.result ?: error(response.error?.message ?: "无法读取会话状态"),
        )
        _state.value = _state.value.copy(snapshot = snapshot, error = null)
    }

    private suspend fun subscribeTerm(agent: String) {
        termSub?.let { old -> runCatching { client.call(Methods.TERM_UNSUBSCRIBE, obj("sub_id" to old)) } }
        termSub = null
        lastRemoteResize = null
        terminal.reset()
        redrawTerminal()
        val response = client.call(
            Methods.TERM_SUBSCRIBE,
            encodeParams(TermSubscribeParams.serializer(), TermSubscribeParams(agent, true)),
        )
        val result = muxlaneJson.decodeFromJsonElement(
            TermSubscribeResult.serializer(),
            response.result ?: error(response.error?.message ?: "无法订阅终端"),
        )
        if (_state.value.selectedAgent != agent) {
            client.call(Methods.TERM_UNSUBSCRIBE, obj("sub_id" to result.subId))
            return
        }
        termSub = result.subId
        replayId = null
        nextChunk = 0
        if (result.replayB64.isNotEmpty()) terminal.write(b64Decode(result.replayB64))
        // Pocket Studio sends the current emulator size once the stream is ready.
        // This also covers a resize that happened while disconnected.
        sendResizeIfNeeded(agent, terminal.cols, terminal.rows)
        markSeen(agent)
        redrawTerminal()
    }

    private fun rpc(block: suspend () -> Unit) {
        if (!_state.value.connected) {
            _state.value = _state.value.copy(error = "连接尚未恢复")
            start()
            return
        }
        scope.launch { runCatching { block() }.onFailure(::setError) }
    }

    private fun redrawTerminal() {
        _terminalRevision.value += 1
    }

    private fun setError(error: Throwable) {
        if (error !is CancellationException) _state.value = _state.value.copy(error = message(error))
    }

    private fun snapshotAgent(id: String): AgentInstance? = _state.value.snapshot?.agents?.find { it.id == id }

    private fun message(error: Throwable): String = error.message?.takeIf { it.isNotBlank() } ?: "连接失败"
}

class MuxlaneViewModel(app: Application) : AndroidViewModel(app) {
    private val session = (app as MuxlaneApp).session
    val terminal = session.terminal
    val terminalRevision = session.terminalRevision
    val state = session.state

    init {
        session.start()
    }

    fun setRelay(url: String) = session.setRelay(url)
    fun setHostId(hostId: String) = session.setHostId(hostId)
    fun pair() = session.pair()
    fun reconnect() = session.retryNow()
    fun beginAddMachine(relayUrl: String = "", hostId: String = "") = session.beginAddMachine(relayUrl, hostId)
    fun cancelAddMachine() = session.cancelAddMachine()
    fun selectMachine(hostId: String) = session.selectMachine(hostId)
    fun openNotification(hostId: String, agentId: String) = session.openNotification(hostId, agentId)
    fun backToMachines() = session.backToMachines()
    fun requestRemoveMachine(hostId: String) = session.requestRemoveMachine(hostId)
    fun cancelRemoveMachine() = session.cancelRemoveMachine()
    fun confirmRemoveMachine() = session.confirmRemoveMachine()
    fun unpair() = session.unpair()
    fun openAgent(id: String) = session.openAgent(id)
    fun closeAgent() = session.closeAgent()
    fun sendInput(text: String, onResult: (Boolean) -> Unit = {}): Boolean = session.sendInput(text, onResult)
    fun scrollTerminal(rows: Int) = session.scrollTerminal(rows)
    fun claimTerminalSize() = session.claimTerminalSize()
    fun resize(cols: Int, rows: Int) = session.resize(cols, rows)
    fun clearTerminal() = session.clearTerminal()
    fun loadPresets(project: String) = session.loadPresets(project)
    fun spawn(preset: AgentPreset) = session.spawn(preset)
    fun closeSpawn() = session.closeSpawn()
    fun requestDelete(agent: String) = session.requestDelete(agent)
    fun cancelDelete() = session.cancelDelete()
    fun confirmDelete() = session.confirmDelete()
    fun markSeen(agent: String) = session.markSeen(agent)
}

fun Project.displayName(): String = name.ifBlank { path.substringAfterLast('/') }

/** Legacy X10 wheel reports are understood by tmux with mouse mode enabled. */
internal fun tmuxWheelReport(up: Boolean, col: Int, row: Int): ByteArray {
    val button = if (up) 64 else 65
    return byteArrayOf(
        0x1B,
        '['.code.toByte(),
        'M'.code.toByte(),
        (32 + button).toByte(),
        (33 + col.coerceIn(0, 222)).toByte(),
        (33 + row.coerceIn(0, 222)).toByte(),
    )
}

private fun obj(vararg pairs: Pair<String, String>): JsonObject =
    JsonObject(pairs.associate { it.first to JsonPrimitive(it.second) })
