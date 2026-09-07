//! Remote host connection, bootstrap, deletion, and agent/project flows.
use crate::app::MuxlaneApp;
use crate::dialogs::ConnectAuthMode;
use crate::i18n;
use crate::menus::{DeleteConfirm, DeleteTarget};
use crate::workspace::ProjectKey;
use gpui::{AppContext, Context, Window};
use muxlane_core::{PaneId, SplitAxis};
use std::sync::Arc;

pub(crate) struct RemoteAgentSpawnRequest {
    pub(crate) host: String,
    pub(crate) project: String,
    pub(crate) preset: Option<muxlane_core::AgentPreset>,
    pub(crate) preferred_pane: Option<PaneId>,
    pub(crate) split_axis: Option<SplitAxis>,
}

pub(crate) fn replaced_machine_id(
    previous: Option<String>,
    current: Option<&str>,
) -> Option<String> {
    previous.filter(|previous| current.is_some_and(|current| current != previous))
}

impl MuxlaneApp {
    pub(crate) fn restore_remotes(
        server: &Arc<muxlane_server::MuxlaneServer>,
        persisted: &muxlane_store::PersistedApp,
        connect_to: &[String],
        tx: &tokio::sync::mpsc::Sender<muxlane_client::ClientEvent>,
    ) -> Vec<Arc<muxlane_client::RemoteHost>> {
        let mut remotes = Vec::new();
        for saved in &persisted.remote_configs {
            let target = muxlane_client::parse_target(&saved.target);
            let name = match &target {
                muxlane_client::Target::Socket(path) => {
                    path.rsplit('/').next().unwrap_or(path).to_string()
                }
                muxlane_client::Target::Ssh { host, .. } => host.clone(),
            };
            let auth = match muxlane_client::SshAuth::try_from(saved.auth.clone()) {
                Ok(auth) => auth,
                Err(error) => {
                    tracing::warn!(target = %saved.target, %error, "skip remote with missing auth secret");
                    continue;
                }
            };
            let remote = muxlane_client::RemoteHost::new(
                muxlane_client::HostCfg {
                    name,
                    target,
                    auth,
                    retry_base_ms: 500,
                },
                tx.clone(),
            );
            remote.restore_machine_id(saved.machine_id.clone());
            server.rt_spawn(Arc::clone(&remote).run_loop());
            remotes.push(remote);
        }
        for target in connect_to {
            let parsed = muxlane_client::parse_target(target);
            let name = match &parsed {
                muxlane_client::Target::Socket(path) => {
                    path.rsplit('/').next().unwrap_or(path).to_string()
                }
                muxlane_client::Target::Ssh { host, .. } => host.clone(),
            };
            let cfg = muxlane_client::HostCfg {
                name,
                target: parsed,
                auth: muxlane_client::SshAuth::SshConfig,
                retry_base_ms: 500,
            };
            if remotes
                .iter()
                .any(|remote: &Arc<muxlane_client::RemoteHost>| remote.cfg.name == cfg.name)
            {
                continue;
            }
            let host = muxlane_client::RemoteHost::new(cfg, tx.clone());
            server.rt_spawn(Arc::clone(&host).run_loop());
            remotes.push(host);
        }
        remotes
    }

    pub(crate) fn add_remote_target(&mut self, target: String, cx: &mut Context<Self>) {
        let target = target.trim().to_string();
        if target.is_empty() {
            self.dialog_error = Some(i18n::text(self.language, "error.ssh_target_required").into());
            cx.notify();
            return;
        }
        let parsed = muxlane_client::parse_target(&target);
        let name = match &parsed {
            muxlane_client::Target::Socket(path) => {
                path.rsplit('/').next().unwrap_or(path).to_string()
            }
            muxlane_client::Target::Ssh { host, .. } => host.clone(),
        };
        let username = self.connect_username.read(cx).text();
        let auth = match self.connect_auth_mode {
            ConnectAuthMode::SshConfig => muxlane_client::SshAuth::SshConfig,
            ConnectAuthMode::PublicKey => muxlane_client::SshAuth::PublicKey {
                username: (!username.trim().is_empty()).then(|| username.trim().to_string()),
                identity_file: {
                    let path = self.connect_key_path.read(cx).text();
                    (!path.trim().is_empty()).then(|| path.trim().to_string())
                },
            },
            ConnectAuthMode::Password => {
                let password = self.connect_password.read(cx).text();
                if username.trim().is_empty() || password.is_empty() {
                    self.dialog_error = Some(
                        i18n::text(self.language, "error.password_credentials_required").into(),
                    );
                    cx.notify();
                    return;
                }
                muxlane_client::SshAuth::Password {
                    username: username.trim().to_string(),
                    password,
                }
            }
        };
        // Validate all credentials before mutating an existing connection.
        let inherited_machine_id =
            if let Some(index) = self.remotes.iter().position(|host| host.cfg.name == name) {
                let machine_id = self.remotes[index].machine_id();
                self.remotes[index].stop();
                let release_name = name.clone();
                self.server.rt_spawn(async move {
                    muxlane_client::release_remote_tunnel(&release_name).await;
                });
                self.remotes.remove(index);
                self.remote_snaps.remove(&name);
                self.remote_states.remove(&name);
                machine_id
            } else {
                None
            };
        let host = muxlane_client::RemoteHost::new(
            muxlane_client::HostCfg {
                name,
                target: parsed,
                auth,
                retry_base_ms: 500,
            },
            self.remote_event_tx.clone(),
        );
        host.restore_machine_id(inherited_machine_id);
        self.server.rt_spawn(Arc::clone(&host).run_loop());
        self.remotes.push(host);
        self.persist();
        self.connect_dialog = false;
        self.dialog_error = None;
        self.connect_input.update(cx, |input, cx| input.reset(cx));
        self.connect_username
            .update(cx, |input, cx| input.reset(cx));
        self.connect_password
            .update(cx, |input, cx| input.reset(cx));
        self.connect_key_path
            .update(cx, |input, cx| input.reset(cx));
        cx.notify();
    }

    pub(crate) fn begin_delete(&mut self, target: DeleteTarget, cx: &mut Context<Self>) {
        if self.delete_busy {
            return;
        }
        let affected_sessions = match &target {
            DeleteTarget::LocalProject { project, .. } => {
                self.last_snapshot
                    .agents
                    .iter()
                    .filter(|agent| &agent.project == project)
                    .count()
                    + self
                        .acp_metadata
                        .values()
                        .filter(|thread| &thread.project_id == project)
                        .count()
            }
            DeleteTarget::RemoteProject { host, project, .. } => self
                .remote_snaps
                .get(host)
                .map(|snapshot| {
                    snapshot
                        .agents
                        .iter()
                        .filter(|agent| &agent.project == project)
                        .count()
                })
                .unwrap_or(0),
            DeleteTarget::RemoteMachine { .. } => 0,
        };
        self.tree_menu = None;
        self.delete_error = None;
        self.delete_confirm = Some(DeleteConfirm {
            target,
            affected_sessions,
        });
        cx.notify();
    }

    pub(crate) fn cancel_delete(&mut self, cx: &mut Context<Self>) {
        if self.delete_busy {
            return;
        }
        self.delete_confirm = None;
        self.delete_error = None;
        cx.notify();
    }

    pub(crate) fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        if self.delete_busy {
            return;
        }
        let Some(confirm) = self.delete_confirm.clone() else {
            return;
        };
        self.delete_busy = true;
        match confirm.target {
            DeleteTarget::LocalProject { project, .. } => {
                let server = Arc::clone(&self.server);
                let project_for_delete = project.clone();
                let affected_agents: Vec<_> = self
                    .last_snapshot
                    .agents
                    .iter()
                    .filter(|agent| agent.project == project)
                    .map(|agent| agent.id.clone())
                    .collect();
                let project_key = ProjectKey::new(self.local_machine_id(), project.clone());
                cx.spawn(async move |this, cx| {
                    let (result, snapshot) = cx
                        .background_spawn(async move {
                            let result = server.delete_project(&project_for_delete).await;
                            let snapshot = server.snapshot().await;
                            (result, snapshot)
                        })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.last_snapshot = snapshot;
                        match result {
                            Ok(result) => {
                                this.cleanup_removed_agents(&result.destroyed_agents, cx);
                                if result.failed_agents.is_empty() {
                                    this.remove_acp_project_sessions(&project_key.project_id, cx);
                                    this.remove_project_workspace(&project_key);
                                    this.ensure_active_terminal(cx);
                                    this.delete_confirm = None;
                                } else {
                                    this.delete_error = Some(
                                        i18n::text(this.language, "error.delete_sessions_project")
                                            .replace(
                                                "{count}",
                                                &result.failed_agents.len().to_string(),
                                            ),
                                    );
                                }
                            }
                            Err(error) => {
                                let removed: Vec<_> = affected_agents
                                    .iter()
                                    .filter(|agent| this.last_snapshot.agent(agent).is_none())
                                    .cloned()
                                    .collect();
                                this.cleanup_removed_agents(&removed, cx);
                                this.delete_error = Some(error.to_string());
                            }
                        }
                        this.delete_busy = false;
                        this.persist();
                        cx.notify();
                    });
                })
                .detach();
            }
            DeleteTarget::RemoteProject { host, project, .. } => {
                let remote = self
                    .remotes
                    .iter()
                    .find(|remote| remote.cfg.name == host)
                    .cloned();
                let Some(remote) = remote else {
                    self.delete_error = Some(
                        i18n::text(self.language, "error.remote_unavailable_for_delete").into(),
                    );
                    self.delete_busy = false;
                    cx.notify();
                    return;
                };
                let project_for_rpc = project.clone();
                let project_key = self.remote_snaps.get(&host).and_then(|snapshot| {
                    snapshot
                        .machine
                        .as_ref()
                        .map(|machine| ProjectKey::new(machine.machine_id.clone(), project.clone()))
                });
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(
                            async move { remote.delete_project(&project_for_rpc).await },
                        )
                        .await;
                    let _ = this.update(cx, |this, cx| match result {
                        Ok(result) => {
                            if let Some(snapshot) = this.remote_snaps.get_mut(&host) {
                                if result.failed_agents.is_empty() {
                                    snapshot.projects.retain(|item| item.id != project);
                                }
                                snapshot
                                    .agents
                                    .retain(|agent| !result.destroyed_agents.contains(&agent.id));
                            }
                            this.cleanup_removed_agents(&result.destroyed_agents, cx);
                            if result.failed_agents.is_empty() {
                                if let Some(project_key) = project_key.as_ref() {
                                    this.remove_project_workspace(project_key);
                                    this.ensure_active_terminal(cx);
                                }
                                this.delete_confirm = None;
                            } else {
                                this.delete_error = Some(
                                    i18n::text(this.language, "error.delete_remote_sessions")
                                        .replace(
                                            "{count}",
                                            &result.failed_agents.len().to_string(),
                                        ),
                                );
                            }
                            this.delete_busy = false;
                            this.persist();
                            cx.notify();
                        }
                        Err(error) => {
                            this.delete_error = Some(error.to_string());
                            this.delete_busy = false;
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
            DeleteTarget::RemoteMachine { host } => {
                let machine_id = self
                    .remotes
                    .iter()
                    .find(|remote| remote.cfg.name == host)
                    .and_then(|remote| remote.machine_id())
                    .or_else(|| {
                        self.remote_snaps
                            .get(&host)
                            .and_then(|snapshot| snapshot.machine.as_ref())
                            .map(|machine| machine.machine_id.clone())
                    });
                let removed_agents: Vec<_> = self
                    .remote_snaps
                    .get(&host)
                    .map(|snapshot| {
                        snapshot
                            .agents
                            .iter()
                            .map(|agent| agent.id.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(remote) = self
                    .remotes
                    .iter()
                    .find(|remote| remote.cfg.name == host)
                    .cloned()
                {
                    remote.stop();
                }
                let release_host = host.clone();
                self.server.rt_spawn(async move {
                    muxlane_client::release_remote_tunnel(&release_host).await;
                });
                self.remotes.retain(|remote| remote.cfg.name != host);
                self.remote_snaps.remove(&host);
                self.remote_states.remove(&host);
                self.cleanup_removed_agents(&removed_agents, cx);
                if let Some(machine_id) = machine_id.as_deref() {
                    self.remove_machine_workspaces(machine_id);
                    self.ensure_active_terminal(cx);
                }
                self.delete_confirm = None;
                self.delete_busy = false;
                self.persist();
                cx.notify();
            }
        }
    }

    pub(crate) fn cancel_bootstrap_for_host(&mut self, host: &str, cx: &mut Context<Self>) {
        if let Some(remote) = self.remotes.iter().find(|r| r.cfg.name == host) {
            remote.cancel_bootstrap();
        }
        self.bootstrap_progress.remove(host);
        if self.bootstrap_confirm.as_ref().map(|c| c.host.as_str()) == Some(host) {
            self.bootstrap_confirm = None;
            self.bootstrap_error = None;
        }
        cx.notify();
    }

    pub(crate) fn confirm_bootstrap(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.bootstrap_confirm.clone() else {
            return;
        };
        let Some(remote) = self
            .remotes
            .iter()
            .find(|remote| remote.cfg.name == confirm.host)
            .cloned()
        else {
            self.bootstrap_error =
                Some(i18n::text(self.language, "error.remote_machine_missing").into());
            cx.notify();
            return;
        };
        let host_name = confirm.host.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if confirm.upgrade {
                        remote.upgrade_and_retry().await
                    } else if confirm.install {
                        remote.install_and_start().await
                    } else {
                        remote
                            .start_and_retry(confirm.binary.as_deref().unwrap_or("muxlane"))
                            .await
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.bootstrap_progress.remove(&host_name);
                match result {
                    Ok(()) => {
                        this.bootstrap_confirm = None;
                        this.bootstrap_error = None;
                        cx.notify();
                    }
                    Err(error) => {
                        let error = error.to_string();
                        if error.contains("已取消") {
                            this.bootstrap_confirm = None;
                            this.bootstrap_error = None;
                        } else {
                            this.bootstrap_error = Some(error);
                        }
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub(crate) fn spawn_remote_agent(
        &mut self,
        request: RemoteAgentSpawnRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let RemoteAgentSpawnRequest {
            host,
            project,
            preset,
            preferred_pane,
            split_axis,
        } = request;
        let remote = self
            .remotes
            .iter()
            .find(|remote| remote.cfg.name == host)
            .cloned();
        let Some(remote) = remote else {
            return;
        };
        let machine_id = self
            .remote_snaps
            .get(&host)
            .and_then(|snapshot| snapshot.machine.as_ref())
            .map(|machine| machine.machine_id.clone());
        let Some(machine_id) = machine_id else {
            return;
        };
        let target_key = ProjectKey::new(machine_id, project.clone());
        let pane = self.capture_spawn_target(&target_key, preferred_pane.as_ref());
        let target_project = project.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { remote.spawn_agent(&project, preset.as_ref()).await },
                )
                .await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(agent) => {
                    let agent_id = agent.id.clone();
                    if let Some(snapshot) = this.remote_snaps.get_mut(&host) {
                        if let Some(project) = snapshot
                            .projects
                            .iter_mut()
                            .find(|project| project.id == agent.project)
                        {
                            project.agents.push(agent_id.clone());
                        }
                        snapshot.agents.push(agent);
                    }
                    let remote_project_key = format!("remote:{}:{}", host, target_project);
                    this.collapsed_projects.remove(&remote_project_key);
                    let remote_machine_key = format!("machine:remote:{}", host);
                    this.collapsed_machines.remove(&remote_machine_key);
                    this.jump_to_project_if_needed(&target_key, cx);
                    this.place_async_agent(
                        &target_key,
                        agent_id,
                        Some(pane),
                        split_axis,
                        window,
                        cx,
                    );
                }
                Err(error) => {
                    let text = error.to_string();
                    this.notifications.update(cx, |center, cx| {
                        center.show_error(
                            i18n::text(this.language, "error.remote_create_session")
                                .replace("{error}", &text),
                            cx,
                        )
                    });
                    // 类型不匹配通常意味着远端仍在运行旧版 Muxlane，
                    // 直接切换到已有的更新引导状态。
                    if matches!(
                        error.downcast_ref::<muxlane_client::RemoteCompatError>(),
                        Some(muxlane_client::RemoteCompatError::VersionSkew { .. })
                    ) {
                        if let Some(remote) = this
                            .remotes
                            .iter()
                            .find(|remote| remote.cfg.name == host)
                            .cloned()
                        {
                            cx.spawn(async move |_, _| {
                                remote.mark_needs_upgrade().await;
                            })
                            .detach();
                        }
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn remote_ready_for_project_add(
        state: Option<&muxlane_client::RemoteState>,
        snapshot: Option<&muxlane_core::model::Snapshot>,
    ) -> bool {
        matches!(state, Some(muxlane_client::RemoteState::Online(_)))
            && snapshot
                .and_then(|snapshot| snapshot.machine.as_ref())
                .is_some()
    }

    pub(crate) fn submit_remote_project(
        &mut self,
        host: String,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.submit_remote_project_with_create(host, path, false, cx);
    }

    pub(crate) fn submit_remote_project_with_create(
        &mut self,
        host: String,
        path: String,
        create_if_missing: bool,
        cx: &mut Context<Self>,
    ) {
        if self.project_add_busy {
            return;
        }
        if !Self::remote_ready_for_project_add(
            self.remote_states.get(&host),
            self.remote_snaps.get(&host),
        ) {
            self.project_add_busy = false;
            let error = i18n::text(self.language, "error.remote_not_connected").into();
            if let Some(pending) = self.pending_project_creation.as_mut() {
                pending.error = Some(error);
            } else {
                self.dialog_error = Some(error);
            }
            cx.notify();
            return;
        }
        let remote = self
            .remotes
            .iter()
            .find(|remote| remote.cfg.name == host)
            .cloned();
        let Some(remote) = remote else {
            self.dialog_error =
                Some(i18n::text(self.language, "error.remote_not_connected").into());
            cx.notify();
            return;
        };
        let requested_path = path.trim().to_string();
        if requested_path.is_empty() {
            self.dialog_error =
                Some(i18n::text(self.language, "error.remote_project_required").into());
            cx.notify();
            return;
        }
        self.project_add_busy = true;
        let supports_create = remote.supports(muxlane_core::protocol::features::PROJECT_CREATE);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    remote.add_project(&requested_path, create_if_missing).await
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.project_add_busy = false;
                match result {
                    Ok(project) => {
                        if let Some(snapshot) = this.remote_snaps.get_mut(&host) {
                            if !snapshot.projects.iter().any(|item| item.id == project.id) {
                                snapshot.projects.push(project);
                            }
                        }
                        this.remote_project_dialog = None;
                        this.pending_project_creation = None;
                        this.dialog_error = None;
                        this.remote_project_input
                            .update(cx, |input, cx| input.reset(cx));
                        this.persist();
                    }
                    Err(error) => {
                        let text = error.to_string();
                        let error_code = error
                            .downcast_ref::<muxlane_client::RpcCallError>()
                            .map(|error| error.code.as_str());
                        let create_unsupported = matches!(
                            error.downcast_ref::<muxlane_client::RemoteCompatError>(),
                            Some(muxlane_client::RemoteCompatError::FeatureUnsupported { .. })
                        );
                        if crate::dialogs::should_prompt_project_creation(
                            error_code,
                            create_if_missing,
                        ) {
                            if supports_create {
                                this.pending_project_creation =
                                    Some(crate::menus::PendingProjectCreation {
                                        target: crate::menus::ProjectCreationTarget::Remote {
                                            host: host.clone(),
                                            path: path.trim().to_string(),
                                        },
                                        error: None,
                                    });
                                this.dialog_error = None;
                            } else {
                                this.dialog_error = Some(
                                    i18n::text(
                                        this.language,
                                        "error.remote_create_directory_unsupported",
                                    )
                                    .into(),
                                );
                            }
                        } else if matches!(
                            error.downcast_ref::<muxlane_client::RemoteCompatError>(),
                            Some(muxlane_client::RemoteCompatError::MethodUnsupported { method })
                                if method == muxlane_core::protocol::methods::PROJECT_ADD
                        ) {
                            // 旧版远端没有 project.add：转入升级引导，而不是弹原始错误
                            this.remote_project_dialog = None;
                            this.dialog_error = None;
                            if let Some(remote) = this
                                .remotes
                                .iter()
                                .find(|remote| remote.cfg.name == host)
                                .cloned()
                            {
                                cx.spawn(async move |this, cx| {
                                    remote.mark_needs_upgrade().await;
                                    let _ = this.update(cx, |_, _| {});
                                })
                                .detach();
                            }
                        } else if let Some(pending) = this.pending_project_creation.as_mut() {
                            pending.error = Some(if create_unsupported {
                                i18n::text(
                                    this.language,
                                    "error.remote_create_directory_unsupported",
                                )
                                .into()
                            } else {
                                text
                            });
                        } else {
                            this.dialog_error = Some(if create_unsupported {
                                i18n::text(
                                    this.language,
                                    "error.remote_create_directory_unsupported",
                                )
                                .into()
                            } else {
                                text
                            });
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_with_machine() -> muxlane_core::model::Snapshot {
        muxlane_core::model::Snapshot {
            machine: Some(muxlane_core::model::MachineInfo {
                machine_id: "machine-remote".into(),
                name: "remote".into(),
                os: "linux".into(),
                version: "test".into(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn replacing_remote_preserves_old_identity_until_new_snapshot_arrives() {
        let (events_tx, _events_rx) = tokio::sync::mpsc::channel(1);
        let old = muxlane_client::RemoteHost::new(
            muxlane_client::HostCfg {
                name: "remote".into(),
                target: muxlane_client::Target::Socket("/tmp/old.sock".into()),
                auth: muxlane_client::SshAuth::default(),
                retry_base_ms: 200,
            },
            events_tx.clone(),
        );
        old.restore_machine_id(Some("machine-a".into()));
        let replacement = muxlane_client::RemoteHost::new(
            muxlane_client::HostCfg {
                name: "remote".into(),
                target: muxlane_client::Target::Socket("/tmp/new.sock".into()),
                auth: muxlane_client::SshAuth::default(),
                retry_base_ms: 200,
            },
            events_tx,
        );
        replacement.restore_machine_id(old.machine_id());

        let previous = replacement.machine_id();
        assert_eq!(previous.as_deref(), Some("machine-a"));
        assert!(replacement.cache_machine_id(Some("machine-b")));
        assert_eq!(
            replaced_machine_id(previous, replacement.machine_id().as_deref()),
            Some("machine-a".into())
        );
        assert_eq!(
            replaced_machine_id(Some("machine-a".into()), Some("machine-a")),
            None
        );
        assert_eq!(replaced_machine_id(Some("machine-a".into()), None), None);
    }

    #[test]
    fn remote_project_submission_requires_online_state_and_machine_snapshot() {
        let snapshot = snapshot_with_machine();
        let online = muxlane_client::RemoteState::Online(snapshot.clone());
        assert!(MuxlaneApp::remote_ready_for_project_add(
            Some(&online),
            Some(&snapshot)
        ));
        assert!(!MuxlaneApp::remote_ready_for_project_add(
            Some(&muxlane_client::RemoteState::Connecting(
                muxlane_client::RemoteStage::Subscribe,
            )),
            Some(&snapshot)
        ));
        assert!(!MuxlaneApp::remote_ready_for_project_add(
            Some(&online),
            Some(&muxlane_core::model::Snapshot::default())
        ));
        assert!(!MuxlaneApp::remote_ready_for_project_add(
            None,
            Some(&snapshot)
        ));
    }
}
