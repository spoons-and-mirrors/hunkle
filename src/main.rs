mod app;
mod diagnostics;
mod filesystem;
mod formatter;
mod git;
mod process;
mod repo_path;
mod repository_session;
#[cfg(unix)]
mod restart;
mod selection;
mod theme;
mod tree;
mod ui;

use std::{io, path::PathBuf, process::Command, thread, time::Duration};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use anyhow::Result;
use app::{App, EditorRequest};
use crossterm::{
    cursor::MoveTo,
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};

fn main() -> Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);

    if let Ok(log_path) = diagnostics::init() {
        diagnostics::event(format!(
            "startup pid={} path={} log={}",
            std::process::id(),
            path.display(),
            log_path.display()
        ));
    }
    install_panic_hook();
    #[cfg(unix)]
    let mut restart_coordinator = match restart::RestartCoordinator::start() {
        Ok(coordinator) => Some(coordinator),
        Err(error) => {
            diagnostics::event(format!("restart coordination unavailable error={error}"));
            None
        }
    };
    let mut terminal = start_terminal()?;
    let _guard = TerminalGuard;
    let mut app = App::opening(path.clone());
    set_kitty_media_enabled(
        app.settings.media_preview_protocol == app::MediaPreviewProtocol::Kitty,
    );
    let mut dirty = true;
    let mut restart_request: Option<PathBuf> = None;
    let mut restarting = false;

    while !app.should_quit {
        dirty |= {
            let _activity = diagnostics::activity("poll-workers", app.diagnostic_context());
            app.poll_worker()
        };
        #[cfg(unix)]
        if restart_request.is_none()
            && let Some(coordinator) = restart_coordinator.as_mut()
        {
            match coordinator.poll() {
                Ok(Some(executable)) => restart_request = Some(executable),
                Ok(None) => {}
                Err(error) => {
                    diagnostics::event(format!("restart coordination failed error={error}"));
                    restart_coordinator = None;
                }
            }
        }
        if restart_request.is_some() {
            if app.can_restart() {
                restarting = true;
                break;
            }
            let notice = "Update ready; restarting after the current operation…";
            if app.notice.as_deref() != Some(notice) {
                app.notice = Some(notice.to_owned());
                dirty = true;
            }
        }
        if dirty {
            let _activity = diagnostics::activity("draw", app.diagnostic_context());
            terminal.draw(|frame| ui::draw(frame, &mut app))?;
            dirty = false;
        }
        let ready = {
            let _activity = diagnostics::activity("terminal-poll", app.diagnostic_context());
            event::poll(Duration::from_millis(50))?
        };
        if !ready {
            continue;
        }
        for _ in 0..64 {
            let _activity = diagnostics::activity("input", app.diagnostic_context());
            let (changed, render_before_next_event) = match event::read()? {
                Event::Key(key) if key.is_press() => {
                    app.handle_key(key);
                    (true, false)
                }
                Event::Mouse(mouse) => {
                    let hover_before = (
                        app.changes.hunk_selection,
                        app.actions.selection,
                        app.graph_state.selected(),
                        app.author_filter.state.selected(),
                        app.repository_browser.state.selected(),
                        app.workspace_panel.selected,
                        app.hovered_hit_target,
                    );
                    app.handle_mouse(mouse);
                    let changed = !matches!(mouse.kind, event::MouseEventKind::Moved)
                        || hover_before
                            != (
                                app.changes.hunk_selection,
                                app.actions.selection,
                                app.graph_state.selected(),
                                app.author_filter.state.selected(),
                                app.repository_browser.state.selected(),
                                app.workspace_panel.selected,
                                app.hovered_hit_target,
                            );
                    (changed, false)
                }
                Event::Paste(text) => {
                    app.handle_paste(&text);
                    (true, false)
                }
                Event::Resize(_, _) => (true, true),
                _ => (false, false),
            };
            dirty |= changed;
            if render_before_next_event
                || app.requires_render_before_next_event()
                || !event::poll(Duration::ZERO)?
            {
                break;
            }
        }
        set_kitty_media_enabled(
            app.settings.media_preview_protocol == app::MediaPreviewProtocol::Kitty,
        );
        if let Some(text) = app.take_copy_request() {
            app.notice = Some(match selection::copy_to_clipboard(&text) {
                Ok(()) => "Copied selection".to_owned(),
                Err(error) => format!("Could not copy selection: {error}"),
            });
            dirty = true;
        }
        if let Some(request) = app.take_editor_request() {
            restore_terminal();
            let result = run_editor(request);
            terminal = start_terminal()?;
            app.reset_media_presentation();
            app.editor_finished(result);
            dirty = true;
        }
    }

    for _ in 0..3 {
        app.flush_commit_draft();
        if !app.commit_draft_pending() {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    diagnostics::event("shutdown clean".to_owned());

    #[cfg(unix)]
    if restarting && let Some(executable) = restart_request {
        let workspace = app
            .repository()
            .map(|repository| repository.root.clone())
            .unwrap_or(path);
        diagnostics::event(format!(
            "restarting executable={} workspace={}",
            executable.display(),
            workspace.display()
        ));
        restore_terminal();
        let error = Command::new(executable).arg(workspace).exec();
        return Err(error.into());
    }

    Ok(())
}

fn run_editor(request: EditorRequest) -> Result<(), String> {
    let Some((program, args)) = request.command.split_first() else {
        return Err("Editor command is empty".to_owned());
    };
    let status = Command::new(program)
        .args(args)
        .arg(&request.file)
        .current_dir(&request.repository)
        .status()
        .map_err(|error| format!("Could not start {program}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Editor exited with status {}",
            status
                .code()
                .map_or_else(|| "unknown".to_owned(), |code| code.to_string())
        ))
    }
}

fn start_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let result = (|| -> Result<_> {
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
            Clear(ClearType::All),
            MoveTo(0, 0)
        )?;
        Ok(Terminal::new(CrosstermBackend::new(stdout))?)
    })();
    if result.is_err() {
        restore_terminal();
    }
    result
}

fn restore_terminal() {
    // Keyboard enhancement was pushed inside the alternate screen, so unwind it first.
    if KITTY_MEDIA_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        use std::io::Write;
        let _ = io::stdout().write_all(b"\x1b_Ga=d,d=A,q=2\x1b\\");
    }
    let _ = execute!(
        io::stdout(),
        PopKeyboardEnhancementFlags,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();
}

static KITTY_MEDIA_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn set_kitty_media_enabled(enabled: bool) {
    KITTY_MEDIA_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        diagnostics::panic(info.to_string());
        restore_terminal();
        original(info);
    }));
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}
