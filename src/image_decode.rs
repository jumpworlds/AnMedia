//! Универсальное декодирование изображений.
//!
//! Цепочка: встроенный декодер (crate `image`, с учётом EXIF-ориентации) →
//! SVG (resvg) → JPEG XL (jxl-oxide) → PSD → системные кодеки Windows (WIC:
//! HEIF/AVIF, JPEG XR, RAW камер) → FFmpeg через libmpv (всё остальное).

use crate::formats::sniff_file;
use image::ImageDecoder;
use image::metadata::Orientation;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

const SVG_TARGET_SIDE: f32 = 2048.0;
const ANIMATION_BYTES_LIMIT: usize = 512 * 1024 * 1024;
const ANIMATION_FRAMES_LIMIT: usize = 2000;

static MAX_TEXTURE_SIDE: AtomicUsize = AtomicUsize::new(8192);

/// Максимальная сторона текстуры видеокарты; большие кадры уменьшаются.
pub fn set_max_texture_side(side: usize) {
    MAX_TEXTURE_SIDE.store(side.clamp(2048, 16384), Ordering::Relaxed);
}

pub struct DecodedRgba {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub struct AnimationFrame {
    pub rgba: Vec<u8>,
    pub delay_ms: u32,
}

pub struct DecodedImage {
    pub image: DecodedRgba,
    /// Размер исходного файла до уменьшения под текстуру.
    pub original_size: [usize; 2],
    /// Кадры анимации (GIF/APNG/WebP); пусто для статичных изображений.
    pub frames: Vec<AnimationFrame>,
    pub format: String,
}

/// Полное декодирование для просмотра: анимация + ограничение размера.
pub fn decode_for_view(path: &Path) -> Result<DecodedImage, String> {
    let format = format_label(path);
    if let Some((width, height, frames)) = decode_animation(path)
        && frames.len() > 1
    {
        let max_side = MAX_TEXTURE_SIDE.load(Ordering::Relaxed);
        if width <= max_side && height <= max_side {
            let first = frames[0].rgba.clone();
            return Ok(DecodedImage {
                image: DecodedRgba {
                    width,
                    height,
                    rgba: first,
                },
                original_size: [width, height],
                frames,
                format,
            });
        }
    }
    let image = decode_any(path)?;
    let original_size = [image.width, image.height];
    let image = fit_to_side(image, MAX_TEXTURE_SIDE.load(Ordering::Relaxed));
    Ok(DecodedImage {
        image,
        original_size,
        frames: Vec::new(),
        format,
    })
}

fn format_label(path: &Path) -> String {
    match sniff_file(path) {
        Some((_, real)) => real.to_ascii_uppercase(),
        None => path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_uppercase())
            .unwrap_or_default(),
    }
}

pub fn decode_any(path: &Path) -> Result<DecodedRgba, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let sniffed = sniff_file(path).map(|(_, extension)| extension);
    let kind = sniffed.unwrap_or(extension.as_str());

    let mut errors = Vec::new();
    let attempt = |result: Result<DecodedRgba, String>, errors: &mut Vec<String>| match result {
        Ok(image) => Some(image),
        Err(error) => {
            errors.push(error);
            None
        }
    };

    let special: Option<fn(&Path) -> Result<DecodedRgba, String>> =
        if extension == "svg" || extension == "svgz" || kind == "svg" {
            Some(decode_svg)
        } else if kind == "jxl" || extension == "jxl" {
            Some(decode_jxl)
        } else if kind == "psd" || extension == "psd" || extension == "psb" {
            Some(decode_psd)
        } else {
            None
        };
    if let Some(decoder) = special
        && let Some(image) = attempt(decoder(path), &mut errors)
    {
        return Ok(image);
    }

    if !matches!(kind, "heic" | "avif" | "cr3" | "jxr" | "jp2" | "j2k")
        && let Some(image) = attempt(decode_builtin(path), &mut errors)
    {
        return Ok(image);
    }
    let heif_like = matches!(kind, "heic" | "avif")
        || matches!(
            extension.as_str(),
            "heic" | "heif" | "hif" | "avif" | "avifs"
        );
    match wic::decode(path, !heif_like) {
        Ok(image) => return Ok(image),
        Err(error) => errors.push(format!("кодеки Windows: {error}")),
    }
    match crate::mpv::grab_frame(path, None, None).and_then(|png| decode_bytes(&png)) {
        Ok(image) => return Ok(image),
        Err(error) => errors.push(error),
    }
    let mut message = errors.join("; ");
    if heif_like {
        message.push_str(
            ". Для HEIC/AVIF поставьте «Расширения для изображений HEIF» и «AV1» из Microsoft Store \
             или положите libmpv-2.dll рядом с AnMedia.exe",
        );
    }
    Err(message)
}

pub fn decode_bytes(bytes: &[u8]) -> Result<DecodedRgba, String> {
    let image = image::load_from_memory(bytes)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    let (width, height) = image.dimensions();
    Ok(DecodedRgba {
        width: width as usize,
        height: height as usize,
        rgba: image.into_raw(),
    })
}

/// Уменьшает изображение, чтобы большая сторона не превышала `side`.
pub fn fit_to_side(image: DecodedRgba, side: usize) -> DecodedRgba {
    if image.width <= side && image.height <= side {
        return image;
    }
    let Some(buffer) =
        image::RgbaImage::from_raw(image.width as u32, image.height as u32, image.rgba)
    else {
        return DecodedRgba {
            width: 0,
            height: 0,
            rgba: Vec::new(),
        };
    };
    let scale = side as f64 / image.width.max(image.height) as f64;
    let width = ((image.width as f64 * scale).round() as u32).max(1);
    let height = ((image.height as f64 * scale).round() as u32).max(1);
    let resized = image::imageops::thumbnail(&buffer, width, height);
    DecodedRgba {
        width: width as usize,
        height: height as usize,
        rgba: resized.into_raw(),
    }
}

fn decode_builtin(path: &Path) -> Result<DecodedRgba, String> {
    let mut reader = image::ImageReader::open(path).map_err(|error| error.to_string())?;
    let mut limits = image::Limits::default();
    // Защита от «бомб»: файл не может заставить выделить больше 1,5 ГБ.
    limits.max_alloc = Some(1536 * 1024 * 1024);
    limits.max_image_width = Some(40_000);
    limits.max_image_height = Some(40_000);
    reader.limits(limits);
    let reader = reader
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let mut decoder = reader.into_decoder().map_err(|error| error.to_string())?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image =
        image::DynamicImage::from_decoder(decoder).map_err(|error| error.to_string())?;
    image.apply_orientation(orientation);
    let image = image.to_rgba8();
    let (width, height) = image.dimensions();
    Ok(DecodedRgba {
        width: width as usize,
        height: height as usize,
        rgba: image.into_raw(),
    })
}

type AnimationResult = (usize, usize, Vec<AnimationFrame>);

fn decode_animation(path: &Path) -> Option<AnimationResult> {
    use image::AnimationDecoder;
    use image::codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder};

    let (_, kind) = sniff_file(path)?;
    let reader = || File::open(path).ok().map(BufReader::new);
    let frames = match kind {
        "gif" => GifDecoder::new(reader()?).ok()?.into_frames(),
        "png" => {
            let decoder = PngDecoder::new(reader()?).ok()?;
            if !decoder.is_apng().ok()? {
                return None;
            }
            decoder.apng().ok()?.into_frames()
        }
        "webp" => {
            let decoder = WebPDecoder::new(reader()?).ok()?;
            if !decoder.has_animation() {
                return None;
            }
            decoder.into_frames()
        }
        _ => return None,
    };

    let mut result = Vec::new();
    let mut size = None;
    let mut total = 0usize;
    for frame in frames {
        let Ok(frame) = frame else {
            break;
        };
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay_ms = numerator.checked_div(denominator).unwrap_or(100);
        // Как в браузерах: слишком короткая задержка означает 100 мс.
        let delay_ms = if delay_ms <= 10 { 100 } else { delay_ms };
        let buffer = frame.into_buffer();
        let dimensions = (buffer.width() as usize, buffer.height() as usize);
        if *size.get_or_insert(dimensions) != dimensions {
            break;
        }
        total += buffer.as_raw().len();
        result.push(AnimationFrame {
            rgba: buffer.into_raw(),
            delay_ms,
        });
        if total > ANIMATION_BYTES_LIMIT || result.len() >= ANIMATION_FRAMES_LIMIT {
            break;
        }
    }
    let (width, height) = size?;
    Some((width, height, result))
}

fn decode_svg(path: &Path) -> Result<DecodedRgba, String> {
    if std::fs::metadata(path)
        .map(|value| value.len())
        .unwrap_or(0)
        > 64 * 1024 * 1024
    {
        return Err("SVG: файл больше 64 МБ".to_string());
    }
    let data = std::fs::read(path).map_err(|error| error.to_string())?;
    static FONTS: std::sync::OnceLock<std::sync::Arc<resvg::usvg::fontdb::Database>> =
        std::sync::OnceLock::new();
    let mut options = resvg::usvg::Options::default();
    options.fontdb = FONTS
        .get_or_init(|| {
            let mut database = resvg::usvg::fontdb::Database::new();
            database.load_system_fonts();
            std::sync::Arc::new(database)
        })
        .clone();
    // SVG не может читать другие файлы с диска — только встроенные данные.
    options.resources_dir = None;
    options.image_href_resolver = resvg::usvg::ImageHrefResolver {
        resolve_data: resvg::usvg::ImageHrefResolver::default_data_resolver(),
        resolve_string: Box::new(|_, _| None),
    };
    let tree =
        resvg::usvg::Tree::from_data(&data, &options).map_err(|error| format!("SVG: {error}"))?;
    let size = tree.size();
    if size.width() <= 0.0 || size.height() <= 0.0 {
        return Err("SVG: нулевой размер изображения".to_string());
    }
    let scale = (SVG_TARGET_SIDE / size.width().max(size.height())).clamp(0.05, 8.0);
    let width = (size.width() * scale).round().max(1.0) as u32;
    let height = (size.height() * scale).round().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| "SVG: не удалось создать буфер растеризации".to_string())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for pixel in pixmap.pixels() {
        let color = pixel.demultiply();
        rgba.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
    }
    Ok(DecodedRgba {
        width: width as usize,
        height: height as usize,
        rgba,
    })
}

fn decode_jxl(path: &Path) -> Result<DecodedRgba, String> {
    let image = jxl_oxide::JxlImage::builder()
        .open(path)
        .map_err(|error| format!("JPEG XL: {error}"))?;
    let render = image
        .render_frame(0)
        .map_err(|error| format!("JPEG XL: {error}"))?;
    let frame = render.image_all_channels();
    let width = frame.width();
    let height = frame.height();
    let channels = frame.channels();
    if width == 0 || height == 0 || channels == 0 {
        return Err("JPEG XL: пустой кадр".to_string());
    }
    let source = frame.buf();
    let mut rgba = Vec::with_capacity(width * height * 4);
    for pixel in source.chunks_exact(channels) {
        let to_byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        let (r, g, b, a) = match channels {
            1 => (pixel[0], pixel[0], pixel[0], 1.0),
            2 => (pixel[0], pixel[0], pixel[0], pixel[1]),
            3 => (pixel[0], pixel[1], pixel[2], 1.0),
            _ => (pixel[0], pixel[1], pixel[2], pixel[3]),
        };
        rgba.extend_from_slice(&[to_byte(r), to_byte(g), to_byte(b), to_byte(a)]);
    }
    Ok(DecodedRgba {
        width,
        height,
        rgba,
    })
}

fn decode_psd(path: &Path) -> Result<DecodedRgba, String> {
    let data = std::fs::read(path).map_err(|error| error.to_string())?;
    let document = psd::Psd::from_bytes(&data).map_err(|error| format!("PSD: {error}"))?;
    let width = document.width() as usize;
    let height = document.height() as usize;
    let rgba = document.rgba();
    if width == 0 || height == 0 || rgba.len() != width * height * 4 {
        return Err("PSD: пустое или повреждённое изображение".to_string());
    }
    Ok(DecodedRgba {
        width,
        height,
        rgba,
    })
}

mod wic {
    use super::DecodedRgba;
    use image::metadata::Orientation;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows::Win32::Foundation::GENERIC_READ;
    use windows::Win32::Graphics::Imaging::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICBitmapFrameDecode,
        IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom,
        WICDecodeMetadataCacheOnDemand,
    };
    use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::System::Variant::VT_UI2;
    use windows::core::{PCWSTR, w};

    pub fn decode(path: &Path, apply_orientation: bool) -> Result<DecodedRgba, String> {
        unsafe {
            // Повторная инициализация в том же потоке безопасна; ошибку смены
            // модели COM игнорируем — поток уже инициализирован.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let factory: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                    .map_err(|error| format!("WIC-фабрика: {error}"))?;
            let wide = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>();
            let decoder = factory
                .CreateDecoderFromFilename(
                    PCWSTR(wide.as_ptr()),
                    None,
                    GENERIC_READ,
                    WICDecodeMetadataCacheOnDemand,
                )
                .map_err(|error| format!("формат не распознан ({})", error.message()))?;
            let frame = decoder
                .GetFrame(0)
                .map_err(|error| format!("кадр не прочитан ({})", error.message()))?;
            let orientation = if apply_orientation {
                read_orientation(&frame)
            } else {
                Orientation::NoTransforms
            };
            let converter = factory
                .CreateFormatConverter()
                .map_err(|error| format!("конвертер пикселей: {error}"))?;
            converter
                .Initialize(
                    &frame,
                    &GUID_WICPixelFormat32bppRGBA,
                    WICBitmapDitherTypeNone,
                    None,
                    0.0,
                    WICBitmapPaletteTypeCustom,
                )
                .map_err(|error| format!("преобразование в RGBA: {error}"))?;
            let mut width = 0u32;
            let mut height = 0u32;
            converter
                .GetSize(&mut width, &mut height)
                .map_err(|error| format!("размер изображения: {error}"))?;
            if width == 0 || height == 0 {
                return Err("пустое изображение".to_string());
            }
            let stride = width as usize * 4;
            let mut rgba = vec![0u8; stride * height as usize];
            converter
                .CopyPixels(std::ptr::null(), stride as u32, &mut rgba)
                .map_err(|error| format!("копирование пикселей: {error}"))?;
            if orientation == Orientation::NoTransforms {
                return Ok(DecodedRgba {
                    width: width as usize,
                    height: height as usize,
                    rgba,
                });
            }
            let buffer = image::RgbaImage::from_raw(width, height, rgba)
                .ok_or_else(|| "некорректный буфер пикселей".to_string())?;
            let mut image = image::DynamicImage::ImageRgba8(buffer);
            image.apply_orientation(orientation);
            let image = image.to_rgba8();
            Ok(DecodedRgba {
                width: image.width() as usize,
                height: image.height() as usize,
                rgba: image.into_raw(),
            })
        }
    }

    unsafe fn read_orientation(frame: &IWICBitmapFrameDecode) -> Orientation {
        let Ok(reader) = (unsafe { frame.GetMetadataQueryReader() }) else {
            return Orientation::NoTransforms;
        };
        let mut value = PROPVARIANT::default();
        if unsafe { reader.GetMetadataByName(w!("System.Photo.Orientation"), &mut value) }.is_err()
        {
            return Orientation::NoTransforms;
        }
        let raw = unsafe {
            let inner = &value.Anonymous.Anonymous;
            (inner.vt == VT_UI2).then(|| inner.Anonymous.uiVal)
        };
        let _ = unsafe { PropVariantClear(&mut value) };
        raw.and_then(|value| u8::try_from(value).ok())
            .and_then(Orientation::from_exif)
            .unwrap_or(Orientation::NoTransforms)
    }
}

#[cfg(test)]
mod tests {
    /// Прогон тестовой папки: `ANMEDIA_TEST_MEDIA=путь cargo test -- --nocapture`.
    #[test]
    fn decodes_test_media_folder() {
        let Ok(folder) = std::env::var("ANMEDIA_TEST_MEDIA") else {
            return;
        };
        let mut entries = std::fs::read_dir(folder)
            .expect("папка с тестовыми файлами")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            let kind = crate::formats::detect_kind(&path);
            let started = std::time::Instant::now();
            let result = match kind {
                crate::formats::MediaKind::Image => super::decode_for_view(&path).map(|image| {
                    format!(
                        "{}x{} кадров={} формат={}",
                        image.image.width,
                        image.image.height,
                        image.frames.len(),
                        image.format
                    )
                }),
                _ => crate::mpv::grab_frame(&path, Some("20%"), Some(256))
                    .map(|png| format!("кадр {} байт", png.len())),
            };
            println!(
                "{:<22} {:<8?} {:>5} мс  {}",
                path.file_name().unwrap().to_string_lossy(),
                kind,
                started.elapsed().as_millis(),
                match result {
                    Ok(value) => value,
                    Err(error) => format!("ОШИБКА: {error}"),
                }
            );
        }
    }
}
