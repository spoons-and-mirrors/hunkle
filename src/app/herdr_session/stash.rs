use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::filesystem::atomic_write;

use super::AgentStatus;

const VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct StashedAgent {
    pub(crate) harness: String,
    pub(crate) agent_name: String,
    pub(crate) session_source: String,
    pub(crate) session_kind: String,
    pub(crate) session_id: String,
    pub(crate) session_name: Option<String>,
    pub(crate) repository: PathBuf,
    pub(crate) repository_label: String,
    pub(crate) worktree: PathBuf,
    pub(crate) branch: String,
    pub(crate) workspace_id: String,
    pub(crate) tab_id: String,
    pub(crate) pane_id: String,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) destination_cwd: Option<PathBuf>,
    pub(crate) focused: bool,
    pub(crate) status: AgentStatus,
    pub(crate) stashed_at_ms: u64,
}

#[derive(Default, Deserialize, Serialize)]
struct StashFile {
    version: u8,
    agents: Vec<StashedAgent>,
}

pub(super) struct AgentStashStore {
    path: Option<PathBuf>,
    load_error: Option<String>,
    pub(super) agents: Vec<StashedAgent>,
}

impl AgentStashStore {
    pub(super) fn new(path: Option<PathBuf>) -> Self {
        let mut store = Self {
            path,
            load_error: None,
            agents: Vec::new(),
        };
        let Some(path) = store.path.as_deref() else {
            return store;
        };
        match fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<StashFile>(&bytes) {
                Ok(file) if file.version == VERSION => store.agents = file.agents,
                Ok(_) => store.load_error = Some("unsupported agent stash version".to_owned()),
                Err(error) => store.load_error = Some(format!("invalid agent stash: {error}")),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => store.load_error = Some(format!("could not read agent stash: {error}")),
        }
        store
    }

    pub(super) fn add(&mut self, agent: StashedAgent) -> Result<(), String> {
        self.ensure_writable()?;
        let previous = self.agents.clone();
        self.agents
            .retain(|candidate| candidate.session_id != agent.session_id);
        self.agents.insert(0, agent);
        if let Err(error) = self.save() {
            self.agents = previous;
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn remove(&mut self, session_id: &str) -> Result<(), String> {
        self.ensure_writable()?;
        let previous = self.agents.clone();
        self.agents
            .retain(|candidate| candidate.session_id != session_id);
        if self.agents.len() == previous.len() {
            return Ok(());
        }
        if let Err(error) = self.save() {
            self.agents = previous;
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn remove_live<'a>(
        &mut self,
        session_ids: impl Iterator<Item = &'a str>,
    ) -> Result<(), String> {
        self.ensure_writable()?;
        let session_ids = session_ids.collect::<std::collections::HashSet<_>>();
        let previous = self.agents.clone();
        self.agents
            .retain(|candidate| !session_ids.contains(candidate.session_id.as_str()));
        if self.agents.len() == previous.len() {
            return Ok(());
        }
        if let Err(error) = self.save() {
            self.agents = previous;
            return Err(error);
        }
        Ok(())
    }

    fn ensure_writable(&self) -> Result<(), String> {
        self.load_error
            .as_ref()
            .map_or(Ok(()), |error| Err(error.clone()))
    }

    fn save(&self) -> Result<(), String> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(&StashFile {
            version: VERSION,
            agents: self.agents.clone(),
        })
        .map_err(|error| format!("could not encode agent stash: {error}"))?;
        atomic_write(path, &bytes).map_err(|error| format!("could not save agent stash: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str) -> StashedAgent {
        StashedAgent {
            harness: "opencode".to_owned(),
            agent_name: "opencode".to_owned(),
            session_source: "opencode".to_owned(),
            session_kind: "session".to_owned(),
            session_id: id.to_owned(),
            session_name: Some("Finish agent stash".to_owned()),
            repository: PathBuf::from("/code/hunkle"),
            repository_label: "hunkle".to_owned(),
            worktree: PathBuf::from("/code/hunkle"),
            branch: "feature/agent-stash".to_owned(),
            workspace_id: "w1".to_owned(),
            tab_id: "w1:t1".to_owned(),
            pane_id: "w1:p2".to_owned(),
            cwd: Some(PathBuf::from("/code/hunkle")),
            destination_cwd: Some(PathBuf::from("/code/hunkle")),
            focused: false,
            status: AgentStatus::Idle,
            stashed_at_ms: 42,
        }
    }

    #[test]
    fn persists_all_resume_metadata_and_removes_restored_sessions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent-stash.json");
        let mut store = AgentStashStore::new(Some(path.clone()));
        store.add(agent("ses_123")).unwrap();

        let mut restored = AgentStashStore::new(Some(path));
        assert_eq!(restored.agents, vec![agent("ses_123")]);
        restored.remove("ses_123").unwrap();
        assert!(restored.agents.is_empty());
    }
}
