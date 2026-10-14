use arboard::{Clipboard, ImageData};
use directories::ProjectDirs;
use image::{ColorType, ImageFormat};
use std::borrow::Cow;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_CLIPBOARD_CACHE_FILES: usize = 32;

pub fn copy_rgba(width: usize, height: usize, rgba: &[u8]) -> Result<(), String> {
    if width == 0 || height == 0 || rgba.len() != width.saturating_mul(height).saturating_mul(4) {
        return Err("Изображение ещё не готово для копирования".to_string());
    }
    let mut clipboard = Clipboard::new().map_err(|error| error.to_string())?;
    clipboard
        .set_image(ImageData {
            width,
            height,
            bytes: Cow::Borrowed(rgba),
        })
        .map_err(|error| error.to_string())
}

pub fn paste_image_to_cache() -> Result<PathBuf, String> {
    let mut clipboard = Clipboard::new().map_err(|error| error.to_string())?;
    let image = clipboard
        .get_image()
        .map_err(|_| "В буфере обмена нет поддерживаемого изображения".to_string())?;
    if image.width == 0
        || image.height == 0
        || image.bytes.len() != image.width.saturating_mul(image.height).saturating_mul(4)
    {
        return Err("Изображение в буфере обмена повреждено".to_string());
    }

    let directory = ProjectDirs::from("", "Anton4k272", "AnMedia")
        .map(|directories| directories.cache_dir().join("clipboard"))
        .unwrap_or_else(|| PathBuf::from("clipboard"));
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let width = u32::try_from(image.width)
        .map_err(|_| "Изображение в буфере обмена слишком широкое".to_string())?;
    let height = u32::try_from(image.height)
        .map_err(|_| "Изображение в буфере обмена слишком высокое".to_string())?;
    let path = directory.join(format!("clipboard-{timestamp}.png"));
    image::save_buffer_with_format(
        &path,
        image.bytes.as_ref(),
        width,
        height,
        ColorType::Rgba8,
        ImageFormat::Png,
    )
    .map_err(|error| error.to_string())?;
    cleanup_clipboard_cache(&directory);
    Ok(path)
}

fn cleanup_clipboard_cache(directory: &std::path::Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let is_clipboard_image = path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.starts_with("clipboard-") && value.ends_with(".png"));
            if !is_clipboard_image {
                return None;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .unwrap_or(UNIX_EPOCH);
            Some((path, modified))
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| right.1.cmp(&left.1));
    for (path, _) in files.into_iter().skip(MAX_CLIPBOARD_CACHE_FILES) {
        let _ = fs::remove_file(path);
    }
}
