use super::*;

pub(super) fn draw_explorer_changes(
    frame: &mut Frame<'_>,
    app: &mut App,
    columns: [Rect; 2],
    draw_details: bool,
) {
    app.regions.worktree_list = None;
    app.regions.commit = None;
    let content = columns[0].inner(Margin::new(1, 0));
    let header = Rect::new(content.x, content.y.saturating_add(1), content.width, 1);
    let controls = Rect::new(
        content.x,
        header.bottom().saturating_add(1),
        content.width,
        1,
    );
    let list_area = layout_agents_pane(app, content, controls.bottom());
    let add_width = 7.min(controls.width);
    let add_button = Rect::new(
        controls.right().saturating_sub(add_width),
        controls.y,
        add_width,
        1,
    );
    let root_target = Rect::new(
        controls.x,
        controls.y,
        controls.width.saturating_sub(add_width),
        1,
    );
    let drop_target = app.file_drop_target().cloned();
    draw_sidebar_tabs(frame, app, header);
    frame.render_widget(
        Paragraph::new(format!("{} FILES", app.changes.explorer_rows().len()))
            .style(Style::default().fg(palette().faint)),
        root_target,
    );
    frame.render_widget(
        Paragraph::new("NEW  +")
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette().accent).bg(palette().raised)),
        add_button,
    );
    if drop_target.as_ref().is_some_and(RepoPath::is_empty) {
        frame.render_widget(
            Block::default().style(Style::default().bg(palette().inactive_selected)),
            root_target,
        );
        frame.render_widget(
            Paragraph::new("DROP FILES HERE").style(Style::default().fg(palette().ink)),
            root_target,
        );
    }
    app.regions.explorer_list = Some(list_area);
    app.regions.files_add = Some(add_button);
    app.regions.files_root = Some(root_target);

    let viewport = usize::from(list_area.height);
    let row_count = app.changes.explorer_rows().len();
    app.changes.explorer_scroll = app
        .changes
        .explorer_scroll
        .min(row_count.saturating_sub(viewport));
    let rows = app.changes.explorer_rows();
    let items: Vec<ListItem<'_>> = if rows.is_empty() {
        vec![ListItem::new(Line::styled(
            " No files",
            Style::default().fg(palette().faint),
        ))]
    } else {
        rows.iter()
            .enumerate()
            .skip(app.changes.explorer_scroll)
            .take(viewport)
            .map(|(index, row)| {
                let path = row.file_path.as_ref().or(row.directory_path.as_ref());
                let code = path.and_then(|path| app.changes.explorer_change_code(path));
                let item = explorer_item(row, code, usize::from(list_area.width));
                if app.changes.explorer_state.selected() == Some(index) {
                    item.style(Style::default().bg(palette().selected))
                } else if drop_target.as_ref().is_some_and(|target| {
                    row.directory_path
                        .as_ref()
                        .is_some_and(|path| path == target)
                }) {
                    item.style(Style::default().bg(palette().inactive_selected))
                } else {
                    item
                }
            })
            .collect()
    };
    frame.render_widget(List::new(items), list_area);
    draw_agents_section(frame, app);
    draw_agent_history_pane(frame, app, content);
    if !draw_details {
        return;
    }

    let selected_path = app
        .selected_explorer_file_path()
        .map_or_else(|| "No file selected".to_owned(), RepoPath::display);
    let preview_header = Rect::new(
        columns[1].x.saturating_add(1),
        columns[1].y.saturating_add(1),
        columns[1].width.saturating_sub(2),
        1,
    );
    let preview_body = Rect::new(
        preview_header.x,
        preview_header.y.saturating_add(2),
        preview_header.width,
        columns[1]
            .bottom()
            .saturating_sub(preview_header.y.saturating_add(3)),
    );
    let media_loaded = app.changes.preview_image.is_some();
    let database_loaded = app.changes.sqlite_browser.is_some();
    let wrap_label = if media_loaded || database_loaded {
        String::new()
    } else if app.changes.diff_wrap {
        format!(
            "  {}:on",
            app.settings.shortcuts.label(ShortcutAction::ToggleWrap)
        )
    } else {
        format!(
            "  {}:off",
            app.settings.shortcuts.label(ShortcutAction::ToggleWrap)
        )
    };
    let markdown_available = app.markdown_preview_available();
    let markdown_rendered = app.markdown_preview_rendered();
    let access_label = if media_loaded || database_loaded || markdown_rendered {
        "read-only"
    } else {
        "click to edit"
    };
    let markdown_button_width = if markdown_available { 11 } else { 0 };
    let header_content_width = preview_header
        .width
        .saturating_sub(markdown_button_width)
        .saturating_sub(u16::from(markdown_available));
    let preview_kind = if database_loaded { "DATABASE" } else { "FILE" };
    let display_path = truncate_width(
        &selected_path,
        usize::from(header_content_width).saturating_sub(
            preview_kind.len()
                + 2
                + access_label.len()
                + UnicodeWidthStr::width(wrap_label.as_str()),
        ),
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{preview_kind}  "),
                Style::default()
                    .fg(palette().muted)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                display_path,
                Style::default()
                    .fg(palette().ink)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {access_label}"),
                Style::default().fg(palette().accent),
            ),
            Span::styled(
                wrap_label,
                Style::default().fg(if app.changes.diff_wrap {
                    palette().accent
                } else {
                    palette().faint
                }),
            ),
        ])),
        Rect::new(
            preview_header.x,
            preview_header.y,
            header_content_width,
            preview_header.height,
        ),
    );
    if markdown_available {
        let button = Rect::new(
            preview_header.right().saturating_sub(markdown_button_width),
            preview_header.y,
            markdown_button_width,
            1,
        );
        app.regions
            .register_hit_target(HitTarget::MarkdownPreviewToggle, button);
        let highlighted =
            markdown_rendered || app.hovered_hit_target == Some(HitTarget::MarkdownPreviewToggle);
        frame.render_widget(
            Paragraph::new(if markdown_rendered {
                " m Source  "
            } else {
                " m Preview "
            })
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .fg(if highlighted {
                        palette().canvas
                    } else {
                        palette().accent
                    })
                    .bg(if highlighted {
                        palette().accent
                    } else {
                        palette().raised
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            button,
        );
    }
    let media_visible = media_loaded && app.view == View::Changes && app.mode == Mode::Normal;
    if media_visible {
        app.regions.diff_scroll_max = 0;
        app.regions.diff_scrollbar = None;
        app.regions.diff_scroll_thumb = None;
        let image = app
            .changes
            .preview_image
            .as_ref()
            .expect("media preview was checked")
            .clone();
        let generation = app.changes.preview_content_generation;
        let protocol = app.settings.media_preview_protocol;
        let (area, effective_protocol, state) = app.changes.preview_presentation.media_state(
            generation,
            &image,
            protocol,
            preview_body,
        );
        if !area.is_empty() {
            frame.render_stateful_widget(
                StatefulImage::new().resize(Resize::Fit(None)),
                area,
                state,
            );
            match effective_protocol {
                crate::media::MediaPreviewProtocol::Kitty => {
                    let transmission = take_kitty_transmission(frame.buffer_mut(), area);
                    app.changes.preview_presentation.queue_kitty_frame(
                        generation,
                        area,
                        transmission,
                    );
                }
                crate::media::MediaPreviewProtocol::Iterm2
                | crate::media::MediaPreviewProtocol::Sixel => {
                    let transmission =
                        take_inline_transmission(frame.buffer_mut(), area, effective_protocol);
                    app.changes.preview_presentation.queue_inline_frame(
                        generation,
                        effective_protocol,
                        area,
                        transmission,
                    );
                }
                crate::media::MediaPreviewProtocol::Auto
                | crate::media::MediaPreviewProtocol::Halfblocks => {}
            }
        } else if effective_protocol != crate::media::MediaPreviewProtocol::Halfblocks {
            app.changes.preview_presentation.hide_media();
        }
        if let Some(error) = app.changes.preview_presentation.media_error() {
            frame.render_widget(
                Paragraph::new(error.to_owned())
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(palette().red).bg(palette().panel)),
                preview_body,
            );
        }
    } else if database_loaded {
        app.changes.preview_presentation.hide_media();
        crate::ui::sqlite::draw(frame, app, preview_body);
    } else {
        app.changes.preview_presentation.hide_media();
        let editable_path = app.selected_explorer_file_path().cloned();
        let path = editable_path
            .as_ref()
            .map_or_else(String::new, RepoPath::display);
        let preview =
            prepare_preview_lines(app, preview_body, &path, false, false, markdown_rendered, 0);
        if !markdown_rendered {
            app.regions.preview_body = Some(preview_body);
            app.regions.preview_path = editable_path;
            app.regions.preview_generation = app.changes.preview_content_generation;
            app.regions.preview_scroll = app.changes.diff_scroll;
        }
        render_scrollable_content(frame, app, columns[1], preview_body, preview, 0);
    }
}

pub(super) fn worktree_item<'a>(
    row: &'a WorktreeRow,
    changes: &'a [Change],
    width: usize,
) -> ListItem<'a> {
    if let Some(section) = row.section {
        let Some((additions, deletions)) = row.section_stats else {
            return ListItem::new("");
        };
        let color = match section {
            WorktreeSection::Staged => palette().green,
            WorktreeSection::Unstaged => palette().yellow,
        };
        let additions = format!("+{additions}");
        let deletions = format!("-{deletions}");
        let stats_width = additions.len() + 1 + deletions.len();
        let show_stats = width >= stats_width + 4;
        let available_label = width.saturating_sub(usize::from(show_stats) * stats_width);
        let label = truncate_width(&format!(" {}", row.label), available_label);
        let padding = available_label.saturating_sub(UnicodeWidthStr::width(label.as_str()));
        let mut spans = vec![
            Span::styled(
                label,
                Style::default()
                    .fg(color)
                    .bg(palette().surface_alt)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " ".repeat(padding),
                Style::default().bg(palette().surface_alt),
            ),
        ];
        if show_stats {
            spans.extend([
                Span::styled(
                    additions,
                    Style::default()
                        .fg(palette().green)
                        .bg(palette().surface_alt),
                ),
                Span::styled(" ", Style::default().bg(palette().surface_alt)),
                Span::styled(
                    deletions,
                    Style::default().fg(palette().red).bg(palette().surface_alt),
                ),
            ]);
        }
        return ListItem::new(Line::from(spans));
    }
    let Some(change_index) = row.change_index else {
        let marker = if row.directory_expanded == Some(false) {
            "▢ "
        } else {
            "▣ "
        };
        let directory = truncate_width(&format!("{}{}{}", row.prefix, marker, row.label), width);
        return ListItem::new(Line::from(Span::styled(directory, folder_style())));
    };
    let change = &changes[change_index];
    let status = if change.code == '?' { 'U' } else { change.code };
    let (checkbox, color) = if change.staged {
        ("◉", palette().green)
    } else {
        ("○", palette().muted)
    };
    let label = change.original_path.as_ref().map_or_else(
        || row.label.clone(),
        |original| {
            let original_name = original
                .file_name()
                .map(display_os_str)
                .unwrap_or_else(|| original.display());
            format!("{original_name} → {}", row.label)
        },
    );
    let additions = format!("+{}", change.additions);
    let deletions = format!("-{}", change.deletions);
    let stats_width = additions.len() + 1 + deletions.len();
    let show_stats = width >= stats_width + 10;
    let controls_width = 2 + usize::from(show_stats) * (stats_width + 1);
    let available_label = width.saturating_sub(controls_width);
    let prefix = truncate_width(&row.prefix, available_label.saturating_sub(2));
    let label_width = available_label
        .saturating_sub(UnicodeWidthStr::width(prefix.as_str()))
        .saturating_sub(2);
    let label = truncate_width(&label, label_width);
    let path_width =
        UnicodeWidthStr::width(prefix.as_str()) + 2 + UnicodeWidthStr::width(label.as_str());
    let padding = available_label.saturating_sub(path_width);
    let mut spans = vec![
        Span::styled(prefix, Style::default().fg(palette().ink)),
        Span::styled(
            format!("{status} "),
            Style::default()
                .fg(explorer_file_color(change.code))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(label, Style::default().fg(palette().ink)),
        Span::raw(" ".repeat(padding)),
    ];
    if show_stats {
        spans.extend([
            Span::styled(additions, Style::default().fg(palette().green)),
            Span::raw(" "),
            Span::styled(deletions, Style::default().fg(palette().red)),
            Span::raw(" "),
        ]);
    }
    spans.push(Span::styled(
        format!("{checkbox} "),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ));
    ListItem::new(Line::from(spans))
}

pub(super) fn explorer_item(
    row: &ExplorerRow,
    change_code: Option<char>,
    width: usize,
) -> ListItem<'static> {
    if row.file_path.is_none() {
        let marker = if row.directory_expanded == Some(false) {
            "> "
        } else {
            "v "
        };
        let prefix = truncate_width(&row.prefix, width.saturating_sub(2));
        let label_width = width
            .saturating_sub(UnicodeWidthStr::width(prefix.as_str()))
            .saturating_sub(2);
        let label = truncate_width(&row.label, label_width);
        let folder_style = explorer_folder_style(change_code);
        return ListItem::new(Line::from(vec![
            Span::styled(prefix, Style::default().fg(palette().faint)),
            Span::styled(marker, folder_style),
            Span::styled(label, folder_style),
        ]));
    }
    let icon = file_icon(&row.label);
    let prefix = truncate_width(&row.prefix, width.saturating_sub(2));
    let label_width = width
        .saturating_sub(UnicodeWidthStr::width(prefix.as_str()))
        .saturating_sub(2);
    let label = truncate_width(&row.label, label_width);
    let color = change_code
        .map(explorer_file_color)
        .unwrap_or(palette().soft);
    let icon_color = change_code.map(explorer_file_color).unwrap_or(icon.1);
    ListItem::new(Line::from(vec![
        Span::styled(prefix, Style::default().fg(palette().faint)),
        Span::styled(format!("{} ", icon.0), Style::default().fg(icon_color)),
        Span::styled(label, Style::default().fg(color)),
    ]))
}

pub(super) fn file_icon(label: &str) -> (&'static str, Color) {
    let name = label.to_ascii_lowercase();
    if matches!(name.as_str(), "cargo.toml" | "cargo.lock") {
        return ("R", palette().orange);
    }
    if matches!(
        name.as_str(),
        "package.json"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "bun.lock"
            | "bun.lockb"
    ) {
        return ("J", palette().yellow);
    }
    if name == "readme" || name.starts_with("readme.") {
        return ("#", palette().cyan);
    }
    if name == "license"
        || name.starts_with("license.")
        || name == "copying"
        || name.starts_with("copying.")
    {
        return ("L", palette().muted);
    }
    if matches!(
        name.as_str(),
        "dockerfile" | "compose.yml" | "compose.yaml" | "containerfile"
    ) {
        return ("D", palette().cyan);
    }
    if matches!(
        name.as_str(),
        "makefile" | "cmakelists.txt" | "justfile" | "taskfile.yml" | "taskfile.yaml"
    ) {
        return ("B", palette().orange);
    }
    if matches!(
        name.as_str(),
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".ignore"
    ) {
        return ("G", palette().muted);
    }

    let extension = name.rsplit_once('.').map_or("", |(_, extension)| extension);
    match extension {
        "rs" => ("R", palette().orange),
        "js" | "jsx" | "mjs" | "cjs" => ("J", palette().yellow),
        "ts" | "tsx" | "mts" | "cts" => ("T", palette().cyan),
        "py" | "pyi" => ("P", palette().yellow),
        "rb" => ("R", palette().red),
        "go" => ("G", palette().cyan),
        "c" | "h" | "cc" | "cpp" | "cxx" | "hh" | "hpp" => ("C", palette().purple),
        "java" | "kt" | "kts" | "scala" => ("J", palette().red),
        "swift" => ("S", palette().orange),
        "ex" | "exs" | "erl" | "hrl" => ("E", palette().purple),
        "sh" | "bash" | "zsh" | "fish" | "nu" => (">", palette().green),
        "html" | "htm" | "xml" | "svg" => ("<", palette().orange),
        "css" | "scss" | "sass" | "less" => ("#", palette().purple),
        "vue" => ("V", palette().green),
        "svelte" => ("S", palette().orange),
        "json" | "jsonc" | "json5" => ("{", palette().yellow),
        "toml" | "ini" | "cfg" | "conf" | "properties" | "env" => ("=", palette().yellow),
        "yaml" | "yml" => ("Y", palette().purple),
        "md" | "mdx" | "rst" | "adoc" => ("#", palette().cyan),
        "txt" | "log" => ("-", palette().muted),
        "sql" | "db" | "sqlite" | "sqlite3" => ("Q", palette().cyan),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" | "bmp" | "avif" => ("@", palette().purple),
        "mp3" | "wav" | "flac" | "ogg" | "mp4" | "mov" | "mkv" | "webm" => (">", palette().cyan),
        "zip" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "tar" | "7z" | "rar" => {
            ("%", palette().orange)
        }
        "pdf" | "doc" | "docx" | "odt" => ("P", palette().red),
        "lock" => ("*", palette().muted),
        "wasm" | "bin" | "exe" | "dll" | "so" | "dylib" => ("!", palette().red),
        _ => ("?", palette().faint),
    }
}

pub(super) fn explorer_file_color(code: char) -> ratatui::style::Color {
    match code {
        'D' | 'U' => palette().red,
        '?' => palette().green,
        'A' => palette().accent,
        'R' => palette().purple,
        'C' => palette().cyan,
        'M' => palette().yellow,
        _ => palette().orange,
    }
}

pub(super) fn explorer_folder_style(change_code: Option<char>) -> Style {
    Style::default().fg(change_code
        .map(explorer_file_color)
        .unwrap_or(palette().ink))
}

pub(super) fn folder_style() -> Style {
    Style::default().fg(palette().muted)
}

pub(super) fn rendered_text_height(lines: &[Line<'_>], width: usize, wrapped: bool) -> usize {
    if !wrapped {
        return lines.len();
    }
    let width = width.max(1);
    lines
        .iter()
        .map(|line| {
            let content = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            word_wrapped_height(&content, width)
        })
        .sum()
}
