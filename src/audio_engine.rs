use lofty::file::AudioFile;
use rodio::stream::{DeviceSinkBuilder, MixerDeviceSink};
use rodio::{Decoder, Player, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

pub struct AudioEngine {
    stream: MixerDeviceSink,
    player: Player,
    duration_millis: Arc<AtomicU64>,
    volume: f64,
    muted: bool,
    speed: f64,
    path: Option<PathBuf>,
    position_offset: f64,
}

impl AudioEngine {
    pub fn create() -> Result<Self, String> {
        let mut stream = DeviceSinkBuilder::open_default_sink()
            .map_err(|error| format!("Не удалось открыть устройство вывода звука: {error}"))?;
        stream.log_on_drop(false);
        let player = Player::connect_new(stream.mixer());
        player.pause();
        Ok(Self {
            stream,
            player,
            duration_millis: Arc::new(AtomicU64::new(0)),
            volume: 1.0,
            muted: false,
            speed: 1.0,
            path: None,
            position_offset: 0.0,
        })
    }

    pub fn load(&mut self, path: &Path) -> Result<(), String> {
        if !path.is_file() {
            return Err(format!("Аудиофайл не найден: {}", path.display()));
        }

        let decoder = open_decoder(path)?;
        let decoder_duration = decoder
            .total_duration()
            .map(|value| value.as_secs_f64())
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(0.0);
        let metadata = read_metadata_duration(path);
        let initial_duration = decoder_duration.max(metadata.exact).max(metadata.estimated);
        let duration_millis = Arc::new(AtomicU64::new(seconds_to_millis(initial_duration)));

        if decoder_duration <= 0.0 && metadata.exact <= 0.0 && metadata.estimated <= 0.0 {
            let path = path.to_path_buf();
            let target = Arc::clone(&duration_millis);
            thread::spawn(move || {
                if let Some(duration) = scan_decoded_duration(&path) {
                    target.store(seconds_to_millis(duration), Ordering::Release);
                }
            });
        }

        let player = Player::connect_new(self.stream.mixer());
        player.pause();
        player.set_volume(if self.muted { 0.0 } else { self.volume as f32 });
        player.set_speed(self.speed as f32);
        player.append(decoder);
        self.player.stop();
        self.player = player;
        self.duration_millis = duration_millis;
        self.path = Some(path.to_path_buf());
        self.position_offset = 0.0;
        Ok(())
    }

    pub fn play(&self) {
        self.player.play();
    }

    pub fn pause(&self) {
        self.player.pause();
    }

    pub fn stop(&mut self) {
        self.player.stop();
        self.position_offset = 0.0;
    }

    pub fn seek(&mut self, seconds: f64) -> Result<(), String> {
        if !seconds.is_finite() {
            return Err("Некорректная позиция перемотки".to_string());
        }
        let duration = self.duration();
        let target = if duration > 0.0 {
            seconds.clamp(0.0, duration)
        } else {
            seconds.max(0.0)
        };
        self.rebuild_at(target)
    }

    fn rebuild_at(&mut self, seconds: f64) -> Result<(), String> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| "Путь к аудиофайлу не сохранён".to_string())?;
        let mut decoder = open_decoder(&path)?;
        let was_paused = self.player.is_paused();
        let target = Duration::from_secs_f64(seconds);
        let player = Player::connect_new(self.stream.mixer());
        player.pause();
        player.set_volume(if self.muted { 0.0 } else { self.volume as f32 });
        player.set_speed(self.speed as f32);
        // Быстрая перемотка по индексу контейнера; если формат её не умеет —
        // пропуск декодированных сэмплов.
        if seconds <= 0.0 || decoder.try_seek(target).is_ok() {
            player.append(decoder);
        } else {
            player.append(open_decoder(&path)?.skip_duration(target));
        }
        self.player.stop();
        self.player = player;
        self.position_offset = seconds;
        if !was_paused {
            self.player.play();
        }
        Ok(())
    }

    pub fn set_volume(&mut self, volume: f64) {
        self.volume = (volume / 100.0).clamp(0.0, 1.0);
        self.player
            .set_volume(if self.muted { 0.0 } else { self.volume as f32 });
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.player
            .set_volume(if self.muted { 0.0 } else { self.volume as f32 });
    }

    pub fn set_speed(&mut self, speed: f64) {
        self.speed = speed.clamp(0.25, 4.0);
        self.player.set_speed(self.speed as f32);
    }

    pub fn current_time(&self) -> f64 {
        let value = self.position_offset + self.player.get_pos().as_secs_f64();
        let duration = self.duration();
        if duration > 0.0 {
            value.clamp(0.0, duration)
        } else {
            value.max(0.0)
        }
    }

    pub fn duration(&self) -> f64 {
        self.duration_millis.load(Ordering::Acquire) as f64 / 1000.0
    }

    pub fn is_paused(&self) -> bool {
        self.player.is_paused()
    }

    pub fn is_finished(&self) -> bool {
        self.player.empty()
    }
}

struct MetadataDuration {
    exact: f64,
    estimated: f64,
}

fn read_metadata_duration(path: &Path) -> MetadataDuration {
    let Ok(tagged) = lofty::read_from_path(path) else {
        return MetadataDuration {
            exact: 0.0,
            estimated: 0.0,
        };
    };
    let properties = tagged.properties();
    let exact = properties.duration().as_secs_f64();
    let exact = if exact.is_finite() && exact > 0.0 {
        exact
    } else {
        0.0
    };
    let bitrate = properties
        .overall_bitrate()
        .or_else(|| properties.audio_bitrate())
        .filter(|value| *value > 0);
    let estimated = bitrate
        .and_then(|value| {
            let bytes = std::fs::metadata(path).ok()?.len() as f64;
            let seconds = bytes * 8.0 / (value as f64 * 1000.0);
            (seconds.is_finite() && seconds > 0.0).then_some(seconds)
        })
        .unwrap_or(0.0);
    MetadataDuration { exact, estimated }
}

fn scan_decoded_duration(path: &Path) -> Option<f64> {
    let mut decoder = open_decoder(path).ok()?;
    let channels = decoder.channels().get() as f64;
    let sample_rate = decoder.sample_rate().get() as f64;
    if channels <= 0.0 || sample_rate <= 0.0 {
        return None;
    }
    let mut samples = 0u64;
    for _ in decoder.by_ref() {
        samples = samples.saturating_add(1);
    }
    let duration = samples as f64 / (channels * sample_rate);
    (duration.is_finite() && duration > 0.0).then_some(duration)
}

fn seconds_to_millis(seconds: f64) -> u64 {
    if seconds.is_finite() && seconds > 0.0 {
        (seconds * 1000.0).round().clamp(1.0, u64::MAX as f64) as u64
    } else {
        0
    }
}

fn open_decoder(path: &Path) -> Result<Decoder<BufReader<File>>, String> {
    let extension = normalized_hint(path);
    match decoder_with_builder(path, &extension) {
        Ok(decoder) => Ok(decoder),
        Err(primary_error) => {
            let file = File::open(path).map_err(|error| {
                format!("Не удалось открыть аудиофайл {}: {error}", path.display())
            })?;
            Decoder::try_from(file).map_err(|fallback_error| {
                format!(
                    "Не удалось декодировать аудиофайл {}. Основной декодер: {primary_error}. Резервный декодер: {fallback_error}",
                    path.display()
                )
            })
        }
    }
}

fn decoder_with_builder(path: &Path, hint: &str) -> Result<Decoder<BufReader<File>>, String> {
    let file = File::open(path)
        .map_err(|error| format!("Не удалось открыть аудиофайл {}: {error}", path.display()))?;
    let byte_len = file
        .metadata()
        .map_err(|error| {
            format!(
                "Не удалось прочитать размер аудиофайла {}: {error}",
                path.display()
            )
        })?
        .len();
    let reader = BufReader::new(file);

    let mut builder = Decoder::builder()
        .with_data(reader)
        .with_byte_len(byte_len)
        .with_seekable(true)
        .with_gapless(true);
    if !hint.is_empty() {
        builder = builder.with_hint(hint);
    }
    if let Some(mime_type) = mime_type_for_hint(hint) {
        builder = builder.with_mime_type(mime_type);
    }
    builder.build().map_err(|error| error.to_string())
}

fn normalized_hint(path: &Path) -> String {
    // Расширение может не совпадать с содержимым — доверяем сигнатуре.
    let extension = crate::formats::sniff_file(path)
        .map(|(_, extension)| extension.to_string())
        .or_else(|| {
            path.extension()
                .and_then(|value| value.to_str())
                .map(|value| value.to_ascii_lowercase())
        })
        .unwrap_or_default();
    match extension.as_str() {
        "oga" | "ogg" | "opus" | "spx" => "ogg",
        "m4a" | "m4b" | "m4r" | "mp4" | "alac" | "3ga" => "mp4",
        "aif" | "aifc" | "aiff" => "aiff",
        "mka" | "mkv" => "mkv",
        "weba" | "webm" => "webm",
        "wave" | "wav" | "rf64" | "w64" => "wav",
        "mp1" | "mpa" => "mp1",
        "mp2" => "mp2",
        "mp3" => "mp3",
        "flac" => "flac",
        "aac" | "adts" => "aac",
        "caf" => "caf",
        _ => "",
    }
    .to_string()
}

fn mime_type_for_hint(hint: &str) -> Option<&'static str> {
    match hint {
        "ogg" => Some("audio/ogg"),
        "mp1" | "mp2" | "mp3" => Some("audio/mpeg"),
        "mp4" => Some("audio/mp4"),
        "wav" => Some("audio/wav"),
        "flac" => Some("audio/flac"),
        "aac" => Some("audio/aac"),
        "aiff" => Some("audio/aiff"),
        "caf" => Some("audio/x-caf"),
        "mkv" => Some("audio/x-matroska"),
        "webm" => Some("audio/webm"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn plays_test_media_folder() {
        let Ok(folder) = std::env::var("ANMEDIA_TEST_MEDIA") else {
            return;
        };
        let mut entries = std::fs::read_dir(folder)
            .expect("папка с тестовыми файлами")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| crate::formats::detect_kind(path) == crate::formats::MediaKind::Audio)
            .collect::<Vec<_>>();
        entries.sort();
        let mut engine = super::AudioEngine::create().expect("звуковое устройство");
        for path in entries {
            let symphonia = engine
                .load(&path)
                .map(|()| format!("{:.1} с", engine.duration()))
                .unwrap_or_else(|error| {
                    format!("нет ({})", error.chars().take(60).collect::<String>())
                });
            let mpv = crate::mpv::probe(&path)
                .map(|(duration, audio, video)| {
                    format!("{duration:.1} с audio={audio} video={video}")
                })
                .unwrap_or_else(|error| format!("ОШИБКА {error}"));
            println!(
                "{:<16} symphonia: {:<22} mpv: {}",
                path.file_name().unwrap().to_string_lossy(),
                symphonia,
                mpv
            );
        }
    }
}
