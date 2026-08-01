use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use super::{TextInput, workspace_panel};

pub(crate) struct HerdrPrompt {
    pub(crate) input: TextInput,
    pub(crate) error: Option<String>,
    pub(crate) sending: bool,
    sender: Sender<Result<String, String>>,
    receiver: Receiver<Result<String, String>>,
    focus_sender: Sender<(u64, Result<Option<(String, String)>, String>)>,
    focus_receiver: Receiver<(u64, Result<Option<(String, String)>, String>)>,
    next_agent_request_id: u64,
    pending_agent: Option<PendingAgent>,
}

struct PendingAgent {
    request_id: u64,
    path: PathBuf,
    workspace_id: String,
    host_pane_id: String,
    probing: bool,
    next_probe: Instant,
}

impl Default for HerdrPrompt {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        let (focus_sender, focus_receiver) = mpsc::channel();
        Self {
            input: TextInput::default(),
            error: None,
            sending: false,
            sender,
            receiver,
            focus_sender,
            focus_receiver,
            next_agent_request_id: 0,
            pending_agent: None,
        }
    }
}

impl HerdrPrompt {
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
            let result = workspace_panel::send_command_below(command)
                .map(|pane_id| format!("Sent to Herdr pane {pane_id}"));
            let _ = sender.send(result);
        });
    }

    pub(crate) fn prepare_agent(&mut self, path: PathBuf) -> Result<(), String> {
        if self.sending {
            return Err("Another Herdr command is still running".to_owned());
        }
        if std::env::var("HERDR_ENV").ok().as_deref() != Some("1") {
            return Err("Agents can only be started inside Herdr".to_owned());
        }
        let host_pane_id = std::env::var("HERDR_PANE_ID")
            .map_err(|_| "Herdr did not identify Hunkle's pane".to_owned())?;
        let workspace_id = std::env::var("HERDR_WORKSPACE_ID")
            .map_err(|_| "Herdr did not identify Hunkle's workspace".to_owned())?;

        self.error = None;
        self.next_agent_request_id = self.next_agent_request_id.wrapping_add(1);
        self.pending_agent = Some(PendingAgent {
            request_id: self.next_agent_request_id,
            path,
            workspace_id,
            host_pane_id,
            probing: false,
            next_probe: Instant::now(),
        });
        Ok(())
    }

    pub(crate) fn cancel_pending_agent(&mut self) -> bool {
        self.pending_agent.take().is_some()
    }

    fn launch_agent_in_pane(&mut self, workspace_id: String, pane_id: String) {
        let pending = self.pending_agent.take().expect("pending agent checked");
        let sender = self.sender.clone();
        self.error = None;
        self.sending = true;
        thread::spawn(move || {
            let result = workspace_panel::replace_pane_with_agent(
                pending.path,
                workspace_id,
                pane_id,
            )
                .map(|pane_id| format!("Started agent in Herdr pane {pane_id}"));
            let _ = sender.send(result);
        });
    }

    pub(crate) fn poll(&mut self) -> Option<Result<String, String>> {
        if let Some(result) = self.poll_agent_pane() {
            return Some(result);
        }
        let result = self.receiver.try_recv().ok()?;
        self.sending = false;
        if result.is_ok() {
            self.input.clear();
        }
        Some(result)
    }

    fn poll_agent_pane(&mut self) -> Option<Result<String, String>> {
        while let Ok((request_id, result)) = self.focus_receiver.try_recv() {
            let Some(pending) = self
                .pending_agent
                .as_mut()
                .filter(|pending| pending.request_id == request_id)
            else {
                continue;
            };
            pending.probing = false;
            match result {
                Ok(Some((workspace_id, pane_id))) if pane_id != pending.host_pane_id => {
                    self.launch_agent_in_pane(workspace_id, pane_id);
                    return None;
                }
                Ok(_) => pending.next_probe = Instant::now() + Duration::from_millis(100),
                Err(error) => {
                    self.pending_agent = None;
                    return Some(Err(error));
                }
            }
        }

        let Some(pending) = self
            .pending_agent
            .as_mut()
            .filter(|pending| !pending.probing && Instant::now() >= pending.next_probe)
        else {
            return None;
        };
        pending.probing = true;
        let request_id = pending.request_id;
        let workspace_id = pending.workspace_id.clone();
        let sender = self.focus_sender.clone();
        thread::spawn(move || {
            let _ = sender.send((request_id, workspace_panel::focused_pane(workspace_id)));
        });
        None
    }
}
