mod app;
mod diagnostics;
mod filesystem;
mod formatter;
mod git;
mod media;
mod process;
mod repo_path;
mod repository_session;
#[cfg(unix)]
mod restart;
mod selection;
mod theme;
mod tree;
mod ui;

use std::{
    io::{self, Write},
    path::PathBuf,
    process::Command,
    thread,
    time::Duration,
};

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
use ratatui_image::picker::Picker;

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
    let _diagnostics_guard = DiagnosticsGuard;
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
    let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    if std::env::var_os("HERDR_ENV").is_some() {
        picker.set_protocol_type(ratatui_image::picker::ProtocolType::Halfblocks);
    }
    app.configure_media_picker(picker, auto_kitty_supported());
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
            let notice = if app.dirty_file_edit() {
                "Update ready; save or discard editor changes to restart"
            } else {
                "Update ready; restarting after the current operation…"
            };
            if app.notice.as_deref() != Some(notice) {
                app.notice = Some(notice.to_owned());
                dirty = true;
            }
        }
        if dirty {
            let _activity = diagnostics::activity("draw", app.diagnostic_context());
            let mut cleanup_error = None;
            terminal.draw(|frame| {
                ui::draw(frame, &mut app);
                if let Err(error) = write_media_terminal_cleanup(&mut app) {
                    cleanup_error = Some(error);
                }
            })?;
            if let Some(error) = cleanup_error {
                return Err(error);
            }
            write_media_terminal_output(&mut app)?;
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
                Event::Resize(_, _) => {
                    app.reset_media_presentation();
                    (true, true)
                }
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
            KITTY_MEDIA_EMITTED.store(false, std::sync::atomic::Ordering::Relaxed);
            app.media_terminal_restarted();
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
    app.shutdown();
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
        diagnostics::shutdown();
        restore_terminal();
        let error = Command::new(executable).arg(workspace).exec();
        return Err(error.into());
    }

    Ok(())
}

struct DiagnosticsGuard;

impl Drop for DiagnosticsGuard {
    fn drop(&mut self) {
        diagnostics::shutdown();
    }
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
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES,
            ),
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
    if KITTY_MEDIA_EMITTED.load(std::sync::atomic::Ordering::Relaxed) {
        let _ = io::stdout().write_all(b"\x1b_Ga=d,d=A,q=2\x1b\\");
        let _ = io::stdout().flush();
    }
    let _ = execute!(
        io::stdout(),
        PopKeyboardEnhancementFlags,
        DisableBracketedPaste,
        DisableMouseCapture,
        Clear(ClearType::All),
        MoveTo(0, 0),
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();
}

static KITTY_MEDIA_EMITTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn auto_kitty_supported() -> bool {
    if std::env::var_os("HERDR_ENV").is_some() {
        return false;
    }
    ["TERM", "TERM_PROGRAM"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .any(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("ghostty") || value.contains("kitty")
        })
        || std::env::var_os("KITTY_WINDOW_ID").is_some()
}

fn write_media_terminal_output(app: &mut App) -> Result<()> {
    let output = app.take_media_terminal_output();
    if output.bytes.is_empty() {
        return Ok(());
    }
    if output.kitty {
        KITTY_MEDIA_EMITTED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let mut stdout = io::stdout().lock();
    stdout.write_all(&output.bytes)?;
    stdout.flush()?;
    Ok(())
}

fn write_media_terminal_cleanup(app: &mut App) -> Result<()> {
    let output = app.take_media_terminal_cleanup();
    if output.is_empty() {
        return Ok(());
    }
    let mut stdout = io::stdout().lock();
    stdout.write_all(&output)?;
    stdout.flush()?;
    Ok(())
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
