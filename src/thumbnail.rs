use crate::cover_art::embedded_cover_bytes;
use crate::formats::{MediaKind, detect_kind};
use crate::image_decode::{DecodedRgba, decode_any, decode_bytes};
use crossbeam_channel::{Receiver, Sender, unbounded};
use directories::ProjectDirs;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::thread;

const THUMBNAIL_SIDE: u32 = 128;
const WORKERS: usize = 2;

pub struct ThumbnailData {
    pub path: PathBuf,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub struct ThumbnailResult {
    pub path: PathBuf,
    pub data: Option<ThumbnailData>,
}

pub fn start_worker() -> (Sender<PathBuf>, Receiver<ThumbnailResult>) {
    let (request_tx, request_rx) = unbounded::<PathBuf>();
    let (result_tx, result_rx) = unbounded::<ThumbnailResult>();

    for index in 0..WORKERS {
        let request_rx = request_rx.clone();
        let result_tx = result_tx.clone();
        let _ = thread::Builder::new()
            .name(format!("AnMedia-Thumbnails-{index}"))
            .spawn(move || {
                while let Ok(path) = request_rx.recv() {
                    let data = make_thumbnail(&path);
                    if result_tx.send(ThumbnailResult { path, data }).is_err() {
                        break;
                    }
                }
            });
    }

    (request_tx, result_rx)
}

fn make_thumbnail(path: &Path) -> Option<ThumbnailData> {
    if matches!(
        crate::security::inspect(path),
        crate::security::Verdict::Blocked(_)
    ) {
        return None;
    }
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let cache = cache_path(path, &metadata);
    if let Some(image) = cache
        .as_ref()
        .and_then(|cache| std::fs::read(cache).ok())
        .and_then(|bytes| decode_bytes(&bytes).ok())
    {
        return Some(into_data(path, image));
    }

    let decoded = match detect_kind(path) {
        MediaKind::Image => decode_any(path).ok(),
        MediaKind::Audio => embedded_cover_bytes(path).and_then(|bytes| decode_bytes(&bytes).ok()),
        MediaKind::Video => crate::mpv::grab_frame(path, Some("20%"), Some(THUMBNAIL_SIDE * 2))
            .ok()
            .and_then(|bytes| decode_bytes(&bytes).ok()),
        MediaKind::Unknown => None,
    }?;
    let full =
        image::RgbaImage::from_raw(decoded.width as u32, decoded.height as u32, decoded.rgba)?;
    let thumbnail = image::DynamicImage::ImageRgba8(full)
        .thumbnail(THUMBNAIL_SIDE, THUMBNAIL_SIDE)
        .to_rgba8();
    if let Some(cache) = cache {
        if let Some(parent) = cache.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = thumbnail.save_with_format(&cache, image::ImageFormat::Png);
    }
    let (width, height) = thumbnail.dimensions();
    Some(into_data(
        path,
        DecodedRgba {
            width: width as usize,
            height: height as usize,
            rgba: thumbnail.into_raw(),
        },
    ))
}

fn into_data(path: &Path, image: DecodedRgba) -> ThumbnailData {
    ThumbnailData {
        path: path.to_path_buf(),
        width: image.width,
        height: image.height,
        rgba: image.rgba,
    }
}

fn cache_path(path: &Path, metadata: &std::fs::Metadata) -> Option<PathBuf> {
    let directory = ProjectDirs::from("", "Anton4k272", "AnMedia")?
        .cache_dir()
        .join("thumbnails");
    let mut hasher = DefaultHasher::new();
    path.to_string_lossy().to_lowercase().hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_secs())
        .hash(&mut hasher);
    THUMBNAIL_SIDE.hash(&mut hasher);
    Some(directory.join(format!("{:016x}.png", hasher.finish())))
}
