use super::LoadKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Operation {
    Commit,
    Fetch,
    Command,
    Mutation,
    Format,
    StatusCheck,
    Load(LoadKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForegroundOperation {
    Commit,
    Command,
    Mutation,
    Format,
}

#[derive(Debug, Default)]
pub(super) struct OperationState {
    foreground: Option<ForegroundOperation>,
    fetching: bool,
    checking_status: bool,
    loading: Option<LoadKind>,
}

impl OperationState {
    pub(super) fn is_idle(&self) -> bool {
        self.foreground.is_none()
            && !self.fetching
            && !self.checking_status
            && self.loading.is_none()
    }

    pub(super) fn can_start(&self, operation: Operation) -> bool {
        match operation {
            Operation::Commit => self.foreground.is_none() && self.loading != Some(LoadKind::Open),
            Operation::Fetch => {
                self.loading.is_none()
                    && !self.fetching
                    && !matches!(
                        self.foreground,
                        Some(
                            ForegroundOperation::Command
                                | ForegroundOperation::Mutation
                                | ForegroundOperation::Format
                        )
                    )
            }
            Operation::Command => {
                self.foreground.is_none() && !self.fetching && self.loading != Some(LoadKind::Open)
            }
            Operation::Mutation | Operation::Format => {
                self.foreground.is_none() && !self.fetching && self.loading.is_none()
            }
            Operation::StatusCheck => {
                self.foreground.is_none()
                    && !self.fetching
                    && !self.checking_status
                    && self.loading.is_none()
            }
            Operation::Load(LoadKind::Open) => self.foreground.is_none() && self.loading.is_none(),
            Operation::Load(LoadKind::Reload) => self.loading.is_none(),
        }
    }

    pub(super) fn start(&mut self, operation: Operation) -> bool {
        if !self.can_start(operation) {
            return false;
        }
        match operation {
            Operation::Commit => self.foreground = Some(ForegroundOperation::Commit),
            Operation::Fetch => self.fetching = true,
            Operation::Command => self.foreground = Some(ForegroundOperation::Command),
            Operation::Mutation => self.foreground = Some(ForegroundOperation::Mutation),
            Operation::Format => self.foreground = Some(ForegroundOperation::Format),
            Operation::StatusCheck => self.checking_status = true,
            Operation::Load(kind) => self.loading = Some(kind),
        }
        true
    }

    pub(super) fn finish(&mut self, operation: Operation) {
        match operation {
            Operation::Commit if self.foreground == Some(ForegroundOperation::Commit) => {
                self.foreground = None;
            }
            Operation::Command if self.foreground == Some(ForegroundOperation::Command) => {
                self.foreground = None;
            }
            Operation::Mutation if self.foreground == Some(ForegroundOperation::Mutation) => {
                self.foreground = None;
            }
            Operation::Format if self.foreground == Some(ForegroundOperation::Format) => {
                self.foreground = None;
            }
            Operation::Fetch => self.fetching = false,
            Operation::StatusCheck => self.checking_status = false,
            Operation::Load(kind) if self.loading == Some(kind) => self.loading = None,
            Operation::Commit | Operation::Command | Operation::Mutation | Operation::Format => {}
            Operation::Load(_) => {}
        }
    }

    pub(super) fn is_running(&self, operation: Operation) -> bool {
        match operation {
            Operation::Commit => self.foreground == Some(ForegroundOperation::Commit),
            Operation::Fetch => self.fetching,
            Operation::Command => self.foreground == Some(ForegroundOperation::Command),
            Operation::Mutation => self.foreground == Some(ForegroundOperation::Mutation),
            Operation::Format => self.foreground == Some(ForegroundOperation::Format),
            Operation::StatusCheck => self.checking_status,
            Operation::Load(kind) => self.loading == Some(kind),
        }
    }
}
