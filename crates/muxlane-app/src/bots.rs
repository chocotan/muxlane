//! Bot Studio panel for configuring headless background Pi/ACP Bots, SOUL.md & MEMORY.md,
//! Cron/Interval/Webhook/IM triggers, permission policies, git worktree isolation, and inspecting runs.
use crate::app::MuxlaneApp;
use crate::i18n;
use crate::text_field::TextField;
use crate::theme::ThemeMode;
use crate::ui::{
    mix, rail_card, square_badge, square_button, ButtonVariant, ChatPalette, CHAT_META_PX,
    CHAT_MICRO_PX, CHAT_PROSE_PX,
};
use crate::ui_scale::px as ui_px;
use gpui::{
    div, prelude::*, rgba, AnyElement, Context, Entity, MouseButton, ParentElement, Styled, Window,
};
use muxlane_core::bot::{
    BotDefinition, BotId, BotPermissionProfile, BotRunRecord, BotRunStatus, BotTrigger,
    ImConnectorConfig, ImPlatform,
};
use muxlane_core::chat::ChatDriverKind;
use muxlane_core::model::{AgentId, AgentType};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum BotEditorTab {
    #[default]
    Config,
    SoulMemory,
    Triggers,
    Runs,
}

pub(crate) struct BotStudioState {
    pub open: bool,
    pub bots: Vec<BotDefinition>,
    pub runs: Vec<BotRunRecord>,
    pub selected_bot: Option<BotId>,
    pub draft: Option<BotDefinition>,
    pub tab: BotEditorTab,
    pub status_note: Option<String>,
    pub im_platform: ImPlatform,
    pub name_input: Entity<TextField>,
    pub prompt_input: Entity<TextField>,
    pub soul_input: Entity<TextField>,
    pub memory_input: Entity<TextField>,
    pub cron_input: Entity<TextField>,
    pub interval_input: Entity<TextField>,
    pub webhook_secret_input: Entity<TextField>,
    pub im_webhook_input: Entity<TextField>,
    pub im_token_input: Entity<TextField>,
    pub im_chat_input: Entity<TextField>,
}

impl BotStudioState {
    pub fn new(theme_mode: ThemeMode, window: &mut Window, cx: &mut gpui::App) -> Self {
        fn make_field(
            placeholder: &str,
            multi: bool,
            mode: ThemeMode,
            window: &mut Window,
            cx: &mut gpui::App,
        ) -> Entity<TextField> {
            cx.new(|cx| {
                let mut f = if multi {
                    TextField::new_chat_composer(placeholder, window, cx)
                } else {
                    TextField::new(placeholder, window, cx)
                };
                f.set_theme_mode(mode, cx);
                f
            })
        }
        Self {
            open: false,
            bots: Vec::new(),
            runs: Vec::new(),
            selected_bot: None,
            draft: None,
            tab: BotEditorTab::Config,
            status_note: None,
            im_platform: ImPlatform::Telegram,
            name_input: make_field("Nightly Reviewer", false, theme_mode, window, cx),
            prompt_input: make_field(
                "Prompt template (supports {{payload}})",
                true,
                theme_mode,
                window,
                cx,
            ),
            soul_input: make_field(
                "# SOUL.md\nBot identity & coding rules...",
                true,
                theme_mode,
                window,
                cx,
            ),
            memory_input: make_field(
                "# MEMORY.md\nDistilled memory across runs...",
                true,
                theme_mode,
                window,
                cx,
            ),
            cron_input: make_field(
                "*/30 * * * * (optional UTC cron)",
                false,
                theme_mode,
                window,
                cx,
            ),
            interval_input: make_field(
                "Seconds between runs (e.g. 3600, optional)",
                false,
                theme_mode,
                window,
                cx,
            ),
            webhook_secret_input: make_field(
                "HMAC-SHA256 / Bearer secret (optional)",
                false,
                theme_mode,
                window,
                cx,
            ),
            im_webhook_input: make_field(
                "Slack / Discord / Feishu / DingTalk Webhook URL",
                false,
                theme_mode,
                window,
                cx,
            ),
            im_token_input: make_field(
                "Telegram Bot Token / Slack Token (optional)",
                false,
                theme_mode,
                window,
                cx,
            ),
            im_chat_input: make_field(
                "Chat / Channel ID (e.g. 123456789)",
                false,
                theme_mode,
                window,
                cx,
            ),
        }
    }

    pub fn set_theme_mode(&mut self, mode: ThemeMode, cx: &mut gpui::App) {
        for field in [
            &self.name_input,
            &self.prompt_input,
            &self.soul_input,
            &self.memory_input,
            &self.cron_input,
            &self.interval_input,
            &self.webhook_secret_input,
            &self.im_webhook_input,
            &self.im_token_input,
            &self.im_chat_input,
        ] {
            field.update(cx, |f, cx| f.set_theme_mode(mode, cx));
        }
    }

    pub fn active_run_count(&self) -> usize {
        self.runs.iter().filter(|r| !r.status.is_terminal()).count()
    }
}

const BOT_AGENT_OPTIONS: &[(AgentType, ChatDriverKind, &str)] = &[
    (AgentType::Pi, ChatDriverKind::PiRpc, "Pi (RPC)"),
    (AgentType::Pie, ChatDriverKind::PiRpc, "Pie (RPC)"),
    (AgentType::Opencode, ChatDriverKind::Acp, "OpenCode (ACP)"),
    (AgentType::Qwen, ChatDriverKind::Acp, "Qwen (ACP)"),
    (AgentType::Kimi, ChatDriverKind::Acp, "Kimi (ACP)"),
    (AgentType::Agy, ChatDriverKind::Acp, "Agy (ACP)"),
];

const PERMISSION_OPTIONS: &[(BotPermissionProfile, &str)] = &[
    (BotPermissionProfile::ReadOnly, "Read Only"),
    (BotPermissionProfile::WorkspaceSafe, "Workspace Safe"),
    (BotPermissionProfile::AskFirst, "Ask First"),
    (BotPermissionProfile::FullAuto, "Full Auto"),
];

const IM_PLATFORMS: &[ImPlatform] = &[
    ImPlatform::Telegram,
    ImPlatform::Slack,
    ImPlatform::Discord,
    ImPlatform::Feishu,
    ImPlatform::DingTalk,
    ImPlatform::GenericWebhook,
];

impl MuxlaneApp {
    pub(crate) fn open_bot_studio(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.palette_open = false;
        self.bot_studio.open = true;
        self.bot_studio.status_note = None;
        self.refresh_bots(Some(window), cx);
        cx.notify();
    }

    pub(crate) fn close_bot_studio(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.bot_studio.open = false;
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn refresh_bots(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let mgr = std::sync::Arc::clone(self.server.bot_manager());
        let had_window = window.is_some();
        cx.spawn(async move |this, cx| {
            let (bots, runs) = cx
                .background_spawn(async move {
                    let bots = mgr.list_bots().await;
                    let runs = mgr.list_runs(None, 60).await;
                    (bots, runs)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.bot_studio.bots = bots;
                this.bot_studio.runs = runs;
                if this.bot_studio.draft.is_none() {
                    if let Some(first) = this.bot_studio.bots.first().cloned() {
                        this.load_bot_into_editor(first, had_window, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_bot_into_editor(
        &mut self,
        bot: BotDefinition,
        sync_fields: bool,
        cx: &mut Context<Self>,
    ) {
        self.bot_studio.selected_bot = Some(bot.id.clone());
        let mut cron = String::new();
        let mut interval = String::new();
        let mut webhook_secret = String::new();
        let mut im_platform = ImPlatform::Telegram;
        let mut im_webhook = String::new();
        let mut im_token = String::new();
        let mut im_chat = String::new();

        for trig in &bot.triggers {
            match trig {
                BotTrigger::Cron { expr } => cron = expr.clone(),
                BotTrigger::Interval { every_secs } => interval = every_secs.to_string(),
                BotTrigger::Webhook { secret } => {
                    webhook_secret = secret.clone().unwrap_or_default()
                }
                BotTrigger::Im { connector, .. } => {
                    im_platform = connector.platform;
                    im_webhook = connector.webhook_url.clone().unwrap_or_default();
                    im_token = connector.bot_token.clone().unwrap_or_default();
                    im_chat = connector.channel_or_chat_id.clone().unwrap_or_default();
                }
                BotTrigger::Manual => {}
            }
        }
        self.bot_studio.im_platform = im_platform;
        if sync_fields || self.bot_studio.draft.is_none() {
            self.bot_studio
                .name_input
                .update(cx, |f, cx| f.set_text(bot.name.clone(), cx));
            self.bot_studio
                .prompt_input
                .update(cx, |f, cx| f.set_text(bot.prompt_template.clone(), cx));
            self.bot_studio
                .soul_input
                .update(cx, |f, cx| f.set_text(bot.soul_markdown.clone(), cx));
            self.bot_studio
                .memory_input
                .update(cx, |f, cx| f.set_text(bot.memory_markdown.clone(), cx));
            self.bot_studio
                .cron_input
                .update(cx, |f, cx| f.set_text(cron, cx));
            self.bot_studio
                .interval_input
                .update(cx, |f, cx| f.set_text(interval, cx));
            self.bot_studio
                .webhook_secret_input
                .update(cx, |f, cx| f.set_text(webhook_secret, cx));
            self.bot_studio
                .im_webhook_input
                .update(cx, |f, cx| f.set_text(im_webhook, cx));
            self.bot_studio
                .im_token_input
                .update(cx, |f, cx| f.set_text(im_token, cx));
            self.bot_studio
                .im_chat_input
                .update(cx, |f, cx| f.set_text(im_chat, cx));
        }
        self.bot_studio.draft = Some(bot);
    }

    pub(crate) fn new_bot_draft(&mut self, cx: &mut Context<Self>) {
        let project_id = self
            .workspace
            .current_project()
            .filter(|k| k.machine_id == self.local_machine_id())
            .map(|k| k.project_id.clone())
            .or_else(|| self.last_snapshot.projects.first().map(|p| p.id.clone()))
            .unwrap_or_default();
        let id = muxlane_core::model::new_id("bot");
        let bot = BotDefinition::new(id, "Nightly Reviewer".into(), project_id);
        self.bot_studio.status_note = None;
        self.bot_studio.tab = BotEditorTab::Config;
        self.load_bot_into_editor(bot, true, cx);
        cx.notify();
    }

    fn collect_draft_from_inputs(&self, cx: &gpui::App) -> Option<BotDefinition> {
        let mut bot = self.bot_studio.draft.clone()?;
        bot.name = self
            .bot_studio
            .name_input
            .read(cx)
            .text()
            .trim()
            .to_string();
        bot.prompt_template = self.bot_studio.prompt_input.read(cx).text();
        bot.soul_markdown = self.bot_studio.soul_input.read(cx).text();
        bot.memory_markdown = self.bot_studio.memory_input.read(cx).text();

        let mut triggers = vec![BotTrigger::Manual];
        let cron = self
            .bot_studio
            .cron_input
            .read(cx)
            .text()
            .trim()
            .to_string();
        if !cron.is_empty() {
            triggers.push(BotTrigger::Cron { expr: cron });
        }
        let interval = self
            .bot_studio
            .interval_input
            .read(cx)
            .text()
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|&v| v > 0);
        if let Some(every_secs) = interval {
            triggers.push(BotTrigger::Interval { every_secs });
        }
        let secret = self
            .bot_studio
            .webhook_secret_input
            .read(cx)
            .text()
            .trim()
            .to_string();
        if !secret.is_empty()
            || bot
                .triggers
                .iter()
                .any(|t| matches!(t, BotTrigger::Webhook { .. }))
        {
            triggers.push(BotTrigger::Webhook {
                secret: (!secret.is_empty()).then_some(secret),
            });
        }

        let im_webhook = self
            .bot_studio
            .im_webhook_input
            .read(cx)
            .text()
            .trim()
            .to_string();
        let im_token = self
            .bot_studio
            .im_token_input
            .read(cx)
            .text()
            .trim()
            .to_string();
        let im_chat = self
            .bot_studio
            .im_chat_input
            .read(cx)
            .text()
            .trim()
            .to_string();
        if !im_webhook.is_empty() || !im_token.is_empty() || !im_chat.is_empty() {
            triggers.push(BotTrigger::Im {
                connector: ImConnectorConfig {
                    platform: self.bot_studio.im_platform,
                    webhook_url: (!im_webhook.is_empty()).then_some(im_webhook),
                    bot_token: (!im_token.is_empty()).then_some(im_token),
                    signing_secret: None,
                    channel_or_chat_id: (!im_chat.is_empty()).then_some(im_chat),
                    poll_interval_secs: 5,
                },
                allow_from: vec!["*".into()],
            });
        }
        bot.triggers = triggers;
        Some(bot)
    }

    pub(crate) fn save_draft_bot(&mut self, cx: &mut Context<Self>) {
        let Some(bot) = self.collect_draft_from_inputs(cx) else {
            return;
        };
        if bot.project_id.trim().is_empty() {
            self.bot_studio.status_note = Some("Select a local project before saving.".into());
            cx.notify();
            return;
        }
        let mgr = std::sync::Arc::clone(self.server.bot_manager());
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move { mgr.save_bot(bot).await })
                .await;
            let _ = this.update(cx, |this, cx| match res {
                Ok(saved) => {
                    this.bot_studio.status_note = Some(format!("Saved {}", saved.name));
                    this.load_bot_into_editor(saved, true, cx);
                    this.refresh_bots(None, cx);
                }
                Err(err) => {
                    this.bot_studio.status_note = Some(format!("Save error: {err}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn trigger_bot_now(&mut self, bot_id: BotId, cx: &mut Context<Self>) {
        // Save unsaved edits first if triggering the active draft!
        let active_draft = if self
            .bot_studio
            .draft
            .as_ref()
            .is_some_and(|d| d.id == bot_id)
        {
            self.collect_draft_from_inputs(cx)
        } else {
            None
        };
        let mgr = std::sync::Arc::clone(self.server.bot_manager());
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_spawn(async move {
                    if let Some(d) = active_draft {
                        mgr.save_bot(d).await?;
                    }
                    mgr.trigger_bot(&bot_id, "manual".into(), None, None).await
                })
                .await;
            let _ = this.update(cx, |this, cx| match res {
                Ok(run) => {
                    this.bot_studio.status_note =
                        Some(format!("Started run {} ({})", run.id, run.status.as_str()));
                    this.bot_studio.tab = BotEditorTab::Runs;
                    this.refresh_bots(None, cx);
                }
                Err(err) => {
                    this.bot_studio.status_note = Some(format!("Trigger error: {err}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn delete_bot_in_studio(&mut self, bot_id: BotId, cx: &mut Context<Self>) {
        let mgr = std::sync::Arc::clone(self.server.bot_manager());
        cx.spawn(async move |this, cx| {
            cx.background_spawn(async move {
                mgr.delete_bot(&bot_id).await;
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.bot_studio.draft = None;
                this.bot_studio.selected_bot = None;
                this.bot_studio.status_note = Some("Deleted bot".into());
                this.refresh_bots(None, cx);
            });
        })
        .detach();
    }

    pub(crate) fn open_bot_run_timeline(
        &mut self,
        agent_id: AgentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.bot_studio.open = false;
        self.open_agent(&agent_id, window, cx);
        cx.notify();
    }

    pub(crate) fn render_bot_studio(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = ChatPalette::for_mode(self.theme_mode);
        let theme = palette.theme;
        let lang = self.language;
        let webhook_addr = self.server.bot_manager().webhook_addr();

        let bot_list = self.bot_studio.bots.clone();
        let selected_id = self.bot_studio.selected_bot.clone();
        let draft = self.bot_studio.draft.clone();
        let current_tab = self.bot_studio.tab;
        let runs = self.bot_studio.runs.clone();
        let projects = self.last_snapshot.projects.clone();
        let im_platform = self.bot_studio.im_platform;

        let mut left_list = div()
            .w(ui_px(240.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(rgba(palette.canvas))
            .border_r_1()
            .border_color(rgba(palette.hairline));

        left_list = left_list.child(
            div()
                .p_3()
                .flex()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(rgba(palette.hairline))
                .child(
                    div()
                        .text_size(ui_px(CHAT_PROSE_PX))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgba(palette.fg_strong))
                        .child(i18n::text(lang, "bots.title").to_string()),
                )
                .child(
                    square_button(
                        "bot-new-btn",
                        i18n::text(lang, "bots.new"),
                        ButtonVariant::Primary,
                        palette,
                    )
                    .on_click(cx.listener(|this, _ev, _window, cx| {
                        this.new_bot_draft(cx);
                    })),
                ),
        );

        let mut bots_scroll = div()
            .id("bot-studio-left-scroll")
            .flex_1()
            .overflow_y_scroll()
            .p_2()
            .flex()
            .flex_col()
            .gap_1p5();

        for b in &bot_list {
            let is_sel = selected_id.as_deref() == Some(b.id.as_str());
            let bot_clone = b.clone();
            let active = runs
                .iter()
                .any(|r| r.bot_id == b.id && !r.status.is_terminal());
            let row_bg = if is_sel {
                palette.user_surface
            } else {
                palette.card
            };
            let border = if is_sel {
                theme.accent
            } else {
                palette.hairline
            };
            bots_scroll = bots_scroll.child(
                div()
                    .id(gpui::SharedString::from(format!("bot-row-{}", b.id)))
                    .w_full()
                    .p_2p5()
                    .bg(rgba(row_bg))
                    .border_1()
                    .border_color(rgba(border))
                    .cursor_pointer()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.load_bot_into_editor(bot_clone.clone(), true, cx);
                        this.bot_studio.status_note = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(ui_px(CHAT_PROSE_PX))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(rgba(palette.fg_strong))
                                    .truncate()
                                    .child(b.name.clone()),
                            )
                            .child(square_badge(
                                if active {
                                    "LIVE"
                                } else if b.enabled {
                                    "ON"
                                } else {
                                    "OFF"
                                },
                                if active || b.enabled {
                                    theme.accent
                                } else {
                                    palette.fg_faint
                                },
                                palette.inset,
                                palette.hairline,
                            )),
                    )
                    .child(
                        div()
                            .text_size(ui_px(CHAT_MICRO_PX))
                            .text_color(rgba(palette.fg_faint))
                            .child(format!(
                                "{} · {} · {} triggers",
                                b.agent_type.as_str(),
                                b.permission.as_str(),
                                b.triggers.len()
                            )),
                    ),
            );
        }
        left_list = left_list.child(bots_scroll);

        // Right editor area
        let mut right_pane = div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgba(palette.card));

        let close_bar = div()
            .px_4()
            .py_2p5()
            .flex()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(rgba(palette.hairline))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .children(draft.as_ref().map(|d| {
                        let id_run = d.id.clone();
                        let id_del = d.id.clone();
                        let is_enabled = d.enabled;
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                square_button(
                                    "bot-toggle-enabled",
                                    if is_enabled {
                                        i18n::text(lang, "bots.enabled")
                                    } else {
                                        i18n::text(lang, "bots.disabled")
                                    },
                                    if is_enabled {
                                        ButtonVariant::Secondary
                                    } else {
                                        ButtonVariant::Outline
                                    },
                                    palette,
                                )
                                .on_click(cx.listener(
                                    |this, _ev, _window, cx| {
                                        if let Some(d) = this.bot_studio.draft.as_mut() {
                                            d.enabled = !d.enabled;
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                square_button(
                                    "bot-save-btn",
                                    i18n::text(lang, "bots.save"),
                                    ButtonVariant::Primary,
                                    palette,
                                )
                                .on_click(cx.listener(
                                    |this, _ev, _window, cx| {
                                        this.save_draft_bot(cx);
                                    },
                                )),
                            )
                            .child(
                                square_button(
                                    "bot-run-now-btn",
                                    i18n::text(lang, "bots.run_now"),
                                    ButtonVariant::Secondary,
                                    palette,
                                )
                                .on_click(cx.listener(
                                    move |this, _ev, _window, cx| {
                                        this.trigger_bot_now(id_run.clone(), cx);
                                    },
                                )),
                            )
                            .child(
                                square_button(
                                    "bot-del-btn",
                                    i18n::text(lang, "bots.delete"),
                                    ButtonVariant::Danger,
                                    palette,
                                )
                                .on_click(cx.listener(
                                    move |this, _ev, _window, cx| {
                                        this.delete_bot_in_studio(id_del.clone(), cx);
                                    },
                                )),
                            )
                    })),
            )
            .child(
                square_button(
                    "bot-close-btn",
                    i18n::text(lang, "bots.close"),
                    ButtonVariant::Outline,
                    palette,
                )
                .on_click(cx.listener(|this, _ev, window, cx| {
                    this.close_bot_studio(window, cx);
                })),
            );
        right_pane = right_pane.child(close_bar);

        if let Some(note) = &self.bot_studio.status_note {
            right_pane = right_pane.child(
                div()
                    .px_4()
                    .py_1p5()
                    .bg(rgba(mix(theme.accent, palette.card, 0.10)))
                    .border_b_1()
                    .border_color(rgba(palette.hairline))
                    .text_size(ui_px(CHAT_META_PX))
                    .text_color(rgba(palette.fg_strong))
                    .child(note.clone()),
            );
        }

        if let Some(d) = draft {
            // Editor Tabs
            let tabs_bar = div()
                .px_4()
                .py_2()
                .flex()
                .items_center()
                .gap_1p5()
                .border_b_1()
                .border_color(rgba(palette.hairline))
                .children(
                    [
                        (BotEditorTab::Config, "bots.tab_config"),
                        (BotEditorTab::SoulMemory, "bots.tab_soul"),
                        (BotEditorTab::Triggers, "bots.tab_triggers"),
                        (BotEditorTab::Runs, "bots.tab_runs"),
                    ]
                    .into_iter()
                    .map(|(tab, key)| {
                        let selected = tab == current_tab;
                        square_button(
                            gpui::SharedString::from(format!("bot-tab-{key}")),
                            i18n::text(lang, key),
                            if selected {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Outline
                            },
                            palette,
                        )
                        .on_click(cx.listener(
                            move |this, _ev, _window, cx| {
                                this.bot_studio.tab = tab;
                                if tab == BotEditorTab::Runs {
                                    this.refresh_bots(None, cx);
                                }
                                cx.notify();
                            },
                        ))
                    }),
                );
            right_pane = right_pane.child(tabs_bar);

            let mut body = div()
                .id("bot-studio-body-scroll")
                .flex_1()
                .overflow_y_scroll()
                .p_4()
                .flex()
                .flex_col()
                .gap_3();

            match current_tab {
                BotEditorTab::Config => {
                    body = body
                        .child(field_label("Name", palette))
                        .child(boxed_input(
                            self.bot_studio.name_input.clone(),
                            palette,
                            34.,
                        ))
                        .child(field_label("Project", palette))
                        .child(div().flex().flex_wrap().gap_1p5().children(
                            projects.into_iter().map(|proj| {
                                let pid = proj.id.clone();
                                let selected = d.project_id == pid;
                                square_button(
                                    gpui::SharedString::from(format!("bot-proj-{pid}")),
                                    proj.name.clone(),
                                    if selected {
                                        ButtonVariant::Primary
                                    } else {
                                        ButtonVariant::Outline
                                    },
                                    palette,
                                )
                                .on_click(cx.listener(
                                    move |this, _ev, _window, cx| {
                                        if let Some(ref mut dr) = this.bot_studio.draft {
                                            dr.project_id = pid.clone();
                                        }
                                        cx.notify();
                                    },
                                ))
                            }),
                        ))
                        .child(field_label("Agent Runtime (Pi & ACP)", palette))
                        .child(
                            div().flex().flex_wrap().gap_1p5().children(
                                BOT_AGENT_OPTIONS
                                    .iter()
                                    .map(|&(agent_type, driver, label)| {
                                        let selected = d.agent_type == agent_type;
                                        square_button(
                                            gpui::SharedString::from(format!(
                                                "bot-agent-{}",
                                                agent_type.as_str()
                                            )),
                                            label,
                                            if selected {
                                                ButtonVariant::Primary
                                            } else {
                                                ButtonVariant::Outline
                                            },
                                            palette,
                                        )
                                        .on_click(
                                            cx.listener(move |this, _ev, _window, cx| {
                                                if let Some(ref mut dr) = this.bot_studio.draft {
                                                    dr.agent_type = agent_type;
                                                    dr.driver = driver;
                                                }
                                                cx.notify();
                                            }),
                                        )
                                    }),
                            ),
                        )
                        .child(field_label("Permission Profile", palette))
                        .child(div().flex().flex_wrap().gap_1p5().children(
                            PERMISSION_OPTIONS.iter().map(|&(perm, label)| {
                                let selected = d.permission == perm;
                                square_button(
                                    gpui::SharedString::from(format!("bot-perm-{}", perm.as_str())),
                                    label,
                                    if selected {
                                        ButtonVariant::Primary
                                    } else {
                                        ButtonVariant::Outline
                                    },
                                    palette,
                                )
                                .on_click(cx.listener(
                                    move |this, _ev, _window, cx| {
                                        if let Some(ref mut dr) = this.bot_studio.draft {
                                            dr.permission = perm;
                                        }
                                        cx.notify();
                                    },
                                ))
                            }),
                        ))
                        .child(
                            div().flex().items_center().gap_2().child(
                                square_button(
                                    "bot-worktree-toggle",
                                    if d.isolated_worktree {
                                        format!("[x] {}", i18n::text(lang, "bots.worktree"))
                                    } else {
                                        format!("[ ] {}", i18n::text(lang, "bots.worktree"))
                                    },
                                    if d.isolated_worktree {
                                        ButtonVariant::Secondary
                                    } else {
                                        ButtonVariant::Outline
                                    },
                                    palette,
                                )
                                .on_click(cx.listener(
                                    |this, _ev, _window, cx| {
                                        if let Some(ref mut dr) = this.bot_studio.draft {
                                            dr.isolated_worktree = !dr.isolated_worktree;
                                        }
                                        cx.notify();
                                    },
                                )),
                            ),
                        )
                        .child(field_label("Task Prompt Template", palette))
                        .child(boxed_input(
                            self.bot_studio.prompt_input.clone(),
                            palette,
                            120.,
                        ));
                }
                BotEditorTab::SoulMemory => {
                    body = body
                        .child(field_label("SOUL.md (Persona & Constraints)", palette))
                        .child(boxed_input(
                            self.bot_studio.soul_input.clone(),
                            palette,
                            180.,
                        ))
                        .child(field_label(
                            "MEMORY.md (Auto-distilled Persistent Memory)",
                            palette,
                        ))
                        .child(boxed_input(
                            self.bot_studio.memory_input.clone(),
                            palette,
                            180.,
                        ));
                }
                BotEditorTab::Triggers => {
                    let webhook_hint = webhook_addr
                        .map(|addr| format!("POST http://{addr}/hooks/{}", d.id))
                        .unwrap_or_else(|| "Webhook listener inactive".into());
                    body = body
                        .child(field_label(
                            "UTC Cron Schedule (min hour dom mon dow)",
                            palette,
                        ))
                        .child(boxed_input(
                            self.bot_studio.cron_input.clone(),
                            palette,
                            34.,
                        ))
                        .child(field_label("Fixed Interval (seconds)", palette))
                        .child(boxed_input(
                            self.bot_studio.interval_input.clone(),
                            palette,
                            34.,
                        ))
                        .child(field_label(
                            format!("HTTP Webhook Secret ({webhook_hint})"),
                            palette,
                        ))
                        .child(boxed_input(
                            self.bot_studio.webhook_secret_input.clone(),
                            palette,
                            34.,
                        ))
                        .child(field_label("IM Platform Connector", palette))
                        .child(div().flex().flex_wrap().gap_1p5().children(
                            IM_PLATFORMS.iter().map(|&plat| {
                                let selected = im_platform == plat;
                                square_button(
                                    gpui::SharedString::from(format!(
                                        "bot-im-plat-{}",
                                        plat.label()
                                    )),
                                    plat.label(),
                                    if selected {
                                        ButtonVariant::Primary
                                    } else {
                                        ButtonVariant::Outline
                                    },
                                    palette,
                                )
                                .on_click(cx.listener(
                                    move |this, _ev, _window, cx| {
                                        this.bot_studio.im_platform = plat;
                                        cx.notify();
                                    },
                                ))
                            }),
                        ))
                        .child(field_label("IM Webhook URL", palette))
                        .child(boxed_input(
                            self.bot_studio.im_webhook_input.clone(),
                            palette,
                            34.,
                        ))
                        .child(field_label("Bot Token (Telegram / Slack)", palette))
                        .child(boxed_input(
                            self.bot_studio.im_token_input.clone(),
                            palette,
                            34.,
                        ))
                        .child(field_label("Chat / Channel ID", palette))
                        .child(boxed_input(
                            self.bot_studio.im_chat_input.clone(),
                            palette,
                            34.,
                        ));
                }
                BotEditorTab::Runs => {
                    let bot_runs: Vec<_> = runs.into_iter().filter(|r| r.bot_id == d.id).collect();
                    if bot_runs.is_empty() {
                        body = body.child(
                            div()
                                .text_size(ui_px(CHAT_META_PX))
                                .text_color(rgba(palette.fg_faint))
                                .child("No runs recorded yet. Click Run Now above to execute."),
                        );
                    } else {
                        for run in bot_runs {
                            let status_color = match run.status {
                                BotRunStatus::Succeeded => theme.green,
                                BotRunStatus::Failed | BotRunStatus::TimedOut => theme.red,
                                BotRunStatus::NeedsApproval => theme.yellow,
                                _ => theme.accent,
                            };
                            let card_body = div()
                                .w_full()
                                .p_2p5()
                                .flex()
                                .flex_col()
                                .gap_1p5()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap_2()
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_1p5()
                                                .child(square_badge(
                                                    run.status.as_str().to_uppercase(),
                                                    status_color,
                                                    palette.inset,
                                                    status_color,
                                                ))
                                                .child(
                                                    div()
                                                        .text_size(ui_px(CHAT_META_PX))
                                                        .text_color(rgba(palette.fg_muted))
                                                        .child(format!(
                                                            "{} · {}",
                                                            run.trigger_source, run.id
                                                        )),
                                                ),
                                        )
                                        .children(run.agent_id.clone().map(|ag| {
                                            let run_id_key = run.id.clone();
                                            square_button(
                                                gpui::SharedString::from(format!(
                                                    "open-run-{run_id_key}"
                                                )),
                                                i18n::text(lang, "bots.open_timeline"),
                                                ButtonVariant::Primary,
                                                palette,
                                            )
                                            .on_click(
                                                cx.listener(move |this, _ev, window, cx| {
                                                    this.open_bot_run_timeline(
                                                        ag.clone(),
                                                        window,
                                                        cx,
                                                    );
                                                }),
                                            )
                                        })),
                                )
                                .children(run.worktree_path.map(|wt| {
                                    div()
                                        .text_size(ui_px(CHAT_MICRO_PX))
                                        .text_color(rgba(palette.fg_faint))
                                        .child(format!("worktree: {wt}"))
                                }))
                                .children(run.summary.or(run.error).map(|text| {
                                    div()
                                        .text_size(ui_px(CHAT_META_PX))
                                        .text_color(rgba(palette.fg_prose))
                                        .child(text)
                                }));
                            body = body.child(rail_card(palette, Some(status_color), card_body));
                        }
                    }
                }
            }
            right_pane = right_pane.child(body);
        } else {
            right_pane = right_pane.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_6()
                    .text_size(ui_px(CHAT_PROSE_PX))
                    .text_color(rgba(palette.fg_faint))
                    .child(i18n::text(lang, "bots.empty").to_string()),
            );
        }

        div()
            .id("bot-studio-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_start()
            .justify_center()
            .pt(ui_px(40.))
            .bg(rgba(theme.overlay()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _ev, window, cx| {
                    this.close_bot_studio(window, cx);
                }),
            )
            .child(
                div()
                    .id("bot-studio-panel")
                    .relative()
                    .occlude()
                    .w(ui_px(940.))
                    .h(ui_px(640.))
                    .flex()
                    .bg(rgba(palette.card))
                    .border_1()
                    .border_color(rgba(palette.hairline))
                    .on_mouse_down(MouseButton::Left, |_, _window, cx| {
                        cx.stop_propagation();
                    })
                    .child(left_list)
                    .child(right_pane),
            )
            .into_any_element()
    }
}

fn field_label(text: impl Into<String>, palette: ChatPalette) -> impl IntoElement {
    div()
        .text_size(ui_px(CHAT_MICRO_PX))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgba(palette.fg_faint))
        .child(text.into())
}

fn boxed_input(entity: Entity<TextField>, palette: ChatPalette, min_h: f32) -> impl IntoElement {
    div()
        .w_full()
        .min_h(ui_px(min_h))
        .bg(rgba(palette.inset))
        .border_1()
        .border_color(rgba(palette.hairline))
        .p_1p5()
        .child(entity)
}
