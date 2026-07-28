use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver, SyncSender},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};

use image::{DynamicImage, ImageReader, Limits as ImageLimits};

use crate::{
    git::{self, Change},
    process::{self, Limits},
    repo_path::RepoPath,
};

const MAX_IMAGE_SOURCE_BYTES: u64 = 100 * 1024 * 1024;
const MAX_DECODED_WIDTH: u32 = 16_384;
const MAX_DECODED_HEIGHT: u32 = 16_384;
const MAX_DECODED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PREVIEW_WIDTH: u32 = 3_840;
const MAX_PREVIEW_HEIGHT: u32 = 2_160;
const MAX_VIDEO_FRAME_BYTES: usize = 32 * 1024 * 1024;

pub(super) enum LoadedPreview {
    Text(String),
    Image(Arc<DynamicImage>),
    Error(String),
}

pub(super) struct PreviewLoader {
    generation: u64,
    pending: Arc<Mutex<Option<Request>>>,
    wake: Option<SyncSender<()>>,
    receiver: Receiver<Completion>,
    worker: Option<JoinHandle<()>>,
}

impl PreviewLoader {
    pub(super) fn new() -> Self {
        let pending = Arc::new(Mutex::new(None::<Request>));
        let worker_pending = Arc::clone(&pending);
        let (wake, request_rx) = mpsc::sync_channel::<()>(1);
        let (result_tx, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            while request_rx.recv().is_ok() {
                let Some(request) = worker_pending.lock().ok().and_then(|mut slot| slot.take())
                else {
                    continue;
                };
                let content = match &request.task {
                    Task::File(path) => load_file_preview(&request.root, path),
                    Task::Commit(oid) => git::commit_diff(&request.root, oid)
                        .map(LoadedPreview::Text)
                        .unwrap_or_else(|error| LoadedPreview::Error(error.to_string())),
                    Task::Diff(change) => git::diff(&request.root, change)
                        .map(LoadedPreview::Text)
                        .unwrap_or_else(|error| LoadedPreview::Error(error.to_string())),
                };
                if result_tx
                    .send(Completion {
                        generation: request.generation,
                        root: request.root,
                        content,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            generation: 0,
            pending,
            wake: Some(wake),
            receiver,
            worker: Some(worker),
        }
    }

    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    pub(super) fn request_file(&mut self, root: &Path, path: RepoPath) {
        self.request(root, Task::File(path));
    }

    pub(super) fn request_commit(&mut self, root: &Path, oid: String) {
        self.request(root, Task::Commit(oid));
    }

    pub(super) fn request_diff(&mut self, root: &Path, change: Change) {
        self.request(root, Task::Diff(change));
    }

    pub(super) fn poll(&mut self, active_root: Option<&Path>) -> Option<LoadedPreview> {
        let mut content = None;
        while let Ok(result) = self.receiver.try_recv() {
            if result.generation == self.generation
                && active_root.is_some_and(|root| root == result.root)
            {
                content = Some(result.content);
            }
        }
        content
    }

    fn request(&mut self, root: &Path, task: Task) {
        self.invalidate();
        let request = Request {
            generation: self.generation,
            root: root.to_path_buf(),
            task,
        };
        if let Ok(mut pending) = self.pending.lock() {
            *pending = Some(request);
            if let Some(wake) = &self.wake {
                let _ = wake.try_send(());
            }
        }
    }

    pub(super) fn shutdown(&mut self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.take();
        }
        self.wake.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for PreviewLoader {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Request {
    generation: u64,
    root: PathBuf,
    task: Task,
}

enum Task {
    File(RepoPath),
    Commit(String),
    Diff(Change),
}

struct Completion {
    generation: u64,
    root: PathBuf,
    content: LoadedPreview,
}

fn load_file_preview(root: &Path, path: &RepoPath) -> LoadedPreview {
    let full_path = root.join(path.as_path());
    match fs::symlink_metadata(&full_path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return git::file_content(root, path)
                .map(LoadedPreview::Text)
                .unwrap_or_else(|error| LoadedPreview::Error(error.to_string()));
        }
        Err(error) => return LoadedPreview::Error(format!("Could not inspect file: {error}")),
    }
    if is_video(path.as_path()) {
        return load_video_frame(&full_path)
            .map(|image| LoadedPreview::Image(Arc::new(image)))
            .unwrap_or_else(LoadedPreview::Error);
    }
    if is_image(path.as_path()) {
        return load_image(&full_path)
            .map(|image| LoadedPreview::Image(Arc::new(image)))
            .unwrap_or_else(LoadedPreview::Error);
    }
    git::file_content(root, path)
        .map(LoadedPreview::Text)
        .unwrap_or_else(|error| LoadedPreview::Error(error.to_string()))
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "bmp"
                    | "gif"
                    | "ico"
                    | "jpg"
                    | "jpeg"
                    | "png"
                    | "pnm"
                    | "pbm"
                    | "pgm"
                    | "ppm"
                    | "qoi"
                    | "tga"
                    | "tif"
                    | "tiff"
                    | "webp"
            )
        })
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "3gp"
                    | "avi"
                    | "flv"
                    | "m4v"
                    | "mkv"
                    | "mov"
                    | "mp4"
                    | "mpeg"
                    | "mpg"
                    | "ogv"
                    | "webm"
                    | "wmv"
            )
        })
}

fn load_image(path: &Path) -> Result<DynamicImage, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("Could not inspect image: {error}"))?;
    if metadata.len() > MAX_IMAGE_SOURCE_BYTES {
        return Err(format!(
            "Image is too large to preview (limit: {} MB)",
            MAX_IMAGE_SOURCE_BYTES / 1024 / 1024
        ));
    }
    let reader = ImageReader::open(path)
        .map_err(|error| format!("Could not open image: {error}"))?
        .with_guessed_format()
        .map_err(|error| format!("Could not identify image: {error}"))?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| format!("Could not read image dimensions: {error}"))?;
    if width > MAX_DECODED_WIDTH || height > MAX_DECODED_HEIGHT {
        return Err(format!(
            "Image dimensions are too large to preview ({width}x{height})"
        ));
    }
    let mut reader = ImageReader::open(path)
        .map_err(|error| format!("Could not open image: {error}"))?
        .with_guessed_format()
        .map_err(|error| format!("Could not identify image: {error}"))?;
    let mut limits = ImageLimits::default();
    limits.max_image_width = Some(MAX_DECODED_WIDTH);
    limits.max_image_height = Some(MAX_DECODED_HEIGHT);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| format!("Could not decode image: {error}"))?;
    Ok(bound_preview_dimensions(image))
}

fn load_video_frame(path: &Path) -> Result<DynamicImage, String> {
    let output = process::run(
        Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(path)
            .args([
                "-map",
                "0:v:0",
                "-vf",
                "thumbnail=30,scale=1920:1080:force_original_aspect_ratio=decrease",
                "-frames:v",
                "1",
                "-f",
                "image2pipe",
                "-vcodec",
                "png",
                "pipe:1",
            ]),
        Limits::new(MAX_VIDEO_FRAME_BYTES, 64 * 1024, Duration::from_secs(8)),
    )
    .map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "Video previews require ffmpeg on PATH".to_owned()
        } else {
            format!("Could not start ffmpeg: {error}")
        }
    })?;
    if output.timed_out {
        return Err("Video thumbnail generation timed out".to_owned());
    }
    if output.stdout_truncated {
        return Err("Video thumbnail exceeded the preview size limit".to_owned());
    }
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.lines().find(|line| !line.trim().is_empty());
        return Err(detail.map_or_else(
            || "Could not generate a video thumbnail".to_owned(),
            |line| format!("Could not generate a video thumbnail: {line}"),
        ));
    }
    let mut reader = ImageReader::new(Cursor::new(output.stdout))
        .with_guessed_format()
        .map_err(|error| format!("Could not identify video thumbnail: {error}"))?;
    let mut limits = ImageLimits::default();
    limits.max_image_width = Some(MAX_PREVIEW_WIDTH);
    limits.max_image_height = Some(MAX_PREVIEW_HEIGHT);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    reader
        .decode()
        .map(bound_preview_dimensions)
        .map_err(|error| format!("Could not decode video thumbnail: {error}"))
}

fn bound_preview_dimensions(image: DynamicImage) -> DynamicImage {
    if image.width() <= MAX_PREVIEW_WIDTH && image.height() <= MAX_PREVIEW_HEIGHT {
        image
    } else {
        image.thumbnail(MAX_PREVIEW_WIDTH, MAX_PREVIEW_HEIGHT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_joins_the_preview_worker_once() {
        let mut loader = PreviewLoader::new();
        loader.shutdown();
        loader.shutdown();
        assert!(loader.worker.is_none());
        assert!(loader.wake.is_none());
    }

    #[test]
    fn recognizes_supported_static_media() {
        for name in [
            "photo.PNG",
            "scan.tiff",
            "animation.GIF",
            "clip.webm",
            "movie.MP4",
        ] {
            let path = Path::new(name);
            assert!(is_image(path) || is_video(path), "{name}");
        }
        assert!(!is_image(Path::new("notes.txt")));
        assert!(!is_video(Path::new("archive.zip")));
    }

    #[test]
    fn rejects_corrupt_images_without_exposing_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bad.png");
        fs::write(&path, b"not actually a png\0\xff").unwrap();
        let error = load_image(&path).unwrap_err();
        assert!(error.starts_with("Could not read image dimensions:"));
    }
}
