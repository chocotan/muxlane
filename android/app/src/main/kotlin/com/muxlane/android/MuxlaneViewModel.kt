package com.muxlane.android

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
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
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

data class UiState(
    val relayUrl: String = "ws://127.0.0.1:9843",
    val pairCode: String = "",
    val connecting: Boolean = false,
    val error: String? = null,
    val pairing: Pairing? = null,
    val snapshot: Snapshot? = null,
    val selectedAgent: String? = null,
    val presets: List<AgentPreset> = emptyList(),
    val spawnProject: String? = null,
    val confirmDelete: String? = null,
)

class MuxlaneViewModel(app: Application) : AndroidViewModel(app) {
    private val store = PrefsStore(app)
    private val client = RelayClient()
    val terminal = VirtualTerminal()
    private val _state = MutableStateFlow(UiState(pairing = store.load()))
    val state: StateFlow<UiState> = _state
    private var eventsJob: Job? = null
    private var termSub: String? = null
    private var replayId: Long? = null
    private var nextChunk = 0

    init {
        viewModelScope.launch { store.load()?.let { reconnect(it) } }
    }

    fun setRelay(url: String) {
        _state.value = _state.value.copy(relayUrl = url)
    }

    fun setCode(code: String) {
        _state.value = _state.value.copy(pairCode = code.filter { it.isDigit() }.take(8))
    }

    fun pair() {
        val url = _state.value.relayUrl.trim().trimEnd('/')
        val code = _state.value.pairCode
        if (url.isEmpty() || code.length != 8) {
            _state.value = _state.value.copy(
                error = when {
                    code.length != 8 -> "请填写桌面上的 8 位配对码"
                    else -> "请填写中继地址"
                },
            )
            return
        }
        viewModelScope.launch {
            _state.value = _state.value.copy(connecting = true, error = null)
            runCatching {
                client.connect(joinRelay(url, "pair/$code"))
                listenEvents()
                val device = android.os.Build.MODEL.replace(" ", "-").take(24).ifEmpty { "phone" }
                val response = client.call(
                    Methods.PAIR_BEGIN,
                    encodeParams(PairBeginParams.serializer(), PairBeginParams(code = code, device = device)),
                )
                val result = muxlaneJson.decodeFromJsonElement(
                    PairBeginResult.serializer(),
                    response.result ?: error(response.error?.message ?: "pair failed"),
                )
                val pairing = Pairing(url, result.machine.machineId, result.token, result.machine.name)
                store.save(pairing)
                _state.value = _state.value.copy(pairing = pairing, connecting = false)
                subscribeState()
            }.onFailure {
                _state.value = _state.value.copy(connecting = false, error = it.message)
            }
        }
    }

    fun reconnect(pairing: Pairing? = _state.value.pairing) {
        val target = pairing ?: return
        viewModelScope.launch {
            _state.value = _state.value.copy(connecting = true, error = null, pairing = target, relayUrl = target.relayUrl)
            runCatching {
                client.connect(joinRelay(target.relayUrl, "phone/${target.hostId}"))
                listenEvents()
                val response = client.call(
                    Methods.PAIR_BEGIN,
                    encodeParams(PairBeginParams.serializer(), PairBeginParams(token = target.token)),
                )
                response.error?.let { error(it.message) }
                val result = muxlaneJson.decodeFromJsonElement(
                    PairBeginResult.serializer(),
                    response.result ?: error("pair failed"),
                )
                val updated = target.copy(token = result.token, machineName = result.machine.name, hostId = result.machine.machineId)
                store.save(updated)
                _state.value = _state.value.copy(pairing = updated, connecting = false)
                subscribeState()
            }.onFailure {
                _state.value = _state.value.copy(connecting = false, error = it.message)
            }
        }
    }

    fun unpair() {
        store.clear()
        client.close()
        _state.value = UiState(relayUrl = _state.value.relayUrl)
    }

    fun openAgent(id: String) {
        _state.value = _state.value.copy(selectedAgent = id)
        viewModelScope.launch { subscribeTerm(id) }
    }

    fun closeAgent() {
        val sub = termSub
        termSub = null
        _state.value = _state.value.copy(selectedAgent = null)
        if (sub != null) {
            viewModelScope.launch {
                runCatching {
                    client.call(Methods.TERM_UNSUBSCRIBE, obj("sub_id" to sub))
                }
            }
        }
    }

    fun sendInput(text: String) {
        val agent = _state.value.selectedAgent ?: return
        viewModelScope.launch {
            runCatching {
                client.call(
                    Methods.TERM_INPUT,
                    encodeParams(TermInputParams.serializer(), TermInputParams(agent, b64Encode(text.toByteArray()))),
                )
            }
        }
    }

    fun resize(cols: Int, rows: Int) {
        terminal.resize(cols, rows)
        val agent = _state.value.selectedAgent ?: return
        viewModelScope.launch {
            runCatching {
                client.call(
                    Methods.TERM_RESIZE,
                    encodeParams(TermResizeParams.serializer(), TermResizeParams(agent, cols, rows)),
                )
            }
        }
    }

    fun loadPresets(project: String) {
        viewModelScope.launch {
            runCatching {
                val response = client.call(
                    Methods.PRESET_LIST,
                    obj("project" to project),
                )
                val presets = muxlaneJson.decodeFromJsonElement(
                    kotlinx.serialization.builtins.ListSerializer(AgentPreset.serializer()),
                    response.result ?: error(response.error?.message ?: "preset.list failed"),
                )
                _state.value = _state.value.copy(presets = presets, spawnProject = project)
            }.onFailure {
                _state.value = _state.value.copy(error = it.message)
            }
        }
    }

    fun spawn(preset: AgentPreset) {
        val project = _state.value.spawnProject ?: return
        viewModelScope.launch {
            runCatching {
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
                _state.value = _state.value.copy(spawnProject = null, presets = emptyList())
                refreshSnapshot()
            }.onFailure {
                _state.value = _state.value.copy(error = it.message)
            }
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
        viewModelScope.launch {
            runCatching {
                val response = client.call(Methods.AGENT_DELETE, obj("agent" to agent))
                response.error?.let { error(it.message) }
                _state.value = _state.value.copy(confirmDelete = null)
                if (_state.value.selectedAgent == agent) closeAgent()
                refreshSnapshot()
            }.onFailure {
                _state.value = _state.value.copy(error = it.message)
            }
        }
    }

    fun markSeen(agent: String) {
        viewModelScope.launch {
            runCatching {
                client.call(Methods.AGENT_MARK_SEEN, obj("agent" to agent))
            }
        }
    }

    private fun listenEvents() {
        eventsJob?.cancel()
        eventsJob = viewModelScope.launch {
            client.events.collect { event ->
                when (event.event) {
                    Events.STATE_CHANGED -> refreshSnapshot()
                    Events.AGENT_STATUS -> {
                        val status = muxlaneJson.decodeFromJsonElement(AgentStatusEvent.serializer(), event.params)
                        RelayService.notifyStatus(getApplication(), snapshotAgent(status.agent), status)
                        refreshSnapshot()
                    }
                    Events.TERM_DATA -> {
                        val data = muxlaneJson.decodeFromJsonElement(TermDataEvent.serializer(), event.params)
                        if (data.agent == _state.value.selectedAgent) {
                            terminal.write(b64Decode(data.dataB64))
                            _state.value = _state.value.copy()
                        }
                    }
                    Events.TERM_RESYNC -> {
                        val data = muxlaneJson.decodeFromJsonElement(TermDataEvent.serializer(), event.params)
                        if (data.agent == _state.value.selectedAgent) {
                            terminal.reset()
                            terminal.write(b64Decode(data.dataB64))
                            _state.value = _state.value.copy()
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
                            _state.value = _state.value.copy()
                        }
                    }
                    Events.TERM_EXIT -> {
                        if (_state.value.selectedAgent != null) refreshSnapshot()
                    }
                }
            }
        }
    }

    private suspend fun subscribeState() {
        client.call(Methods.EVENTS_SUBSCRIBE, JsonNull)
        refreshSnapshot()
    }

    private suspend fun refreshSnapshot() {
        val response = client.call(Methods.STATE_LIST, JsonNull)
        val snapshot = muxlaneJson.decodeFromJsonElement(
            Snapshot.serializer(),
            response.result ?: return,
        )
        _state.value = _state.value.copy(snapshot = snapshot)
    }

    private suspend fun subscribeTerm(agent: String) {
        terminal.reset()
        val response = client.call(
            Methods.TERM_SUBSCRIBE,
            encodeParams(TermSubscribeParams.serializer(), TermSubscribeParams(agent, true)),
        )
        val result = muxlaneJson.decodeFromJsonElement(
            TermSubscribeResult.serializer(),
            response.result ?: error(response.error?.message ?: "subscribe failed"),
        )
        termSub = result.subId
        replayId = null
        nextChunk = 0
        if (result.replayB64.isNotEmpty()) {
            terminal.write(b64Decode(result.replayB64))
        }
        markSeen(agent)
        _state.value = _state.value.copy()
    }

    private fun snapshotAgent(id: String): AgentInstance? =
        _state.value.snapshot?.agents?.find { it.id == id }
}

fun Project.displayName(): String = name.ifBlank { path.substringAfterLast('/') }

private fun obj(vararg pairs: Pair<String, String>): JsonObject =
    JsonObject(pairs.associate { it.first to JsonPrimitive(it.second) })
