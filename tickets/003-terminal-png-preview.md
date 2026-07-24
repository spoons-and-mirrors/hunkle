# Ticket 003: Terminal Media Preview

**Status:** Implemented; native Kitty verified in Ghostty

**Blocked by:** Nothing. Native compatibility outside Ghostty remains terminal-dependent.

## What Was Built

Render static previews for selected image and video files inside the existing Files preview pane. Images are decoded directly. Videos are converted to a representative still frame by a bounded `ffmpeg` subprocess. Both sources then share the same asynchronous image rendering path.

The default backend is a deterministic true-color Unicode half-block reconstruction. Ghostty users can select `Kitty (Ghostty)` under Settings -> Media protocol to render native pixels through Kitty virtual placements.

## Supported Media

- Images: BMP, GIF, ICO, JPEG, PNG, PNM/PBM/PGM/PPM, QOI, TGA, TIFF, and WebP.
- Videos: 3GP, AVI, FLV, M4V, MKV, MOV, MP4, MPEG/MPG, OGV, WebM, and WMV.
- GIF previews use the decoded still frame; animation is not attempted.
- Video previews require `ffmpeg` on `PATH`. Its absence or failure produces an informative in-pane error.

## Product Decisions

- Preserve existing text, diff, source, and rendered-Markdown behavior for non-media content.
- Treat a video preview as a thumbnail, not playback.
- Fail closed to Unicode half-blocks. Kitty is enabled only by an explicit user setting and labelled for the verified Ghostty path rather than inferred from terminal environment variables.
- Preserve aspect ratio, account for terminal cell geometry, scale down to the preview body, and center the result without covering headers or adjacent panes.
- Decode media and resize/encode terminal presentation away from the render loop.
- Keep the existing generation and active-workspace checks so late file loads cannot replace the current selection. The threaded renderer also rejects stale resize results.
- Clear native presentation state when selection, geometry, interaction, workspace, terminal session, or application lifecycle changes.
- Hide text wrapping, scrolling, and Markdown controls in media mode while preserving the selected path and read-only state in the header.

## Compatibility and Safety

- Direct image sources are limited to 100 MB, 16384x16384 dimensions, and 256 MB decoder allocation. Decoded previews are bounded to 3840x2160.
- Video extraction has an 8-second timeout and bounded stdout/stderr capture. Process groups/job objects ensure timed-out `ffmpeg` work is terminated.
- Corrupt, unsupported, excessive, and unavailable media produce text errors rather than exposing binary bytes.
- Symlinks, directories, and special files retain the existing safe text description instead of being followed as media.
- `ratatui-image` 11.0.6 matches Hunkle's Ratatui 0.30 and Crossterm 0.29 versions and provides both Kitty virtual placements and the half-block fallback.
- Native-pixel image preview is manually verified in Ghostty running as a Windows GUI WSL terminal.
- Direct WezTerm did not render the Kitty preview in manual testing because its virtual-placeholder support is incomplete.
- Herdr did not render the Kitty preview in manual testing even with `[experimental] kitty_graphics = true`; it therefore uses the Unicode fallback unless its graphics forwarding path is fixed independently.
- Kitty cleanup is emitted before terminal restoration when the native backend is enabled.

## Acceptance Criteria

- [x] Selecting a supported raster image displays it in the Files preview body with preserved aspect ratio.
- [x] With Kitty disabled, images render using true-color Unicode half-block cells.
- [x] Selecting a supported video requests a bounded `ffmpeg` thumbnail and renders it through the image path.
- [x] Missing `ffmpeg`, corrupt images, and unsupported media failures show useful in-pane errors.
- [x] Media loading, decoding, and terminal resize encoding remain asynchronous.
- [x] Rapid selection and resize results retain generation-based stale-result rejection.
- [x] Switching from image to text and opening an overlay clears image presentation.
- [x] Existing text, diff, source, Markdown, wrapping, scrolling, and large-preview behavior remain covered by the full test suite.
- [x] Full-surface Ratatui tests force half-block rendering and cover async rendering, bounds, image-to-text replacement, overlay cleanup, and corrupt content.
- [x] Settings persistence fails closed to half-blocks and preserves explicit Kitty selection.
- [x] A selected PNG displays as native pixels in Ghostty.
- [ ] Manual Ghostty acceptance covers video thumbnails, resize/repaint, rapid selection changes, overlays, editor suspend/resume, and clean shutdown.
- [ ] A manual run with Herdr Kitty graphics disabled confirms the default fallback is readable and emits no protocol artifacts.

## Not In This Ticket

- Animated image presentation or video playback.
- PDF, PostScript, SVG, document, archive, or 3D-model preview providers.
- Image editing, cropping, exporting, or mutation.
- Rendering media from historical commits or binary diffs in Changes or Graph.
- Adding a Sixel or iTerm2 output path.
