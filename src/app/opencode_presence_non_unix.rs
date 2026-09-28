use std::path::PathBuf;

pub(crate) struct OpenCodePresence;

impl OpenCodePresence {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn poll(&mut self) -> Option<PathBuf> {
        None
    }
}
