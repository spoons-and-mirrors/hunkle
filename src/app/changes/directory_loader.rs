use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
};

use crate::{filesystem, filesystem::WorkspaceEntry, repo_path::RepoPath};

pub(super) struct DirectoryLoader {
    sender: Option<Sender<Request>>,
    receiver: Receiver<Completion>,
    worker: Option<JoinHandle<()>>,
    latest_generation: Arc<AtomicU64>,
}

pub(super) struct Completion {
    pub(super) generation: u64,
    pub(super) root: PathBuf,
    pub(super) directory: RepoPath,
    pub(super) result: Result<Vec<WorkspaceEntry>, String>,
}

struct Request {
    generation: u64,
    root: PathBuf,
    directory: RepoPath,
}

impl DirectoryLoader {
    pub(super) fn new() -> Self {
        let (sender, request_rx) = mpsc::channel::<Request>();
        let (result_tx, receiver) = mpsc::channel();
        let latest_generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&latest_generation);
        let worker = thread::spawn(move || {
            while let Ok(request) = request_rx.recv() {
                if request.generation != worker_generation.load(Ordering::Relaxed) {
                    continue;
                }
                let result =
                    filesystem::read_workspace_directory(&request.root, &request.directory)
                        .map_err(|error| error.to_string());
                if result_tx
                    .send(Completion {
                        generation: request.generation,
                        root: request.root,
                        directory: request.directory,
                        result,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            sender: Some(sender),
            receiver,
            worker: Some(worker),
            latest_generation,
        }
    }

    pub(super) fn request(&self, generation: u64, root: &Path, directory: RepoPath) {
        self.latest_generation.store(generation, Ordering::Relaxed);
        if let Some(sender) = &self.sender {
            let _ = sender.send(Request {
                generation,
                root: root.to_owned(),
                directory,
            });
        }
    }

    pub(super) fn poll(&self) -> Option<Completion> {
        self.receiver.try_recv().ok()
    }

    pub(super) fn shutdown(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for DirectoryLoader {
    fn drop(&mut self) {
        self.shutdown();
    }
}
