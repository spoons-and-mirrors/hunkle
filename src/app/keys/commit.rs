use super::super::*;

impl App {
    pub(crate) fn restore_commit_draft(&mut self) {
        self.commit_input.clear();
        self.commit_scroll = None;
        self.commit_draft_due = None;
        self.commit_draft_path = None;
        self.commit_draft_rx = None;
        let Some(root) = self
            .repository()
            .filter(|repo| !repo.is_local())
            .map(|repo| repo.root.clone())
        else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.commit_draft_rx = Some(receiver);
        thread::spawn(move || {
            let result = git::commit_draft_path(&root)
                .map_err(|error| error.to_string())
                .and_then(|path| match fs::read_to_string(&path) {
                    Ok(message) => Ok((path, Some(message))),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((path, None)),
                    Err(error) => Err(error.to_string()),
                });
            let _ = sender.send(CommitDraftResult { root, result });
        });
    }

    pub(crate) fn schedule_commit_draft(&mut self) {
        self.commit_draft_due = Some(Instant::now() + Duration::from_millis(300));
    }

    pub(crate) fn flush_commit_draft_if_due(&mut self) -> bool {
        if self
            .commit_draft_due
            .is_some_and(|due| Instant::now() >= due)
        {
            return self.flush_commit_draft();
        }
        false
    }

    pub(crate) fn flush_commit_draft(&mut self) -> bool {
        if self.commit_draft_due.is_none() {
            return false;
        }
        let Some(path) = &self.commit_draft_path else {
            if self.commit_draft_rx.is_some() {
                return false;
            }
            self.commit_draft_due = None;
            return false;
        };
        let result = if self.commit_input.is_empty() {
            match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            }
        } else {
            atomic_write(path, self.commit_input.text().as_bytes())
        };
        if let Err(error) = result {
            self.commit_draft_due = Some(Instant::now() + Duration::from_secs(1));
            self.notice = Some(format!("Could not save commit draft: {error}"));
            return true;
        }
        self.commit_draft_due = None;
        false
    }

    pub(crate) fn commit_draft_pending(&self) -> bool {
        self.commit_draft_due.is_some()
    }

    pub(crate) fn start_commit(&mut self) {
        if !self.require_git_repository() {
            self.mode = Mode::Normal;
            return;
        }
        if self.magic_commit_running_for_active_repository() {
            self.notice = Some("Magic Commit is still running".to_owned());
            return;
        }
        if self.session.commit_running() {
            self.notice = Some("A commit is already running".to_owned());
            return;
        }
        if self.session.command_running() {
            self.notice = Some("Another Git operation is already running".to_owned());
            return;
        }
        let message = self.commit_input.text().trim().to_owned();
        if message.is_empty() {
            self.notice = Some("Commit message cannot be empty".to_owned());
            return;
        }
        self.flush_commit_draft();
        if self.session.start_commit(message) {
            self.mode = Mode::Normal;
        }
    }

    pub(crate) fn generate_commit_message(&mut self) {
        if self.magic_commit_running_for_active_repository() {
            self.notice = Some("Magic Commit is still running".to_owned());
            return;
        }
        let Some(repo) = self.git_repository() else {
            self.notice = Some("Open a Git repository first".to_owned());
            return;
        };
        if repo.changes.is_empty() {
            self.notice = Some("No changes to describe".to_owned());
            return;
        }
        let root = repo.root.clone();
        let baseline = self.commit_input.text().to_owned();
        let model = self.settings.opencode_model.clone();
        let variant = self
            .settings
            .opencode_reasoning
            .variant()
            .map(str::to_owned);
        match self
            .commit_message_generator
            .start(root, baseline, model, variant)
        {
            Ok(()) => {
                self.mode = Mode::Commit;
                self.commit_input.focus();
                self.notice = Some("Generating commit message with OpenCode…".to_owned());
            }
            Err(error) => self.notice = Some(error),
        }
    }

    pub(crate) fn start_magic_commit(&mut self) {
        if self.mode != Mode::Normal || self.session.open_running() {
            return;
        }
        self.header_picker.close();
        if self.magic_commit_running_for_active_repository() {
            if self.magic_commit_runner.cancel() {
                self.notice = Some("Cancelling Magic Commit…".to_owned());
            }
            return;
        }
        if self.session.commit_running() || self.session.command_running() {
            self.notice = Some("Another Git operation is already running".to_owned());
            return;
        }
        if self.commit_message_running() {
            self.notice = Some("Wait for commit message generation to finish".to_owned());
            return;
        }
        let Some(repository) = self.git_repository() else {
            self.notice = Some("Magic Commit requires a Git repository".to_owned());
            return;
        };
        if !repository.details_ready {
            self.notice = Some("Repository details are still loading".to_owned());
            return;
        }
        if repository.changes.is_empty() {
            self.notice = Some("No changes to commit".to_owned());
            return;
        }
        let root = repository.root.clone();
        let model = self.settings.opencode_model.clone();
        let variant = self
            .settings
            .opencode_reasoning
            .variant()
            .map(str::to_owned);
        match self.magic_commit_runner.start(root, model, variant) {
            Ok(()) => self.notice = Some("Magic Commit is organizing logical commits…".to_owned()),
            Err(error) => self.notice = Some(error),
        }
    }

    pub(crate) fn receive_magic_commit(&mut self, completion: MagicCommitCompletion) {
        let active = self
            .repository()
            .is_some_and(|repository| same_path(&repository.root, &completion.root));
        if active
            && let Some(request) = self
                .session
                .request_refresh(RefreshScope::ALL, self.settings.fetch_interval())
        {
            self.track_refresh_request(request, false);
        }
        self.notice = Some(match completion.result {
            Ok(()) if active => "Magic Commit finished".to_owned(),
            Ok(()) => format!("Magic Commit finished for {}", completion.root.display()),
            Err(error) if error.starts_with("Magic Commit cancelled;") => error,
            Err(error) if active => error,
            Err(error) => format!(
                "Magic Commit failed for {}: {error}",
                completion.root.display()
            ),
        });
    }

    pub(crate) fn receive_generated_commit_message(&mut self, completion: CommitMessageCompletion) {
        let active = self
            .repository()
            .is_some_and(|repository| same_path(&repository.root, &completion.root));
        let message = match completion.result {
            Ok(message) => message,
            Err(error) => {
                self.notice = Some(if active {
                    error
                } else {
                    format!(
                        "Could not generate a commit message for {}: {error}",
                        completion.root.display()
                    )
                });
                return;
            }
        };
        let draft_path = match git::commit_draft_path(&completion.root) {
            Ok(path) => path,
            Err(error) => {
                self.notice = Some(format!(
                    "Could not save the generated commit message for {}: {error}",
                    completion.root.display()
                ));
                return;
            }
        };
        let current_message = if active {
            self.commit_input.text().to_owned()
        } else {
            match fs::read_to_string(&draft_path) {
                Ok(message) => message,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(error) => {
                    self.notice = Some(format!(
                        "Could not read the commit draft for {}: {error}",
                        completion.root.display()
                    ));
                    return;
                }
            }
        };
        if current_message != completion.baseline {
            self.notice = Some(format!(
                "Generated commit message for {} was not applied because its draft was edited",
                completion.root.display()
            ));
            return;
        }
        if let Err(error) = atomic_write(&draft_path, message.as_bytes()) {
            self.notice = Some(format!(
                "Could not save the generated commit message for {}: {error}",
                completion.root.display()
            ));
            return;
        }

        if active {
            self.commit_input.set(message);
            self.commit_scroll = None;
            self.commit_input.focus();
            self.commit_draft_path = Some(draft_path);
            self.commit_draft_due = None;
            self.mode = Mode::Commit;
            self.notice = Some("Commit message generated with OpenCode".to_owned());
        } else {
            self.notice = Some(format!(
                "Commit message generated for {}",
                completion.root.display()
            ));
        }
    }

    pub(crate) fn focus_commit(&mut self) {
        if !self.require_git_repository() {
            return;
        }
        self.mode = Mode::Commit;
        self.commit_scroll = None;
        self.commit_input.focus();
    }
}
