//! Конвертация видео, музыки и фото в другие форматы.
//!
//! Видео и звук кодирует libmpv (FFmpeg) в режиме `--o`, фото — crate `image`.

use crate::formats::MediaKind;
use crate::image_decode::decode_any;
use crossbeam_channel::{Receiver, bounded};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    Video,
    Audio,
    Image,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    High,
    Balanced,
    Small,
}

pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub extension: &'static str,
    pub kind: TargetKind,
    container: &'static str,
    video_codec: &'static str,
    audio_codec: &'static str,
}

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "mp4",
        label: "MP4",
        description: "H.264 + AAC, открывается везде",
        extension: "mp4",
        kind: TargetKind::Video,
        container: "mp4",
        video_codec: "libx264",
        audio_codec: "aac",
    },
    Preset {
        id: "mkv",
        label: "MKV",
        description: "H.265 + Opus, меньше размер",
        extension: "mkv",
        kind: TargetKind::Video,
        container: "matroska",
        video_codec: "libx265",
        audio_codec: "libopus",
    },
    Preset {
        id: "webm",
        label: "WebM",
        description: "VP9 + Opus, для сайтов",
        extension: "webm",
        kind: TargetKind::Video,
        container: "webm",
        video_codec: "libvpx-vp9",
        audio_codec: "libopus",
    },
    Preset {
        id: "mov",
        label: "MOV",
        description: "H.264 + AAC для Apple",
        extension: "mov",
        kind: TargetKind::Video,
        container: "mov",
        video_codec: "libx264",
        audio_codec: "aac",
    },
    Preset {
        id: "avi",
        label: "AVI",
        description: "MPEG-4 + MP3, старые плееры",
        extension: "avi",
        kind: TargetKind::Video,
        container: "avi",
        video_codec: "mpeg4",
        audio_codec: "libmp3lame",
    },
    Preset {
        id: "gif",
        label: "GIF",
        description: "анимация без звука",
        extension: "gif",
        kind: TargetKind::Video,
        container: "gif",
        video_codec: "gif",
        audio_codec: "",
    },
    Preset {
        id: "mp3",
        label: "MP3",
        description: "самый совместимый",
        extension: "mp3",
        kind: TargetKind::Audio,
        container: "mp3",
        video_codec: "",
        audio_codec: "libmp3lame",
    },
    Preset {
        id: "m4a",
        label: "M4A",
        description: "AAC, iPhone и iTunes",
        extension: "m4a",
        kind: TargetKind::Audio,
        container: "ipod",
        video_codec: "",
        audio_codec: "aac",
    },
    Preset {
        id: "flac",
        label: "FLAC",
        description: "без потерь",
        extension: "flac",
        kind: TargetKind::Audio,
        container: "flac",
        video_codec: "",
        audio_codec: "flac",
    },
    Preset {
        id: "wav",
        label: "WAV",
        description: "без сжатия",
        extension: "wav",
        kind: TargetKind::Audio,
        container: "wav",
        video_codec: "",
        audio_codec: "pcm_s16le",
    },
    Preset {
        id: "opus",
        label: "Opus",
        description: "лучшее качество на размер",
        extension: "opus",
        kind: TargetKind::Audio,
        container: "opus",
        video_codec: "",
        audio_codec: "libopus",
    },
    Preset {
        id: "ogg",
        label: "OGG",
        description: "Vorbis",
        extension: "ogg",
        kind: TargetKind::Audio,
        container: "ogg",
        video_codec: "",
        audio_codec: "libvorbis",
    },
    Preset {
        id: "wma",
        label: "WMA",
        description: "Windows Media Audio",
        extension: "wma",
        kind: TargetKind::Audio,
        container: "asf",
        video_codec: "",
        audio_codec: "wmav2",
    },
    Preset {
        id: "aiff",
        label: "AIFF",
        description: "без сжатия, macOS",
        extension: "aiff",
        kind: TargetKind::Audio,
        container: "aiff",
        video_codec: "",
        audio_codec: "pcm_s16be",
    },
    Preset {
        id: "png",
        label: "PNG",
        description: "без потерь, прозрачность",
        extension: "png",
        kind: TargetKind::Image,
        container: "",
        video_codec: "",
        audio_codec: "",
    },
    Preset {
        id: "jpg",
        label: "JPG",
        description: "фото, маленький размер",
        extension: "jpg",
        kind: TargetKind::Image,
        container: "",
        video_codec: "",
        audio_codec: "",
    },
    Preset {
        id: "webp",
        label: "WebP",
        description: "без потерь, для сайтов",
        extension: "webp",
        kind: TargetKind::Image,
        container: "",
        video_codec: "",
        audio_codec: "",
    },
    Preset {
        id: "avif",
        label: "AVIF",
        description: "современный, очень компактный",
        extension: "avif",
        kind: TargetKind::Image,
        container: "",
        video_codec: "",
        audio_codec: "",
    },
    Preset {
        id: "bmp",
        label: "BMP",
        description: "без сжатия",
        extension: "bmp",
        kind: TargetKind::Image,
        container: "",
        video_codec: "",
        audio_codec: "",
    },
    Preset {
        id: "tiff",
        label: "TIFF",
        description: "для печати",
        extension: "tiff",
        kind: TargetKind::Image,
        container: "",
        video_codec: "",
        audio_codec: "",
    },
    Preset {
        id: "ico",
        label: "ICO",
        description: "иконка Windows",
        extension: "ico",
        kind: TargetKind::Image,
        container: "",
        video_codec: "",
        audio_codec: "",
    },
    Preset {
        id: "gif-image",
        label: "GIF",
        description: "256 цветов",
        extension: "gif",
        kind: TargetKind::Image,
        container: "",
        video_codec: "",
        audio_codec: "",
    },
    Preset {
        id: "tga",
        label: "TGA",
        description: "для игр и 3D",
        extension: "tga",
        kind: TargetKind::Image,
        container: "",
        video_codec: "",
        audio_codec: "",
    },
    Preset {
        id: "qoi",
        label: "QOI",
        description: "быстрый без потерь",
        extension: "qoi",
        kind: TargetKind::Image,
        container: "",
        video_codec: "",
        audio_codec: "",
    },
];

/// Форматы, в которые можно перевести файл данного типа.
pub fn presets_for(kind: MediaKind) -> Vec<&'static Preset> {
    PRESETS
        .iter()
        .filter(|preset| match kind {
            MediaKind::Image => preset.kind == TargetKind::Image,
            MediaKind::Audio => preset.kind == TargetKind::Audio,
            MediaKind::Video | MediaKind::Unknown => {
                matches!(preset.kind, TargetKind::Video | TargetKind::Audio)
            }
        })
        .collect()
}

pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.id == id)
}

/// Свободное имя рядом с исходником: «имя.ext», «имя (2).ext»…
pub fn default_output(source: &Path, preset: &Preset) -> PathBuf {
    let directory = source.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("converted");
    let mut candidate = directory.join(format!("{stem}.{}", preset.extension));
    let mut index = 2;
    while candidate.exists() || same_file(&candidate, source) {
        candidate = directory.join(format!("{stem} ({index}).{}", preset.extension));
        index += 1;
    }
    candidate
}

fn same_file(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

pub enum JobResult {
    Done(PathBuf),
    Cancelled,
    Failed(String),
}

pub struct Job {
    pub output: PathBuf,
    pub preset_label: &'static str,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    result: Receiver<JobResult>,
}

impl Job {
    /// Прогресс 0.0…1.0.
    pub fn progress(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn try_result(&self) -> Option<JobResult> {
        self.result.try_recv().ok()
    }
}

pub fn start(source: &Path, output: &Path, preset: &'static Preset, quality: Quality) -> Job {
    let progress = Arc::new(AtomicU32::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let (sender, result) = bounded(1);
    let job = Job {
        output: output.to_path_buf(),
        preset_label: preset.label,
        progress: Arc::clone(&progress),
        cancel: Arc::clone(&cancel),
        result,
    };
    let source = source.to_path_buf();
    let output = output.to_path_buf();
    let _ = thread::Builder::new()
        .name("AnMedia-Convert".to_string())
        .spawn(move || {
            let outcome = if same_file(&source, &output) {
                Err("нельзя перезаписать исходный файл".to_string())
            } else if preset.kind == TargetKind::Image {
                convert_image(&source, &output, preset, quality)
            } else {
                convert_media(&source, &output, preset, quality, &progress, &cancel)
            };
            let message = match outcome {
                Ok(()) if cancel.load(Ordering::Relaxed) => {
                    let _ = std::fs::remove_file(&output);
                    JobResult::Cancelled
                }
                Ok(()) => {
                    progress.store(1000, Ordering::Relaxed);
                    JobResult::Done(output)
                }
                Err(error) => {
                    let _ = std::fs::remove_file(&output);
                    if cancel.load(Ordering::Relaxed) {
                        JobResult::Cancelled
                    } else {
                        JobResult::Failed(error)
                    }
                }
            };
            let _ = sender.send(message);
        });
    job
}

fn convert_media(
    source: &Path,
    output: &Path,
    preset: &Preset,
    quality: Quality,
    progress: &AtomicU32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let output_text = output
        .to_str()
        .ok_or_else(|| "путь сохранения содержит недопустимые символы".to_string())?;
    let source_text = source
        .to_str()
        .ok_or_else(|| "путь к файлу содержит недопустимые символы".to_string())?;
    let (video_options, audio_options) = codec_options(preset, quality);
    let mut options: Vec<(&str, &str)> = vec![
        ("terminal", "no"),
        ("idle", "no"),
        ("keep-open", "no"),
        ("o", output_text),
        ("of", preset.container),
        ("audio-display", "no"),
        ("sid", "no"),
        ("hwdec", "no"),
        // Минимальный кэш чтения: файл локальный, читать наперёд не нужно.
        ("cache", "no"),
        ("demuxer-max-bytes", "16MiB"),
        ("demuxer-max-back-bytes", "0"),
        ("demuxer-readahead-secs", "1"),
        ("vd-lavc-threads", "4"),
    ];
    if preset.video_codec.is_empty() {
        options.push(("vid", "no"));
    } else {
        options.push(("ovc", preset.video_codec));
        if !video_options.is_empty() {
            options.push(("ovcopts", video_options));
        }
    }
    if preset.audio_codec.is_empty() {
        options.push(("aid", "no"));
    } else {
        options.push(("oac", preset.audio_codec));
        if !audio_options.is_empty() {
            options.push(("oacopts", audio_options));
        }
    }
    if preset.id == "gif" {
        options.push((
            "vf",
            // Палитра считается для каждого кадра (stats_mode=single), поэтому
            // кадры не копятся в памяти до конца видео.
            "lavfi=[fps=12,scale=w='min(480,iw)':h=-2:flags=lanczos,split[a][b];[a]palettegen=stats_mode=single[p];[b][p]paletteuse=new=1:dither=bayer:bayer_scale=3]",
        ));
    }
    let instance = crate::mpv::Instance::new(&options)?;
    instance.command(&["loadfile", source_text])?;

    let mut error = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        match instance.next_event(0.2) {
            Some((7, _, Some((reason, code)))) => {
                if reason == 4 {
                    error = Some(instance.error_text(code));
                }
                break;
            }
            Some((1, _, _)) => break,
            _ => {}
        }
        if let (Some(position), Some(duration)) = (
            instance.get_double("time-pos"),
            instance.get_double("duration"),
        ) && duration > 0.0
        {
            let value = ((position / duration).clamp(0.0, 0.999) * 1000.0) as u32;
            progress.store(value, Ordering::Relaxed);
        }
    }
    drop(instance);
    thread::sleep(Duration::from_millis(50));
    if let Some(error) = error {
        return Err(format!("FFmpeg: {error}"));
    }
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    match std::fs::metadata(output) {
        Ok(metadata) if metadata.len() > 0 => Ok(()),
        _ => Err("кодировщик не создал файл (в исходнике нет подходящих дорожек?)".to_string()),
    }
}

fn codec_options(preset: &Preset, quality: Quality) -> (&'static str, &'static str) {
    use Quality::{Balanced, High, Small};
    let video = match (preset.video_codec, quality) {
        ("libx264", High) => "preset=medium,crf=18,rc-lookahead=20",
        ("libx264", Balanced) => "preset=veryfast,crf=22",
        ("libx264", Small) => "preset=veryfast,crf=28",
        ("libx265", High) => {
            "preset=fast,crf=22,x265-params=pools=6:frame-threads=2:rc-lookahead=15"
        }
        ("libx265", Balanced) => {
            "preset=faster,crf=26,x265-params=pools=6:frame-threads=2:rc-lookahead=15"
        }
        ("libx265", Small) => {
            "preset=faster,crf=31,x265-params=pools=6:frame-threads=2:rc-lookahead=15"
        }
        ("libvpx-vp9", High) => "b=0,crf=28,deadline=realtime,cpu-used=6,row-mt=1",
        ("libvpx-vp9", Balanced) => "b=0,crf=34,deadline=realtime,cpu-used=8,row-mt=1",
        ("libvpx-vp9", Small) => "b=0,crf=42,deadline=realtime,cpu-used=8,row-mt=1",
        ("mpeg4", High) => "qscale=2",
        ("mpeg4", Balanced) => "qscale=4",
        ("mpeg4", Small) => "qscale=8",
        _ => "",
    };
    let audio = match (preset.audio_codec, quality) {
        ("libmp3lame" | "aac" | "wmav2", High) => "b=320k",
        ("libmp3lame" | "aac" | "wmav2", Balanced) => "b=192k",
        ("libmp3lame" | "aac" | "wmav2", Small) => "b=96k",
        ("libopus", High) => "b=192k",
        ("libopus", Balanced) => "b=128k",
        ("libopus", Small) => "b=64k",
        ("libvorbis", High) => "b=256k",
        ("libvorbis", Balanced) => "b=160k",
        ("libvorbis", Small) => "b=96k",
        _ => "",
    };
    (video, audio)
}

fn convert_image(
    source: &Path,
    output: &Path,
    preset: &Preset,
    quality: Quality,
) -> Result<(), String> {
    use image::ImageFormat;
    let decoded = decode_any(source)?;
    let buffer =
        image::RgbaImage::from_raw(decoded.width as u32, decoded.height as u32, decoded.rgba)
            .ok_or_else(|| "некорректный буфер изображения".to_string())?;
    let image = image::DynamicImage::ImageRgba8(buffer);
    let file = std::fs::File::create(output)
        .map_err(|error| format!("не удалось создать файл: {error}"))?;
    let mut writer = std::io::BufWriter::new(file);
    let jpeg_quality = match quality {
        Quality::High => 95,
        Quality::Balanced => 88,
        Quality::Small => 72,
    };
    let result = match preset.id {
        "jpg" => {
            let encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, jpeg_quality);
            image.to_rgb8().write_with_encoder(encoder)
        }
        "avif" => {
            let encoder = image::codecs::avif::AvifEncoder::new_with_speed_quality(
                &mut writer,
                8,
                jpeg_quality,
            );
            image.write_with_encoder(encoder)
        }
        "ico" => image
            .resize(256, 256, image::imageops::FilterType::Lanczos3)
            .write_to(&mut writer, ImageFormat::Ico),
        "bmp" => image.write_to(&mut writer, ImageFormat::Bmp),
        "png" => image.write_to(&mut writer, ImageFormat::Png),
        "webp" => image.write_to(&mut writer, ImageFormat::WebP),
        "tiff" => image.write_to(&mut writer, ImageFormat::Tiff),
        "gif-image" => image.write_to(&mut writer, ImageFormat::Gif),
        "tga" => image.write_to(&mut writer, ImageFormat::Tga),
        "qoi" => image.write_to(&mut writer, ImageFormat::Qoi),
        other => return Err(format!("неизвестный формат {other}")),
    };
    result.map_err(|error| error.to_string())?;
    use std::io::Write;
    writer.flush().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn converts_test_media() {
        let Ok(folder) = std::env::var("ANMEDIA_TEST_MEDIA") else {
            return;
        };
        let folder = std::path::Path::new(&folder);
        let out_dir = std::env::temp_dir().join("anmedia-convert-test");
        let _ = std::fs::create_dir_all(&out_dir);
        let cases = [
            ("hevc.mkv", "mp4"),
            ("h264.mp4", "webm"),
            ("video.flv", "gif"),
            ("wmv2.wmv", "mp3"),
            ("tone.opus", "flac"),
            ("tone.wv", "m4a"),
            ("photo.jxl", "jpg"),
            ("photo.avif", "png"),
            ("photo.png", "avif"),
            ("anim.gif", "ico"),
        ];
        for (file, id) in cases {
            let preset = super::preset(id).unwrap();
            let output = out_dir.join(format!("{file}.{}", preset.extension));
            let _ = std::fs::remove_file(&output);
            let started = std::time::Instant::now();
            let job = super::start(
                &folder.join(file),
                &output,
                preset,
                super::Quality::Balanced,
            );
            let result = loop {
                if let Some(result) = job.try_result() {
                    break result;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            };
            let status = match result {
                super::JobResult::Done(path) => {
                    format!("ok {} байт", std::fs::metadata(path).unwrap().len())
                }
                super::JobResult::Cancelled => "отменено".to_string(),
                super::JobResult::Failed(error) => format!("ОШИБКА {error}"),
            };
            println!("{file} -> {id}: {status} за {:?}", started.elapsed());
        }
    }

    fn peak_memory_mb() -> u64 {
        use windows_sys::Win32::System::ProcessStatus::{
            K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
        counters.PeakWorkingSetSize as u64 / 1024 / 1024
    }

    #[test]
    fn conversion_memory() {
        let Ok(path) = std::env::var("ANMEDIA_BIG_VIDEO") else {
            return;
        };
        let id = std::env::var("ANMEDIA_PRESET").unwrap_or_else(|_| "mp4".to_string());
        let preset = super::preset(&id).unwrap();
        let output = std::env::temp_dir().join(format!("anmedia-mem.{}", preset.extension));
        let _ = std::fs::remove_file(&output);
        let started = std::time::Instant::now();
        let job = super::start(
            std::path::Path::new(&path),
            &output,
            preset,
            super::Quality::Balanced,
        );
        let result = loop {
            if let Some(result) = job.try_result() {
                break result;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        };
        let ok = matches!(result, super::JobResult::Done(_));
        println!(
            "{id}: ok={ok} пик памяти {} МБ за {:?}",
            peak_memory_mb(),
            started.elapsed()
        );
    }
}
