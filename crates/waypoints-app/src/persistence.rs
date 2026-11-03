//! Serializes workspace snapshots off the UI thread and flushes them before quit.

use gpui_kit::App;
use tokio::sync::{mpsc, oneshot};

use crate::preferences::{self, WorkspacePreferences};

enum Request {
    Save(WorkspacePreferences),
    Flush(oneshot::Sender<Result<(), String>>),
}

#[derive(Clone)]
pub(crate) struct PreferencesWriter {
    requests: mpsc::UnboundedSender<Request>,
}

impl PreferencesWriter {
    pub(crate) fn new(cx: &App) -> (Self, mpsc::UnboundedReceiver<String>) {
        let (requests, receiver) = mpsc::unbounded_channel();
        let (errors, error_receiver) = mpsc::unbounded_channel();
        cx.background_executor()
            .spawn(write_requests(
                receiver,
                errors,
                preferences::save_preferences,
            ))
            .detach();
        (Self { requests }, error_receiver)
    }

    pub(crate) fn save(&self, preferences: WorkspacePreferences) {
        // The receiver lives until the last sender is dropped.
        let _ = self.requests.send(Request::Save(preferences));
    }

    pub(crate) async fn flush(&self) -> Result<(), String> {
        let (sender, receiver) = oneshot::channel();
        self.requests
            .send(Request::Flush(sender))
            .map_err(|_| "Workspace writer stopped.".to_owned())?;
        receiver
            .await
            .map_err(|_| "Workspace writer stopped before saving.".to_owned())?
    }
}

async fn write_requests(
    mut requests: mpsc::UnboundedReceiver<Request>,
    errors: mpsc::UnboundedSender<String>,
    mut save: impl FnMut(&WorkspacePreferences) -> anyhow::Result<()>,
) {
    let mut pending = None;
    while let Some(first) = requests.recv().await {
        let mut barriers = Vec::new();
        let mut request = Some(first);
        while let Some(next) = request {
            match next {
                Request::Save(preferences) => pending = Some(preferences),
                Request::Flush(barrier) => barriers.push(barrier),
            }
            request = requests.try_recv().ok();
        }
        // Coalesce queued resize/layout changes, but never run concurrent writes.
        let outcome = pending
            .as_ref()
            .map(&mut save)
            .transpose()
            .map(|_| ())
            .map_err(|error| {
                let message = format!("Couldn’t save workspace preferences: {error:#}");
                let _ = errors.send(message.clone());
                message
            });
        if outcome.is_ok() {
            pending = None;
        }
        for barrier in barriers {
            let _ = barrier.send(outcome.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_snapshots_are_coalesced_before_flush() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime");
        runtime.block_on(async {
            let (requests, receiver) = mpsc::unbounded_channel();
            let (errors, mut error_receiver) = mpsc::unbounded_channel();
            let mut first = WorkspacePreferences::default();
            requests
                .send(Request::Save(first.clone()))
                .expect("queue first");
            first.sidebar_visible = false;
            requests
                .send(Request::Save(first.clone()))
                .expect("queue latest");
            let (barrier, flushed) = oneshot::channel();
            requests.send(Request::Flush(barrier)).expect("queue flush");
            drop(requests);
            let mut saved = Vec::new();
            write_requests(receiver, errors, |snapshot| {
                saved.push(snapshot.clone());
                Ok(())
            })
            .await;
            flushed.await.expect("flush acknowledged").expect("saved");
            assert_eq!(saved, vec![first]);
            assert!(error_receiver.recv().await.is_none());
        });
    }

    #[test]
    fn save_failure_is_reported_to_flush() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime");
        runtime.block_on(async {
            let (requests, receiver) = mpsc::unbounded_channel();
            let (errors, mut error_receiver) = mpsc::unbounded_channel();
            requests
                .send(Request::Save(WorkspacePreferences::default()))
                .expect("queue snapshot");
            let (barrier, flushed) = oneshot::channel();
            requests.send(Request::Flush(barrier)).expect("queue flush");
            drop(requests);
            write_requests(receiver, errors, |_| anyhow::bail!("disk unavailable")).await;
            assert!(
                flushed
                    .await
                    .expect("flush acknowledged after failure")
                    .is_err()
            );
            assert!(
                error_receiver
                    .recv()
                    .await
                    .expect("visible error")
                    .contains("disk unavailable")
            );
        });
    }

    #[test]
    fn flush_retries_the_latest_snapshot_after_transient_failure() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime");
        runtime.block_on(async {
            let (requests, receiver) = mpsc::unbounded_channel();
            let (errors, mut error_receiver) = mpsc::unbounded_channel();
            let expected = WorkspacePreferences {
                sidebar_visible: false,
                ..WorkspacePreferences::default()
            };
            requests
                .send(Request::Save(expected.clone()))
                .expect("queue snapshot");
            let mut attempts = Vec::new();
            let writing = write_requests(receiver, errors, |snapshot| {
                attempts.push(snapshot.clone());
                if attempts.len() == 1 {
                    anyhow::bail!("transient failure");
                }
                Ok(())
            });
            let recovering = async {
                error_receiver.recv().await.expect("first attempt failed");
                let (barrier, flushed) = oneshot::channel();
                requests.send(Request::Flush(barrier)).expect("queue retry");
                flushed
                    .await
                    .expect("flush acknowledged")
                    .expect("retry saved");
                drop(requests);
            };
            tokio::join!(writing, recovering);
            assert_eq!(attempts, vec![expected.clone(), expected]);
        });
    }
}
