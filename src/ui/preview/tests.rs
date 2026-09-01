use image::{ImageBuffer, Rgba};
use ratatui::widgets::{StatefulWidget, Widget};
use ratatui_image::{Image, Resize, ResizeEncodeRender, StatefulImage};

use std::time::{Duration, Instant};

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
fn disconnected_media_worker_clears_pending_fast_polling() {
    let mut preview = PreviewPresentation::default();
    preview.media_request_pending = true;
    preview.shutdown();

    assert!(preview.poll_media());
    assert!(!preview.media_work_pending());
    assert_eq!(preview.media_error(), Some("Media preview worker stopped"));
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
fn threaded_sixel_view_queues_positioned_terminal_output() {
    let mut preview = PreviewPresentation::default();
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Sixel);
    preview.configure_media_picker(picker, false);
    let image = Arc::new(DynamicImage::ImageRgba8(ImageBuffer::from_fn(
        800,
        400,
        |x, y| Rgba([x as u8, y as u8, x.wrapping_add(y) as u8, 255]),
    )));
    let available = Rect::new(5, 6, 40, 10);
    let deadline = Instant::now() + Duration::from_secs(2);

    let (area, protocol, _, render_state) = preview.media_state(
        1,
        &image,
        MediaPreviewProtocol::Sixel,
        SixelQuality::Fast,
        available,
    );
    assert_eq!(protocol, MediaPreviewProtocol::Halfblocks);
    let MediaRenderState::Immediate(progressive) = render_state else {
        panic!("initial SIXEL render did not use a progressive preview");
    };
    let mut progressive_buffer = Buffer::empty(Rect::new(0, 0, 60, 20));
    Widget::render(Image::new(progressive), area, &mut progressive_buffer);
    assert!(area.positions().any(|position| {
        matches!(
            progressive_buffer.cell(position).unwrap().symbol(),
            "▀" | "▄"
        )
    }));

    loop {
        preview.poll_media();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 60, 20));
        let (area, protocol, frame_revision, render_state) = preview.media_state(
            1,
            &image,
            MediaPreviewProtocol::Sixel,
            SixelQuality::Fast,
            available,
        );
        match render_state {
            MediaRenderState::Immediate(state) => {
                Widget::render(Image::new(state), area, &mut buffer);
            }
            MediaRenderState::Threaded(state) => StatefulWidget::render(
                StatefulImage::new().resize(Resize::Scale(None)),
                area,
                &mut buffer,
                state,
            ),
            MediaRenderState::Empty => {}
        }
        let transmission = take_inline_transmission(&mut buffer, area, protocol);
        preview.queue_inline_frame(frame_revision, protocol, area, transmission);
        let output = preview.take_terminal_output();
        if !output.bytes.is_empty() {
            assert!(!output.kitty);
            assert!(output.bytes.starts_with(b"\x1b[s\x1b[7;6H"));
            assert!(output.bytes.windows(2).any(|bytes| bytes == b"\x1bP"));
            assert!(output.bytes.ends_with(b"\x1b[u"));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "threaded SIXEL encoding did not produce terminal output"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn fast_sixel_is_smaller_than_quality_sixel() {
    let image = DynamicImage::ImageRgba8(ImageBuffer::from_fn(800, 400, |x, y| {
        Rgba([
            x.wrapping_mul(13) as u8,
            y.wrapping_mul(17) as u8,
            x.wrapping_add(y).wrapping_mul(7) as u8,
            255,
        ])
    }));
    let size = Size::new(40, 10);
    let font_size = Picker::halfblocks().font_size();
    let resized = Resize::Scale(None).resize(&image, font_size, size, None);
    let fast = encode_fast_sixel(&resized, size, false).unwrap();
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Sixel);
    let mut quality = picker.new_resize_protocol(image);
    quality.resize_encode(&Resize::Scale(None), size);
    quality.last_encoding_result().unwrap().unwrap();
    let StatefulProtocolType::Sixel(quality) = quality.protocol_type() else {
        panic!("quality encoder did not produce SIXEL");
    };

    assert!(fast.data.contains("\u{1b}P"));
    assert!(fast.data.len() < quality.data.len());
}

#[test]
fn fast_sixel_keeps_a_photo_sized_color_palette() {
    let image = DynamicImage::ImageRgba8(ImageBuffer::from_fn(64, 64, |x, y| {
        Rgba([
            x.wrapping_mul(5) as u8,
            y.wrapping_mul(7) as u8,
            x.wrapping_add(y).wrapping_mul(11) as u8,
            255,
        ])
    }));
    let sixel = encode_fast_sixel(&image, Size::new(8, 4), false).unwrap();
    let palette_entries = sixel.data.matches(";2;").count();

    assert!(
        palette_entries >= 128,
        "fast SIXEL only encoded {palette_entries} colors"
    );
}

#[test]
fn media_picker_uses_the_panel_color_for_cell_padding() {
    let mut preview = PreviewPresentation::default();
    preview.configure_media_picker(Picker::halfblocks(), false);
    let state = preview
        .media_picker
        .new_resize_protocol(DynamicImage::new_rgba8(1, 1));

    assert_eq!(state.background_color(), Some(media_background_color()));
}

#[test]
fn fast_sixel_declares_its_raster_and_wraps_tmux() {
    let image =
        DynamicImage::ImageRgba8(ImageBuffer::from_pixel(20, 20, Rgba([40, 120, 220, 255])));
    let plain = encode_fast_sixel(&image, Size::new(2, 1), false).unwrap();
    let tmux = encode_fast_sixel(&image, Size::new(2, 1), true).unwrap();

    assert!(plain.data.starts_with("\u{1b}[2X\u{1b}P9;1;0q\"1;1;20;20"));
    assert!(plain.data.ends_with("\u{1b}\\"));
    assert!(
        tmux.data
            .starts_with("\u{1b}Ptmux;\u{1b}\u{1b}[2X\u{1b}\u{1b}P9;1;0q\"1;1;20;20")
    );
    assert!(tmux.data.ends_with("\u{1b}\\"));
}

#[test]
fn fast_sixel_cache_reuses_an_encoded_frame_after_selection_changes() {
    let mut preview = PreviewPresentation::default();
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Sixel);
    preview.configure_media_picker(picker, false);
    let image = Arc::new(DynamicImage::ImageRgba8(ImageBuffer::from_fn(
        400,
        200,
        |x, y| Rgba([x as u8, y as u8, x.wrapping_add(y) as u8, 255]),
    )));
    let available = Rect::new(0, 0, 40, 10);
    let deadline = Instant::now() + Duration::from_secs(2);

    let _ = preview.media_state(
        1,
        &image,
        MediaPreviewProtocol::Sixel,
        SixelQuality::Fast,
        available,
    );
    assert!(preview.media_work_pending());

    loop {
        preview.poll_media();
        let (_, protocol, _, render_state) = preview.media_state(
            1,
            &image,
            MediaPreviewProtocol::Sixel,
            SixelQuality::Fast,
            available,
        );
        if protocol == MediaPreviewProtocol::Sixel
            && matches!(render_state, MediaRenderState::Immediate(_))
        {
            break;
        }
        assert!(Instant::now() < deadline, "fast SIXEL did not finish");
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!preview.media_work_pending());
    assert_eq!(preview.sixel_cache.len(), 1);
    let cached_bytes = preview.sixel_cache_bytes;

    preview.hide_media();
    let (_, protocol, _, render_state) = preview.media_state(
        2,
        &image,
        MediaPreviewProtocol::Sixel,
        SixelQuality::Fast,
        available,
    );

    assert_eq!(protocol, MediaPreviewProtocol::Sixel);
    assert!(matches!(render_state, MediaRenderState::Immediate(_)));
    assert_eq!(preview.sixel_cache.len(), 1);
    assert_eq!(preview.sixel_cache_bytes, cached_bytes);
}

#[test]
fn rapid_sixel_replacement_fences_stale_worker_results() {
    let mut preview = PreviewPresentation::default();
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Sixel);
    preview.configure_media_picker(picker, false);
    let stale = Arc::new(DynamicImage::ImageRgba8(ImageBuffer::from_fn(
        1200,
        700,
        |x, y| Rgba([x as u8, y as u8, x.wrapping_add(y) as u8, 255]),
    )));
    let current = Arc::new(DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
        200,
        100,
        Rgba([20, 180, 90, 255]),
    )));
    let available = Rect::new(0, 0, 60, 20);

    let _ = preview.media_state(
        1,
        &stale,
        MediaPreviewProtocol::Sixel,
        SixelQuality::Quality,
        available,
    );
    let stale_id = preview.media_state.as_ref().unwrap().request_id;
    let _ = preview.media_state(
        2,
        &current,
        MediaPreviewProtocol::Sixel,
        SixelQuality::Fast,
        available,
    );
    let current_id = preview.media_state.as_ref().unwrap().request_id;
    assert_ne!(stale_id, current_id);
    assert!(!preview.media_state.as_ref().unwrap().accepts(stale_id));
    assert!(preview.media_state.as_ref().unwrap().accepts(current_id));

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        preview.poll_media();
        let (_, protocol, _, render_state) = preview.media_state(
            2,
            &current,
            MediaPreviewProtocol::Sixel,
            SixelQuality::Fast,
            available,
        );
        if protocol == MediaPreviewProtocol::Sixel
            && matches!(render_state, MediaRenderState::Immediate(_))
        {
            break;
        }
        assert!(Instant::now() < deadline, "latest SIXEL did not finish");
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(preview.sixel_cache.len(), 1);
    assert_eq!(preview.sixel_cache[0].key.source_width, current.width());
    assert_eq!(preview.sixel_cache[0].key.source_height, current.height());
}

#[test]
fn media_interactions_render_immediately_then_restore_sixel() {
    let mut preview = PreviewPresentation::default();
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Sixel);
    preview.configure_media_picker(picker, false);
    let image = Arc::new(DynamicImage::ImageRgba8(ImageBuffer::from_fn(
        800,
        400,
        |x, y| Rgba([x as u8, y as u8, x.wrapping_add(y) as u8, 255]),
    )));
    let available = Rect::new(5, 6, 40, 10);

    assert!(preview.zoom_media(true, None));
    let (area, protocol, _, render_state) = preview.media_state(
        1,
        &image,
        MediaPreviewProtocol::Sixel,
        SixelQuality::Fast,
        available,
    );
    assert_eq!(protocol, MediaPreviewProtocol::Halfblocks);
    let MediaRenderState::Immediate(state) = render_state else {
        panic!("zoom did not produce an immediate preview");
    };
    let mut buffer = Buffer::empty(Rect::new(0, 0, 60, 20));
    Widget::render(Image::new(state), area, &mut buffer);
    assert!(
        area.positions()
            .any(|position| buffer.cell(position).unwrap().symbol() == "▀")
    );

    let settled = preview.media_view.preview_until.unwrap() + Duration::from_millis(1);
    assert!(preview.poll_media_at(settled));
    assert!(!preview.media_live_preview_active_at(settled));
    let (_, protocol, _, render_state) = preview.media_state(
        1,
        &image,
        MediaPreviewProtocol::Sixel,
        SixelQuality::Fast,
        available,
    );
    assert_eq!(protocol, MediaPreviewProtocol::Halfblocks);
    assert!(matches!(render_state, MediaRenderState::Immediate(_)));

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if preview.poll_media() {
            let (_, protocol, _, render_state) = preview.media_state(
                1,
                &image,
                MediaPreviewProtocol::Sixel,
                SixelQuality::Fast,
                available,
            );
            if protocol == MediaPreviewProtocol::Sixel {
                assert!(matches!(render_state, MediaRenderState::Immediate(_)));
                break;
            }
        }
        assert!(
            Instant::now() < deadline,
            "final SIXEL frame did not replace the interactive preview"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn large_media_drag_previews_stay_interactive() {
    let mut preview = PreviewPresentation::default();
    let image = Arc::new(DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
        3840,
        2160,
        Rgba([40, 120, 220, 255]),
    )));
    let available = Rect::new(0, 0, 120, 40);

    assert!(preview.zoom_media(true, None));
    let (_, protocol, _, render_state) = preview.media_state(
        1,
        &image,
        MediaPreviewProtocol::Sixel,
        SixelQuality::Fast,
        available,
    );
    assert_eq!(protocol, MediaPreviewProtocol::Halfblocks);
    assert!(matches!(render_state, MediaRenderState::Immediate(_)));
    preview.begin_media_pan(Position::new(80, 20));

    let started = Instant::now();
    for step in 0..40 {
        let column = if step % 2 == 0 { 79 } else { 80 };
        assert!(preview.pan_media(Position::new(column, 20)));
        let (_, protocol, _, render_state) = preview.media_state(
            1,
            &image,
            MediaPreviewProtocol::Sixel,
            SixelQuality::Fast,
            available,
        );
        assert_eq!(protocol, MediaPreviewProtocol::Halfblocks);
        assert!(matches!(render_state, MediaRenderState::Immediate(_)));
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "40 large-image drag previews took {elapsed:?}"
    );
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
    assert!(output.starts_with("\u{1b}[s\u{1b}[4;3H\u{1b}[48;2;"));
    assert!(output.ends_with("minline-image\u{1b}[49m\u{1b}[u"));

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
    assert!(cleanup.starts_with("\u{1b}[s\u{1b}[4;3H\u{1b}[48;2;"));
    assert!(cleanup.contains("m\u{1b}[10X"));
    assert!(cleanup.ends_with("\u{1b}[49m\u{1b}[u"));
    assert!(!preview.take_terminal_output().bytes.is_empty());

    preview.queue_inline_frame(8, MediaPreviewProtocol::Iterm2, area, None);
    assert!(preview.take_terminal_cleanup().is_empty());

    preview.hide_media();
    let cleanup = String::from_utf8(preview.take_terminal_cleanup()).unwrap();
    assert!(cleanup.starts_with("\u{1b}[s\u{1b}[6;5H\u{1b}[48;2;"));
    assert!(cleanup.contains("m\u{1b}[10X"));
    assert!(cleanup.ends_with("\u{1b}[49m\u{1b}[u"));
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
fn media_view_zooms_pans_and_resets_to_fit() {
    let mut preview = PreviewPresentation::default();
    let image = Arc::new(DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
        800,
        400,
        Rgba([255, 0, 0, 255]),
    )));
    let available = Rect::new(5, 6, 40, 10);

    let (fit_area, _, _, _) = preview.media_state(
        1,
        &image,
        MediaPreviewProtocol::Halfblocks,
        SixelQuality::Fast,
        available,
    );
    assert_eq!(fit_area, available);
    assert_eq!(preview.media_zoom_percent(), 100);

    assert!(preview.zoom_media(true, None));
    assert_eq!(preview.media_zoom_percent(), 125);
    let (zoomed_area, _, _, _) = preview.media_state(
        1,
        &image,
        MediaPreviewProtocol::Halfblocks,
        SixelQuality::Fast,
        available,
    );
    assert_eq!(zoomed_area, available);

    preview.begin_media_pan(Position::new(25, 10));
    assert!(preview.pan_media(Position::new(15, 10)));
    preview.end_media_pan();
    let (center_x, center_y) = preview.media_center_for_test();
    assert!(center_x > 0.5);
    assert_eq!(center_y, 0.5);

    assert!(preview.zoom_media(false, None));
    assert_eq!(preview.media_zoom_percent(), 100);
    assert_eq!(preview.media_center_for_test(), (0.5, 0.5));
}

#[test]
fn wheel_zoom_keeps_the_source_point_under_the_cursor() {
    let mut preview = PreviewPresentation::default();
    let image = Arc::new(DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
        800,
        400,
        Rgba([255, 0, 0, 255]),
    )));
    let available = Rect::new(5, 6, 40, 10);
    let cursor = Position::new(39, 13);
    let source_at_cursor = |metrics: MediaViewMetrics, center: (f64, f64), cursor: Position| {
        let x =
            (f64::from(cursor.x) + 0.5 - f64::from(metrics.area.x)) / f64::from(metrics.area.width);
        let y = (f64::from(cursor.y) + 0.5 - f64::from(metrics.area.y))
            / f64::from(metrics.area.height);
        (
            center.0
                + (x - 0.5) * (f64::from(metrics.viewport.width) / metrics.scaled_width).min(1.0),
            center.1
                + (y - 0.5) * (f64::from(metrics.viewport.height) / metrics.scaled_height).min(1.0),
        )
    };

    let _ = preview.media_state(
        1,
        &image,
        MediaPreviewProtocol::Halfblocks,
        SixelQuality::Fast,
        available,
    );
    let before = source_at_cursor(
        preview.media_view.metrics.unwrap(),
        preview.media_center_for_test(),
        cursor,
    );
    assert!(preview.zoom_media(true, Some(cursor)));
    let after_first_wheel = source_at_cursor(
        preview.media_view.metrics.unwrap(),
        preview.media_center_for_test(),
        cursor,
    );
    assert!((before.0 - after_first_wheel.0).abs() < 1e-9);
    assert!((before.1 - after_first_wheel.1).abs() < 1e-9);
    assert!(preview.zoom_media(true, Some(cursor)));
    let after_queued_wheel = source_at_cursor(
        preview.media_view.metrics.unwrap(),
        preview.media_center_for_test(),
        cursor,
    );
    assert!((before.0 - after_queued_wheel.0).abs() < 1e-9);
    assert!((before.1 - after_queued_wheel.1).abs() < 1e-9);

    let _ = preview.media_state(
        1,
        &image,
        MediaPreviewProtocol::Halfblocks,
        SixelQuality::Fast,
        available,
    );
    let after = source_at_cursor(
        preview.media_view.metrics.unwrap(),
        preview.media_center_for_test(),
        cursor,
    );

    assert!((before.0 - after.0).abs() < 1e-9);
    assert!((before.1 - after.1).abs() < 1e-9);
    let (center_x, center_y) = preview.media_center_for_test();
    assert!(center_x > 0.5);
    assert!(center_y > 0.5);
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
            content: PreviewContent::Source(
                "# Heading\n\nA paragraph that wraps across multiple rows.",
            ),
            generation: 1,
            path: "README.md",
            markdown: true,
            show_initial_diff_header: false,
            width: 16,
            viewport_height: 8,
            wrapped: true,
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
            content: PreviewContent::Source("alpha beta gamma"),
            generation: 1,
            path: "notes.txt",
            markdown: false,
            show_initial_diff_header: false,
            width: 10,
            viewport_height: 4,
            wrapped: true,
        },
        &mut scroll,
    );

    assert_eq!(
        presentation.source_position_at_rendered_position("alpha beta gamma", 1, 3, 0,),
        Some((1, 14))
    );

    let diff = "@@ -1 +1 @@\n+alpha beta gamma";
    let document = DiffDocument::parse(diff.to_owned());
    presentation.prepare(
        PreviewInput {
            content: PreviewContent::Diff(&document),
            generation: 2,
            path: "notes.txt",
            markdown: false,
            show_initial_diff_header: false,
            width: 11,
            viewport_height: 4,
            wrapped: true,
        },
        &mut scroll,
    );
    assert_eq!(
        presentation.diff_position_at_rendered_position(&document, 2, 4, 1),
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
            content: PreviewContent::Source(&content),
            generation: 1,
            path: "README.md",
            markdown: true,
            show_initial_diff_header: false,
            width: 80,
            viewport_height: 8,
            wrapped: false,
        },
        &mut scroll,
    );

    let cache = presentation.cache.as_ref().unwrap();
    assert!(!cache.markdown);
    assert!(!cache.fully_styled);
}

#[test]
fn sparse_source_line_index_matches_str_lines() {
    for content in ["", "a", "a\n", "\n", "\n\n", "a\r\nb\r\n", "a\rb", "a\r"] {
        let index = SourceLineIndex::new(content);
        let expected = content.lines().collect::<Vec<_>>();
        assert_eq!(index.count, expected.len(), "content={content:?}");
        for (line, expected) in expected.iter().enumerate() {
            assert_eq!(index.line(content, line), Some(*expected));
        }
        assert_eq!(index.line(content, expected.len()), None);
    }

    let content = (0..1_000)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    let index = SourceLineIndex::new(&content);
    assert_eq!(index.checkpoints.len(), 4);
    assert_eq!(index.line(&content, 999), Some("line 999"));
}

#[test]
fn windowed_source_keeps_deep_line_numbers_and_positions() {
    let content = (0..31_000)
        .map(|line| format!("value {line}\n"))
        .collect::<String>();
    let mut presentation = PreviewPresentation::default();
    let mut scroll = 30_500;

    let preview = presentation.prepare(
        PreviewInput {
            content: PreviewContent::Source(&content),
            generation: 1,
            path: "src/generated.rs",
            markdown: false,
            show_initial_diff_header: false,
            width: 100,
            viewport_height: 4,
            wrapped: false,
        },
        &mut scroll,
    );

    assert!(presentation.is_windowed());
    assert_eq!(preview.lines.len(), 4);
    assert!(preview.lines[0].spans[0].content.starts_with("30501  "));
    assert_eq!(
        presentation.source_position_at_rendered_position(&content, 30_500, 7, 7),
        Some((30_501, 0)),
    );
    assert_eq!(
        presentation.source_line(&content, 30_500),
        Some("value 30500")
    );
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
                content: PreviewContent::Source(content),
                generation: 1,
                path: "README.md",
                markdown: true,
                show_initial_diff_header: false,
                width: 30,
                viewport_height: 30,
                wrapped,
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

#[test]
fn editor_presentation_reuses_deep_wrapping_and_styled_window() {
    use std::fmt::Write;

    let mut source = String::new();
    let mut line_starts = vec![0];
    for line in 0..50_000 {
        if line > 0 {
            source.push('\n');
            line_starts.push(source.len());
        }
        write!(source, "line {line:05} value").unwrap();
    }
    let path = RepoPath::from("source.rs");
    let input = EditorPreviewInput {
        source: &source,
        line_starts: &line_starts,
        revision: 1,
        revision_changed_from_line: 0,
        repo_path: &path,
        path: "source.rs",
        width: 24,
        viewport_height: 24,
        wrapped: true,
    };
    let mut presentation = PreviewPresentation::default();

    let (cursor_row, _) = presentation.editor_rendered_position(input, 49_999, 5);
    assert_eq!(presentation.editor_cache_metrics(), (50_000, 0));
    let mut scroll = cursor_row.saturating_sub(23);
    let first = presentation.prepare_editor(input, &mut scroll);
    assert_eq!(first.lines.len(), 24);
    assert_eq!(presentation.editor_cache_metrics(), (50_000, 1));

    let second = presentation.prepare_editor(input, &mut scroll);
    assert_eq!(second.lines.len(), 24);
    assert_eq!(presentation.editor_cache_metrics(), (50_000, 1));

    let mut edited = source.clone();
    edited.pop();
    edited.push(' ');
    let edited_input = EditorPreviewInput {
        source: &edited,
        revision: 4,
        revision_changed_from_line: 49_999,
        ..input
    };
    presentation.editor_rendered_position(edited_input, 49_999, 5);
    assert_eq!(presentation.editor_cache_metrics(), (50_001, 1));
}
