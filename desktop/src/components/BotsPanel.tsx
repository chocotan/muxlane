import React, { useEffect, useState } from "react";
import {
  Bot,
  ExternalLink,
  Play,
  Plus,
  Save,
  Trash2,
} from "lucide-react";
import type {
  AgentType,
  BotDefinition,
  BotPermissionProfile,
  BotRunRecord,
  BotTrigger,
  ChatDriverKind,
  Project,
} from "../shared/protocol";

interface BotsPanelProps {
  projects: Project[];
  bots: BotDefinition[];
  runs: BotRunRecord[];
  onSaveBot: (bot: BotDefinition) => Promise<void>;
  onDeleteBot: (botId: string) => Promise<void>;
  onTriggerBot: (botId: string, payload?: string) => Promise<void>;
  onOpenAgent: (agentId: string) => void;
}

function createDraftBot(projectId: string): BotDefinition {
  const now = Math.floor(Date.now() / 1000);
  return {
    id: `bot_${Date.now().toString(36)}`,
    name: "新建自动化 Bot",
    project_id: projectId,
    enabled: true,
    agent_type: "pi",
    args: [],
    driver: "pi_rpc",
    prompt_template: "检查仓库任务并给出清晰的执行结论：\n{{payload}}",
    soul_markdown:
      "# Role\nYou are an autonomous Muxlane engineer.\n\n# Principles\n- Verify edits with relevant tests.\n- Provide concise summaries.",
    memory_markdown: "",
    permission: "ask_first",
    isolated_worktree: true,
    max_concurrency: 1,
    timeout_secs: 600,
    triggers: [{ kind: "manual" }],
    notify_on_finish: true,
    created_at: now,
    updated_at: now,
  };
}

export const BotsPanel: React.FC<BotsPanelProps> = ({
  projects,
  bots,
  runs,
  onSaveBot,
  onDeleteBot,
  onTriggerBot,
  onOpenAgent,
}) => {
  const [selectedId, setSelectedId] = useState<string | null>(bots[0]?.id ?? null);
  const [draft, setDraft] = useState<BotDefinition | null>(bots[0] ?? null);
  const [tab, setTab] = useState<"config" | "soul" | "runs">("config");
  const [manualPayload, setManualPayload] = useState("");
  const [cronExpr, setCronExpr] = useState("");

  useEffect(() => {
    if (selectedId) {
      const found = bots.find((b) => b.id === selectedId);
      if (found) {
        setDraft(found);
        const cron = found.triggers.find((t) => t.kind === "cron");
        setCronExpr(cron && cron.kind === "cron" ? cron.expr : "");
      }
    } else if (bots.length > 0 && !draft) {
      setSelectedId(bots[0].id);
      setDraft(bots[0]);
    }
  }, [bots, draft, selectedId]);

  const handleCreate = () => {
    const pid = projects[0]?.id ?? "";
    const fresh = createDraftBot(pid);
    setSelectedId(fresh.id);
    setDraft(fresh);
    setCronExpr("");
    setTab("config");
  };

  const handleSave = async () => {
    if (!draft) return;
    const triggers: BotTrigger[] = draft.triggers.filter((t) => t.kind !== "cron");
    if (cronExpr.trim()) {
      triggers.push({ kind: "cron", expr: cronExpr.trim() });
    }
    if (triggers.length === 0) triggers.push({ kind: "manual" });
    await onSaveBot({
      ...draft,
      triggers,
      updated_at: Math.floor(Date.now() / 1000),
    });
  };

  const botRuns = draft ? runs.filter((r) => r.bot_id === draft.id) : [];

  return (
    <div className="flex h-full min-h-0 bg-canvas">
      {/* Left Bot Navigation Column */}
      <div className="flex w-[256px] shrink-0 flex-col border-r border-line-subtle bg-sidebar select-none">
        <div className="flex h-9 shrink-0 items-center justify-between border-b border-line-subtle px-3">
          <div className="flex items-center gap-2 text-[12.5px] font-semibold text-ink-strong">
            <Bot className="h-3.5 w-3.5 text-ink-muted" strokeWidth={1.75} />
            <span>Bots</span>
          </div>
          <button
            type="button"
            onClick={handleCreate}
            title="新建自动化 Bot"
            className="flex h-6 items-center gap-1 bg-primary-btn px-2 text-[11px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover transition-colors cursor-pointer"
          >
            <Plus className="h-3 w-3" />
            <span>新建</span>
          </button>
        </div>

        <div className="mux-scroll-quiet flex-1 overflow-y-auto p-1.5 space-y-[2px]">
          {bots.length === 0 ? (
            <div className="m-2 border border-dashed border-line p-4 text-center text-[12px] text-ink-muted">
              点击右上角新建你的第一个 Pi / ACP 后台自动化 Bot
            </div>
          ) : (
            bots.map((b) => {
              const active = draft?.id === b.id;
              return (
                <button
                  key={b.id}
                  type="button"
                  onClick={() => {
                    setSelectedId(b.id);
                    setDraft(b);
                  }}
                  className={`relative block w-full px-2.5 py-2 text-left transition-colors cursor-pointer ${
                    active
                      ? "bg-active text-ink-strong"
                      : "text-ink-2 hover:bg-hover hover:text-ink"
                  }`}
                >
                  {active && (
                    <span className="absolute left-0 top-1 bottom-1 w-[2px] bg-ink-strong" />
                  )}
                  <div className="flex items-center justify-between gap-2">
                    <span className="truncate text-[12.5px] font-medium">{b.name}</span>
                    <span
                      className={`h-1.5 w-1.5 shrink-0 ${
                        b.enabled ? "bg-success" : "bg-ink-faint"
                      }`}
                    />
                  </div>
                  <div className="mt-1 flex items-center gap-1.5 font-mono text-[10px] text-ink-muted">
                    <span>{b.agent_type.toUpperCase()}</span>
                    <span>·</span>
                    <span>{b.permission}</span>
                  </div>
                </button>
              );
            })
          )}
        </div>
      </div>

      {/* Right Bot Editor & Runs */}
      <div className="flex min-w-0 flex-1 flex-col">
        {!draft ? (
          <div className="flex flex-1 items-center justify-center text-[13px] text-ink-muted">
            从左侧选择一个 Bot 或点击「新建」
          </div>
        ) : (
          <>
            <div className="flex h-9 shrink-0 items-center justify-between border-b border-line-subtle bg-tab-bar px-4 select-none">
              <div className="flex h-full items-stretch gap-1">
                {(
                  [
                    { id: "config", label: "基础与触发器" },
                    { id: "soul", label: "SOUL & 记忆" },
                    { id: "runs", label: `运行记录 (${botRuns.length})` },
                  ] as const
                ).map((item) => {
                  const isCurrent = tab === item.id;
                  return (
                    <button
                      key={item.id}
                      type="button"
                      onClick={() => setTab(item.id)}
                      className={`relative px-3 text-[12px] transition-colors cursor-pointer ${
                        isCurrent
                          ? "font-medium text-ink-strong"
                          : "text-ink-muted hover:text-ink"
                      }`}
                    >
                      {item.label}
                      {isCurrent && (
                        <span className="absolute inset-x-0 bottom-0 h-[2px] bg-ink-strong" />
                      )}
                    </button>
                  );
                })}
              </div>

              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => void handleSave()}
                  className="flex h-6 items-center gap-1.5 bg-primary-btn px-2.5 text-[11.5px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover transition-colors cursor-pointer"
                >
                  <Save className="h-3 w-3" />
                  <span>保存配置</span>
                </button>
                <button
                  type="button"
                  onClick={() => {
                    void onDeleteBot(draft.id);
                    setDraft(null);
                    setSelectedId(null);
                  }}
                  className="flex h-6 items-center gap-1 px-2 text-[11.5px] text-ink-muted hover:bg-danger-soft hover:text-danger transition-colors cursor-pointer"
                >
                  <Trash2 className="h-3 w-3" />
                  <span>删除</span>
                </button>
              </div>
            </div>

            <div className="flex-1 overflow-y-auto px-6 py-6">
              <div className="mx-auto max-w-[46rem] space-y-5">
                {tab === "config" && (
                  <>
                    <div className="grid grid-cols-2 gap-3.5">
                      <div>
                        <label className="mb-1.5 block font-mono text-[10.5px] uppercase tracking-wider text-ink-muted">
                          Bot 名称
                        </label>
                        <input
                          value={draft.name}
                          onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                          className="h-9 w-full border border-line bg-elevated px-3 text-[13px] text-ink-strong focus:border-line-strong focus:outline-none select-text"
                        />
                      </div>
                      <div>
                        <label className="mb-1.5 block font-mono text-[10.5px] uppercase tracking-wider text-ink-muted">
                          绑定项目
                        </label>
                        <select
                          value={draft.project_id}
                          onChange={(e) => setDraft({ ...draft, project_id: e.target.value })}
                          className="h-9 w-full border border-line bg-elevated px-3 text-[13px] text-ink-strong focus:border-line-strong focus:outline-none"
                        >
                          {projects.map((p) => (
                            <option key={p.id} value={p.id}>
                              {p.name}
                            </option>
                          ))}
                        </select>
                      </div>
                    </div>

                    <div className="grid grid-cols-3 gap-3.5">
                      <div>
                        <label className="mb-1.5 block font-mono text-[10.5px] uppercase tracking-wider text-ink-muted">
                          Agent 类型
                        </label>
                        <select
                          value={draft.agent_type}
                          onChange={(e) => {
                            const nextType = e.target.value as AgentType;
                            const nextDriver: ChatDriverKind =
                              nextType === "pi" || nextType === "pie" ? "pi_rpc" : "acp";
                            setDraft({ ...draft, agent_type: nextType, driver: nextDriver });
                          }}
                          className="h-9 w-full border border-line bg-elevated px-3 text-[13px] text-ink-strong focus:border-line-strong focus:outline-none"
                        >
                          <option value="pi">Pi (RPC)</option>
                          <option value="pie">Pie (RPC)</option>
                          <option value="opencode">OpenCode (ACP)</option>
                          <option value="qwen">Qwen (ACP)</option>
                          <option value="kimi">Kimi (ACP)</option>
                        </select>
                      </div>
                      <div>
                        <label className="mb-1.5 block font-mono text-[10.5px] uppercase tracking-wider text-ink-muted">
                          权限策略
                        </label>
                        <select
                          value={draft.permission}
                          onChange={(e) =>
                            setDraft({
                              ...draft,
                              permission: e.target.value as BotPermissionProfile,
                            })
                          }
                          className="h-9 w-full border border-line bg-elevated px-3 text-[13px] text-ink-strong focus:border-line-strong focus:outline-none"
                        >
                          <option value="read_only">只读 (read_only)</option>
                          <option value="workspace_safe">工作区安全 (workspace_safe)</option>
                          <option value="ask_first">高危确认 (ask_first)</option>
                          <option value="full_auto">全自动 (full_auto)</option>
                        </select>
                      </div>
                      <div>
                        <label className="mb-1.5 block font-mono text-[10.5px] uppercase tracking-wider text-ink-muted">
                          Cron 定时（可选）
                        </label>
                        <input
                          value={cronExpr}
                          onChange={(e) => setCronExpr(e.target.value)}
                          placeholder="例如 */30 * * * *"
                          className="h-9 w-full border border-line bg-elevated px-3 font-mono text-[12.5px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
                        />
                      </div>
                    </div>

                    <div className="flex items-center gap-6 border border-line-subtle bg-elevated/60 px-3.5 py-2.5 text-[12.5px] text-ink">
                      <label className="flex items-center gap-2 cursor-pointer">
                        <input
                          type="checkbox"
                          checked={draft.enabled}
                          onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })}
                        />
                        <span>启用此 Bot</span>
                      </label>
                      <label className="flex items-center gap-2 cursor-pointer">
                        <input
                          type="checkbox"
                          checked={draft.isolated_worktree}
                          onChange={(e) =>
                            setDraft({ ...draft, isolated_worktree: e.target.checked })
                          }
                        />
                        <span>使用 Git Worktree 隔离运行目录</span>
                      </label>
                    </div>

                    <div>
                      <label className="mb-1.5 block font-mono text-[10.5px] uppercase tracking-wider text-ink-muted">
                        Prompt 模板（支持 {"{{payload}}"} 变量）
                      </label>
                      <textarea
                        rows={4}
                        value={draft.prompt_template}
                        onChange={(e) => setDraft({ ...draft, prompt_template: e.target.value })}
                        className="w-full border border-line bg-elevated p-3 font-mono text-[12.5px] leading-[1.6] text-ink-strong focus:border-line-strong focus:outline-none select-text"
                      />
                    </div>

                    {/* Manual Trigger Bar */}
                    <div className="border border-line bg-elevated p-4">
                      <div className="mb-2.5 font-mono text-[10.5px] uppercase tracking-wider text-ink-muted">
                        手动立即触发
                      </div>
                      <div className="flex gap-2">
                        <input
                          value={manualPayload}
                          onChange={(e) => setManualPayload(e.target.value)}
                          placeholder="输入本次运行任务说明 (payload)…"
                          className="h-9 flex-1 border border-line bg-canvas px-3 text-[13px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
                        />
                        <button
                          type="button"
                          onClick={() => void onTriggerBot(draft.id, manualPayload)}
                          className="flex h-9 items-center gap-1.5 bg-primary-btn px-4 text-[12.5px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover transition-colors cursor-pointer"
                        >
                          <Play className="h-3.5 w-3.5" />
                          <span>立即运行</span>
                        </button>
                      </div>
                    </div>
                  </>
                )}

                {tab === "soul" && (
                  <>
                    <div>
                      <label className="mb-1.5 block font-mono text-[10.5px] uppercase tracking-wider text-ink-muted">
                        SOUL.md
                      </label>
                      <textarea
                        rows={11}
                        value={draft.soul_markdown}
                        onChange={(e) => setDraft({ ...draft, soul_markdown: e.target.value })}
                        className="w-full border border-line bg-elevated p-3 font-mono text-[12.5px] leading-[1.6] text-ink-strong focus:border-line-strong focus:outline-none select-text"
                      />
                    </div>
                    <div>
                      <label className="mb-1.5 block font-mono text-[10.5px] uppercase tracking-wider text-ink-muted">
                        MEMORY.md
                      </label>
                      <textarea
                        rows={8}
                        value={draft.memory_markdown}
                        onChange={(e) => setDraft({ ...draft, memory_markdown: e.target.value })}
                        className="w-full border border-line bg-elevated p-3 font-mono text-[12.5px] leading-[1.6] text-ink-strong focus:border-line-strong focus:outline-none select-text"
                      />
                    </div>
                  </>
                )}

                {tab === "runs" && (
                  <div className="space-y-2">
                    {botRuns.length === 0 ? (
                      <div className="border border-line-subtle bg-elevated/40 p-6 text-center text-[12.5px] text-ink-muted">
                        暂无运行记录
                      </div>
                    ) : (
                      botRuns.map((run) => (
                        <div
                          key={run.id}
                          className="border border-line-subtle bg-elevated/50 p-3.5 space-y-1.5"
                        >
                          <div className="flex items-center justify-between gap-2">
                            <div className="flex items-center gap-2 font-mono text-[11px]">
                              <span
                                className={`h-1.5 w-1.5 ${
                                  run.status === "succeeded"
                                    ? "bg-success"
                                    : run.status === "failed" || run.status === "timed_out"
                                      ? "bg-danger"
                                      : "bg-working"
                                }`}
                              />
                              <span className="uppercase text-ink-strong">{run.status}</span>
                              <span className="text-ink-faint">·</span>
                              <span className="text-ink-muted">{run.trigger_source}</span>
                            </div>
                            {run.agent_id && (
                              <button
                                type="button"
                                onClick={() => onOpenAgent(run.agent_id!)}
                                className="flex items-center gap-1 px-2 py-0.5 text-[11.5px] text-ink-2 hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
                              >
                                <span>查看对话时间线</span>
                                <ExternalLink className="h-3 w-3" />
                              </button>
                            )}
                          </div>
                          {run.summary && (
                            <p className="text-[12.5px] text-ink select-text">{run.summary}</p>
                          )}
                          {run.error && (
                            <p className="font-mono text-[12px] text-danger select-text">
                              {run.error}
                            </p>
                          )}
                        </div>
                      ))
                    )}
                  </div>
                )}
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
};
