import { useCallback, useEffect, useRef, useState } from "react";
import {
  Events,
  Methods,
  presetChatDriver,
  type AddHostInput,
  type AgentInstance,
  type AgentPreset,
  type ApprovalReply,
  type BotDefinition,
  type BotRunRecord,
  type ChatApprovalNoticeEvent,
  type ChatAttachment,
  type ChatDeltaEvent,
  type ChatListResult,
  type ChatSessionSummary,
  type ChatSubscribeResult,
  type DaemonConnectionStatus,
  type HostsState,
  type PendingApproval,
  type Project,
  type SessionViewMode,
  type Snapshot,
  type StreamingBehavior,
  type TermDataEvent,
  type TermReplayChunkEvent,
  type TermResyncEvent,
} from "../shared/protocol";
import {
  applyChatDelta,
  timelineFromHistory,
  type ChatTimelineState,
} from "../shared/timeline";

export interface ToastItem {
  id: string;
  message: string;
  level: "info" | "error" | "success";
}

export type TermStreamChunk =
  | { kind: "reset"; bytes: Uint8Array }
  | { kind: "append"; bytes: Uint8Array }
  | { kind: "exit" };

type TermListener = (chunk: TermStreamChunk) => void;

function decodeBase64ToBytes(b64: string): Uint8Array {
  if (!b64) return new Uint8Array(0);
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) {
    out[i] = bin.charCodeAt(i);
  }
  return out;
}

export function encodeStringToBase64(str: string): string {
  const bytes = new TextEncoder().encode(str);
  let bin = "";
  for (let i = 0; i < bytes.length; i++) {
    bin += String.fromCharCode(bytes[i]);
  }
  return btoa(bin);
}

export interface MuxlaneStoreOptions {
  notificationsEnabled?: boolean;
}

export function isChatOnlySession(agent: AgentInstance | undefined): boolean {
  return Boolean(agent && !agent.tmux_session);
}

export function useMuxlaneStore(options: MuxlaneStoreOptions = {}) {
  const notificationsEnabledRef = useRef(options.notificationsEnabled ?? true);
  notificationsEnabledRef.current = options.notificationsEnabled ?? true;

  const [status, setStatus] = useState<DaemonConnectionStatus>({
    connected: false,
    connecting: true,
    socketPath: "",
    error: null,
  });
  const [hostsState, setHostsState] = useState<HostsState>({
    activeHostId: "local",
    hosts: [],
    tunnelStatus: null,
  });
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [chatSummaries, setChatSummaries] = useState<Record<string, ChatSessionSummary>>({});
  const [timelines, setTimelines] = useState<Record<string, ChatTimelineState>>({});
  const [openTabs, setOpenTabs] = useState<string[]>([]);
  const [activeAgentId, setActiveAgentId] = useState<string | null>(null);
  const [activeSection, setActiveSection] = useState<"workspace" | "bots">("workspace");
  const [globalApprovals, setGlobalApprovals] = useState<PendingApproval[]>([]);
  const [bots, setBots] = useState<BotDefinition[]>([]);
  const [botRuns, setBotRuns] = useState<BotRunRecord[]>([]);
  const [presetsByProject, setPresetsByProject] = useState<Record<string, AgentPreset[]>>({});
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const [outdatedDaemon, setOutdatedDaemon] = useState(false);

  const chatSubsRef = useRef<Map<string, string>>(new Map());
  const pendingChatSubsRef = useRef<Set<string>>(new Set());
  const termListenersRef = useRef<Map<string, Set<TermListener>>>(new Map());
  const activeAgentRef = useRef<string | null>(null);
  const snapshotRef = useRef<Snapshot | null>(null);

  activeAgentRef.current = activeAgentId;
  snapshotRef.current = snapshot;

  const pushToast = useCallback((message: string, level: ToastItem["level"] = "info") => {
    const id = `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    setToasts((prev) => [...prev.slice(-4), { id, message, level }]);
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, 4500);
  }, []);

  const dismissToast = useCallback((id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const refreshState = useCallback(async () => {
    try {
      let chatUnsupported = false;
      const [snap, chatList, botList, runsRes] = await Promise.all([
        window.muxlane.rpc<Snapshot>(Methods.STATE_LIST),
        window.muxlane.rpc<ChatListResult>(Methods.CHAT_LIST).catch((err) => {
          const msg = err instanceof Error ? err.message : String(err);
          if (msg.includes("unknown method") || msg.includes("unknown_method")) {
            chatUnsupported = true;
          }
          return { sessions: [] };
        }),
        window.muxlane.rpc<{ bots: BotDefinition[] }>(Methods.BOT_LIST).catch(() => ({ bots: [] })),
        window.muxlane
          .rpc<{ runs: BotRunRecord[] }>(Methods.BOT_RUNS, { limit: 60 })
          .catch(() => ({ runs: [] })),
      ]);

      setOutdatedDaemon(chatUnsupported);
      setSnapshot(snap);
      const summaryMap: Record<string, ChatSessionSummary> = {};
      for (const s of chatList.sessions ?? []) {
        summaryMap[s.agent] = s;
      }
      setChatSummaries(summaryMap);
      setBots(botList.bots ?? []);
      setBotRuns(runsRes.runs ?? []);

      const validIds = new Set(snap.agents.map((a) => a.id));
      setOpenTabs((prev) => {
        const filtered = prev.filter((id) => validIds.has(id));
        if (filtered.length === 0 && snap.agents.length > 0 && !activeAgentRef.current) {
          return [snap.agents[0].id];
        }
        return filtered;
      });
      setActiveAgentId((prev) => {
        if (prev && validIds.has(prev)) return prev;
        return snap.agents[0]?.id ?? null;
      });
    } catch (err) {
      pushToast(`刷新状态失败: ${(err as Error).message}`, "error");
    }
  }, [pushToast]);

  const ensureChatSubscribed = useCallback(async (agentId: string) => {
    if (chatSubsRef.current.has(agentId) || pendingChatSubsRef.current.has(agentId)) return;
    pendingChatSubsRef.current.add(agentId);
    try {
      const res = await window.muxlane.rpc<ChatSubscribeResult>(Methods.CHAT_SUBSCRIBE, {
        agent: agentId,
      });
      chatSubsRef.current.set(agentId, res.sub_id);
      const tl = timelineFromHistory(res.history);
      setTimelines((prev) => ({ ...prev, [agentId]: tl }));
      if (tl.pending_approvals.length > 0) {
        setGlobalApprovals((prev) => {
          const rest = prev.filter((a) => a.agent !== agentId);
          return [...rest, ...tl.pending_approvals];
        });
      }
    } catch {
      // Pure Shell or unavailable chat session; Terminal view handles it cleanly.
    } finally {
      pendingChatSubsRef.current.delete(agentId);
    }
  }, []);

  const unsubscribeChat = useCallback(async (agentId: string) => {
    const subId = chatSubsRef.current.get(agentId);
    if (!subId) return;
    chatSubsRef.current.delete(agentId);
    await window.muxlane.rpc(Methods.CHAT_UNSUBSCRIBE, { sub_id: subId }).catch(() => {});
  }, []);

  // Wire daemon status + hosts + real-time push event listeners
  useEffect(() => {
    let mounted = true;
    void window.muxlane.listHosts().then((hs) => {
      if (mounted) setHostsState(hs);
    }).catch(() => {});

    void window.muxlane.getStatus().then((st) => {
      if (!mounted) return;
      setStatus(st);
      if (st.connected) void refreshState();
    });

    const offHosts = window.muxlane.onHostsChanged((hs) => {
      if (!mounted) return;
      setHostsState(hs);
    });

    const offStatus = window.muxlane.onStatus((next) => {
      if (!mounted) return;
      setStatus(next);
      if (next.connected) {
        // Re-sync state and clear stale subscription ids after reconnect or host switch.
        chatSubsRef.current.clear();
        pendingChatSubsRef.current.clear();
        void refreshState().then(() => {
          if (activeAgentRef.current) {
            void ensureChatSubscribed(activeAgentRef.current);
          }
        });
      }
    });

    const offEvent = window.muxlane.onEvent((msg) => {
      if (!mounted) return;
      switch (msg.event) {
        case Events.STATE_CHANGED:
        case Events.AGENT_STATUS:
          void refreshState();
          break;
        case Events.BOT_UPDATED:
        case Events.BOT_RUN_UPDATED:
          void refreshState();
          break;
        case Events.CHAT_DELTA: {
          const ev = msg.params as ChatDeltaEvent;
          setTimelines((prev) => {
            const cur = prev[ev.agent] ?? {
              agent: ev.agent,
              seq: 0,
              meta: {
                driver: "terminal_pty",
                supports_chat_input: true,
                supports_terminal_switch: true,
                has_pty: true,
                available_models: [],
                available_thinking_levels: [],
                available_commands: [],
                queued_messages: [],
                tasks: [],
                subagents: [],
                background_jobs: [],
                is_streaming: false,
              },
              entries: [],
              pending_approvals: [],
            };
            const next = applyChatDelta(cur, ev);
            if (next === cur) return prev;
            return { ...prev, [ev.agent]: next };
          });
          if (ev.delta.op === "meta_updated") {
            const updatedMeta = ev.delta.meta;
            setChatSummaries((prev) => ({
              ...prev,
              [ev.agent]: {
                agent: ev.agent,
                meta: updatedMeta,
                pending_approvals: prev[ev.agent]?.pending_approvals ?? 0,
                seq: ev.seq,
              },
            }));
          } else if (ev.delta.op === "approval_requested") {
            const approval = ev.delta.approval;
            setGlobalApprovals((prev) => [
              ...prev.filter((a) => a.id !== approval.id),
              approval,
            ]);
            if (
              notificationsEnabledRef.current &&
              typeof Notification !== "undefined" &&
              Notification.permission === "granted" &&
              (document.hidden || activeAgentRef.current !== approval.agent)
            ) {
              new Notification(`Muxlane · ${approval.title}`, {
                body: approval.message || "有会话等待你的审批确认",
              });
            }
          } else if (ev.delta.op === "approval_resolved") {
            const resolvedId = ev.delta.approval_id;
            setGlobalApprovals((prev) => prev.filter((a) => a.id !== resolvedId));
          }
          break;
        }
        case Events.CHAT_APPROVAL: {
          const notice = msg.params as ChatApprovalNoticeEvent;
          if (notice.approval) {
            const ap = notice.approval;
            setGlobalApprovals((prev) => [...prev.filter((a) => a.id !== ap.id), ap]);
            if (
              notificationsEnabledRef.current &&
              typeof Notification !== "undefined" &&
              Notification.permission === "granted" &&
              (document.hidden || activeAgentRef.current !== ap.agent)
            ) {
              new Notification(`Muxlane · ${ap.title}`, {
                body: ap.message || "有会话等待你的审批确认",
              });
            }
          }
          if (notice.resolved_id) {
            setGlobalApprovals((prev) => prev.filter((a) => a.id !== notice.resolved_id));
          }
          break;
        }
        case Events.TERM_DATA: {
          const td = msg.params as TermDataEvent;
          const listeners = termListenersRef.current.get(td.agent);
          if (listeners && listeners.size > 0) {
            const bytes = decodeBase64ToBytes(td.data_b64);
            for (const fn of listeners) fn({ kind: "append", bytes });
          }
          break;
        }
        case Events.TERM_RESYNC: {
          const tr = msg.params as TermResyncEvent;
          const listeners = termListenersRef.current.get(tr.agent);
          if (listeners && listeners.size > 0) {
            const bytes = decodeBase64ToBytes(tr.replay_b64);
            for (const fn of listeners) fn({ kind: "reset", bytes });
          }
          break;
        }
        case Events.TERM_REPLAY_CHUNK: {
          const rc = msg.params as TermReplayChunkEvent;
          const listeners = termListenersRef.current.get(rc.agent);
          if (listeners && listeners.size > 0) {
            const bytes = decodeBase64ToBytes(rc.data_b64);
            const kind = rc.chunk_index === 0 ? "reset" : "append";
            for (const fn of listeners) fn({ kind, bytes });
          }
          break;
        }
        case Events.TERM_EXIT: {
          const te = msg.params as { agent: string };
          const listeners = termListenersRef.current.get(te.agent);
          if (listeners && listeners.size > 0) {
            for (const fn of listeners) fn({ kind: "exit" });
          }
          break;
        }
      }
    });

    return () => {
      mounted = false;
      offHosts();
      offStatus();
      offEvent();
    };
  }, [ensureChatSubscribed, refreshState]);

  const openAgent = useCallback(
    (agentId: string) => {
      setActiveSection("workspace");
      setActiveAgentId(agentId);
      setOpenTabs((prev) => (prev.includes(agentId) ? prev : [...prev, agentId]));
      const agent = snapshotRef.current?.agents.find((a) => a.id === agentId);
      if (isChatOnlySession(agent)) {
        void ensureChatSubscribed(agentId);
      }
      if (agent && (!agent.seen || agent.status === "done" || agent.status === "failed")) {
        void window.muxlane.rpc(Methods.AGENT_MARK_SEEN, { agent: agentId }).catch(() => {});
      }
    },
    [ensureChatSubscribed],
  );

  const closeTab = useCallback(
    (agentId: string) => {
      void unsubscribeChat(agentId);
      setOpenTabs((prev) => {
        const idx = prev.indexOf(agentId);
        const next = prev.filter((id) => id !== agentId);
        if (activeAgentRef.current === agentId) {
          const nextActive = next[idx] ?? next[next.length - 1] ?? null;
          setActiveAgentId(nextActive);
        }
        return next;
      });
    },
    [unsubscribeChat],
  );

  const registerTermListener = useCallback((agentId: string, fn: TermListener) => {
    let set = termListenersRef.current.get(agentId);
    if (!set) {
      set = new Set();
      termListenersRef.current.set(agentId, set);
    }
    set.add(fn);
    return () => {
      set?.delete(fn);
      if (set && set.size === 0) {
        termListenersRef.current.delete(agentId);
      }
    };
  }, []);

  const loadPresets = useCallback(async (projectId: string): Promise<AgentPreset[]> => {
    try {
      const presets = await window.muxlane.rpc<AgentPreset[]>(Methods.PRESET_LIST, {
        project: projectId,
      });
      setPresetsByProject((prev) => ({ ...prev, [projectId]: presets }));
      return presets;
    } catch (err) {
      pushToast(`加载预设失败: ${(err as Error).message}`, "error");
      return [];
    }
  }, [pushToast]);

  const addProject = useCallback(
    async (projectPath: string, createIfMissing = false): Promise<Project | null> => {
      try {
        const proj = await window.muxlane.rpc<Project>(Methods.PROJECT_ADD, {
          path: projectPath,
          create_if_missing: createIfMissing,
        });
        await refreshState();
        pushToast(`已添加项目 ${proj.name}`, "success");
        return proj;
      } catch (err) {
        pushToast(`添加项目失败: ${(err as Error).message}`, "error");
        return null;
      }
    },
    [pushToast, refreshState],
  );

  const deleteProject = useCallback(
    async (projectId: string) => {
      try {
        await window.muxlane.rpc(Methods.PROJECT_DELETE, { project: projectId });
        await refreshState();
        pushToast("已移除项目", "info");
      } catch (err) {
        pushToast(`删除项目失败: ${(err as Error).message}`, "error");
      }
    },
    [pushToast, refreshState],
  );

  const spawnSession = useCallback(
    async (opts: {
      projectId: string;
      preset: AgentPreset;
      surface: SessionViewMode;
      initialPrompt?: string;
    }) => {
      try {
        const driverKind = presetChatDriver(opts.preset.agent_type);
        let inst: AgentInstance;
        if (opts.surface === "chat" && driverKind) {
          inst = await window.muxlane.rpc<AgentInstance>(Methods.CHAT_SPAWN, {
            project: opts.projectId,
            agent_type: opts.preset.agent_type,
            program: opts.preset.program,
            preset_name: opts.preset.label,
            driver: driverKind,
            initial_prompt: opts.initialPrompt?.trim() || undefined,
          });
        } else {
          inst = await window.muxlane.rpc<AgentInstance>(Methods.AGENT_SPAWN, {
            project: opts.projectId,
            agent_type: opts.preset.agent_type,
            program: opts.preset.agent_type === "shell" ? undefined : opts.preset.program,
            args: opts.preset.args,
            preset_name: opts.preset.label,
          });
        }
        await refreshState();
        openAgent(inst.id);
        return inst;
      } catch (err) {
        pushToast(`启动会话失败: ${(err as Error).message}`, "error");
        return null;
      }
    },
    [openAgent, pushToast, refreshState],
  );

  const renameAgent = useCallback(
    async (agentId: string, title: string) => {
      try {
        await window.muxlane.rpc(Methods.AGENT_RENAME, { agent: agentId, title });
        await refreshState();
      } catch (err) {
        pushToast(`重命名失败: ${(err as Error).message}`, "error");
      }
    },
    [pushToast, refreshState],
  );

  const deleteAgent = useCallback(
    async (agentId: string) => {
      try {
        await unsubscribeChat(agentId);
        await window.muxlane.rpc(Methods.AGENT_DELETE, { agent: agentId });
        closeTab(agentId);
        await refreshState();
      } catch (err) {
        pushToast(`删除会话失败: ${(err as Error).message}`, "error");
      }
    },
    [closeTab, pushToast, refreshState, unsubscribeChat],
  );

  const sendChatPrompt = useCallback(
    async (
      agentId: string,
      text: string,
      attachments: ChatAttachment[] = [],
      streamingBehavior?: StreamingBehavior,
    ) => {
      try {
        await window.muxlane.rpc(Methods.CHAT_SEND, {
          agent: agentId,
          text,
          attachments,
          streaming_behavior: streamingBehavior,
        });
      } catch (err) {
        pushToast(`发送失败: ${(err as Error).message}`, "error");
        throw err;
      }
    },
    [pushToast],
  );

  const abortChat = useCallback(
    async (agentId: string) => {
      try {
        await window.muxlane.rpc(Methods.CHAT_ABORT, { agent: agentId });
      } catch (err) {
        pushToast(`停止失败: ${(err as Error).message}`, "error");
      }
    },
    [pushToast],
  );

  const approveChat = useCallback(
    async (agentId: string, approvalId: string, reply: ApprovalReply) => {
      try {
        await window.muxlane.rpc(Methods.CHAT_APPROVE, {
          agent: agentId,
          approval_id: approvalId,
          reply,
        });
        setGlobalApprovals((prev) => prev.filter((a) => a.id !== approvalId));
      } catch (err) {
        pushToast(`审批提交失败: ${(err as Error).message}`, "error");
      }
    },
    [pushToast],
  );

  const configureChat = useCallback(
    async (
      agentId: string,
      opts: { provider?: string; model_id?: string; thinking_level?: string },
    ) => {
      try {
        await window.muxlane.rpc(Methods.CHAT_CONFIGURE, {
          agent: agentId,
          provider: opts.provider,
          model_id: opts.model_id,
          thinking_level: opts.thinking_level,
        });
      } catch (err) {
        pushToast(`配置切换失败: ${(err as Error).message}`, "error");
      }
    },
    [pushToast],
  );

  const saveBot = useCallback(
    async (bot: BotDefinition) => {
      try {
        await window.muxlane.rpc(Methods.BOT_SAVE, { bot });
        await refreshState();
        pushToast(`已保存 Bot: ${bot.name}`, "success");
      } catch (err) {
        pushToast(`保存 Bot 失败: ${(err as Error).message}`, "error");
      }
    },
    [pushToast, refreshState],
  );

  const deleteBot = useCallback(
    async (botId: string) => {
      try {
        await window.muxlane.rpc(Methods.BOT_DELETE, { bot_id: botId });
        await refreshState();
        pushToast("已删除 Bot", "info");
      } catch (err) {
        pushToast(`删除 Bot 失败: ${(err as Error).message}`, "error");
      }
    },
    [pushToast, refreshState],
  );

  const triggerBot = useCallback(
    async (botId: string, payload?: string) => {
      try {
        await window.muxlane.rpc(Methods.BOT_TRIGGER, {
          bot_id: botId,
          source: "manual",
          payload: payload?.trim() || undefined,
        });
        await refreshState();
        pushToast("已触发 Bot 运行", "success");
      } catch (err) {
        pushToast(`触发 Bot 失败: ${(err as Error).message}`, "error");
      }
    },
    [pushToast, refreshState],
  );

  const resetHostWorkspaceState = useCallback(() => {
    chatSubsRef.current.clear();
    pendingChatSubsRef.current.clear();
    setSnapshot(null);
    setChatSummaries({});
    setTimelines({});
    setOpenTabs([]);
    setActiveAgentId(null);
    setGlobalApprovals([]);
    setBots([]);
    setBotRuns([]);
    setPresetsByProject({});
    setOutdatedDaemon(false);
  }, []);

  const addHost = useCallback(
    async (input: AddHostInput) => {
      resetHostWorkspaceState();
      const next = await window.muxlane.addHost(input);
      setHostsState(next);
      pushToast(`已添加并连接到远程主机 ${input.name?.trim() || input.target}`, "success");
      return next;
    },
    [pushToast, resetHostWorkspaceState],
  );

  const removeHost = useCallback(
    async (hostId: string) => {
      const wasActive = hostsState.activeHostId === hostId;
      if (wasActive) resetHostWorkspaceState();
      const next = await window.muxlane.removeHost(hostId);
      setHostsState(next);
      pushToast("已移除远程主机", "info");
      return next;
    },
    [hostsState.activeHostId, pushToast, resetHostWorkspaceState],
  );

  const selectHost = useCallback(
    async (hostId: string) => {
      if (hostsState.activeHostId === hostId && status.connected) return hostsState;
      resetHostWorkspaceState();
      const next = await window.muxlane.selectHost(hostId);
      setHostsState(next);
      return next;
    },
    [hostsState, resetHostWorkspaceState, status.connected],
  );

  // Auto-subscribe only for pure chat sessions (native pi_rpc / acp without tmux PTY).
  const activeAgentObj = snapshot?.agents.find((a) => a.id === activeAgentId);
  const activeIsChat = isChatOnlySession(activeAgentObj);
  useEffect(() => {
    if (!status.connected || !activeAgentId || !activeIsChat) return;
    void ensureChatSubscribed(activeAgentId);
  }, [activeAgentId, activeIsChat, ensureChatSubscribed, status.connected]);

  return {
    status,
    hostsState,
    outdatedDaemon,
    snapshot,
    chatSummaries,
    timelines,
    openTabs,
    activeAgentId,
    activeSection,
    setActiveSection,
    globalApprovals,
    bots,
    botRuns,
    presetsByProject,
    toasts,
    dismissToast,
    pushToast,
    refreshState,
    openAgent,
    closeTab,
    registerTermListener,
    loadPresets,
    addProject,
    deleteProject,
    spawnSession,
    renameAgent,
    deleteAgent,
    sendChatPrompt,
    abortChat,
    approveChat,
    configureChat,
    saveBot,
    deleteBot,
    triggerBot,
    addHost,
    removeHost,
    selectHost,
  };
}

export type MuxlaneStore = ReturnType<typeof useMuxlaneStore>;
