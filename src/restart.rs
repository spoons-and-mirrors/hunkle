use std::{
    env, fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const CHECK_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Generation {
    device: u64,
    inode: u64,
}

impl Generation {
    fn read(path: &Path) -> io::Result<Self> {
        let metadata = fs::metadata(path)?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

pub(crate) struct RestartWatcher {
    executable: PathBuf,
    generation: Generation,
    next_check: Instant,
    check_interval: Duration,
}

impl RestartWatcher {
    pub(crate) fn start() -> io::Result<Self> {
        Self::at(env::current_exe()?, CHECK_INTERVAL)
    }

    fn at(executable: PathBuf, check_interval: Duration) -> io::Result<Self> {
        let generation = Generation::read(&executable)?;
        Ok(Self {
            executable,
            generation,
            next_check: Instant::now() + check_interval,
            check_interval,
        })
    }

    pub(crate) fn poll(&mut self) -> io::Result<Option<PathBuf>> {
        if Instant::now() < self.next_check {
            return Ok(None);
        }
        self.next_check = Instant::now() + self.check_interval;
        let generation = match Generation::read(&self.executable) {
            Ok(generation) => generation,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok((generation != self.generation).then(|| self.executable.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_replaced_executable() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("hunkle");
        fs::write(&executable, "old").unwrap();
        let mut watcher = RestartWatcher::at(executable.clone(), Duration::ZERO).unwrap();

        let replacement = directory.path().join("replacement");
        fs::write(&replacement, "new").unwrap();
        fs::rename(replacement, &executable).unwrap();

        assert_eq!(watcher.poll().unwrap(), Some(executable));
    }

    #[test]
    fn leaves_an_unchanged_executable_running() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("hunkle");
        fs::write(&executable, "installed").unwrap();
        let mut watcher = RestartWatcher::at(executable, Duration::ZERO).unwrap();

        assert_eq!(watcher.poll().unwrap(), None);
    }
}
