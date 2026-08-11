use std::{
    io::{self, Read, Write},
    process::{Command, ExitStatus, Stdio},
    sync::mpsc::{self, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use process_wrap::std::CommandWrap;

#[cfg(windows)]
use process_wrap::std::JobObject;
#[cfg(unix)]
use process_wrap::std::ProcessGroup;

const FAST_COMMAND_WINDOW: Duration = Duration::from_millis(25);
const LONG_COMMAND_THRESHOLD: Duration = Duration::from_secs(5);
const FAST_POLL_INTERVAL: Duration = Duration::from_millis(1);
const NORMAL_POLL_INTERVAL: Duration = Duration::from_millis(10);
const LONG_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) stdout_bytes: usize,
    pub(crate) stderr_bytes: usize,
    pub(crate) timeout: Duration,
    pub(crate) stop_at_stdout_limit: bool,
}

impl Limits {
    pub(crate) const fn new(stdout_bytes: usize, stderr_bytes: usize, timeout: Duration) -> Self {
        Self {
            stdout_bytes,
            stderr_bytes,
            timeout,
            stop_at_stdout_limit: false,
        }
    }

    pub(crate) const fn stopping_at_stdout_limit(mut self) -> Self {
        self.stop_at_stdout_limit = true;
        self
    }
}

#[derive(Debug)]
pub(crate) struct Output {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) stdout_truncated: bool,
    pub(crate) stderr_truncated: bool,
    pub(crate) timed_out: bool,
    pub(crate) stdout_limit_reached: bool,
}

pub(crate) fn run(command: &mut Command, limits: Limits) -> io::Result<Output> {
    run_inner(command, None, limits, None)
}

pub(crate) fn run_cancellable(
    command: &mut Command,
    limits: Limits,
    cancelled: &dyn Fn() -> bool,
) -> io::Result<Output> {
    run_inner(command, None, limits, Some(cancelled))
}

pub(crate) fn run_with_input(
    command: &mut Command,
    input: Vec<u8>,
    limits: Limits,
) -> io::Result<Output> {
    run_inner(command, Some(input), limits, None)
}

pub(crate) fn run_cancellable_with_input(
    command: &mut Command,
    input: Vec<u8>,
    limits: Limits,
    cancelled: &dyn Fn() -> bool,
) -> io::Result<Output> {
    run_inner(command, Some(input), limits, Some(cancelled))
}

fn run_inner(
    command: &mut Command,
    input: Option<Vec<u8>>,
    limits: Limits,
    cancelled: Option<&dyn Fn() -> bool>,
) -> io::Result<Output> {
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let owned_command = std::mem::replace(command, Command::new(""));
    let mut wrapped_command = CommandWrap::from(owned_command);
    #[cfg(unix)]
    wrapped_command.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    wrapped_command.wrap(JobObject);
    let spawned = wrapped_command.spawn();
    *command = wrapped_command.into_command();
    let mut child = spawned?;
    let result = (|| -> io::Result<Output> {
        let stdout = child
            .stdout()
            .take()
            .ok_or_else(|| io::Error::other("child stdout was unavailable"))?;
        let stderr = child
            .stderr()
            .take()
            .ok_or_else(|| io::Error::other("child stderr was unavailable"))?;
        let stdout_limit = limits.stdout_bytes;
        let stderr_limit = limits.stderr_bytes;
        let (stdout_sender, stdout_receiver) = mpsc::sync_channel(1);
        let (stdout_limit_sender, stdout_limit_receiver) = mpsc::sync_channel(1);
        let (stderr_sender, stderr_receiver) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let _ = stdout_sender.send(read_bounded(
                stdout,
                stdout_limit,
                Some(stdout_limit_sender),
            ));
        });
        thread::spawn(move || {
            let _ = stderr_sender.send(read_bounded(stderr, stderr_limit, None));
        });
        let input_receiver = input.map(|input| {
            let mut stdin = child.stdin().take().expect("piped stdin was requested");
            let (sender, receiver) = mpsc::sync_channel(1);
            thread::spawn(move || {
                let _ = sender.send(stdin.write_all(&input));
            });
            receiver
        });

        let started = Instant::now();
        let mut status = None;
        let mut stdout_result = None;
        let mut stderr_result = None;
        let mut input_finished = input_receiver.is_none();
        let mut process_cancelled = false;
        let mut stdout_limit_reached = false;
        let mut timed_out = false;
        let terminated_early = loop {
            if status.is_none() {
                status = child.try_wait()?;
            }
            receive_result(&stdout_receiver, &mut stdout_result, "child stdout reader")?;
            receive_result(&stderr_receiver, &mut stderr_result, "child stderr reader")?;
            if !input_finished {
                let receiver = input_receiver.as_ref().expect("input receiver is present");
                match receiver.try_recv() {
                    Ok(result) => {
                        let _ = result;
                        input_finished = true;
                    }
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => {
                        return Err(io::Error::other("child stdin writer panicked"));
                    }
                }
            }
            if status.is_some()
                && stdout_result.is_some()
                && stderr_result.is_some()
                && input_finished
            {
                break false;
            }
            let was_cancelled = cancelled.is_some_and(|cancelled| cancelled());
            if !stdout_limit_reached
                && limits.stop_at_stdout_limit
                && stdout_limit_receiver.try_recv().is_ok()
            {
                stdout_limit_reached = true;
            }
            if was_cancelled || stdout_limit_reached || started.elapsed() >= limits.timeout {
                match child.start_kill() {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
                if status.is_none() {
                    status = Some(child.wait()?);
                }
                if was_cancelled {
                    process_cancelled = true;
                }
                timed_out =
                    !was_cancelled && !stdout_limit_reached && started.elapsed() >= limits.timeout;
                break true;
            }
            let elapsed = started.elapsed();
            thread::sleep(
                process_poll_interval(elapsed).min(limits.timeout.saturating_sub(elapsed)),
            );
        };

        if terminated_early {
            let deadline = Instant::now() + Duration::from_secs(1);
            while (stdout_result.is_none() || stderr_result.is_none() || !input_finished)
                && Instant::now() < deadline
            {
                receive_result(&stdout_receiver, &mut stdout_result, "child stdout reader")?;
                receive_result(&stderr_receiver, &mut stderr_result, "child stderr reader")?;
                if !input_finished {
                    let receiver = input_receiver.as_ref().expect("input receiver is present");
                    match receiver.try_recv() {
                        Ok(_) => input_finished = true,
                        Err(TryRecvError::Empty) => {}
                        Err(TryRecvError::Disconnected) => {
                            return Err(io::Error::other("child stdin writer panicked"));
                        }
                    }
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
        if process_cancelled {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "process was cancelled",
            ));
        }
        let (stdout, stdout_truncated) = stdout_result.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "child stdout remained open after process-tree termination",
            )
        })??;
        let (stderr, stderr_truncated) = stderr_result.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "child stderr remained open after process-tree termination",
            )
        })??;
        let status = status.expect("a completed process has an exit status");

        Ok(Output {
            status,
            stdout,
            stderr,
            stdout_truncated,
            stderr_truncated,
            timed_out,
            stdout_limit_reached,
        })
    })();
    if result.is_err() {
        let _ = child.start_kill();
        let _ = child.wait();
    }
    result
}

fn process_poll_interval(elapsed: Duration) -> Duration {
    if elapsed < FAST_COMMAND_WINDOW {
        FAST_POLL_INTERVAL
    } else if elapsed < LONG_COMMAND_THRESHOLD {
        NORMAL_POLL_INTERVAL
    } else {
        LONG_POLL_INTERVAL
    }
}

fn receive_result<T>(
    receiver: &mpsc::Receiver<T>,
    result: &mut Option<T>,
    worker: &str,
) -> io::Result<()> {
    if result.is_some() {
        return Ok(());
    }
    match receiver.try_recv() {
        Ok(received) => *result = Some(received),
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => {
            return Err(io::Error::other(format!("{worker} panicked")));
        }
    }
    Ok(())
}

fn read_bounded(
    mut reader: impl Read,
    limit: usize,
    limit_sender: Option<mpsc::SyncSender<()>>,
) -> io::Result<(Vec<u8>, bool)> {
    let mut retained = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = [0_u8; 16 * 1024];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let available = limit.saturating_sub(retained.len());
        let keep = available.min(read);
        retained.extend_from_slice(&buffer[..keep]);
        if keep < read && !truncated {
            if let Some(sender) = &limit_sender {
                let _ = sender.try_send(());
            }
            truncated = true;
        }
    }
    Ok((retained, truncated))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{process_poll_interval, read_bounded};

    #[test]
    fn process_polling_backs_off_for_long_running_commands() {
        assert_eq!(
            process_poll_interval(std::time::Duration::ZERO),
            std::time::Duration::from_millis(1)
        );
        assert_eq!(
            process_poll_interval(std::time::Duration::from_millis(25)),
            std::time::Duration::from_millis(10)
        );
        assert_eq!(
            process_poll_interval(std::time::Duration::from_secs(5)),
            std::time::Duration::from_millis(100)
        );
    }

    #[test]
    fn drains_input_while_retaining_only_the_limit() {
        let (retained, truncated) = read_bounded(Cursor::new(b"abcdef"), 4, None).unwrap();

        assert_eq!(retained, b"abcd");
        assert!(truncated);
    }

    #[cfg(unix)]
    #[test]
    fn hard_stdout_limit_stops_the_process() {
        use std::{process::Command, time::Duration};

        use super::{Limits, run};

        let started = std::time::Instant::now();
        let output = run(
            Command::new("sh").args(["-c", "while :; do printf 1234567890; done"]),
            Limits::new(1024, 1024, Duration::from_secs(10)).stopping_at_stdout_limit(),
        )
        .unwrap();

        assert!(output.stdout_limit_reached);
        assert!(output.stdout_truncated);
        assert!(!output.timed_out);
        assert_eq!(output.stdout.len(), 1024);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_descendants_that_keep_output_open() {
        use std::{process::Command, time::Duration};

        use super::{Limits, run};

        let started = std::time::Instant::now();
        let output = run(
            Command::new("sh").args(["-c", "sleep 10 &"]),
            Limits::new(1024, 1024, Duration::from_millis(150)),
        )
        .unwrap();

        assert!(output.timed_out);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_kills_a_running_process_group_promptly() {
        use std::{
            process::Command,
            sync::{
                Arc,
                atomic::{AtomicBool, Ordering},
            },
            thread,
            time::Duration,
        };

        use super::{Limits, run_cancellable};

        let cancelled = Arc::new(AtomicBool::new(false));
        let trigger = Arc::clone(&cancelled);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            trigger.store(true, Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let result = run_cancellable(
            Command::new("sh").args(["-c", "sleep 10"]),
            Limits::new(1024, 1024, Duration::from_secs(10)),
            &|| cancelled.load(Ordering::Relaxed),
        );

        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
