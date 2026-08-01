use std::{
    path::Path,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use super::{AgentPaneDirection, HerdrPaneLayout, TextInput, workspace_panel};

pub(crate) struct HerdrPromptPoll {
    pub(crate) changed: bool,
    pub(crate) completion: Option<Result<HerdrPromptCompletion, String>>,
}

pub(crate) struct HerdrPromptCompletion {
    pub(crate) message: String,
    pub(crate) reopen_path: Option<PathBuf>,
}

pub(crate) struct HerdrPrompt {
    pub(crate) input: TextInput,
    pub(crate) error: Option<String>,
    pub(crate) sending: bool,
    sender: Sender<Result<HerdrPromptCompletion, String>>,
    receiver: Receiver<Result<HerdrPromptCompletion, String>>,
    layout_sender: Sender<(u64, Result<HerdrPaneLayout, String>)>,
    layout_receiver: Receiver<(u64, Result<HerdrPaneLayout, String>)>,
    next_agent_request_id: u64,
    pending_agent: Option<PendingAgent>,
}

struct PendingAgent {
    request_id: u64,
    path: PathBuf,
    branch: String,
    host_pane_id: String,
    layout: Option<HerdrPaneLayout>,
}

impl Default for HerdrPrompt {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        let (layout_sender, layout_receiver) = mpsc::channel();
        Self {
            input: TextInput::default(),
            error: None,
            sending: false,
            sender,
            receiver,
            layout_sender,
            layout_receiver,
            next_agent_request_id: 0,
            pending_agent: None,
        }
    }
}

impl HerdrPrompt {
    #[cfg(test)]
    pub(crate) fn complete_for_test(
        &self,
        message: impl Into<String>,
        reopen_path: Option<PathBuf>,
    ) {
        self.sender
            .send(Ok(HerdrPromptCompletion {
                message: message.into(),
                reopen_path,
            }))
            .unwrap();
    }

    #[cfg(test)]
    pub(crate) fn show_agent_pane_picker(
        &mut self,
        path: PathBuf,
        branch: String,
        host_pane_id: String,
        layout: HerdrPaneLayout,
    ) {
        self.next_agent_request_id = self.next_agent_request_id.wrapping_add(1);
        self.pending_agent = Some(PendingAgent {
            request_id: self.next_agent_request_id,
            path,
            branch,
            host_pane_id,
            layout: Some(layout),
        });
    }

    pub(crate) fn open(&mut self) {
        self.input.clear();
        self.input.focus();
        self.error = None;
    }

    pub(crate) fn submit(&mut self) {
        if self.sending {
            return;
        }
        if self.pending_agent.is_some() {
            self.error = Some("Cancel agent pane selection first".to_owned());
            return;
        }
        if self.input.text().trim().is_empty() {
            self.error = Some("Enter a command or prompt".to_owned());
            return;
        }

        let command = self.input.text().to_owned();
        let sender = self.sender.clone();
        self.error = None;
        self.sending = true;
        thread::spawn(move || {
            let result =
                workspace_panel::send_command_below(command).map(|pane_id| HerdrPromptCompletion {
                    message: format!("Sent to Herdr pane {pane_id}"),
                    reopen_path: None,
                });
            let _ = sender.send(result);
        });
    }

    pub(crate) fn prepare_agent(&mut self, path: PathBuf, branch: String) -> Result<(), String> {
        if self.sending {
            return Err("Another Herdr command is still running".to_owned());
        }
        if std::env::var("HERDR_ENV").ok().as_deref() != Some("1") {
            return Err("Agents can only be started inside Herdr".to_owned());
        }
        let host_pane_id = std::env::var("HERDR_PANE_ID")
            .map_err(|_| "Herdr did not identify Hunkle's pane".to_owned())?;
        self.error = None;
        self.next_agent_request_id = self.next_agent_request_id.wrapping_add(1);
        let request_id = self.next_agent_request_id;
        self.pending_agent = Some(PendingAgent {
            request_id,
            path,
            branch,
            host_pane_id: host_pane_id.clone(),
            layout: None,
        });
        let sender = self.layout_sender.clone();
        thread::spawn(move || {
            let _ = sender.send((request_id, workspace_panel::pane_layout(host_pane_id)));
        });
        Ok(())
    }

    pub(crate) fn agent_pane_picker_open(&self) -> bool {
        self.pending_agent.is_some()
    }

    pub(crate) fn agent_pane_layout(&self) -> Option<&HerdrPaneLayout> {
        self.pending_agent.as_ref()?.layout.as_ref()
    }

    pub(crate) fn agent_destination(&self) -> Option<&Path> {
        self.pending_agent
            .as_ref()
            .map(|pending| pending.path.as_path())
    }

    pub(crate) fn agent_destination_branch(&self) -> Option<&str> {
        self.pending_agent
            .as_ref()
            .map(|pending| pending.branch.as_str())
    }

    pub(crate) fn agent_host_pane_id(&self) -> Option<&str> {
        self.pending_agent
            .as_ref()
            .map(|pending| pending.host_pane_id.as_str())
    }

    pub(crate) fn cancel_pending_agent(&mut self) -> bool {
        self.pending_agent.take().is_some()
    }

    pub(crate) fn select_agent_pane(&mut self, index: usize) -> Result<(), String> {
        let pending = self
            .pending_agent
            .as_ref()
            .ok_or_else(|| "Agent pane selection is no longer open".to_owned())?;
        let layout = pending
            .layout
            .as_ref()
            .ok_or_else(|| "Herdr pane layout is still loading".to_owned())?;
        let pane = layout
            .panes
            .get(index)
            .ok_or_else(|| "That Herdr pane is no longer available".to_owned())?;
        if pane.pane_id == pending.host_pane_id {
            return Err("Hunkle cannot replace its own pane".to_owned());
        }
        let workspace_id = layout.workspace_id.clone();
        let pane_id = pane.pane_id.clone();
        let pending = self.pending_agent.take().expect("pending agent checked");
        let sender = self.sender.clone();
        self.error = None;
        self.sending = true;
        thread::spawn(move || {
            crate::diagnostics::event(format!(
                "new agent replacing pane={} path={}",
                pane_id,
                pending.path.display()
            ));
            let reopen_path = pending.path.clone();
            let result =
                workspace_panel::replace_pane_with_agent(pending.path, workspace_id, pane_id).map(
                    |pane_id| HerdrPromptCompletion {
                        message: format!("Started agent in Herdr pane {pane_id}"),
                        reopen_path: Some(reopen_path),
                    },
                );
            let _ = sender.send(result);
        });
        Ok(())
    }

    pub(crate) fn split_agent_pane(
        &mut self,
        index: usize,
        direction: AgentPaneDirection,
    ) -> Result<(), String> {
        let pending = self
            .pending_agent
            .as_ref()
            .ok_or_else(|| "Agent pane selection is no longer open".to_owned())?;
        let pane_id = pending
            .layout
            .as_ref()
            .ok_or_else(|| "Herdr pane layout is still loading".to_owned())?
            .panes
            .get(index)
            .ok_or_else(|| "That Herdr pane is no longer available".to_owned())?
            .pane_id
            .clone();
        let pending = self.pending_agent.take().expect("pending agent checked");
        let sender = self.sender.clone();
        self.error = None;
        self.sending = true;
        thread::spawn(move || {
            crate::diagnostics::event(format!(
                "new agent splitting pane={} direction={} path={}",
                pane_id,
                direction.as_str(),
                pending.path.display()
            ));
            let reopen_path = pending.path.clone();
            let result = workspace_panel::split_pane_with_agent(pending.path, pane_id, direction)
                .map(|pane_id| HerdrPromptCompletion {
                    message: format!("Started agent in new Herdr pane {pane_id}"),
                    reopen_path: Some(reopen_path),
                });
            let _ = sender.send(result);
        });
        Ok(())
    }

    pub(crate) fn poll(&mut self) -> HerdrPromptPoll {
        let mut changed = false;
        let mut completion = None;
        while let Ok((request_id, result)) = self.layout_receiver.try_recv() {
            if self
                .pending_agent
                .as_ref()
                .is_none_or(|pending| pending.request_id != request_id)
            {
                continue;
            }
            changed = true;
            match result {
                Ok(layout) => {
                    if let Some(pending) = self.pending_agent.as_mut() {
                        pending.layout = Some(layout);
                    }
                }
                Err(error) => {
                    self.pending_agent = None;
                    completion = Some(Err(error));
                }
            }
        }
        if completion.is_none()
            && let Ok(result) = self.receiver.try_recv()
        {
            changed = true;
            self.sending = false;
            if result.is_ok() {
                self.input.clear();
            }
            completion = Some(result);
        }
        HerdrPromptPoll {
            changed,
            completion,
        }
    }
}
