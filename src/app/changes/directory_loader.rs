use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
};

use crate::{filesystem, filesystem::WorkspaceEntry, repo_path::RepoPath};

pub(super) struct DirectoryLoader {
    queue: Arc<RequestQueue>,
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

#[derive(Clone, Copy)]
enum Priority {
    Interactive,
    Background,
}

#[derive(Default)]
struct QueueState {
    interactive: VecDeque<Request>,
    background: VecDeque<Request>,
    closed: bool,
}

struct RequestQueue {
    state: Mutex<QueueState>,
    ready: Condvar,
}

impl RequestQueue {
    fn new() -> Self {
        Self {
            state: Mutex::new(QueueState::default()),
            ready: Condvar::new(),
        }
    }

    fn pop(&self) -> Option<Request> {
        let mut state = self.state.lock().expect("directory request queue poisoned");
        loop {
            if state.closed {
                return None;
            }
            if let Some(request) = state
                .interactive
                .pop_front()
                .or_else(|| state.background.pop_front())
            {
                return Some(request);
            }
            state = self
                .ready
                .wait(state)
                .expect("directory request queue poisoned");
        }
    }

    fn push(&self, request: Request, priority: Priority, clear: bool) {
        let mut state = self.state.lock().expect("directory request queue poisoned");
        if clear {
            state.interactive.clear();
            state.background.clear();
        }
        match priority {
            Priority::Interactive => state.interactive.push_back(request),
            Priority::Background => state.background.push_back(request),
        }
        self.ready.notify_one();
    }

    fn promote(&self, generation: u64, directory: &RepoPath) {
        let mut state = self.state.lock().expect("directory request queue poisoned");
        let Some(index) = state.background.iter().position(|request| {
            request.generation == generation && &request.directory == directory
        }) else {
            return;
        };
        if let Some(request) = state.background.remove(index) {
            state.interactive.push_back(request);
        }
        self.ready.notify_one();
    }

    fn close(&self) {
        let mut state = self.state.lock().expect("directory request queue poisoned");
        state.closed = true;
        state.interactive.clear();
        state.background.clear();
        self.ready.notify_one();
    }
}

impl DirectoryLoader {
    pub(super) fn new() -> Self {
        let (result_tx, receiver) = mpsc::channel();
        let latest_generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&latest_generation);
        let queue = Arc::new(RequestQueue::new());
        let worker_queue = Arc::clone(&queue);
        let worker = thread::spawn(move || {
            while let Some(request) = worker_queue.pop() {
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
            queue,
            receiver,
            worker: Some(worker),
            latest_generation,
        }
    }

    pub(super) fn request_interactive(&self, generation: u64, root: &Path, directory: RepoPath) {
        self.request(generation, root, directory, Priority::Interactive);
    }

    pub(super) fn request_background(&self, generation: u64, root: &Path, directory: RepoPath) {
        self.request(generation, root, directory, Priority::Background);
    }

    fn request(&self, generation: u64, root: &Path, directory: RepoPath, priority: Priority) {
        let previous = self.latest_generation.swap(generation, Ordering::Relaxed);
        self.queue.push(
            Request {
                generation,
                root: root.to_owned(),
                directory,
            },
            priority,
            previous != generation,
        );
    }

    pub(super) fn prioritize(&self, generation: u64, directory: &RepoPath) {
        self.queue.promote(generation, directory);
    }

    pub(super) fn poll(&self) -> Option<Completion> {
        self.receiver.try_recv().ok()
    }

    pub(super) fn shutdown(&mut self) {
        self.queue.close();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_requests_run_before_background_requests() {
        let queue = RequestQueue::new();
        queue.push(request("background"), Priority::Background, false);
        queue.push(request("interactive"), Priority::Interactive, false);

        assert_eq!(
            queue.pop().unwrap().directory,
            RepoPath::from("interactive")
        );
        assert_eq!(queue.pop().unwrap().directory, RepoPath::from("background"));
    }

    fn request(directory: &str) -> Request {
        Request {
            generation: 0,
            root: PathBuf::new(),
            directory: directory.into(),
        }
    }
}
