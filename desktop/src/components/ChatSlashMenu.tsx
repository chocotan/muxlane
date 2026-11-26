import React, { memo, useEffect, useRef } from "react";
import { FileText, Package, Terminal } from "lucide-react";
import {
  slashSourceLabel,
  type SlashAutocompleteState,
  type SlashPickerItem,
} from "../shared/slashCommands";

interface ChatSlashMenuProps {
  autocomplete: Extract<SlashAutocompleteState, { open: true }>;
  activeIndex: number;
  listboxId: string;
  onChoose: (item: SlashPickerItem, mode: "dispatch" | "complete") => void;
  onHover: (index: number) => void;
}

/**
 * Orca-ported slash command & skill autocomplete listbox.
 * Strictly 0px square corners per AGENTS.md.
 */
export const ChatSlashMenu: React.FC<ChatSlashMenuProps> = memo(function ChatSlashMenu({
  autocomplete,
  activeIndex,
  listboxId,
  onChoose,
  onHover,
}) {
  const activeItemRef = useRef<HTMLButtonElement | null>(null);
  const { commands, prompts, skills, items, dispatchable } = autocomplete;
  const clampedActive = items.length > 0 ? Math.min(activeIndex, items.length - 1) : 0;

  useEffect(() => {
    activeItemRef.current?.scrollIntoView({ block: "nearest" });
  }, [clampedActive, items]);

  const showGroupHeadings =
    [commands.length > 0, prompts.length > 0, skills.length > 0].filter(Boolean).length > 1 ||
    skills.length > 0;

  let optionIndex = 0;

  const renderItem = (item: SlashPickerItem) => {
    const idx = optionIndex++;
    const selected = idx === clampedActive;
    const chooseMode: "dispatch" | "complete" =
      dispatchable && item.kind === "command" && !item.argumentHint ? "dispatch" : "complete";
    const sourceBadge = slashSourceLabel(item);

    return (
      <button
        key={item.id}
        id={`${listboxId}-option-${idx}`}
        ref={selected ? activeItemRef : null}
        role="option"
        aria-selected={selected}
        type="button"
        onPointerDown={(event) => {
          // Orca pattern: accept before focus transfer so textarea retains caret ownership.
          event.preventDefault();
          if (event.button === 0) {
            onChoose(item, chooseMode);
          }
        }}
        onMouseMove={(event) => {
          // Orca pattern: only real pointer movement steals selection, never rows scrolling under a parked mouse.
          if ((event.movementX !== 0 || event.movementY !== 0) && !selected) {
            onHover(idx);
          }
        }}
        className={`flex w-full items-start gap-2 border-l-2 px-2.5 py-1.5 text-left transition-colors cursor-pointer ${
          selected
            ? "border-l-accent bg-active text-ink-strong"
            : "border-l-transparent text-ink hover:bg-hover"
        }`}
      >
        <span className="mt-0.5 flex h-3.5 w-3.5 shrink-0 items-center justify-center text-ink-muted">
          {item.kind === "skill" ? (
            <Package className="h-3.5 w-3.5 text-accent" />
          ) : item.kind === "prompt" ? (
            <FileText className="h-3.5 w-3.5 text-ink-muted" />
          ) : (
            <Terminal className="h-3.5 w-3.5 text-ink-muted" />
          )}
        </span>

        <span className="min-w-0 flex-1">
          <span className="flex min-w-0 items-baseline gap-1.5">
            <span className="truncate font-mono text-[12.5px] font-medium text-ink-strong">
              {item.token}
            </span>
            {item.argumentHint && (
              <span className="truncate font-mono text-[11px] text-ink-muted">
                {item.argumentHint}
              </span>
            )}
          </span>
          {item.description && (
            <span className="mt-0.5 block truncate text-[11.5px] text-ink-muted">
              {item.description}
            </span>
          )}
        </span>

        <span className="shrink-0 pt-0.5 font-mono text-[10px] text-ink-muted">
          {sourceBadge}
        </span>
      </button>
    );
  };

  return (
    <div
      id={listboxId}
      role="listbox"
      aria-label="斜杠命令与技能"
      className="absolute bottom-full left-0 right-0 z-30 mb-1.5 flex max-h-[300px] flex-col border border-line bg-elevated shadow-lg"
    >
      <div className="min-h-0 flex-1 overflow-y-auto py-1">
        {items.length === 0 ? (
          <div className="px-3 py-2.5 text-[12px] text-ink-muted">
            没有匹配「/{autocomplete.query}」的命令或技能
          </div>
        ) : (
          <>
            {commands.length > 0 && (
              <div>
                {showGroupHeadings && (
                  <div className="px-2.5 pb-1 pt-1 text-[10px] font-semibold uppercase tracking-wider text-ink-muted">
                    命令 ({commands.length})
                  </div>
                )}
                {commands.map(renderItem)}
              </div>
            )}

            {prompts.length > 0 && (
              <div className={commands.length > 0 ? "mt-1 border-t border-line-subtle pt-1" : ""}>
                {showGroupHeadings && (
                  <div className="px-2.5 pb-1 pt-1 text-[10px] font-semibold uppercase tracking-wider text-ink-muted">
                    模板 ({prompts.length})
                  </div>
                )}
                {prompts.map(renderItem)}
              </div>
            )}

            {skills.length > 0 && (
              <div
                className={
                  commands.length > 0 || prompts.length > 0
                    ? "mt-1 border-t border-line-subtle pt-1"
                    : ""
                }
              >
                {showGroupHeadings && (
                  <div className="px-2.5 pb-1 pt-1 text-[10px] font-semibold uppercase tracking-wider text-ink-muted">
                    技能 ({skills.length})
                  </div>
                )}
                {skills.map(renderItem)}
              </div>
            )}
          </>
        )}
      </div>

      <div className="flex items-center justify-between border-t border-line-subtle bg-sidebar/60 px-2.5 py-1 text-[10.5px] text-ink-muted">
        <span className="flex items-center gap-2.5">
          <span>
            <kbd className="font-mono text-ink-2">↑↓</kbd> 选择
          </span>
          <span>
            <kbd className="font-mono text-ink-2">Tab</kbd> 补全
          </span>
          <span>
            <kbd className="font-mono text-ink-2">Enter</kbd>{" "}
            {dispatchable ? "执行 / 补全" : "补全"}
          </span>
          <span>
            <kbd className="font-mono text-ink-2">Esc</kbd> 关闭
          </span>
        </span>
        {autocomplete.query && (
          <span className="font-mono text-[10px] text-ink-muted">/{autocomplete.query}</span>
        )}
      </div>
    </div>
  );
});
