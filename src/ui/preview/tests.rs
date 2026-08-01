use image::{ImageBuffer, Rgba};
use ratatui_image::ResizeEncodeRender;

use super::*;

#[test]
fn shutdown_joins_the_media_worker_once() {
    let mut preview = PreviewPresentation::default();
    preview.shutdown();
    preview.shutdown();
    assert!(preview.media_worker.is_none());
    assert!(preview.media_state.is_none());
}

#[test]
fn extracts_superfile_style_kitty_transmission_after_placeholders() {
    let command =
        "\u{1b}_Gq=2,i=42,a=T,U=1,f=32,t=d,s=80,v=48,m=0;data\u{1b}\\\u{1b}[splaceholders";
    let area = Rect::new(2, 3, 10, 3);
    let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 10));
    buffer.cell_mut((2, 3)).unwrap().set_symbol(command);
    buffer.cell_mut((3, 3)).unwrap().set_symbol("placeholder");

    let transmission = take_kitty_transmission(&mut buffer, area).unwrap();

    let patched = String::from_utf8(transmission.bytes).unwrap();
    assert_eq!(transmission.image_id, 42);
    assert!(patched.contains("i=42,a=T,U=1,c=10,r=3,f=32,"));
    assert!(!patched.contains("\u{10eeee}"));
    assert_eq!(
        buffer.cell((2, 3)).unwrap().symbol(),
        "\u{1b}[splaceholders"
    );
    assert_eq!(buffer.cell((3, 3)).unwrap().symbol(), "placeholder");

    buffer
        .cell_mut((2, 3))
        .unwrap()
        .set_symbol("\u{1b}[s\u{10eeee}placeholder");
    buffer.cell_mut((3, 3)).unwrap().set_symbol("placeholder");
    assert!(take_kitty_transmission(&mut buffer, area).is_none());
    assert_eq!(
        buffer.cell((2, 3)).unwrap().symbol(),
        "\u{1b}[s\u{10eeee}placeholder"
    );
    assert_eq!(buffer.cell((3, 3)).unwrap().symbol(), "placeholder");
}

#[test]
fn queues_kitty_output_outside_the_ratatui_buffer() {
    let mut preview = PreviewPresentation::default();
    let area = Rect::new(2, 3, 10, 4);
    preview.queue_kitty_frame(
        7,
        area,
        Some(KittyTransmission {
            image_id: 42,
            bytes: b"\x1b_Gq=2,i=42,a=T,U=1,c=10,r=4;data\x1b\\".to_vec(),
        }),
    );

    let output = preview.take_terminal_output();
    assert!(output.kitty);
    let output = String::from_utf8(output.bytes).unwrap();
    assert_eq!(preview.take_terminal_cleanup(), KITTY_DELETE_ALL.as_bytes());
    assert!(output.contains("\x1b[s\x1b[4;3H"));
    assert!(output.contains("i=42,a=T,U=1,c=10,r=4"));
    assert!(output.ends_with("\x1b[u"));

    preview.queue_kitty_frame(7, Rect::new(4, 5, 10, 4), None);
    let reposition = String::from_utf8(preview.take_terminal_output().bytes).unwrap();
    assert_eq!(
        preview.take_terminal_cleanup(),
        KITTY_DELETE_PLACEMENTS.as_bytes()
    );
    assert!(reposition.contains("\x1b[s\x1b[6;5H"));
    assert!(reposition.contains("a=p,i=42,c=10,r=4,C=1,q=2"));

    preview.hide_media();
    assert_eq!(preview.take_terminal_cleanup(), KITTY_DELETE_ALL.as_bytes());
}

#[test]
fn extracts_inline_protocols_for_out_of_band_output() {
    let area = Rect::new(2, 3, 3, 2);
    for (protocol, payload) in [
        (
            MediaPreviewProtocol::Iterm2,
            "clear\u{1b}]1337;File=inline=1:data\u{7}",
        ),
        (MediaPreviewProtocol::Sixel, "clear\u{1b}Pqdata\u{1b}\\"),
    ] {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 10));
        buffer.cell_mut((2, 3)).unwrap().set_symbol(payload);
        buffer.cell_mut((3, 3)).unwrap().set_symbol("covered");

        let extracted = take_inline_transmission(&mut buffer, area, protocol).unwrap();

        assert_eq!(extracted, payload.as_bytes());
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let cell = buffer.cell((x, y)).unwrap();
                assert_eq!(cell.symbol(), " ");
                assert_eq!(cell.diff_option, CellDiffOption::Skip);
            }
        }
    }
}

#[test]
fn real_inline_encoders_produce_extractable_terminal_payloads() {
    let image = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(2, 2, Rgba([255, 0, 0, 255])));
    for protocol in [MediaPreviewProtocol::Iterm2, MediaPreviewProtocol::Sixel] {
        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(match protocol {
            MediaPreviewProtocol::Iterm2 => ProtocolType::Iterm2,
            MediaPreviewProtocol::Sixel => ProtocolType::Sixel,
            _ => unreachable!(),
        });
        let mut state = picker.new_resize_protocol(image.clone());
        state.resize_encode(&Resize::Fit(None), Size::new(2, 1));
        state.last_encoding_result().unwrap().unwrap();
        let payload = match state.protocol_type() {
            StatefulProtocolType::ITerm2(encoded) => &encoded.data,
            StatefulProtocolType::Sixel(encoded) => &encoded.data,
            _ => unreachable!(),
        };
        let area = Rect::new(1, 1, 2, 1);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 5, 5));
        buffer.cell_mut((1, 1)).unwrap().set_symbol(payload);
        assert!(take_inline_transmission(&mut buffer, area, protocol).is_some());
    }
}

#[test]
fn queues_inline_output_only_when_placement_changes() {
    let mut preview = PreviewPresentation::default();
    let area = Rect::new(2, 3, 10, 4);
    preview.queue_inline_frame(
        7,
        MediaPreviewProtocol::Iterm2,
        area,
        Some(b"inline-image".to_vec()),
    );
    let output = preview.take_terminal_output();
    assert!(!output.kitty);
    let output = String::from_utf8(output.bytes).unwrap();
    assert!(output.contains("\u{1b}[s\u{1b}[4;3Hinline-image\u{1b}[u"));

    preview.queue_inline_frame(
        7,
        MediaPreviewProtocol::Iterm2,
        area,
        Some(b"inline-image".to_vec()),
    );
    assert!(preview.take_terminal_output().bytes.is_empty());

    preview.queue_inline_frame(
        7,
        MediaPreviewProtocol::Iterm2,
        Rect::new(4, 5, 10, 4),
        Some(b"inline-image".to_vec()),
    );
    let cleanup = String::from_utf8(preview.take_terminal_cleanup()).unwrap();
    assert!(cleanup.starts_with("\u{1b}[s\u{1b}[4;3H\u{1b}[10X"));
    assert!(cleanup.ends_with("\u{1b}[u"));
    assert!(!preview.take_terminal_output().bytes.is_empty());

    preview.hide_media();
    let cleanup = String::from_utf8(preview.take_terminal_cleanup()).unwrap();
    assert!(cleanup.starts_with("\u{1b}[s\u{1b}[6;5H\u{1b}[10X"));
    assert!(cleanup.ends_with("\u{1b}[u"));
}

#[test]
fn auto_uses_detected_protocols_but_requires_a_known_kitty_terminal() {
    let mut preview = PreviewPresentation::default();
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Iterm2);
    preview.configure_media_picker(picker, false);
    assert_eq!(
        preview.effective_protocol(MediaPreviewProtocol::Auto),
        MediaPreviewProtocol::Iterm2
    );

    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Kitty);
    preview.configure_media_picker(picker.clone(), false);
    assert_eq!(
        preview.effective_protocol(MediaPreviewProtocol::Auto),
        MediaPreviewProtocol::Halfblocks
    );
    preview.configure_media_picker(picker, true);
    assert_eq!(
        preview.effective_protocol(MediaPreviewProtocol::Auto),
        MediaPreviewProtocol::Kitty
    );
}

#[test]
fn wrapped_source_continuations_stay_after_the_line_number_gutter() {
    let lines = vec![Line::from(vec![
        Span::raw("    1  "),
        Span::raw("abcdefghijklmnop"),
    ])];

    let wrapped = hard_wrap_lines(lines, 12, 0, 10, false, false);

    assert_eq!(wrapped.len(), 4);
    assert!(wrapped[0].spans[0].content.starts_with("    1  "));
    assert!(
        wrapped[1..]
            .iter()
            .all(|line| line.spans[0].content.starts_with("       "))
    );

    let lines = vec![Line::from(vec![
        Span::raw("    1  "),
        Span::raw("word committing"),
    ])];
    let wrapped = hard_wrap_lines(lines, 18, 0, 10, false, false);
    assert_eq!(wrapped.len(), 2);
    assert_eq!(wrapped[1].spans[0].content, "       committing");
}

#[test]
fn wrapped_diff_continuations_stay_after_the_line_number_gutter() {
    let lines = vec![Line::from(vec![
        Span::raw("    1 "),
        Span::raw("+"),
        Span::raw("abcdefghijklmnop"),
    ])];

    let wrapped = hard_wrap_lines(lines, 12, 0, 10, true, false);

    assert_eq!(wrapped.len(), 4);
    let first = wrapped[0]
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert!(first.starts_with("    1 +"));
    assert!(
        wrapped[1..]
            .iter()
            .all(|line| line.spans[0].content.starts_with("       "))
    );
}

#[test]
fn measures_wrapped_markdown_without_unbounded_allocation() {
    let mut presentation = PreviewPresentation::default();
    let mut scroll = 0;

    let preview = presentation.prepare(
        PreviewInput {
            content: "# Heading\n\nA paragraph that wraps across multiple rows.",
            generation: 1,
            path: "README.md",
            is_diff: false,
            markdown: true,
            show_initial_diff_header: false,
            width: 16,
            viewport_height: 8,
            wrapped: true,
            hunk_selected: false,
        },
        &mut scroll,
    );

    assert!(preview.wrapped);
    assert!(preview.rendered_height > 3);
    assert!(!preview.lines.is_empty());
}

#[test]
fn maps_wrapped_preview_cells_to_exact_source_positions() {
    let mut presentation = PreviewPresentation::default();
    let mut scroll = 0;
    presentation.prepare(
        PreviewInput {
            content: "alpha beta gamma",
            generation: 1,
            path: "notes.txt",
            is_diff: false,
            markdown: false,
            show_initial_diff_header: false,
            width: 10,
            viewport_height: 4,
            wrapped: true,
            hunk_selected: false,
        },
        &mut scroll,
    );

    assert_eq!(
        presentation.source_position_at_rendered_position("alpha beta gamma", 1, 3, 0,),
        Some((1, 14))
    );

    let diff = "@@ -1 +1 @@\n+alpha beta gamma";
    presentation.prepare(
        PreviewInput {
            content: diff,
            generation: 2,
            path: "notes.txt",
            is_diff: true,
            markdown: false,
            show_initial_diff_header: false,
            width: 11,
            viewport_height: 4,
            wrapped: true,
            hunk_selected: false,
        },
        &mut scroll,
    );
    assert_eq!(
        presentation.diff_position_at_rendered_position(diff, 2, 4, 1),
        Some((1, 14))
    );
}

#[test]
fn oversized_markdown_uses_the_windowed_source_cache() {
    let content = "x".repeat(MAX_CACHED_PREVIEW_BYTES + 1);
    let mut presentation = PreviewPresentation::default();
    let mut scroll = 0;

    presentation.prepare(
        PreviewInput {
            content: &content,
            generation: 1,
            path: "README.md",
            is_diff: false,
            markdown: true,
            show_initial_diff_header: false,
            width: 80,
            viewport_height: 8,
            wrapped: false,
            hunk_selected: false,
        },
        &mut scroll,
    );

    let cache = presentation.cache.as_ref().unwrap();
    assert!(!cache.markdown);
    assert!(!cache.fully_styled);
}

#[test]
fn numbers_markdown_rows_and_leaves_wrapped_continuations_blank() {
    let lines = numbered_markdown_lines(
        styled_markdown(
            "- This list item contains enough words to wrap across rows.\n",
            markdown_content_width(24),
            false,
        ),
        24,
    );
    let wrapped = hard_wrap_lines(lines, 24, 0, 20, false, true)
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(
        wrapped
            .first()
            .is_some_and(|line| line.starts_with("    1  * "))
    );
    assert!(
        wrapped[1..]
            .iter()
            .all(|line| line.starts_with("         ")),
        "{wrapped:#?}"
    );
}

#[test]
fn wrapped_markdown_uses_hanging_list_and_quote_prefixes() {
    let lines = styled_markdown(
        "- This list item contains enough words to wrap.\n\n> This quote also contains enough words to wrap.\n",
        80,
        false,
    );
    let wrapped = hard_wrap_lines(lines, 18, 0, 20, false, true)
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(wrapped.first().is_some_and(|line| line.starts_with("* ")));
    assert!(wrapped.get(1).is_some_and(|line| line.starts_with("  ")));
    let quote = wrapped
        .iter()
        .position(|line| line.starts_with("> This"))
        .expect("quote should be rendered");
    assert!(
        wrapped
            .get(quote + 1)
            .is_some_and(|line| line.starts_with("> "))
    );
}

#[test]
fn markdown_table_cache_tracks_wrap_mode() {
    let content = "| Key | Description |\n| --- | --- |\n| alpha | beginning words continue across rows until TAIL |\n";
    let mut presentation = PreviewPresentation::default();
    let mut scroll = 0;
    let mut prepare = |presentation: &mut PreviewPresentation, wrapped| {
        presentation.prepare(
            PreviewInput {
                content,
                generation: 1,
                path: "README.md",
                is_diff: false,
                markdown: true,
                show_initial_diff_header: false,
                width: 30,
                viewport_height: 30,
                wrapped,
                hunk_selected: false,
            },
            &mut scroll,
        )
    };
    let contains_tail = |preview: &PreparedPreview| {
        preview
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .any(|span| span.content.contains("TAIL"))
    };

    let unwrapped = prepare(&mut presentation, false);
    assert!(!contains_tail(&unwrapped));

    let wrapped = prepare(&mut presentation, true);
    assert!(contains_tail(&wrapped));
    assert!(wrapped.rendered_height > unwrapped.rendered_height);
    assert!(wrapped.lines.iter().all(|line| {
        line.spans
            .iter()
            .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
            .sum::<usize>()
            <= 30
    }));
    assert!(
        wrapped
            .lines
            .first()
            .and_then(|line| line.spans.first())
            .is_some_and(|span| span.content.starts_with("    1  "))
    );

    let unwrapped_again = prepare(&mut presentation, false);
    assert!(!contains_tail(&unwrapped_again));
}
