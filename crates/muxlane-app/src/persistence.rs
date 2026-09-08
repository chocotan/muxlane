use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::JoinHandle;

pub(crate) struct PersistenceWriter {
    sender: Option<mpsc::Sender<muxlane_store::PersistedApp>>,
    worker: Option<JoinHandle<()>>,
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
        if let Some(sender) = &self.sender {
            if let Err(error) = sender.send(app) {
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

fn run(path: PathBuf, receiver: mpsc::Receiver<muxlane_store::PersistedApp>) {
    while let Ok(mut app) = receiver.recv() {
        while let Ok(newer) = receiver.try_recv() {
            app = newer;
        }
        if let Err(error) = muxlane_store::save(&path, &app) {
            tracing::warn!(%error, "persist state failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_flushes_latest_app() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let app = muxlane_store::PersistedApp {
            sidebar_width: 317.0,
            ..Default::default()
        };
        let writer = PersistenceWriter::new(path.clone());
        writer.submit_app(muxlane_store::PersistedApp::default());
        writer.submit_app(app);
        drop(writer);

        assert_eq!(muxlane_store::load(&path).unwrap().sidebar_width, 317.0);
    }
}
