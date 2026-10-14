use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoEngine {
    /// libmpv (FFmpeg), при ошибке — Media Foundation.
    Mpv,
    /// Media Foundation, при ошибке — libmpv.
    MediaFoundation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeMode,
    pub accent: usize,
    pub block_suspicious: bool,
    pub online_covers: bool,
    pub video_engine: VideoEngine,
    pub volume: f64,
    pub speed: f64,
    pub viewer_zoom: f32,
    pub brightness: f32,
    pub muted: bool,
    pub autoplay_next: bool,
    pub loop_file: bool,
    pub shuffle: bool,
    pub always_on_top: bool,
    pub show_playlist: bool,
    pub remember_playlist: bool,
    pub remember_position: bool,
    pub custom_extensions: String,
    pub association_prompt_done: bool,
    pub association_prompt_disabled: bool,
    pub last_directory: String,
    pub playlist: Vec<PathBuf>,
    pub playlist_index: Option<usize>,
    pub positions: BTreeMap<String, f64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeMode::System,
            accent: 0,
            block_suspicious: true,
            online_covers: true,
            video_engine: VideoEngine::Mpv,
            volume: 70.0,
            speed: 1.0,
            viewer_zoom: 1.0,
            brightness: 1.0,
            muted: false,
            autoplay_next: true,
            loop_file: false,
            shuffle: false,
            always_on_top: false,
            show_playlist: true,
            remember_playlist: true,
            remember_position: true,
            custom_extensions: String::new(),
            association_prompt_done: false,
            association_prompt_disabled: false,
            last_directory: String::new(),
            playlist: Vec::new(),
            playlist_index: None,
            positions: BTreeMap::new(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let Ok(data) = fs::read_to_string(settings_path()) else {
            return Self::default();
        };
        let mut settings: Self = serde_json::from_str(&data).unwrap_or_default();
        settings.normalize();
        settings
    }

    pub fn save(&self) -> Result<(), String> {
        let path = settings_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let data = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        fs::write(path, data).map_err(|error| error.to_string())
    }

    fn normalize(&mut self) {
        self.volume = finite_or(self.volume, 70.0).clamp(0.0, 100.0);
        self.accent = self.accent.min(crate::theme::ACCENTS.len() - 1);
        self.speed = finite_or(self.speed, 1.0).clamp(0.25, 4.0);
        // Масштаб больше не запоминается между файлами.
        self.viewer_zoom = 1.0;
        self.brightness = finite_or_f32(self.brightness, 1.0).clamp(0.25, 2.0);
        self.positions = std::mem::take(&mut self.positions)
            .into_iter()
            .filter(|(_, value)| value.is_finite() && *value >= 0.0)
            .map(|(path, value)| (normalize_position_key(Path::new(&path)), value))
            .collect();
        if self
            .playlist_index
            .is_some_and(|index| index >= self.playlist.len())
        {
            self.playlist_index = None;
        }
    }

    pub fn set_last_directory_from_path(&mut self, path: &Path) {
        let directory = if path.is_dir() {
            Some(path)
        } else {
            path.parent()
        };
        if let Some(directory) = directory {
            self.last_directory = directory.to_string_lossy().into_owned();
        }
    }
}

pub fn settings_path() -> PathBuf {
    ProjectDirs::from("", "Anton4k272", "AnMedia")
        .map(|directories| directories.config_dir().join("settings.json"))
        .unwrap_or_else(|| PathBuf::from("settings.json"))
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() { value } else { fallback }
}

fn finite_or_f32(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn normalize_position_key(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    absolute.to_string_lossy().replace('/', "\\").to_lowercase()
}
