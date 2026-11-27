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
import muxlane.chat.ApprovalReply
import muxlane.chat.ChatApproveParams
import muxlane.chat.ChatApprovalNoticeEvent
import muxlane.chat.ChatConfigureParams
import muxlane.chat.ChatDeltaEvent
import muxlane.chat.ChatDeltaOp
import muxlane.chat.ChatDriverKind
import muxlane.chat.ChatHistoryResult
import muxlane.chat.ChatListResult
import muxlane.chat.ChatQueueDeleteParams
import muxlane.chat.ChatSendParams
import muxlane.chat.ChatSessionSummary
import muxlane.chat.ChatSpawnParams
import muxlane.chat.ChatSubscribeParams
import muxlane.chat.ChatSubscribeResult
import muxlane.chat.ChatSwitchModeParams
import muxlane.chat.ChatTimelineSnapshot
import muxlane.chat.PendingApproval
import muxlane.chat.SessionDriverMode
import muxlane.chat.StreamingBehavior
import muxlane.crypto.E2eeKeyPair
import muxlane.crypto.E2eeSession
import muxlane.crypto.E2eeUpgradeParams
import muxlane.crypto.E2eeUpgradeResult
import muxlane.protocol.AgentInstance
import muxlane.protocol.AgentPreset
import muxlane.protocol.AgentSpawnParams
import muxlane.protocol.AgentStatusEvent
import muxlane.protocol.AgentType
import muxlane.protocol.BotDefinitionSummary
import muxlane.protocol.BotListResult
import muxlane.protocol.BotRunRecord
import muxlane.protocol.BotRunsParams
import muxlane.protocol.BotRunsResult
import muxlane.protocol.BotTriggerParams
import muxlane.protocol.Events
import muxlane.protocol.Features
import muxlane.protocol.HelloResult
import muxlane.protocol.Methods
import muxlane.protocol.PairBeginParams
import muxlane.protocol.PairBeginResult
import muxlane.protocol.Project
import muxlane.protocol.Snapshot
import muxlane.protocol.TermDataEvent
import muxlane.protocol.TermInputParams
import muxlane.protocol.TermReplayChunkEvent
import muxlane.protocol.TermResizeParams
import muxlane.protocol.TermResyncEvent
import muxlane.protocol.TermSubscribeParams
import muxlane.protocol.TermSubscribeResult
import muxlane.protocol.b64Decode
import muxlane.protocol.b64Encode
import muxlane.protocol.encodeParams
import muxlane.protocol.muxlaneJson
import muxlane.session.Pairing
import muxlane.term.VirtualTerminal
import kotlin.math.min

enum class SessionPaneMode { CHAT, TERMINAL }

data class UiState(
    val relayUrl: String = "",
    val hostId: String = "",
    val connecting: Boolean = false,
    val connected: Boolean = false,
    val e2eeActive: Boolean = false,
    val usingLan: Boolean = false,
    val error: String? = null,
    val pairings: List<Pairing> = emptyList(),
    val pairing: Pairing? = null,
    val addingMachine: Boolean = false,
    val snapshot: Snapshot? = null,
    val selectedAgent: String? = null,
    val openTabs: List<String> = emptyList(),
    val sessionMode: SessionPaneMode = SessionPaneMode.TERMINAL,
    val hubTab: HubTab = HubTab.FEED,
    val workspaceQuery: String = "",
    val agentGroupBy: AgentGroupBy = AgentGroupBy.STATUS,
    val agentsNeedsYouOnly: Boolean = false,
    val collapsedProjects: Set<String> = emptySet(),
    val terminalTextScale: Float = 1.0f,
    val chatTextScale: Float = 1.0f,
    val chatSessions: Map<String, ChatSessionSummary> = emptyMap(),
    val chatTimelines: Map<String, ChatTimelineSnapshot> = emptyMap(),
    val globalApprovals: List<PendingApproval> = emptyList(),
    val bots: List<BotDefinitionSummary> = emptyList(),
    val botRuns: List<BotRunRecord> = emptyList(),
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
    private var chatSub: String? = null
    private var replayId: Long? = null
    private var nextChunk = 0
    private var lastRemoteResize: RemoteResize? = null

    init {
        scope.launch {
            client.events.collect { event ->
                runCatching {
                    when (event.event) {
                        Events.STATE_CHANGED -> {
                            refreshSnapshot()
                            refreshChatList()
                            refreshBots()
                        }
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
                            val data = muxlaneJson.decodeFromJsonElement(TermResyncEvent.serializer(), event.params)
                            if (data.agent == _state.value.selectedAgent) {
                                terminal.reset()
                                terminal.write(b64Decode(data.replayB64))
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
                        Events.CHAT_DELTA -> {
                            val delta = muxlaneJson.decodeFromJsonElement(ChatDeltaEvent.serializer(), event.params)
                            applyChatDeltaEvent(delta)
                        }
                        Events.CHAT_APPROVAL -> {
                            val notice = muxlaneJson.decodeFromJsonElement(ChatApprovalNoticeEvent.serializer(), event.params)
                            handleChatApprovalNotice(notice)
                        }
                        Events.BOT_UPDATED, Events.BOT_RUN_UPDATED -> refreshBots()
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
                openTabs = emptyList(),
                chatSessions = emptyMap(),
                chatTimelines = emptyMap(),
                globalApprovals = emptyList(),
                e2eeActive = false,
                usingLan = false,
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
                openTabs = emptyList(),
                chatSessions = emptyMap(),
                chatTimelines = emptyMap(),
                globalApprovals = emptyList(),
                error = null,
            )
            start()
        }
    }

    fun openNotification(hostId: String, agentId: String) {
        val target = _state.value.pairings.find { it.hostId == hostId } ?: return
        val currentlyConnectedSameHost = _state.value.pairing?.hostId == hostId && _state.value.connected
        if (currentlyConnectedSameHost) {
            openAgent(agentId)
            return
        }
        scope.launch {
            stopConnection()
            store.setActiveHost(hostId)
            _state.value = _state.value.copy(
                pairing = target,
                addingMachine = false,
                relayUrl = target.relayUrl,
                hostId = "",
                selectedAgent = agentId,
                openTabs = appendOrFocusTab(emptyList(), agentId),
                sessionMode = SessionPaneMode.CHAT,
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
                openTabs = emptyList(),
                chatSessions = emptyMap(),
                chatTimelines = emptyMap(),
                globalApprovals = emptyList(),
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
                openTabs = if (removingActive) emptyList() else _state.value.openTabs,
                chatSessions = if (removingActive) emptyMap() else _state.value.chatSessions,
                chatTimelines = if (removingActive) emptyMap() else _state.value.chatTimelines,
                globalApprovals = if (removingActive) emptyList() else _state.value.globalApprovals,
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
        chatSub = null
        lastRemoteResize = null
        _state.value = _state.value.copy(
            connecting = false,
            connected = false,
            e2eeActive = false,
            usingLan = false,
            snapshot = null,
        )
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
            _state.value = _state.value.copy(error = "缺少机器 ID")
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
                val pairing = Pairing(
                    relayUrl = relay,
                    hostId = result.machine.machineId,
                    token = result.token,
                    machineName = result.machine.name,
                    lanUrl = result.lanWsUrl,
                )
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
                _state.value = _state.value.copy(
                    connecting = false,
                    connected = false,
                    error = when {
                        message(it) == "尚未连接" -> "中继未连接"
                        else -> message(it)
                    },
                )
            }
        }
    }

    fun unpair() {
        val hostId = _state.value.pairing?.hostId ?: return
        _state.value = _state.value.copy(confirmRemoveHost = hostId)
    }

    fun isChatCapable(agentId: String): Boolean {
        if (_state.value.chatSessions.containsKey(agentId)) return true
        val type = snapshotAgent(agentId)?.agentType ?: return false
        return type != AgentType.SHELL && type != AgentType.UNKNOWN
    }

    fun openAgent(id: String) {
        val mode = if (isChatCapable(id)) SessionPaneMode.CHAT else SessionPaneMode.TERMINAL
        val nextTabs = appendOrFocusTab(_state.value.openTabs, id)
        _state.value = _state.value.copy(
            selectedAgent = id,
            openTabs = nextTabs,
            sessionMode = mode,
            error = null,
        )
        if (_state.value.connected) {
            scope.launch {
                runCatching {
                    if (mode == SessionPaneMode.CHAT) {
                        unsubscribeTermLocked()
                        subscribeChat(id)
                    } else {
                        unsubscribeChatLocked()
                        subscribeTerm(id)
                    }
                }.onFailure(::setError)
            }
        } else {
            start()
        }
    }

    fun stepTab(delta: Int) {
        val nextId = stepTabInOrder(_state.value.openTabs, _state.value.selectedAgent, delta) ?: return
        if (nextId != _state.value.selectedAgent) openAgent(nextId)
    }

    fun closeTab(agentId: String) {
        val closingActive = _state.value.selectedAgent == agentId
        val (nextTabs, nextSelected) = removeTabAndNextSelection(
            openTabs = _state.value.openTabs,
            closingId = agentId,
            currentSelected = _state.value.selectedAgent,
        )
        if (!closingActive) {
            _state.value = _state.value.copy(openTabs = nextTabs)
            return
        }
        val oldTerm = termSub
        val oldChat = chatSub
        termSub = null
        chatSub = null
        _state.value = _state.value.copy(openTabs = nextTabs, selectedAgent = null)
        if (_state.value.connected) {
            scope.launch {
                if (oldTerm != null) runCatching { client.call(Methods.TERM_UNSUBSCRIBE, obj("sub_id" to oldTerm)) }
                if (oldChat != null) runCatching { client.call(Methods.CHAT_UNSUBSCRIBE, obj("sub_id" to oldChat)) }
                if (nextSelected != null) openAgent(nextSelected)
            }
        } else if (nextSelected != null) {
            openAgent(nextSelected)
        }
    }

    fun setHubTab(tab: HubTab) {
        _state.value = _state.value.copy(hubTab = tab)
    }

    fun setWorkspaceQuery(query: String) {
        _state.value = _state.value.copy(workspaceQuery = query)
    }

    fun setAgentGroupBy(groupBy: AgentGroupBy) {
        _state.value = _state.value.copy(agentGroupBy = groupBy)
    }

    fun toggleAgentsNeedsYouOnly() {
        _state.value = _state.value.copy(agentsNeedsYouOnly = !_state.value.agentsNeedsYouOnly)
    }

    fun toggleProjectCollapsed(projectId: String) {
        val current = _state.value.collapsedProjects
        val next = if (projectId in current) current - projectId else current + projectId
        _state.value = _state.value.copy(collapsedProjects = next)
    }

    fun setTerminalTextScale(scale: Float) {
        _state.value = _state.value.copy(terminalTextScale = scale.coerceIn(0.7f, 1.65f))
    }

    fun setChatTextScale(scale: Float) {
        _state.value = _state.value.copy(chatTextScale = scale.coerceIn(0.85f, 1.4f))
    }

    fun setSessionMode(mode: SessionPaneMode) {
        val agent = _state.value.selectedAgent ?: return
        if (_state.value.sessionMode == mode) return
        val meta = _state.value.chatTimelines[agent]?.meta ?: _state.value.chatSessions[agent]?.meta
        if (mode == SessionPaneMode.CHAT && !isChatCapable(agent)) return
        if (mode == SessionPaneMode.TERMINAL && meta != null && !meta.hasPty && !meta.supportsTerminalSwitch) {
            _state.value = _state.value.copy(error = "当前会话未开启交互终端")
            return
        }
        _state.value = _state.value.copy(sessionMode = mode, error = null)
        if (!_state.value.connected) return
        scope.launch {
            runCatching {
                when (mode) {
                    SessionPaneMode.TERMINAL -> {
                        unsubscribeChatLocked()
                        if (meta != null && meta.driver != SessionDriverMode.TERMINAL_PTY && meta.supportsTerminalSwitch) {
                            val resp = client.call(
                                Methods.CHAT_SWITCH_MODE,
                                encodeParams(ChatSwitchModeParams.serializer(), ChatSwitchModeParams(agent, SessionDriverMode.TERMINAL_PTY)),
                            )
                            resp.error?.let { error(it.message) }
                        }
                        subscribeTerm(agent)
                    }
                    SessionPaneMode.CHAT -> {
                        unsubscribeTermLocked()
                        // Orca-style Dual-Lane Bridge: never kill/restart a live PTY on view toggle.
                        // `chat.subscribe` attaches directly to the live PTY Bridge or structured driver.
                        subscribeChat(agent)
                    }
                }
            }.onFailure(::setError)
        }
    }

    fun closeAgent() {
        val oldTerm = termSub
        val oldChat = chatSub
        termSub = null
        chatSub = null
        _state.value = _state.value.copy(selectedAgent = null)
        if (_state.value.connected) {
            scope.launch {
                if (oldTerm != null) runCatching { client.call(Methods.TERM_UNSUBSCRIBE, obj("sub_id" to oldTerm)) }
                if (oldChat != null) runCatching { client.call(Methods.CHAT_UNSUBSCRIBE, obj("sub_id" to oldChat)) }
            }
        }
    }

    fun sendChatPrompt(text: String, steer: Boolean = true) {
        val agent = _state.value.selectedAgent ?: return
        val trimmed = text.trim()
        if (trimmed.isEmpty()) return
        val streaming = _state.value.chatTimelines[agent]?.meta?.isStreaming == true
        val behavior = if (streaming) {
            if (steer) StreamingBehavior.STEER else StreamingBehavior.FOLLOW_UP
        } else {
            null
        }
        rpc {
            val resp = client.call(
                Methods.CHAT_SEND,
                encodeParams(
                    ChatSendParams.serializer(),
                    ChatSendParams(
                        agent = agent,
                        text = trimmed,
                        streamingBehavior = behavior,
                    ),
                ),
            )
            resp.error?.let { error(it.message) }
        }
    }

    fun deleteQueuedMessage(agent: String, messageId: String) {
        rpc {
            val resp = client.call(
                Methods.CHAT_QUEUE_DELETE,
                encodeParams(
                    ChatQueueDeleteParams.serializer(),
                    ChatQueueDeleteParams(agent = agent, messageId = messageId),
                ),
            )
            resp.error?.let { error(it.message) }
        }
    }

    fun abortChat() {
        val agent = _state.value.selectedAgent ?: return
        rpc {
            client.call(Methods.CHAT_ABORT, obj("agent" to agent)).error?.let { error(it.message) }
        }
    }

    fun configureChat(provider: String? = null, modelId: String? = null, thinkingLevel: String? = null) {
        val agent = _state.value.selectedAgent ?: return
        rpc {
            client.call(
                Methods.CHAT_CONFIGURE,
                encodeParams(
                    ChatConfigureParams.serializer(),
                    ChatConfigureParams(
                        agent = agent,
                        provider = provider,
                        modelId = modelId,
                        thinkingLevel = thinkingLevel,
                    ),
                ),
            ).error?.let { error(it.message) }
        }
    }

    fun respondApproval(agent: String, approvalId: String, reply: ApprovalReply) {
        scope.launch {
            if (!_state.value.connected) {
                setError(IllegalStateException("连接未就绪，请稍后再试"))
                return@launch
            }
            runCatching {
                val resp = client.call(
                    Methods.CHAT_APPROVE,
                    encodeParams(ChatApproveParams.serializer(), ChatApproveParams(agent, approvalId, reply)),
                )
                resp.error?.let { error(it.message) }
                removeApprovalLocal(agent, approvalId)
                RelayService.cancelApprovalNotification(app, approvalId)
            }.onFailure(::setError)
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
        if (!_state.value.connected || _state.value.sessionMode != SessionPaneMode.TERMINAL) return
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
            val spawned = response.result?.let {
                runCatching { muxlaneJson.decodeFromJsonElement(AgentInstance.serializer(), it) }.getOrNull()
            }
            _state.value = _state.value.copy(spawnProject = null, presets = emptyList(), error = null)
            refreshSnapshot()
            if (spawned != null) openAgent(spawned.id)
        }
    }

    fun spawnChat(preset: AgentPreset, initialPrompt: String? = null) {
        val project = _state.value.spawnProject ?: return
        val driver = presetChatDriver(preset.agentType)
        val prompt = initialPrompt?.trim()?.takeIf { it.isNotEmpty() }
        rpc {
            val response = if (driver != null) {
                client.call(
                    Methods.CHAT_SPAWN,
                    encodeParams(
                        ChatSpawnParams.serializer(),
                        ChatSpawnParams(
                            project = project,
                            agentType = preset.agentType,
                            program = preset.program,
                            args = preset.args,
                            presetName = preset.label,
                            driver = driver,
                            initialPrompt = prompt,
                        ),
                    ),
                )
            } else {
                client.call(
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
            }
            val spawned = muxlaneJson.decodeFromJsonElement(
                AgentInstance.serializer(),
                response.result ?: error(response.error?.message ?: "无法创建对话会话"),
            )
            _state.value = _state.value.copy(spawnProject = null, presets = emptyList(), error = null)
            refreshSnapshot()
            refreshChatList()
            openAgent(spawned.id)
            if (driver == null && prompt != null) {
                // For PTY dual-lane agents (Claude/Codex), dispatch optional initial prompt through chat.send.
                runCatching {
                    client.call(
                        Methods.CHAT_SEND,
                        encodeParams(
                            ChatSendParams.serializer(),
                            ChatSendParams(agent = spawned.id, text = prompt),
                        ),
                    )
                }
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
        rpc {
            val response = client.call(Methods.AGENT_DELETE, obj("agent" to agent))
            response.error?.let { error(it.message) }
            val closingActive = _state.value.selectedAgent == agent
            val (nextTabs, nextSelected) = removeTabAndNextSelection(
                openTabs = _state.value.openTabs,
                closingId = agent,
                currentSelected = _state.value.selectedAgent,
            )
            _state.value = _state.value.copy(
                confirmDelete = null,
                openTabs = nextTabs,
                chatTimelines = _state.value.chatTimelines - agent,
                chatSessions = _state.value.chatSessions - agent,
                globalApprovals = _state.value.globalApprovals.filterNot { it.agent == agent },
                error = null,
            )
            if (closingActive) {
                closeAgent()
                if (nextSelected != null) openAgent(nextSelected)
            }
            refreshSnapshot()
            refreshChatList()
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
            _state.value = _state.value.copy(
                connecting = true,
                connected = false,
                e2eeActive = false,
                usingLan = false,
                snapshot = null,
            )
            try {
                var usedLan = false
                val closed = target.lanUrl?.takeIf { it.isNotBlank() }?.let { lanUrl ->
                    runCatching {
                        val c = client.connect(lanUrl, timeoutMs = 1_500L)
                        usedLan = true
                        c
                    }.getOrNull()
                } ?: client.connect(joinRelay(target.relayUrl, "phone/${target.hostId}"))

                val response = client.call(
                    Methods.PAIR_BEGIN,
                    encodeParams(PairBeginParams.serializer(), PairBeginParams(token = target.token)),
                )
                response.error?.let { error(it.message) }
                val result = muxlaneJson.decodeFromJsonElement(
                    PairBeginResult.serializer(),
                    response.result ?: error("认证失败"),
                )

                val helloResp = runCatching { client.call(Methods.SYSTEM_HELLO, JsonObject(emptyMap())) }.getOrNull()
                val hello = helloResp?.result?.let {
                    runCatching { muxlaneJson.decodeFromJsonElement(HelloResult.serializer(), it) }.getOrNull()
                }

                var e2eeActive = false
                if (hello?.features?.contains(Features.E2EE) == true) {
                    val kp = E2eeKeyPair.generate()
                    val clientNonce = E2eeKeyPair.randomNonce16()
                    val upResp = client.call(
                        Methods.E2EE_UPGRADE,
                        encodeParams(
                            E2eeUpgradeParams.serializer(),
                            E2eeUpgradeParams(
                                client_pubkey_b64 = b64Encode(kp.publicSec1),
                                client_nonce_b64 = b64Encode(clientNonce),
                            ),
                        ),
                    )
                    upResp.error?.let { error(it.message) }
                    val upRes = muxlaneJson.decodeFromJsonElement(
                        E2eeUpgradeResult.serializer(),
                        upResp.result ?: error("E2EE 握手失败"),
                    )
                    val e2eeSession = E2eeSession.deriveClient(
                        localKeys = kp,
                        serverPubSec1 = b64Decode(upRes.server_pubkey_b64),
                        pairingToken = result.token,
                        clientNonce = clientNonce,
                        serverNonce = b64Decode(upRes.server_nonce_b64),
                    )
                    client.setE2eeSession(e2eeSession)
                    e2eeActive = true
                }

                val updated = target.copy(
                    token = result.token,
                    machineName = result.machine.name,
                    hostId = result.machine.machineId,
                    lanUrl = result.lanWsUrl ?: hello?.lanWsUrl ?: target.lanUrl,
                )
                store.save(updated)
                _state.value = _state.value.copy(
                    pairings = store.loadAll(),
                    pairing = updated,
                    relayUrl = updated.relayUrl,
                    connecting = false,
                    connected = true,
                    usingLan = usedLan,
                    e2eeActive = e2eeActive,
                    error = null,
                )
                client.call(Methods.EVENTS_SUBSCRIBE, JsonNull).error?.let { error(it.message) }
                refreshSnapshot()
                refreshChatList()
                refreshBots()
                _state.value.selectedAgent?.let { agent ->
                    val mode = if (isChatCapable(agent)) SessionPaneMode.CHAT else SessionPaneMode.TERMINAL
                    _state.value = _state.value.copy(sessionMode = mode)
                    if (mode == SessionPaneMode.CHAT) subscribeChat(agent) else subscribeTerm(agent)
                }
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
                    e2eeActive = false,
                    usingLan = false,
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
        chatSub = null
        _state.value = _state.value.copy(
            connecting = false,
            connected = false,
            e2eeActive = false,
            usingLan = false,
            snapshot = null,
        )
    }

    private suspend fun refreshSnapshot() {
        val response = client.call(Methods.STATE_LIST, JsonNull)
        val snapshot = muxlaneJson.decodeFromJsonElement(
            Snapshot.serializer(),
            response.result ?: error(response.error?.message ?: "无法读取会话状态"),
        )
        val prunedTabs = pruneStaleTabs(_state.value.openTabs, snapshot.agents)
        val currentSelected = _state.value.selectedAgent
        val nextSelected = if (currentSelected != null && snapshot.agents.any { it.id == currentSelected }) {
            currentSelected
        } else {
            null
        }
        _state.value = _state.value.copy(
            snapshot = snapshot,
            openTabs = prunedTabs,
            selectedAgent = nextSelected,
            error = null,
        )
    }

    private suspend fun refreshChatList() {
        val resp = runCatching { client.call(Methods.CHAT_LIST, JsonObject(emptyMap())) }.getOrNull() ?: return
        val result = resp.result?.let {
            runCatching { muxlaneJson.decodeFromJsonElement(ChatListResult.serializer(), it) }.getOrNull()
        } ?: return
        val sessions = result.sessions.associateBy { it.agent }
        val collectedApprovals = ArrayList<PendingApproval>()
        result.sessions.filter { it.pendingApprovals > 0 }.forEach { summary ->
            val histResp = runCatching {
                client.call(Methods.CHAT_HISTORY, obj("agent" to summary.agent))
            }.getOrNull()
            val hist = histResp?.result?.let {
                runCatching { muxlaneJson.decodeFromJsonElement(ChatHistoryResult.serializer(), it) }.getOrNull()
            }
            if (hist != null) collectedApprovals.addAll(hist.pendingApprovals)
        }
        _state.value = _state.value.copy(
            chatSessions = sessions,
            globalApprovals = collectedApprovals.distinctBy { it.id },
        )
    }

    private suspend fun refreshBots() {
        val botsResp = runCatching { client.call(Methods.BOT_LIST, JsonObject(emptyMap())) }.getOrNull() ?: return
        val bots = botsResp.result?.let {
            runCatching { muxlaneJson.decodeFromJsonElement(BotListResult.serializer(), it) }.getOrNull()
        }?.bots ?: return
        val runsResp = runCatching {
            client.call(Methods.BOT_RUNS, encodeParams(BotRunsParams.serializer(), BotRunsParams(limit = 30)))
        }.getOrNull()
        val runs = runsResp?.result?.let {
            runCatching { muxlaneJson.decodeFromJsonElement(BotRunsResult.serializer(), it) }.getOrNull()
        }?.runs ?: _state.value.botRuns
        _state.value = _state.value.copy(bots = bots, botRuns = runs)
    }

    fun triggerBot(botId: String) {
        rpc {
            val resp = client.call(
                Methods.BOT_TRIGGER,
                encodeParams(BotTriggerParams.serializer(), BotTriggerParams(botId = botId, source = "manual")),
            )
            resp.error?.let { error(it.message) }
            refreshBots()
        }
    }

    private suspend fun subscribeChat(agent: String) {
        unsubscribeChatLocked()
        val response = client.call(
            Methods.CHAT_SUBSCRIBE,
            encodeParams(ChatSubscribeParams.serializer(), ChatSubscribeParams(agent)),
        )
        val result = muxlaneJson.decodeFromJsonElement(
            ChatSubscribeResult.serializer(),
            response.result ?: error(response.error?.message ?: "无法订阅对话流"),
        )
        if (_state.value.selectedAgent != agent) {
            runCatching { client.call(Methods.CHAT_UNSUBSCRIBE, obj("sub_id" to result.subId)) }
            return
        }
        chatSub = result.subId
        val snap = ChatTimelineSnapshot.fromHistory(result.history)
        val mergedGlobal = (_state.value.globalApprovals.filterNot { it.agent == agent } + snap.pendingApprovals)
            .distinctBy { it.id }
        _state.value = _state.value.copy(
            chatTimelines = _state.value.chatTimelines + (agent to snap),
            globalApprovals = mergedGlobal,
        )
        markSeen(agent)
    }

    private suspend fun unsubscribeChatLocked() {
        val old = chatSub ?: return
        chatSub = null
        runCatching { client.call(Methods.CHAT_UNSUBSCRIBE, obj("sub_id" to old)) }
    }

    private suspend fun unsubscribeTermLocked() {
        val old = termSub ?: return
        termSub = null
        runCatching { client.call(Methods.TERM_UNSUBSCRIBE, obj("sub_id" to old)) }
    }

    private suspend fun subscribeTerm(agent: String) {
        unsubscribeTermLocked()
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

    private fun applyChatDeltaEvent(delta: ChatDeltaEvent) {
        val current = _state.value.chatTimelines[delta.agent] ?: ChatTimelineSnapshot(agent = delta.agent)
        val next = current.applyDelta(delta)
        val nextApprovals = when (val op = delta.delta) {
            is ChatDeltaOp.ApprovalRequested ->
                (_state.value.globalApprovals.filterNot { it.id == op.approval.id } + op.approval)
            is ChatDeltaOp.ApprovalResolved ->
                _state.value.globalApprovals.filterNot { it.id == op.approvalId }
            else -> _state.value.globalApprovals
        }
        val existingSummary = _state.value.chatSessions[delta.agent]
        val nextSessions = if (existingSummary != null) {
            _state.value.chatSessions + (
                delta.agent to existingSummary.copy(
                    meta = next.meta,
                    pendingApprovals = next.pendingApprovals.size,
                    seq = next.seq,
                )
            )
        } else {
            _state.value.chatSessions
        }
        _state.value = _state.value.copy(
            chatTimelines = _state.value.chatTimelines + (delta.agent to next),
            chatSessions = nextSessions,
            globalApprovals = nextApprovals,
        )
    }

    private fun handleChatApprovalNotice(notice: ChatApprovalNoticeEvent) {
        val pairing = _state.value.pairing
        notice.approval?.let { approval ->
            _state.value = _state.value.copy(
                globalApprovals = _state.value.globalApprovals.filterNot { it.id == approval.id } + approval,
            )
            if (pairing != null) {
                RelayService.notifyApproval(app, pairing.hostId, snapshotAgent(notice.agent), approval)
            }
        }
        notice.resolvedId?.let { id ->
            removeApprovalLocal(notice.agent, id)
            RelayService.cancelApprovalNotification(app, id)
        }
    }

    private fun removeApprovalLocal(agent: String, approvalId: String) {
        val currentTimeline = _state.value.chatTimelines[agent]
        val updatedTimelines = if (currentTimeline != null) {
            _state.value.chatTimelines + (
                agent to currentTimeline.copy(
                    pendingApprovals = currentTimeline.pendingApprovals.filterNot { it.id == approvalId },
                )
            )
        } else {
            _state.value.chatTimelines
        }
        _state.value = _state.value.copy(
            chatTimelines = updatedTimelines,
            globalApprovals = _state.value.globalApprovals.filterNot { it.id == approvalId },
        )
    }

    private fun rpc(block: suspend () -> Unit) {
        if (!_state.value.connected) {
            _state.value = _state.value.copy(
                error = if (_state.value.connecting) "正在连接…" else "连接未恢复",
            )
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
    fun isChatCapable(agentId: String) = session.isChatCapable(agentId)
    fun openAgent(id: String) = session.openAgent(id)
    fun stepTab(delta: Int) = session.stepTab(delta)
    fun closeTab(agentId: String) = session.closeTab(agentId)
    fun setHubTab(tab: HubTab) = session.setHubTab(tab)
    fun setWorkspaceQuery(query: String) = session.setWorkspaceQuery(query)
    fun setAgentGroupBy(groupBy: AgentGroupBy) = session.setAgentGroupBy(groupBy)
    fun toggleAgentsNeedsYouOnly() = session.toggleAgentsNeedsYouOnly()
    fun toggleProjectCollapsed(projectId: String) = session.toggleProjectCollapsed(projectId)
    fun setTerminalTextScale(scale: Float) = session.setTerminalTextScale(scale)
    fun setChatTextScale(scale: Float) = session.setChatTextScale(scale)
    fun setSessionMode(mode: SessionPaneMode) = session.setSessionMode(mode)
    fun closeAgent() = session.closeAgent()
    fun sendChatPrompt(text: String, steer: Boolean = true) = session.sendChatPrompt(text, steer)
    fun deleteQueuedMessage(agent: String, messageId: String) = session.deleteQueuedMessage(agent, messageId)
    fun abortChat() = session.abortChat()
    fun configureChat(provider: String? = null, modelId: String? = null, thinkingLevel: String? = null) =
        session.configureChat(provider, modelId, thinkingLevel)
    fun respondApproval(agent: String, approvalId: String, reply: ApprovalReply) =
        session.respondApproval(agent, approvalId, reply)
    fun sendInput(text: String, onResult: (Boolean) -> Unit = {}): Boolean = session.sendInput(text, onResult)
    fun scrollTerminal(rows: Int) = session.scrollTerminal(rows)
    fun claimTerminalSize() = session.claimTerminalSize()
    fun resize(cols: Int, rows: Int) = session.resize(cols, rows)
    fun clearTerminal() = session.clearTerminal()
    fun loadPresets(project: String) = session.loadPresets(project)
    fun spawn(preset: AgentPreset) = session.spawn(preset)
    fun spawnChat(preset: AgentPreset, initialPrompt: String? = null) = session.spawnChat(preset, initialPrompt)
    fun closeSpawn() = session.closeSpawn()
    fun requestDelete(agent: String) = session.requestDelete(agent)
    fun cancelDelete() = session.cancelDelete()
    fun confirmDelete() = session.confirmDelete()
    fun markSeen(agent: String) = session.markSeen(agent)
    fun triggerBot(botId: String) = session.triggerBot(botId)
}

internal fun presetChatDriver(agentType: AgentType): ChatDriverKind? = when (agentType) {
    AgentType.PI, AgentType.PIE -> ChatDriverKind.PI_RPC
    AgentType.OPENCODE, AgentType.GEMINI, AgentType.QWEN, AgentType.KIMI, AgentType.AGY -> ChatDriverKind.ACP
    else -> null
}

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
