use crate::git::CommandOutput;
use std::{collections::VecDeque, ops::Range, sync::Arc};

const MAX_COMMAND_RECORDS: usize = 32;
const MAX_TRANSCRIPT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActionId {
    Commit,
    Push,
    Fetch,
    PullRebase,
    Custom,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ActionItem {
    pub(crate) id: ActionId,
    pub(crate) label: &'static str,
    pub(crate) detail: &'static str,
}

pub(crate) const ACTION_ITEMS: [ActionItem; 5] = [
    ActionItem {
        id: ActionId::Commit,
        label: "Commit",
        detail: "staged changes",
    },
    ActionItem {
        id: ActionId::Push,
        label: "Push",
        detail: "git push",
    },
    ActionItem {
        id: ActionId::Fetch,
        label: "Fetch",
        detail: "all remotes",
    },
    ActionItem {
        id: ActionId::PullRebase,
        label: "Pull --rebase",
        detail: "update branch",
    },
    ActionItem {
        id: ActionId::Custom,
        label: "Run Git command...",
        detail: "non-interactive",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandStatus {
    Input,
    Running,
    Complete {
        success: bool,
        exit_code: Option<i32>,
    },
}

pub(crate) struct CommandRecord {
    pub(crate) command: String,
    pub(crate) stdout: Arc<str>,
    pub(crate) stderr: Arc<str>,
    pub(crate) success: bool,
    pub(crate) exit_code: Option<i32>,
}

pub(crate) enum CommandLineSource {
    Intro,
    IntroSpacer,
    IntroExamples,
    IntroShellNote,
    RecordSpacer,
    RecordHeader(usize),
    Stdout { record: usize, range: Range<usize> },
    Stderr { record: usize, range: Range<usize> },
    EmptyRecord,
    CurrentError(Range<usize>),
    Waiting,
    Truncated,
    Empty,
}

#[derive(Default)]
pub(crate) struct CommandLayout {
    pub(crate) revision: u64,
    pub(crate) width: usize,
    pub(crate) sources: Vec<CommandLineSource>,
    pub(crate) starts: Vec<usize>,
    pub(crate) height: usize,
}

pub(crate) struct ActionsState {
    pub(crate) selection: usize,
    pub(crate) input: String,
    pub(crate) command: String,
    pub(crate) status: CommandStatus,
    pub(crate) stdout: Arc<str>,
    pub(crate) stderr: Arc<str>,
    pub(crate) transcript: VecDeque<CommandRecord>,
    pub(crate) presentation_revision: u64,
    pub(crate) command_layout: CommandLayout,
    pub(crate) scroll: u16,
    pub(crate) scroll_max: u16,
}

impl Default for ActionsState {
    fn default() -> Self {
        Self {
            selection: 0,
            input: String::new(),
            command: String::new(),
            status: CommandStatus::Input,
            stdout: Arc::from(""),
            stderr: Arc::from(""),
            transcript: VecDeque::new(),
            presentation_revision: 1,
            command_layout: CommandLayout::default(),
            scroll: 0,
            scroll_max: 0,
        }
    }
}

impl ActionsState {
    pub(crate) fn selected(&self) -> ActionId {
        ACTION_ITEMS[self.selection.min(ACTION_ITEMS.len() - 1)].id
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        let len = ACTION_ITEMS.len() as isize;
        self.selection = (self.selection as isize + delta).rem_euclid(len) as usize;
    }

    pub(crate) fn begin_input(&mut self) {
        self.input.clear();
        self.command.clear();
        self.stdout = Arc::from("");
        self.stderr = Arc::from("");
        self.transcript.clear();
        self.invalidate_presentation();
        self.scroll = 0;
        self.scroll_max = 0;
        self.status = CommandStatus::Input;
    }

    pub(crate) fn begin_command(&mut self, command: String) {
        self.command = command;
        self.stdout = Arc::from("");
        self.stderr = Arc::from("");
        self.scroll = u16::MAX;
        self.scroll_max = 0;
        self.status = CommandStatus::Running;
        self.invalidate_presentation();
    }

    pub(crate) fn complete(&mut self, output: CommandOutput) {
        self.input.clear();
        self.stdout = Arc::from(output.stdout);
        self.stderr = Arc::from(output.stderr);
        self.transcript.push_back(CommandRecord {
            command: self.command.clone(),
            stdout: Arc::clone(&self.stdout),
            stderr: Arc::clone(&self.stderr),
            success: output.success,
            exit_code: output.exit_code,
        });
        self.scroll = u16::MAX;
        self.status = CommandStatus::Complete {
            success: output.success,
            exit_code: output.exit_code,
        };
        self.prune_transcript();
        self.invalidate_presentation();
    }

    pub(crate) fn fail(&mut self, error: String) {
        self.input.clear();
        self.stdout = Arc::from("");
        self.stderr = Arc::from(error);
        self.transcript.push_back(CommandRecord {
            command: self.command.clone(),
            stdout: Arc::from(""),
            stderr: Arc::clone(&self.stderr),
            success: false,
            exit_code: None,
        });
        self.scroll = u16::MAX;
        self.status = CommandStatus::Complete {
            success: false,
            exit_code: None,
        };
        self.prune_transcript();
        self.invalidate_presentation();
    }

    pub(crate) fn set_input_error(&mut self, error: String) {
        self.status = CommandStatus::Input;
        self.stderr = Arc::from(error);
        self.invalidate_presentation();
    }

    pub(crate) fn clear_input_error(&mut self) {
        if self.status == CommandStatus::Input && !self.stderr.is_empty() {
            self.stderr = Arc::from("");
            self.invalidate_presentation();
        }
    }

    pub(crate) fn clear_error(&mut self) {
        if !self.stderr.is_empty() {
            self.stderr = Arc::from("");
            self.invalidate_presentation();
        }
    }

    fn invalidate_presentation(&mut self) {
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
    }

    fn prune_transcript(&mut self) {
        let mut bytes = self
            .transcript
            .iter()
            .map(command_record_bytes)
            .sum::<usize>();
        while self.transcript.len() > 1
            && (self.transcript.len() > MAX_COMMAND_RECORDS || bytes > MAX_TRANSCRIPT_BYTES)
        {
            if let Some(record) = self.transcript.pop_front() {
                bytes = bytes.saturating_sub(command_record_bytes(&record));
            }
        }
    }

    pub(crate) fn scroll_by(&mut self, delta: isize) {
        self.scroll = if delta > 0 {
            self.scroll
                .saturating_add(delta as u16)
                .min(self.scroll_max)
        } else {
            self.scroll.saturating_sub(delta.unsigned_abs() as u16)
        };
    }
}

fn command_record_bytes(record: &CommandRecord) -> usize {
    record
        .command
        .len()
        .saturating_add(record.stdout.len())
        .saturating_add(record.stderr.len())
}

pub(crate) fn action_command(action: ActionId) -> Option<(&'static str, Vec<String>)> {
    match action {
        ActionId::Commit => None,
        ActionId::Push => Some(("Push", vec!["push".to_owned()])),
        ActionId::Fetch => Some((
            "Fetch",
            vec!["fetch".to_owned(), "--all".to_owned(), "--prune".to_owned()],
        )),
        ActionId::PullRebase => Some((
            "Pull --rebase",
            vec!["pull".to_owned(), "--rebase".to_owned()],
        )),
        ActionId::Custom => None,
    }
}

pub(crate) fn parse_git_args(input: &str) -> Result<Vec<String>, String> {
    let mut args = parse_command_args(input)?;
    if args.first().is_some_and(|argument| argument == "git") {
        args.remove(0);
    }
    if args.is_empty() {
        return Err("Enter a Git command".to_owned());
    }
    Ok(args)
}

pub(crate) fn parse_command_args(input: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote = None;
    let mut escaped = false;

    for character in input.chars() {
        if escaped {
            current.push(character);
            started = true;
            escaped = false;
            continue;
        }
        match (quote, character) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('\''), _) => {
                current.push(character);
                started = true;
            }
            (Some('"'), '\\') => escaped = true,
            (Some('"'), _) => {
                current.push(character);
                started = true;
            }
            (None, '\'' | '"') => {
                quote = Some(character);
                started = true;
            }
            (None, '\\') => {
                escaped = true;
                started = true;
            }
            (None, character) if character.is_whitespace() => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (None, _) => {
                current.push(character);
                started = true;
            }
            _ => unreachable!(),
        }
    }

    if escaped {
        return Err("Command ends with an unfinished escape".to_owned());
    }
    if quote.is_some() {
        return Err("Command contains an unterminated quote".to_owned());
    }
    if started {
        args.push(current);
    }
    if args.is_empty() {
        return Err("Enter a command".to_owned());
    }
    Ok(args)
}

pub(crate) fn display_git_command(args: &[String]) -> String {
    let arguments = args
        .iter()
        .map(|argument| {
            if argument.chars().all(|character| {
                character.is_ascii_alphanumeric() || "-_=./:@,".contains(character)
            }) {
                argument.clone()
            } else {
                format!("'{0}'", argument.replace('\'', "'\"'\"'"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("git {arguments}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_custom_git_arguments_without_a_shell() {
        assert_eq!(
            parse_git_args("git log --format='hello world' -- README.md").unwrap(),
            ["log", "--format=hello world", "--", "README.md"]
        );
        assert_eq!(parse_git_args("show \"\"").unwrap(), ["show", ""]);
        assert_eq!(
            display_git_command(&["commit".to_owned(), "hello world".to_owned()]),
            "git commit 'hello world'"
        );
        assert!(parse_git_args("commit -m 'unfinished").is_err());
        assert!(parse_git_args("git").is_err());
        assert_eq!(
            parse_command_args("code --wait").unwrap(),
            ["code", "--wait"]
        );
    }

    #[test]
    fn command_transcript_is_bounded_and_shares_completed_output() {
        let mut actions = ActionsState::default();
        for index in 0..40 {
            actions.begin_command(format!("git command-{index}"));
            actions.complete(CommandOutput {
                stdout: format!("output-{index}"),
                stderr: String::new(),
                success: true,
                exit_code: Some(0),
            });
        }

        assert_eq!(actions.transcript.len(), MAX_COMMAND_RECORDS);
        assert_eq!(actions.transcript.front().unwrap().command, "git command-8");
        assert!(Arc::ptr_eq(
            &actions.stdout,
            &actions.transcript.back().unwrap().stdout
        ));
    }
}
