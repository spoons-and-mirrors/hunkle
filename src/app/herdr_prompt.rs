use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use super::{AgentPaneDirection, HerdrPaneLayout, HitTarget, TextInput, herdr_session};

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
    host_pane_id: String,
    layout: Option<HerdrPaneLayout>,
    pane_focus: Option<HitTarget>,
    session_id: Option<String>,
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
        host_pane_id: String,
        layout: HerdrPaneLayout,
    ) {
        self.next_agent_request_id = self.next_agent_request_id.wrapping_add(1);
        let pane_focus = initial_pane_focus(&layout, &host_pane_id);
        self.pending_agent = Some(PendingAgent {
            request_id: self.next_agent_request_id,
            path,
            host_pane_id,
            pane_focus,
            layout: Some(layout),
            session_id: None,
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
                herdr_session::send_command_below(command).map(|pane_id| HerdrPromptCompletion {
                    message: format!("Sent to Herdr pane {pane_id}"),
                    reopen_path: None,
                });
            let _ = sender.send(result);
        });
    }

    pub(crate) fn prepare_agent(&mut self, path: PathBuf) -> Result<(), String> {
        self.prepare_agent_session(path, None)
    }

    pub(crate) fn prepare_stashed_agent(
        &mut self,
        path: PathBuf,
        session_id: String,
    ) -> Result<(), String> {
        self.prepare_agent_session(path, Some(session_id))
    }

    fn prepare_agent_session(
        &mut self,
        path: PathBuf,
        session_id: Option<String>,
    ) -> Result<(), String> {
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
            host_pane_id: host_pane_id.clone(),
            layout: None,
            pane_focus: None,
            session_id,
        });
        let sender = self.layout_sender.clone();
        thread::spawn(move || {
            let _ = sender.send((request_id, herdr_session::pane_layout(host_pane_id)));
        });
        Ok(())
    }

    pub(crate) fn agent_pane_picker_open(&self) -> bool {
        self.pending_agent.is_some()
    }

    pub(crate) fn agent_pane_layout(&self) -> Option<&HerdrPaneLayout> {
        self.pending_agent.as_ref()?.layout.as_ref()
    }

    pub(crate) fn agent_pane_focus(&self) -> Option<HitTarget> {
        self.pending_agent.as_ref()?.pane_focus.clone()
    }

    pub(crate) fn cycle_agent_pane_focus(&mut self, backwards: bool) {
        let Some(pending) = self.pending_agent.as_mut() else {
            return;
        };
        let Some(layout) = pending.layout.as_ref() else {
            return;
        };
        let count = layout.panes.len();
        if count == 0 {
            pending.pane_focus = None;
            return;
        }
        let focus_count = count * 5;
        let current = pending
            .pane_focus
            .clone()
            .and_then(pane_focus_ordinal)
            .unwrap_or(0)
            .min(focus_count - 1);
        let next = if backwards {
            current.checked_sub(1).unwrap_or(focus_count - 1)
        } else {
            (current + 1) % focus_count
        };
        pending.pane_focus = Some(pane_focus_from_ordinal(next));
    }

    pub(crate) fn move_agent_pane_focus(&mut self, direction: AgentPaneDirection) {
        let Some(pending) = self.pending_agent.as_mut() else {
            return;
        };
        let Some(layout) = pending.layout.as_ref() else {
            return;
        };
        let Some(focus) = pending.pane_focus.clone() else {
            pending.pane_focus = initial_pane_focus(layout, &pending.host_pane_id);
            return;
        };
        let (current_index, edge) = match focus {
            HitTarget::AgentPane(index) => (index, None),
            HitTarget::AgentPaneSplit(index, edge) => (index, Some(edge)),
            _ => {
                pending.pane_focus = initial_pane_focus(layout, &pending.host_pane_id);
                return;
            }
        };
        if layout.panes.get(current_index).is_none() {
            pending.pane_focus = initial_pane_focus(layout, &pending.host_pane_id);
            return;
        }
        pending.pane_focus = match edge {
            None => Some(HitTarget::AgentPaneSplit(current_index, direction)),
            Some(edge) if edge == direction => neighboring_pane(layout, current_index, direction)
                .map(HitTarget::AgentPane)
                .or(Some(focus)),
            Some(edge) if opposite_direction(edge) == direction => {
                Some(HitTarget::AgentPane(current_index))
            }
            Some(_) => Some(HitTarget::AgentPaneSplit(current_index, direction)),
        };
    }

    pub(crate) fn activate_agent_pane_focus(&mut self) -> Result<(), String> {
        match self.agent_pane_focus() {
            Some(HitTarget::AgentPane(index)) => self.select_agent_pane(index),
            Some(HitTarget::AgentPaneSplit(index, direction)) => {
                self.split_agent_pane(index, direction)
            }
            _ => Err("No Herdr pane position is selected".to_owned()),
        }
    }

    pub(crate) fn update_agent_destination(&mut self, path: PathBuf) {
        if let Some(pending) = self.pending_agent.as_mut() {
            pending.path = path;
        }
    }

    #[cfg(test)]
    pub(crate) fn agent_destination(&self) -> Option<&std::path::Path> {
        self.pending_agent
            .as_ref()
            .map(|pending| pending.path.as_path())
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
        let PendingAgent {
            path, session_id, ..
        } = pending;
        let sender = self.sender.clone();
        self.error = None;
        self.sending = true;
        thread::spawn(move || {
            crate::diagnostics::event(format!(
                "new agent replacing pane={} path={}",
                pane_id,
                path.display()
            ));
            let reopen_path = path.clone();
            let result =
                herdr_session::replace_pane_with_agent(path, workspace_id, pane_id, session_id)
                    .map(|pane_id| HerdrPromptCompletion {
                        message: format!("Started agent in Herdr pane {pane_id}"),
                        reopen_path: Some(reopen_path),
                    });
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
        let PendingAgent {
            path, session_id, ..
        } = pending;
        let sender = self.sender.clone();
        self.error = None;
        self.sending = true;
        thread::spawn(move || {
            crate::diagnostics::event(format!(
                "new agent splitting pane={} direction={} path={}",
                pane_id,
                direction.as_str(),
                path.display()
            ));
            let reopen_path = path.clone();
            let result = herdr_session::split_pane_with_agent(path, pane_id, direction, session_id)
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
                        pending.pane_focus = initial_pane_focus(&layout, &pending.host_pane_id);
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

fn initial_pane_focus(layout: &HerdrPaneLayout, host_pane_id: &str) -> Option<HitTarget> {
    layout
        .panes
        .iter()
        .position(|pane| pane.pane_id != host_pane_id)
        .or_else(|| (!layout.panes.is_empty()).then_some(0))
        .map(HitTarget::AgentPane)
}

fn pane_focus_ordinal(focus: HitTarget) -> Option<usize> {
    match focus {
        HitTarget::AgentPane(index) => Some(index * 5),
        HitTarget::AgentPaneSplit(index, AgentPaneDirection::Up) => Some(index * 5 + 1),
        HitTarget::AgentPaneSplit(index, AgentPaneDirection::Right) => Some(index * 5 + 2),
        HitTarget::AgentPaneSplit(index, AgentPaneDirection::Down) => Some(index * 5 + 3),
        HitTarget::AgentPaneSplit(index, AgentPaneDirection::Left) => Some(index * 5 + 4),
        _ => None,
    }
}

fn pane_focus_from_ordinal(ordinal: usize) -> HitTarget {
    let index = ordinal / 5;
    match ordinal % 5 {
        0 => HitTarget::AgentPane(index),
        1 => HitTarget::AgentPaneSplit(index, AgentPaneDirection::Up),
        2 => HitTarget::AgentPaneSplit(index, AgentPaneDirection::Right),
        3 => HitTarget::AgentPaneSplit(index, AgentPaneDirection::Down),
        4 => HitTarget::AgentPaneSplit(index, AgentPaneDirection::Left),
        _ => unreachable!(),
    }
}

fn opposite_direction(direction: AgentPaneDirection) -> AgentPaneDirection {
    match direction {
        AgentPaneDirection::Up => AgentPaneDirection::Down,
        AgentPaneDirection::Down => AgentPaneDirection::Up,
        AgentPaneDirection::Left => AgentPaneDirection::Right,
        AgentPaneDirection::Right => AgentPaneDirection::Left,
    }
}

fn neighboring_pane(
    layout: &HerdrPaneLayout,
    current_index: usize,
    direction: AgentPaneDirection,
) -> Option<usize> {
    let current = layout.panes.get(current_index)?;
    let current_x = i32::from(current.x) * 2 + i32::from(current.width);
    let current_y = i32::from(current.y) * 2 + i32::from(current.height);
    layout
        .panes
        .iter()
        .enumerate()
        .filter_map(|(index, pane)| {
            let x = i32::from(pane.x) * 2 + i32::from(pane.width);
            let y = i32::from(pane.y) * 2 + i32::from(pane.height);
            let (primary, secondary) = match direction {
                AgentPaneDirection::Up if y < current_y => (current_y - y, (x - current_x).abs()),
                AgentPaneDirection::Down if y > current_y => (y - current_y, (x - current_x).abs()),
                AgentPaneDirection::Left if x < current_x => (current_x - x, (y - current_y).abs()),
                AgentPaneDirection::Right if x > current_x => {
                    (x - current_x, (y - current_y).abs())
                }
                _ => return None,
            };
            Some(((secondary, primary, index), index))
        })
        .min_by_key(|(score, _)| *score)
        .map(|(_, index)| index)
}
