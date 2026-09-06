use muxlane_core::model::AgentId;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::JoinHandle;

pub(crate) struct PersistenceWriter {
    sender: Option<mpsc::Sender<PersistenceCommand>>,
    worker: Option<JoinHandle<()>>,
}

#[allow(clippy::large_enum_variant)]
enum PersistenceCommand {
    App(muxlane_store::PersistedApp),
    UpsertAcp(muxlane_store::PersistedAcpThreadData),
    DeleteAcp { ui_id: AgentId, revision: u64 },
}

enum AcpOperation {
    Upsert(Box<muxlane_store::PersistedAcpThreadData>),
    Delete { ui_id: AgentId, revision: u64 },
}

#[derive(Clone, Copy)]
struct AcpOperationStamp {
    revision: u64,
    delete: bool,
}

impl AcpOperationStamp {
    fn supersedes(self, existing: Self) -> bool {
        self.revision > existing.revision
            || (self.revision == existing.revision && self.delete && !existing.delete)
    }
}

impl AcpOperation {
    fn stamp(&self) -> AcpOperationStamp {
        match self {
            Self::Upsert(record) => AcpOperationStamp {
                revision: record.write_revision,
                delete: false,
            },
            Self::Delete { revision, .. } => AcpOperationStamp {
                revision: *revision,
                delete: true,
            },
        }
    }
}

impl PersistenceWriter {
    pub(crate) fn new(path: PathBuf) -> Self {
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || run(path, receiver));
        Self {
            sender: Some(sender),
            worker: Some(worker),
        }
    }

    pub(crate) fn submit_app(&self, app: muxlane_store::PersistedApp) {
        self.send(PersistenceCommand::App(app));
    }

    pub(crate) fn upsert_acp(&self, record: muxlane_store::PersistedAcpThreadData) {
        self.send(PersistenceCommand::UpsertAcp(record));
    }

    pub(crate) fn delete_acp(&self, ui_id: AgentId, revision: u64) {
        self.send(PersistenceCommand::DeleteAcp { ui_id, revision });
    }

    fn send(&self, command: PersistenceCommand) {
        if let Some(sender) = &self.sender {
            if let Err(error) = sender.send(command) {
                tracing::warn!(%error, "persist queue failed");
            }
        }
    }
}

impl Drop for PersistenceWriter {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                tracing::warn!("persist worker terminated unexpectedly");
            }
        }
    }
}

fn run(path: PathBuf, receiver: mpsc::Receiver<PersistenceCommand>) {
    let mut applied_acp = BTreeMap::<AgentId, AcpOperationStamp>::new();
    while let Ok(command) = receiver.recv() {
        let mut latest_app = None;
        let mut latest_acp = BTreeMap::new();
        absorb(command, &mut latest_app, &mut latest_acp);
        while let Ok(command) = receiver.try_recv() {
            absorb(command, &mut latest_app, &mut latest_acp);
        }

        for (ui_id, operation) in latest_acp {
            let stamp = operation.stamp();
            if applied_acp
                .get(&ui_id)
                .is_some_and(|existing| !stamp.supersedes(*existing))
            {
                continue;
            }
            let result = match operation {
                AcpOperation::Upsert(record) => {
                    muxlane_store::save_acp_thread_if_newer(&path, &record).map(|_| ())
                }
                AcpOperation::Delete { ui_id, revision } => {
                    muxlane_store::delete_acp_thread_if_not_newer(&path, &ui_id, revision)
                        .map(|_| ())
                }
            };
            match result {
                Ok(()) => {
                    applied_acp.insert(ui_id, stamp);
                }
                Err(error) => tracing::warn!(%error, "persist ACP thread failed"),
            }
        }
        if let Some(app) = latest_app {
            if let Err(error) = muxlane_store::save(&path, &app) {
                tracing::warn!(%error, "persist state failed");
            }
        }
    }
}

fn absorb(
    command: PersistenceCommand,
    latest_app: &mut Option<muxlane_store::PersistedApp>,
    latest_acp: &mut BTreeMap<AgentId, AcpOperation>,
) {
    match command {
        PersistenceCommand::App(app) => *latest_app = Some(app),
        PersistenceCommand::UpsertAcp(record) => {
            let ui_id = record.metadata.ui_id.clone();
            replace_if_newer(latest_acp, ui_id, AcpOperation::Upsert(Box::new(record)));
        }
        PersistenceCommand::DeleteAcp { ui_id, revision } => {
            replace_if_newer(
                latest_acp,
                ui_id.clone(),
                AcpOperation::Delete { ui_id, revision },
            );
        }
    }
}

fn replace_if_newer(
    latest_acp: &mut BTreeMap<AgentId, AcpOperation>,
    ui_id: AgentId,
    operation: AcpOperation,
) {
    let should_replace = latest_acp
        .get(&ui_id)
        .is_none_or(|existing| operation.stamp().supersedes(existing.stamp()));
    if should_replace {
        latest_acp.insert(ui_id, operation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(
        ui_id: &str,
        revision: u64,
        snapshot_revision: u64,
    ) -> muxlane_store::PersistedAcpThreadData {
        let mut record =
            muxlane_store::PersistedAcpThreadData::new(muxlane_store::PersistedAcpThread {
                ui_id: ui_id.into(),
                ..Default::default()
            });
        record.write_revision = revision;
        record.snapshot.revision = snapshot_revision;
        record
    }

    #[test]
    fn newest_upsert_wins() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let writer = PersistenceWriter::new(path.clone());
        writer.upsert_acp(record("acp_batch", 1, 1));
        writer.upsert_acp(record("acp_batch", 2, 2));
        drop(writer);

        let loaded = muxlane_store::load_acp_threads(&path);
        assert_eq!(loaded.records[0].snapshot.revision, 2);
    }

    #[test]
    fn delete_supersedes_older_upsert() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let writer = PersistenceWriter::new(path.clone());
        writer.upsert_acp(record("acp_batch", 1, 1));
        writer.delete_acp("acp_batch".into(), 2);
        drop(writer);

        assert!(muxlane_store::load_acp_threads(&path).records.is_empty());
    }

    #[test]
    fn stale_later_upsert_cannot_resurrect_newer_delete() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let writer = PersistenceWriter::new(path.clone());
        writer.upsert_acp(record("acp_batch", 2, 2));
        writer.delete_acp("acp_batch".into(), 3);
        writer.upsert_acp(record("acp_batch", 2, 4));
        drop(writer);

        assert!(muxlane_store::load_acp_threads(&path).records.is_empty());
    }

    #[test]
    fn drop_flushes_latest_app_and_acp() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let app = muxlane_store::PersistedApp {
            sidebar_width: 317.0,
            ..Default::default()
        };
        let writer = PersistenceWriter::new(path.clone());
        writer.submit_app(app);
        writer.upsert_acp(record("acp_batch", 1, 7));
        drop(writer);

        assert_eq!(muxlane_store::load(&path).unwrap().sidebar_width, 317.0);
        assert_eq!(
            muxlane_store::load_acp_threads(&path).records[0]
                .snapshot
                .revision,
            7
        );
    }
}

#[cfg(test)]
mod coalescing_tests {
    use super::*;

    #[test]
    fn equal_revision_uses_later_command_deterministically() {
        let mut app = None;
        let mut acp = BTreeMap::new();
        absorb(
            PersistenceCommand::UpsertAcp({
                let mut record =
                    muxlane_store::PersistedAcpThreadData::new(muxlane_store::PersistedAcpThread {
                        ui_id: "acp_equal".into(),
                        ..Default::default()
                    });
                record.write_revision = 4;
                record
            }),
            &mut app,
            &mut acp,
        );
        absorb(
            PersistenceCommand::DeleteAcp {
                ui_id: "acp_equal".into(),
                revision: 4,
            },
            &mut app,
            &mut acp,
        );
        assert!(matches!(
            acp.get("acp_equal"),
            Some(AcpOperation::Delete { .. })
        ));
    }

    #[test]
    fn equal_revision_delete_remains_authoritative_over_later_upsert() {
        let mut app = None;
        let mut acp = BTreeMap::new();
        absorb(
            PersistenceCommand::DeleteAcp {
                ui_id: "acp_equal".into(),
                revision: 4,
            },
            &mut app,
            &mut acp,
        );
        absorb(
            PersistenceCommand::UpsertAcp({
                let mut record =
                    muxlane_store::PersistedAcpThreadData::new(muxlane_store::PersistedAcpThread {
                        ui_id: "acp_equal".into(),
                        ..Default::default()
                    });
                record.write_revision = 4;
                record
            }),
            &mut app,
            &mut acp,
        );
        assert!(matches!(
            acp.get("acp_equal"),
            Some(AcpOperation::Delete { .. })
        ));
    }
}
