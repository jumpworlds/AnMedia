use crate::associations::{
    open_default_apps_settings, rebuild_shell_preview_cache, register_file_associations,
};
use crate::audio_engine::AudioEngine;
use crate::clipboard::{copy_rgba, paste_image_to_cache};
use crate::config::{Settings, ThemeMode, VideoEngine};
use crate::convert::{self, Job, JobResult, Quality, TargetKind};
use crate::cover_art::{CoverResult, start_worker as start_cover_worker};
use crate::formats::{MediaKind, detect_kind, kind_by_extension};
use crate::image_decode::set_max_texture_side;
use crate::image_loader::{ImageResult, start_worker as start_image_worker};
use crate::media::{
    collect_media_from_directory, collect_media_from_parent_directory, normalize_input_paths,
};
use crate::mpv::{self, MpvEvent, MpvPlayer};
use crate::native_media::{NativeMediaEngine, NativeMediaEvent};
use crate::platform::{MediaHostWindow, parent_hwnd};
use crate::security::{self, Verdict};
use crate::single_instance::SingleInstance;
use crate::theme::{self, ACCENTS, Palette, icon, icon_font, semibold};
use crate::thumbnail::{ThumbnailResult, start_worker as start_thumbnail_worker};
use crate::ui_kit::{
    action_button, chip, format_size, format_time, icon_button, paint_brightness_overlay,
    paint_checkerboard, paint_equalizer, paint_glow, paint_kind_tile, paint_texture_cover,
    pill_button, play_button, seek_bar, segmented, toggle, tool_button, volume_slider,
};
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui::{
    self, Align, Align2, Color32, ColorImage, CornerRadius, FontId, Frame, Galley, Key, Layout,
    Margin, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions, UiBuilder, Vec2,
};
use rand::Rng;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const THUMBNAIL_CACHE_LIMIT: usize = 512;
const SPEEDS: [f64; 10] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];
const CONTROLS_HIDE_DELAY: Duration = Duration::from_millis(2500);

const TEXTURE_OPTIONS: TextureOptions = TextureOptions {
    magnification: egui::TextureFilter::Linear,
    minification: egui::TextureFilter::Linear,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: Some(egui::TextureFilter::Linear),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
    None,
    Image,
    /// Встроенный декодер Symphonia (rodio).
    Rodio,
    /// libmpv (FFmpeg).
    Mpv,
    /// Media Foundation.
    Windows,
}

impl Backend {
    fn label(self) -> &'static str {
        match self {
            Self::None => "—",
            Self::Image => "Декодер изображений",
            Self::Rodio => "Symphonia",
            Self::Mpv => "mpv",
            Self::Windows => "Windows",
        }
    }
}

#[derive(Default)]
struct ImageView {
    path: Option<PathBuf>,
    loading: bool,
    texture: Option<TextureHandle>,
    frames: Vec<TextureHandle>,
    delays: Vec<u32>,
    frame: usize,
    next_frame_at: Option<Instant>,
    animation_paused: bool,
    size: [usize; 2],
    original_size: [usize; 2],
    format: String,
    file_size: u64,
    has_alpha: bool,
    rgba: Vec<u8>,
    zoom: f32,
    pan: Vec2,
    rotation: u8,
}

impl ImageView {
    fn loading(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            loading: true,
            zoom: 1.0,
            ..Self::default()
        }
    }

    fn current_texture(&self) -> Option<&TextureHandle> {
        self.frames.get(self.frame).or(self.texture.as_ref())
    }

    fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
    }
}

pub struct AnMediaApp {
    logo: TextureHandle,
    media_zoom: f32,
    media_pan: Vec2,
    blocked: bool,
    show_convert: bool,
    convert_preset: &'static str,
    convert_quality: Quality,
    convert_output: Option<PathBuf>,
    convert_source: Option<PathBuf>,
    convert_job: Option<Job>,
    convert_result: Option<Result<PathBuf, String>>,
    settings: Settings,
    media_host: Option<MediaHostWindow>,
    host_error: Option<String>,
    native_media: Option<NativeMediaEngine>,
    mpv: Option<MpvPlayer>,
    audio_engine: Option<AudioEngine>,
    backend: Backend,
    backend_queue: VecDeque<Backend>,
    load_errors: Vec<String>,
    load_failed: Option<String>,
    end_handled: bool,
    windows_video_checked: bool,
    playlist: Vec<PathBuf>,
    current_index: Option<usize>,
    current_kind: MediaKind,
    current_time: f64,
    duration: f64,
    paused: bool,
    has_video: bool,
    has_audio: bool,
    media_loaded: bool,
    current_title: String,
    status: String,
    toast: Option<(String, Instant)>,
    shown_status: String,
    show_settings: bool,
    show_association_prompt: bool,
    fullscreen: bool,
    applied_always_on_top: Option<bool>,
    applied_theme: Option<(bool, usize)>,
    video_rect: Option<Rect>,
    playlist_filter: String,
    playlist_kind_filter: Option<MediaKind>,
    scan_rx: Option<Receiver<Vec<PathBuf>>>,
    scanning_directory: bool,
    dirty: bool,
    last_save: Instant,
    last_theme_check: Instant,
    last_position_update: Instant,
    last_pointer_activity: Instant,
    system_dark: bool,
    pending_initial_load: bool,
    pending_seek: Option<f64>,
    pending_autoplay: bool,
    texture_side_known: bool,
    instance_rx: Receiver<Vec<PathBuf>>,
    thumbnail_tx: Sender<PathBuf>,
    thumbnail_rx: Receiver<ThumbnailResult>,
    thumbnail_cache: HashMap<PathBuf, Option<TextureHandle>>,
    thumbnail_pending: HashSet<PathBuf>,
    thumbnail_order: VecDeque<PathBuf>,
    image_tx: Sender<PathBuf>,
    image_rx: Receiver<ImageResult>,
    image: ImageView,
    cover_tx: Sender<PathBuf>,
    cover_rx: Receiver<CoverResult>,
    cover_texture: Option<TextureHandle>,
    cover_backdrop: Option<TextureHandle>,
    cover_path: Option<PathBuf>,
    cover_title: String,
    cover_artist: Option<String>,
    cover_source: Option<String>,
    cover_loading: bool,
    last_video_surface_size: Option<(i32, i32)>,
    applied_title: Option<String>,
}

impl AnMediaApp {
    pub fn new(cc: &eframe::CreationContext<'_>, single_instance: SingleInstance) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        cc.egui_ctx.options_mut(|options| {
            options.input_options.zoom_modifier = egui::Modifiers::NONE;
        });
        let mut settings = Settings::load();
        let arguments = single_instance.initial_paths().to_vec();
        let instance_rx = single_instance.receiver();
        let (thumbnail_tx, thumbnail_rx) = start_thumbnail_worker();
        let (image_tx, image_rx) = start_image_worker();
        let (cover_tx, cover_rx) = start_cover_worker();
        let association_error = std::env::current_exe().ok().and_then(|executable| {
            register_file_associations(&executable, &settings.custom_extensions).err()
        });

        let remembered_path = settings
            .playlist_index
            .and_then(|index| settings.playlist.get(index))
            .cloned();
        let mut playlist = if settings.remember_playlist {
            normalize_input_paths(settings.playlist.clone(), &settings.custom_extensions)
        } else {
            Vec::new()
        };
        let mut current_index = remembered_path
            .as_ref()
            .and_then(|remembered| {
                let key = path_identity(remembered);
                playlist
                    .iter()
                    .position(|candidate| path_identity(candidate) == key)
            })
            .or_else(|| {
                settings
                    .playlist_index
                    .filter(|index| *index < playlist.len())
            });

        if !arguments.is_empty() {
            let files = normalize_input_paths(arguments, &settings.custom_extensions);
            if !files.is_empty() {
                playlist = files;
                current_index = Some(0);
            }
        }

        settings.playlist = playlist.clone();
        settings.playlist_index = current_index;
        let system_dark = detect_system_dark();
        let show_association_prompt =
            !settings.association_prompt_done && !settings.association_prompt_disabled;

        let logo = load_logo(&cc.egui_ctx);
        Self {
            logo,
            media_zoom: 1.0,
            media_pan: Vec2::ZERO,
            blocked: false,
            show_convert: false,
            convert_preset: "mp4",
            convert_quality: Quality::Balanced,
            convert_output: None,
            convert_source: None,
            convert_job: None,
            convert_result: None,
            settings,
            media_host: None,
            host_error: None,
            native_media: None,
            mpv: None,
            audio_engine: None,
            backend: Backend::None,
            backend_queue: VecDeque::new(),
            load_errors: Vec::new(),
            load_failed: None,
            end_handled: false,
            windows_video_checked: false,
            playlist,
            current_index,
            current_kind: MediaKind::Unknown,
            current_time: 0.0,
            duration: 0.0,
            paused: true,
            has_video: false,
            has_audio: false,
            media_loaded: false,
            current_title: String::new(),
            status: association_error
                .map(|error| format!("Не удалось восстановить ассоциации: {error}"))
                .unwrap_or_else(|| "Готово".to_string()),
            toast: None,
            shown_status: "Готово".to_string(),
            show_settings: false,
            show_association_prompt,
            fullscreen: false,
            applied_always_on_top: None,
            applied_theme: None,
            video_rect: None,
            playlist_filter: String::new(),
            playlist_kind_filter: None,
            scan_rx: None,
            scanning_directory: false,
            dirty: false,
            last_save: Instant::now(),
            last_theme_check: Instant::now(),
            last_position_update: Instant::now(),
            last_pointer_activity: Instant::now(),
            system_dark,
            pending_initial_load: current_index.is_some(),
            pending_seek: None,
            pending_autoplay: false,
            texture_side_known: false,
            instance_rx,
            thumbnail_tx,
            thumbnail_rx,
            thumbnail_cache: HashMap::new(),
            thumbnail_pending: HashSet::new(),
            thumbnail_order: VecDeque::new(),
            image_tx,
            image_rx,
            image: ImageView::default(),
            cover_tx,
            cover_rx,
            cover_texture: None,
            cover_backdrop: None,
            cover_path: None,
            cover_title: String::new(),
            cover_artist: None,
            cover_source: None,
            cover_loading: false,
            last_video_surface_size: None,
            applied_title: None,
        }
    }

    // ───────────────────────────── состояние ─────────────────────────────

    fn dark_mode(&self) -> bool {
        match self.settings.theme {
            ThemeMode::System => self.system_dark,
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
        }
    }

    fn palette(&self) -> Palette {
        theme::palette(self.dark_mode(), self.settings.accent)
    }

    fn sync_theme(&mut self, ctx: &egui::Context) {
        let key = (self.dark_mode(), self.settings.accent);
        if self.applied_theme != Some(key) {
            theme::apply(ctx, &self.palette());
            self.applied_theme = Some(key);
        }
    }

    fn current_path(&self) -> Option<&PathBuf> {
        self.current_index
            .and_then(|index| self.playlist.get(index))
    }

    fn current_is_image(&self) -> bool {
        self.current_index.is_some() && self.current_kind == MediaKind::Image
    }

    fn ensure_host(&mut self, frame: &eframe::Frame) {
        if self.media_host.is_some() || self.host_error.is_some() {
            return;
        }
        let Some(parent) = parent_hwnd(frame) else {
            return;
        };
        match MediaHostWindow::create(parent) {
            Ok(mut host) => {
                host.set_visible(false);
                host.set_zoom(1.0);
                host.set_brightness(self.settings.brightness);
                self.media_host = Some(host);
            }
            Err(error) => self.host_error = Some(error),
        }
    }

    fn host_hwnd(&self) -> Result<u64, String> {
        self.media_host
            .as_ref()
            .map(MediaHostWindow::hwnd_value)
            .ok_or_else(|| {
                self.host_error
                    .clone()
                    .unwrap_or_else(|| "окно вывода видео ещё не создано".to_string())
            })
    }

    fn ensure_mpv(&mut self) -> Result<(), String> {
        if self.mpv.is_none() {
            let hwnd = self.host_hwnd()?;
            self.mpv = Some(MpvPlayer::create(hwnd)?);
        }
        Ok(())
    }

    fn ensure_native(&mut self) -> Result<(), String> {
        if self.native_media.is_none() {
            let hwnd = self.host_hwnd()?;
            self.native_media = Some(NativeMediaEngine::create(hwnd)?);
        }
        Ok(())
    }

    // ───────────────────────────── загрузка ─────────────────────────────

    fn backend_chain(&self, kind: MediaKind) -> VecDeque<Backend> {
        let mpv_ready = mpv::is_available();
        let chain: Vec<Backend> = match kind {
            MediaKind::Image => vec![Backend::Image],
            MediaKind::Audio => vec![Backend::Rodio, Backend::Mpv, Backend::Windows],
            MediaKind::Video | MediaKind::Unknown => match self.settings.video_engine {
                VideoEngine::Mpv => vec![Backend::Mpv, Backend::Windows],
                VideoEngine::MediaFoundation => vec![Backend::Windows, Backend::Mpv],
            },
        };
        chain
            .into_iter()
            .filter(|backend| *backend != Backend::Mpv || mpv_ready)
            .collect()
    }

    fn reset_media_state(&mut self) {
        self.media_zoom = 1.0;
        self.media_pan = Vec2::ZERO;
        self.blocked = false;
        self.current_time = 0.0;
        self.duration = 0.0;
        self.paused = true;
        self.has_video = false;
        self.has_audio = false;
        self.media_loaded = false;
        self.end_handled = false;
        self.windows_video_checked = false;
        self.last_video_surface_size = None;
        self.load_errors.clear();
        self.load_failed = None;
        self.backend_queue.clear();
        self.image = ImageView::default();
        self.cover_texture = None;
        self.cover_backdrop = None;
        self.cover_path = None;
        self.cover_title.clear();
        self.cover_artist = None;
        self.cover_source = None;
        self.cover_loading = false;
    }

    fn load_current(&mut self) {
        let Some(path) = self.current_path().cloned() else {
            self.clear_current();
            return;
        };
        self.reset_media_state();
        self.current_title = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Файл")
            .to_string();
        self.current_kind = detect_kind(&path);
        if self.settings.block_suspicious {
            match security::inspect(&path) {
                Verdict::Blocked(reason) => {
                    self.halt_except(Backend::None);
                    self.backend = Backend::None;
                    self.blocked = true;
                    self.pending_autoplay = false;
                    self.pending_seek = None;
                    self.load_failed = Some(format!(
                        "Причина: {reason}.\n\nНастоящие фото, видео и музыка так не выглядят. \
                         Не запускайте этот файл и удалите его, если не знаете, откуда он."
                    ));
                    self.status =
                        format!("Заблокирован подозрительный файл: {}", self.current_title);
                    return;
                }
                Verdict::FromInternet => {
                    self.status =
                        "Файл скачан из интернета — открыт в защищённом режиме".to_string();
                }
                Verdict::Safe => {}
            }
        }
        self.pending_seek = if self.current_kind == MediaKind::Image {
            None
        } else {
            self.settings
                .positions
                .get(&position_key(&path))
                .copied()
                .filter(|value| *value > 1.0)
        };
        self.pending_autoplay = self.current_kind != MediaKind::Image;

        if self.current_kind == MediaKind::Audio {
            self.cover_path = Some(path.clone());
            self.cover_loading = true;
            self.cover_title = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or(self.current_title.as_str())
                .replace('_', " ");
            if self.cover_tx.send(path.clone()).is_err() {
                self.cover_loading = false;
            }
        }

        self.backend_queue = self.backend_chain(self.current_kind);
        self.start_next_backend();
    }

    fn start_next_backend(&mut self) {
        let Some(path) = self.current_path().cloned() else {
            return;
        };
        while let Some(backend) = self.backend_queue.pop_front() {
            self.halt_except(backend);
            self.media_loaded = false;
            self.has_video = false;
            self.has_audio = false;
            match self.start_backend(backend, &path) {
                Ok(()) => {
                    self.backend = backend;
                    return;
                }
                Err(error) => self.push_load_error(backend, &error),
            }
        }
        self.backend = Backend::None;
        self.fail_load();
    }

    fn push_load_error(&mut self, backend: Backend, error: &str) {
        let error = error.trim();
        let line = if error.starts_with("mpv") {
            error.to_string()
        } else {
            format!("{}: {error}", backend.label())
        };
        if !self.load_errors.contains(&line) {
            self.load_errors.push(line);
        }
    }

    fn fail_load(&mut self) {
        let mut message = self.load_errors.join("\n");
        if message.is_empty() {
            message = "Нет подходящего декодера".to_string();
        }
        if !mpv::is_available() && self.current_kind != MediaKind::Image {
            message.push_str(
                "\n\nПоложите libmpv-2.dll рядом с AnMedia.exe — тогда откроются любые форматы.",
            );
        }
        self.load_failed = Some(message);
        self.media_loaded = false;
        self.pending_autoplay = false;
        self.pending_seek = None;
        self.status = format!("Не удалось открыть {}", self.current_title);
    }

    fn halt_except(&mut self, keep: Backend) {
        if keep != Backend::Rodio
            && let Some(engine) = self.audio_engine.as_mut()
        {
            engine.stop();
        }
        if keep != Backend::Mpv
            && let Some(player) = self.mpv.as_mut()
            && player.is_loaded()
        {
            player.stop();
        }
        if keep != Backend::Windows
            && self.backend == Backend::Windows
            && let Some(engine) = &self.native_media
        {
            let _ = engine.pause();
        }
    }

    fn start_backend(&mut self, backend: Backend, path: &Path) -> Result<(), String> {
        match backend {
            Backend::None => Err("нет движка".to_string()),
            Backend::Image => {
                self.image = ImageView::loading(path.to_path_buf());
                self.status = format!("Открытие: {}", self.current_title);
                self.image_tx
                    .send(path.to_path_buf())
                    .map_err(|_| "обработчик изображений недоступен".to_string())
            }
            Backend::Rodio => {
                if self.audio_engine.is_none() {
                    self.audio_engine = Some(AudioEngine::create()?);
                }
                let Some(engine) = self.audio_engine.as_mut() else {
                    return Err("аудиодвижок недоступен".to_string());
                };
                engine.load(path)?;
                engine.set_volume(self.settings.volume);
                engine.set_muted(self.settings.muted);
                engine.set_speed(self.settings.speed);
                self.duration = engine.duration();
                if let Some(position) = self.pending_seek.take()
                    && (position + 5.0 < self.duration || self.duration <= 0.0)
                {
                    let _ = engine.seek(position);
                }
                if self.pending_autoplay {
                    self.pending_autoplay = false;
                    engine.play();
                }
                self.current_time = engine.current_time();
                self.paused = engine.is_paused();
                self.has_audio = true;
                self.media_loaded = true;
                self.status = format!("Открыто: {}", self.current_title);
                Ok(())
            }
            Backend::Mpv => {
                self.ensure_mpv()?;
                let start = self.pending_seek.take();
                let paused = !std::mem::take(&mut self.pending_autoplay);
                let Some(player) = self.mpv.as_mut() else {
                    return Err("mpv недоступен".to_string());
                };
                let _ = player.set_volume(self.settings.volume);
                let _ = player.set_muted(self.settings.muted);
                let _ = player.set_speed(self.settings.speed);
                let _ = player.set_loop(self.settings.loop_file);
                player.load(path, start, paused)?;
                self.paused = paused;
                self.status = format!("Загрузка: {}", self.current_title);
                Ok(())
            }
            Backend::Windows => {
                self.ensure_native()?;
                let Some(engine) = &self.native_media else {
                    return Err("Media Foundation недоступна".to_string());
                };
                engine.load(path)?;
                let _ = engine.set_volume(self.settings.volume);
                let _ = engine.set_muted(self.settings.muted);
                let _ = engine.set_speed(self.settings.speed);
                let _ = engine.set_loop(self.settings.loop_file);
                self.windows_video_checked = false;
                self.pending_autoplay = true;
                self.status = format!("Загрузка: {}", self.current_title);
                Ok(())
            }
        }
    }

    fn clear_current(&mut self) {
        self.halt_except(Backend::None);
        if let Some(engine) = &self.native_media {
            let _ = engine.stop();
        }
        self.reset_media_state();
        self.backend = Backend::None;
        self.current_index = None;
        self.current_kind = MediaKind::Unknown;
        self.current_title.clear();
        self.pending_seek = None;
        self.pending_autoplay = false;
        self.status = "Готово".to_string();
        self.mark_dirty();
    }

    fn select_index(&mut self, index: usize) {
        if index >= self.playlist.len() {
            return;
        }
        self.store_current_position();
        self.current_index = Some(index);
        self.load_current();
        self.mark_dirty();
    }

    fn previous_track(&mut self) {
        if self.playlist.is_empty() {
            return;
        }
        if !self.current_is_image() && self.current_time > 5.0 {
            self.seek_current(0.0);
            return;
        }
        let index = match self.current_index {
            Some(0) | None => self.playlist.len() - 1,
            Some(index) => index - 1,
        };
        self.select_index(index);
    }

    fn next_track(&mut self) {
        if self.playlist.is_empty() {
            return;
        }
        let index = if self.settings.shuffle && self.playlist.len() > 1 {
            let current = self.current_index.unwrap_or(0);
            let mut rng = rand::rng();
            let mut next = rng.random_range(0..self.playlist.len());
            if next == current {
                next = (next + 1) % self.playlist.len();
            }
            next
        } else {
            self.current_index
                .map(|index| (index + 1) % self.playlist.len())
                .unwrap_or(0)
        };
        self.select_index(index);
    }

    // ─────────────────────────── управление ───────────────────────────

    fn toggle_playback(&mut self) {
        match self.backend {
            Backend::Image => {
                if self.image.frames.len() > 1 {
                    self.image.animation_paused = !self.image.animation_paused;
                } else {
                    self.next_track();
                }
            }
            Backend::Rodio => {
                if let Some(engine) = &self.audio_engine {
                    if engine.is_finished() {
                        self.load_current();
                        return;
                    }
                    if self.paused {
                        engine.play();
                    } else {
                        engine.pause();
                    }
                    self.paused = engine.is_paused();
                }
            }
            Backend::Mpv => {
                if let Some(player) = &self.mpv {
                    let result = if self.paused {
                        self.end_handled = false;
                        player.play()
                    } else {
                        player.pause()
                    };
                    match result {
                        Ok(()) => self.paused = !self.paused,
                        Err(error) => self.status = error,
                    }
                }
            }
            Backend::Windows => {
                if let Some(engine) = &self.native_media {
                    let result = if self.paused {
                        engine.play()
                    } else {
                        engine.pause()
                    };
                    if let Err(error) = result {
                        self.status = error;
                    }
                }
            }
            Backend::None => {
                if self.load_failed.is_some() {
                    self.next_track();
                }
            }
        }
    }

    fn stop_playback(&mut self) {
        match self.backend {
            Backend::Rodio => {
                if let Some(engine) = self.audio_engine.as_mut() {
                    engine.pause();
                    let _ = engine.seek(0.0);
                }
            }
            Backend::Mpv => {
                if let Some(player) = &self.mpv
                    && let Err(error) = player.rewind()
                {
                    self.status = error;
                }
            }
            Backend::Windows => {
                if let Some(engine) = &self.native_media
                    && let Err(error) = engine.stop()
                {
                    self.status = error;
                }
            }
            Backend::Image | Backend::None => return,
        }
        self.current_time = 0.0;
        self.paused = true;
        self.end_handled = false;
    }

    fn seek_current(&mut self, seconds: f64) {
        let seconds = seconds.max(0.0);
        let result = match self.backend {
            Backend::Rodio => self
                .audio_engine
                .as_mut()
                .map(|engine| engine.seek(seconds))
                .unwrap_or(Ok(())),
            Backend::Mpv => self
                .mpv
                .as_ref()
                .map(|player| player.seek(seconds))
                .unwrap_or(Ok(())),
            Backend::Windows => self
                .native_media
                .as_ref()
                .map(|engine| engine.seek(seconds))
                .unwrap_or(Ok(())),
            Backend::Image | Backend::None => return,
        };
        match result {
            Ok(()) => {
                self.current_time = seconds;
                self.end_handled = false;
            }
            Err(error) => self.status = error,
        }
    }

    fn seek_relative(&mut self, delta: f64) {
        if !self.media_loaded || self.current_is_image() {
            return;
        }
        let mut target = (self.current_time + delta).max(0.0);
        if self.duration > 0.0 {
            target = target.min(self.duration);
        }
        self.seek_current(target);
    }

    fn apply_volume(&mut self) {
        if let Some(engine) = self.audio_engine.as_mut() {
            engine.set_volume(self.settings.volume);
        }
        if let Some(player) = &self.mpv {
            let _ = player.set_volume(self.settings.volume);
        }
        if let Some(engine) = &self.native_media {
            let _ = engine.set_volume(self.settings.volume);
        }
    }

    fn apply_muted(&mut self) {
        if let Some(engine) = self.audio_engine.as_mut() {
            engine.set_muted(self.settings.muted);
        }
        if let Some(player) = &self.mpv {
            let _ = player.set_muted(self.settings.muted);
        }
        if let Some(engine) = &self.native_media {
            let _ = engine.set_muted(self.settings.muted);
        }
    }

    fn apply_speed(&mut self) {
        if let Some(engine) = self.audio_engine.as_mut() {
            engine.set_speed(self.settings.speed);
        }
        if let Some(player) = &self.mpv {
            let _ = player.set_speed(self.settings.speed);
        }
        if let Some(engine) = &self.native_media {
            let _ = engine.set_speed(self.settings.speed);
        }
        self.status = format!("Скорость: {}", format_speed(self.settings.speed));
    }

    fn apply_loop(&mut self) {
        if let Some(player) = &self.mpv {
            let _ = player.set_loop(self.settings.loop_file);
        }
        if let Some(engine) = &self.native_media {
            let _ = engine.set_loop(self.settings.loop_file);
        }
    }

    fn change_volume(&mut self, delta: f64) {
        self.settings.volume = (self.settings.volume + delta).clamp(0.0, 100.0);
        if self.settings.muted && delta > 0.0 {
            self.settings.muted = false;
            self.apply_muted();
        }
        self.apply_volume();
        self.status = format!("Громкость: {:.0}%", self.settings.volume);
        self.mark_dirty();
    }

    fn step_speed(&mut self, faster: bool) {
        let current = SPEEDS
            .iter()
            .position(|value| (*value - self.settings.speed).abs() < 0.01)
            .unwrap_or(3);
        let next = if faster {
            (current + 1).min(SPEEDS.len() - 1)
        } else {
            current.saturating_sub(1)
        };
        self.settings.speed = SPEEDS[next];
        self.apply_speed();
        self.mark_dirty();
    }

    fn on_media_ended(&mut self) {
        self.clear_current_position();
        if self.settings.loop_file {
            if self.backend == Backend::Rodio {
                self.load_current();
            }
            return;
        }
        if self.settings.autoplay_next && self.playlist.len() > 1 {
            self.next_track();
        } else {
            self.paused = true;
            self.current_time = self.duration;
        }
    }

    // ─────────────────────────── события движков ───────────────────────────

    fn process_mpv_events(&mut self) {
        let Some(player) = self.mpv.as_mut() else {
            return;
        };
        let events = player.poll_events();
        if self.backend != Backend::Mpv {
            return;
        }
        for event in events {
            match event {
                MpvEvent::Loaded => {
                    self.media_loaded = true;
                    self.end_handled = false;
                    self.status = format!("Открыто: {}", self.current_title);
                }
                MpvEvent::Restarted | MpvEvent::Ended => {}
                MpvEvent::Error(error) => {
                    self.push_load_error(Backend::Mpv, &error);
                    self.start_next_backend();
                    return;
                }
            }
        }
    }

    fn process_native_events(&mut self) {
        let events = self
            .native_media
            .as_ref()
            .map(|engine| engine.events().try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        if self.backend != Backend::Windows {
            return;
        }
        for event in events {
            match event {
                NativeMediaEvent::LoadStarted => {
                    self.status = format!("Загрузка: {}", self.current_title);
                }
                NativeMediaEvent::MetadataLoaded
                | NativeMediaEvent::DataLoaded
                | NativeMediaEvent::Ready => {
                    let Some(engine) = &self.native_media else {
                        continue;
                    };
                    self.media_loaded = true;
                    self.duration = engine.duration();
                    self.has_video = engine.has_video();
                    self.has_audio = engine.has_audio();
                    // Media Foundation молча играет файл без картинки, если нет
                    // видеокодека, — тогда передаём файл mpv.
                    if !self.windows_video_checked {
                        self.windows_video_checked = true;
                        if self.current_kind == MediaKind::Video
                            && !self.has_video
                            && self.backend_queue.contains(&Backend::Mpv)
                        {
                            let _ = engine.pause();
                            self.push_load_error(
                                Backend::Windows,
                                "нет подходящего видеокодека Windows",
                            );
                            self.start_next_backend();
                            return;
                        }
                    }
                    let _ = engine.set_volume(self.settings.volume);
                    let _ = engine.set_muted(self.settings.muted);
                    let _ = engine.set_speed(self.settings.speed);
                    let _ = engine.set_loop(self.settings.loop_file);
                    if let Some(position) = self.pending_seek.take()
                        && (position + 5.0 < self.duration || self.duration <= 0.0)
                    {
                        let _ = engine.seek(position);
                    }
                    if self.pending_autoplay {
                        self.pending_autoplay = false;
                        let _ = engine.play();
                    }
                    self.status = format!("Открыто: {}", self.current_title);
                }
                NativeMediaEvent::Playing => self.paused = false,
                NativeMediaEvent::Paused => self.paused = true,
                NativeMediaEvent::Waiting => self.status = "Буферизация…".to_string(),
                NativeMediaEvent::Seeking | NativeMediaEvent::Seeked => {}
                NativeMediaEvent::TimeUpdated
                | NativeMediaEvent::DurationChanged
                | NativeMediaEvent::FormatChanged
                | NativeMediaEvent::VolumeChanged
                | NativeMediaEvent::RateChanged => {}
                NativeMediaEvent::Ended => self.on_media_ended(),
                NativeMediaEvent::Error(error) => {
                    let _ = self.native_media.as_ref().map(NativeMediaEngine::pause);
                    self.push_load_error(Backend::Windows, &error);
                    self.start_next_backend();
                    return;
                }
            }
        }
    }

    fn poll_playback(&mut self) {
        match self.backend {
            Backend::Rodio => {
                let Some((current_time, duration, paused, finished)) =
                    self.audio_engine.as_ref().map(|engine| {
                        (
                            engine.current_time(),
                            engine.duration(),
                            engine.is_paused(),
                            engine.is_finished(),
                        )
                    })
                else {
                    return;
                };
                self.current_time = current_time;
                self.duration = duration;
                self.paused = paused;
                if finished && !self.end_handled {
                    self.end_handled = true;
                    self.on_media_ended();
                    return;
                }
            }
            Backend::Mpv => {
                let Some(player) = &self.mpv else {
                    return;
                };
                if !player.is_loaded() {
                    return;
                }
                self.media_loaded = true;
                self.current_time = player.current_time();
                self.duration = player.duration();
                self.paused = player.is_paused();
                self.has_video = player.has_video();
                self.has_audio = player.has_audio();
                if !self.settings.loop_file && player.eof_reached() && !self.end_handled {
                    self.end_handled = true;
                    self.on_media_ended();
                    return;
                }
            }
            Backend::Windows => {
                let Some(engine) = &self.native_media else {
                    return;
                };
                self.current_time = engine.current_time();
                self.duration = engine.duration();
                self.paused = engine.is_paused();
                self.has_video = engine.has_video();
                self.has_audio = engine.has_audio();
            }
            Backend::Image | Backend::None => return,
        }
        if self.settings.remember_position
            && self.last_position_update.elapsed() >= Duration::from_secs(5)
        {
            self.store_current_position();
            self.last_position_update = Instant::now();
        }
    }

    fn process_image_results(&mut self, ctx: &egui::Context) {
        for result in self.image_rx.try_iter().collect::<Vec<_>>() {
            if self.backend != Backend::Image || self.image.path.as_ref() != Some(&result.path) {
                continue;
            }
            self.image.loading = false;
            match result.data {
                Ok(data) => {
                    let name = data.path.display().to_string();
                    let size = [data.width, data.height];
                    self.image.has_alpha = data.rgba.chunks_exact(4).any(|pixel| pixel[3] < 250);
                    self.image.texture = Some(ctx.load_texture(
                        format!("image:{name}"),
                        ColorImage::from_rgba_unmultiplied(size, &data.rgba),
                        TEXTURE_OPTIONS,
                    ));
                    self.image.frames = data
                        .frames
                        .iter()
                        .enumerate()
                        .map(|(index, frame)| {
                            ctx.load_texture(
                                format!("image:{name}#{index}"),
                                ColorImage::from_rgba_unmultiplied(size, &frame.rgba),
                                TextureOptions::LINEAR,
                            )
                        })
                        .collect();
                    self.image.delays = data.frames.iter().map(|frame| frame.delay_ms).collect();
                    self.image.frame = 0;
                    self.image.next_frame_at = self
                        .image
                        .delays
                        .first()
                        .map(|delay| Instant::now() + Duration::from_millis(*delay as u64));
                    self.image.size = size;
                    self.image.original_size = data.original_size;
                    self.image.format = data.format;
                    self.image.file_size = data.file_size;
                    self.image.rgba = data.rgba;
                    self.media_loaded = true;
                    self.status = format!("Открыто: {}", self.current_title);
                }
                Err(error) => {
                    self.push_load_error(Backend::Image, &error);
                    self.backend = Backend::None;
                    self.fail_load();
                }
            }
        }
    }

    fn process_cover_results(&mut self, ctx: &egui::Context) {
        for result in self.cover_rx.try_iter().collect::<Vec<_>>() {
            let CoverResult {
                path,
                title,
                artist,
                source,
                data,
                error,
            } = result;
            if self.cover_path.as_ref() != Some(&path) {
                continue;
            }
            self.cover_loading = false;
            self.cover_title = title;
            self.cover_artist = artist;
            self.cover_source = match (source, data.is_some(), error) {
                (Some(source), _, _) => Some(source),
                (None, true, _) => None,
                (None, false, Some(error)) => Some(format_cover_error(&error)),
                (None, false, None) => None,
            };
            self.cover_backdrop = data.as_ref().and_then(|value| {
                let buffer = image::RgbaImage::from_raw(
                    value.width as u32,
                    value.height as u32,
                    value.rgba.clone(),
                )?;
                let small =
                    image::imageops::resize(&buffer, 12, 12, image::imageops::FilterType::Triangle);
                Some(ctx.load_texture(
                    format!("cover-backdrop:{}", path.display()),
                    ColorImage::from_rgba_unmultiplied([12, 12], small.as_raw()),
                    TextureOptions::LINEAR,
                ))
            });
            self.cover_texture = data.map(|value| {
                ctx.load_texture(
                    format!("cover:{}", path.display()),
                    ColorImage::from_rgba_unmultiplied([value.width, value.height], &value.rgba),
                    TEXTURE_OPTIONS,
                )
            });
        }
    }

    fn process_thumbnail_results(&mut self, ctx: &egui::Context) {
        for result in self.thumbnail_rx.try_iter().collect::<Vec<_>>() {
            self.thumbnail_pending.remove(&result.path);
            if !self.playlist.iter().any(|path| path == &result.path) {
                continue;
            }
            let texture = result.data.map(|data| {
                ctx.load_texture(
                    format!("thumb:{}", data.path.display()),
                    ColorImage::from_rgba_unmultiplied([data.width, data.height], &data.rgba),
                    TEXTURE_OPTIONS,
                )
            });
            if let Some(position) = self
                .thumbnail_order
                .iter()
                .position(|path| path == &result.path)
            {
                self.thumbnail_order.remove(position);
            }
            self.thumbnail_order.push_back(result.path.clone());
            self.thumbnail_cache.insert(result.path, texture);
            while self.thumbnail_cache.len() > THUMBNAIL_CACHE_LIMIT {
                let Some(oldest) = self.thumbnail_order.pop_front() else {
                    break;
                };
                self.thumbnail_cache.remove(&oldest);
            }
        }
    }

    fn thumbnail_for(&mut self, path: &Path) -> Option<TextureHandle> {
        if let Some(entry) = self.thumbnail_cache.get(path) {
            return entry.clone();
        }
        if !self.thumbnail_pending.contains(path) {
            self.thumbnail_pending.insert(path.to_path_buf());
            if self.thumbnail_tx.send(path.to_path_buf()).is_err() {
                self.thumbnail_pending.remove(path);
            }
        }
        None
    }

    fn clear_thumbnails(&mut self) {
        self.thumbnail_cache.clear();
        self.thumbnail_pending.clear();
        self.thumbnail_order.clear();
    }

    fn process_instance_requests(&mut self) {
        for paths in self.instance_rx.try_iter().collect::<Vec<_>>() {
            self.add_paths(paths, true);
        }
    }

    fn process_folder_scan(&mut self) {
        let result = self.scan_rx.as_ref().map(|receiver| receiver.try_recv());
        match result {
            Some(Ok(files)) => {
                self.scan_rx = None;
                self.scanning_directory = false;
                if files.is_empty() {
                    self.status = "В папке нет поддерживаемых файлов".to_string();
                } else {
                    self.replace_playlist(files);
                }
            }
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.scan_rx = None;
                self.scanning_directory = false;
                self.status = "Не удалось завершить сканирование папки".to_string();
            }
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => {}
        }
    }

    fn process_dropped_files(&mut self, ctx: &egui::Context) {
        let paths = ctx
            .input(|input| input.raw.dropped_files.clone())
            .into_iter()
            .filter_map(|file| file.path)
            .collect::<Vec<_>>();
        if !paths.is_empty() {
            self.add_paths(paths, true);
        }
    }

    // ─────────────────────────── плейлист ───────────────────────────

    fn add_paths(&mut self, paths: Vec<PathBuf>, replace: bool) {
        let files = normalize_input_paths(paths, &self.settings.custom_extensions);
        if files.is_empty() {
            self.status = "Поддерживаемые файлы не найдены".to_string();
            return;
        }
        if replace {
            self.replace_playlist(files);
            return;
        }
        let mut added = 0usize;
        let mut known = self
            .playlist
            .iter()
            .map(|path| path_identity(path))
            .collect::<HashSet<_>>();
        for path in files {
            if known.insert(path_identity(&path)) {
                self.playlist.push(path);
                added += 1;
            }
        }
        if self.current_index.is_none() && !self.playlist.is_empty() {
            self.current_index = Some(0);
            self.load_current();
        }
        self.status = format!("Добавлено файлов: {added}");
        self.mark_dirty();
    }

    fn replace_playlist(&mut self, files: Vec<PathBuf>) {
        self.store_current_position();
        self.playlist = files;
        self.current_index = (!self.playlist.is_empty()).then_some(0);
        self.clear_thumbnails();
        if self.current_index.is_some() {
            self.load_current();
        } else {
            self.clear_current();
        }
        if self.playlist.len() > 1 {
            self.status = format!("Файлов в плейлисте: {}", self.playlist.len());
        }
        self.mark_dirty();
    }

    fn file_dialog(&self, title: &str) -> rfd::FileDialog {
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if !self.settings.last_directory.is_empty() {
            dialog = dialog.set_directory(&self.settings.last_directory);
        }
        dialog
    }

    fn open_files_dialog(&mut self) {
        if let Some(files) = self.file_dialog("Открыть медиафайлы").pick_files() {
            if let Some(first) = files.first() {
                self.settings.set_last_directory_from_path(first);
            }
            self.add_paths(files, true);
        }
    }

    fn add_files_dialog(&mut self) {
        if let Some(files) = self.file_dialog("Добавить медиафайлы").pick_files()
        {
            if let Some(first) = files.first() {
                self.settings.set_last_directory_from_path(first);
            }
            self.add_paths(files, false);
        }
    }

    fn open_folder_dialog(&mut self) {
        let Some(folder) = self.file_dialog("Открыть папку").pick_folder() else {
            return;
        };
        self.settings.set_last_directory_from_path(&folder);
        let extensions = self.settings.custom_extensions.clone();
        let (sender, receiver) = unbounded();
        self.scan_rx = Some(receiver);
        self.scanning_directory = true;
        self.status = format!("Сканирование папки: {}", folder.display());
        thread::spawn(move || {
            let files = collect_media_from_directory(&folder, &extensions);
            let _ = sender.send(files);
        });
    }

    fn clear_playlist(&mut self) {
        self.store_current_position();
        self.playlist.clear();
        self.clear_thumbnails();
        self.clear_current();
    }

    fn remove_playlist_index(&mut self, index: usize) {
        if index >= self.playlist.len() {
            return;
        }
        let was_current = self.current_index == Some(index);
        let removed_path = self.playlist.remove(index);
        self.thumbnail_cache.remove(&removed_path);
        self.thumbnail_pending.remove(&removed_path);
        if let Some(position) = self
            .thumbnail_order
            .iter()
            .position(|path| path == &removed_path)
        {
            self.thumbnail_order.remove(position);
        }
        self.current_index = match self.current_index {
            None => None,
            Some(_) if self.playlist.is_empty() => None,
            Some(current) if current > index => Some(current - 1),
            Some(current) if current == index => Some(index.min(self.playlist.len() - 1)),
            Some(current) => Some(current),
        };
        if was_current {
            if self.current_index.is_some() {
                self.load_current();
            } else {
                self.clear_current();
            }
        }
        self.mark_dirty();
    }

    fn navigate_images(&mut self, forward: bool) {
        if self.playlist.len() > 1 {
            if forward {
                let index = self
                    .current_index
                    .map(|index| (index + 1) % self.playlist.len())
                    .unwrap_or(0);
                self.select_index(index);
            } else {
                let index = match self.current_index {
                    Some(0) | None => self.playlist.len() - 1,
                    Some(index) => index - 1,
                };
                self.select_index(index);
            }
            return;
        }
        self.navigate_directory(forward);
    }

    fn navigate_directory(&mut self, forward: bool) {
        let Some(current) = self.current_path().cloned() else {
            return;
        };
        let mut files =
            collect_media_from_parent_directory(&current, &self.settings.custom_extensions);
        if !files.iter().any(|path| path == &current) {
            files.push(current.clone());
        }
        if files.len() <= 1 {
            self.status = "В папке нет других поддерживаемых файлов".to_string();
            return;
        }
        let current_position = files.iter().position(|path| path == &current).unwrap_or(0);
        let target = if forward {
            (current_position + 1) % files.len()
        } else if current_position == 0 {
            files.len() - 1
        } else {
            current_position - 1
        };
        self.store_current_position();
        self.playlist = files;
        self.current_index = Some(target);
        self.clear_thumbnails();
        self.load_current();
        self.mark_dirty();
    }

    // ─────────────────────────── изображения ───────────────────────────

    fn copy_current_image(&mut self) {
        if !self.current_is_image() || self.image.rgba.is_empty() {
            self.status = "Сейчас не открыто готовое изображение".to_string();
            return;
        }
        match copy_rgba(self.image.size[0], self.image.size[1], &self.image.rgba) {
            Ok(()) => self.status = "Изображение скопировано в буфер обмена".to_string(),
            Err(error) => self.status = format!("Не удалось скопировать изображение: {error}"),
        }
    }

    fn paste_clipboard_image(&mut self) {
        match paste_image_to_cache() {
            Ok(path) => {
                self.replace_playlist(vec![path]);
                self.status = "Изображение открыто из буфера обмена".to_string();
            }
            Err(error) => self.status = error,
        }
    }

    fn image_fit_scale(&self, viewer: Rect) -> f32 {
        let source = self.rotated_image_size();
        if source.x <= 0.0 || source.y <= 0.0 {
            return 1.0;
        }
        (viewer.width() / source.x)
            .min(viewer.height() / source.y)
            .min(self.texture_to_original_ratio().recip())
            .max(0.0001)
    }

    /// Во сколько раз текстура меньше исходного файла (большие фото уменьшаются).
    fn texture_to_original_ratio(&self) -> f32 {
        if self.image.original_size[0] == 0 || self.image.size[0] == 0 {
            1.0
        } else {
            self.image.size[0] as f32 / self.image.original_size[0] as f32
        }
    }

    fn rotated_image_size(&self) -> Vec2 {
        let size = Vec2::new(self.image.size[0] as f32, self.image.size[1] as f32);
        if self.image.rotation % 2 == 1 {
            Vec2::new(size.y, size.x)
        } else {
            size
        }
    }

    fn image_zoom_percent(&self) -> f32 {
        let Some(viewer) = self.video_rect else {
            return 100.0;
        };
        self.image_fit_scale(viewer) * self.image.zoom * self.texture_to_original_ratio() * 100.0
    }

    fn zoom_image(&mut self, factor: f32, anchor: Option<Pos2>) {
        let Some(viewer) = self.video_rect else {
            return;
        };
        let old = self.image.zoom;
        let new = (old * factor).clamp(0.05, 64.0);
        if let Some(anchor) = anchor {
            let offset = anchor - viewer.center();
            self.image.pan = offset - (offset - self.image.pan) * (new / old);
        } else {
            self.image.pan *= new / old;
        }
        self.image.zoom = new;
        if (new - 1.0).abs() < 0.001 {
            self.image.pan = Vec2::ZERO;
        }
    }

    fn image_actual_size(&mut self) {
        let Some(viewer) = self.video_rect else {
            return;
        };
        let fit = self.image_fit_scale(viewer) * self.texture_to_original_ratio();
        self.image.zoom = 1.0 / fit.max(0.0001);
        self.image.pan = Vec2::ZERO;
    }

    fn rotate_image(&mut self, clockwise: bool) {
        self.image.rotation = if clockwise {
            (self.image.rotation + 1) % 4
        } else {
            (self.image.rotation + 3) % 4
        };
        self.image.reset_view();
    }

    fn advance_animation(&mut self, ctx: &egui::Context) {
        if self.image.frames.len() <= 1 || self.image.animation_paused {
            return;
        }
        let Some(next) = self.image.next_frame_at else {
            return;
        };
        let now = Instant::now();
        if now >= next {
            self.image.frame = (self.image.frame + 1) % self.image.frames.len();
            let delay = self
                .image
                .delays
                .get(self.image.frame)
                .copied()
                .unwrap_or(100);
            let mut target = next + Duration::from_millis(delay as u64);
            if target < now {
                target = now + Duration::from_millis(delay as u64);
            }
            self.image.next_frame_at = Some(target);
        }
        if let Some(next) = self.image.next_frame_at {
            ctx.request_repaint_after(next.saturating_duration_since(now));
        }
    }

    // ─────────────────────────── видео: масштаб и яркость ───────────────────────────

    /// Масштаб видео и обложки — как у фото: относительно курсора.
    fn zoom_media(&mut self, factor: f32, anchor: Option<Pos2>) {
        if self.current_index.is_none() || self.current_is_image() {
            return;
        }
        let Some(viewer) = self.video_rect else {
            return;
        };
        let old = self.media_zoom;
        let mut new = (old * factor).clamp(1.0, 8.0);
        if (new - 1.0).abs() < 0.03 {
            new = 1.0;
        }
        if let Some(anchor) = anchor {
            let offset = anchor - viewer.center();
            self.media_pan = offset - (offset - self.media_pan) * (new / old);
        } else {
            self.media_pan *= new / old;
        }
        self.media_zoom = new;
        self.clamp_media_pan();
    }

    fn clamp_media_pan(&mut self) {
        let Some(viewer) = self.video_rect else {
            return;
        };
        let limit = viewer.size() * (self.media_zoom - 1.0) / 2.0;
        self.media_pan.x = self.media_pan.x.clamp(-limit.x, limit.x);
        self.media_pan.y = self.media_pan.y.clamp(-limit.y, limit.y);
    }

    fn reset_media_zoom(&mut self) {
        self.media_zoom = 1.0;
        self.media_pan = Vec2::ZERO;
    }

    fn adjust_brightness(&mut self, delta: f32) {
        if self.current_index.is_none() || self.current_is_image() {
            return;
        }
        self.settings.brightness = (self.settings.brightness + delta).clamp(0.25, 2.0);
        if let Some(host) = self.media_host.as_mut() {
            host.set_brightness(self.settings.brightness);
        }
        self.status = format!("Яркость: {:.0}%", self.settings.brightness * 100.0);
        self.mark_dirty();
    }

    fn reset_settings(&mut self) {
        let playlist = self.playlist.clone();
        let playlist_index = self.current_index;
        let association_done = self.settings.association_prompt_done;
        self.settings = Settings::default();
        self.settings.playlist = playlist;
        self.settings.playlist_index = playlist_index;
        self.settings.association_prompt_done = association_done;
        self.applied_theme = None;
        self.apply_volume();
        self.apply_muted();
        self.apply_speed();
        self.apply_loop();
        if let Some(host) = self.media_host.as_mut() {
            host.set_zoom(1.0);
            host.set_brightness(self.settings.brightness);
        }
        self.applied_always_on_top = None;
        match self.settings.save() {
            Ok(()) => {
                self.status = "Настройки программы сброшены".to_string();
                self.dirty = false;
            }
            Err(error) => {
                self.status = format!("Не удалось сохранить сброшенные настройки: {error}");
                self.dirty = true;
            }
        }
        self.last_save = Instant::now();
    }

    // ─────────────────────────── клавиатура ───────────────────────────

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.show_settings || self.show_convert {
            return;
        }
        let pressed = |key: Key| ctx.input(|input| input.key_pressed(key));
        let (ctrl, shift) = ctx.input(|input| (input.modifiers.ctrl, input.modifiers.shift));
        let is_image = self.current_is_image();

        if ctrl && pressed(Key::C) {
            self.copy_current_image();
        }
        if ctrl && pressed(Key::E) {
            if let Some(path) = self.current_path().cloned() {
                self.open_converter(path);
            }
        }
        if ctrl && pressed(Key::V) {
            self.paste_clipboard_image();
        }
        if ctrl && pressed(Key::O) {
            if shift {
                self.open_folder_dialog();
            } else {
                self.open_files_dialog();
            }
        }
        if pressed(Key::Space) {
            self.toggle_playback();
        }
        if pressed(Key::ArrowRight) {
            if is_image {
                self.navigate_images(true);
            } else if self.media_loaded {
                self.seek_relative(if ctrl { 30.0 } else { 5.0 });
            }
        }
        if pressed(Key::ArrowLeft) {
            if is_image {
                self.navigate_images(false);
            } else if self.media_loaded {
                self.seek_relative(if ctrl { -30.0 } else { -5.0 });
            }
        }
        if pressed(Key::PageUp) {
            self.previous_track();
        }
        if pressed(Key::PageDown) {
            self.next_track();
        }
        if ctrl && pressed(Key::ArrowUp) {
            self.adjust_brightness(0.1);
        } else if ctrl && pressed(Key::ArrowDown) {
            self.adjust_brightness(-0.1);
        } else if !is_image && pressed(Key::ArrowUp) {
            self.change_volume(5.0);
        } else if !is_image && pressed(Key::ArrowDown) {
            self.change_volume(-5.0);
        }
        if !is_image && !ctrl && pressed(Key::S) {
            self.stop_playback();
        }
        if pressed(Key::M) {
            self.settings.muted = !self.settings.muted;
            self.apply_muted();
            self.mark_dirty();
        }
        if is_image {
            if pressed(Key::Plus) || pressed(Key::Equals) {
                self.zoom_image(1.25, None);
            }
            if pressed(Key::Minus) {
                self.zoom_image(1.0 / 1.25, None);
            }
            if pressed(Key::Num0) {
                self.image.reset_view();
            }
            if pressed(Key::Num1) {
                self.image_actual_size();
            }
            if pressed(Key::R) {
                self.rotate_image(!shift);
            }
        } else {
            if pressed(Key::Plus) || pressed(Key::Equals) {
                self.zoom_media(1.25, None);
            }
            if pressed(Key::Minus) {
                self.zoom_media(1.0 / 1.25, None);
            }
            if pressed(Key::Num0) || pressed(Key::Num1) {
                self.reset_media_zoom();
            }
            if pressed(Key::CloseBracket) {
                self.step_speed(true);
            }
            if pressed(Key::OpenBracket) {
                self.step_speed(false);
            }
        }
        if pressed(Key::F11) || (!ctrl && pressed(Key::F)) {
            self.set_fullscreen(ctx, !self.fullscreen);
        }
        if self.fullscreen && pressed(Key::Escape) {
            self.set_fullscreen(ctx, false);
        }
    }

    fn set_fullscreen(&mut self, ctx: &egui::Context, fullscreen: bool) {
        self.fullscreen = fullscreen;
        self.last_pointer_activity = Instant::now();
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(fullscreen));
    }

    fn track_pointer_activity(&mut self, ctx: &egui::Context) {
        let active = ctx.input(|input| {
            input.pointer.delta() != Vec2::ZERO
                || input.pointer.any_pressed()
                || !input.events.is_empty()
                    && input.events.iter().any(|event| {
                        matches!(
                            event,
                            egui::Event::Key { .. } | egui::Event::MouseWheel { .. }
                        )
                    })
        });
        if active {
            self.last_pointer_activity = Instant::now();
        }
    }

    fn controls_visible(&self) -> bool {
        !self.fullscreen
            || self.current_index.is_none()
            || self.last_pointer_activity.elapsed() < CONTROLS_HIDE_DELAY
            || (self.paused && !self.current_is_image())
    }

    // ─────────────────────────── интерфейс ───────────────────────────

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        egui::Panel::top("top_bar")
            .exact_size(54.0)
            .frame(
                Frame::new()
                    .fill(palette.surface)
                    .inner_margin(Margin::symmetric(14, 0)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let (logo_rect, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
                    paint_logo(ui.painter(), &self.logo, logo_rect);
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("AnMedia")
                            .font(semibold(16.5))
                            .color(palette.text),
                    );
                    ui.add_space(10.0);
                    let (line, _) = ui.allocate_exact_size(Vec2::new(1.0, 22.0), Sense::hover());
                    ui.painter().rect_filled(line, 0.0, palette.outline);
                    ui.add_space(6.0);
                    if tool_button(
                        ui,
                        &palette,
                        icon::OPEN_FILE,
                        Some("Открыть"),
                        false,
                        "Открыть файлы (Ctrl+O)",
                    )
                    .clicked()
                    {
                        self.open_files_dialog();
                    }
                    if tool_button(
                        ui,
                        &palette,
                        icon::FOLDER,
                        Some("Папка"),
                        false,
                        "Открыть папку целиком (Ctrl+Shift+O)",
                    )
                    .clicked()
                    {
                        self.open_folder_dialog();
                    }
                    if tool_button(
                        ui,
                        &palette,
                        icon::ADD,
                        Some("Добавить"),
                        false,
                        "Добавить файлы в плейлист",
                    )
                    .clicked()
                    {
                        self.add_files_dialog();
                    }
                    let convert_label = self
                        .convert_job
                        .as_ref()
                        .map(|job| format!("{:.0}%", job.progress() * 100.0))
                        .unwrap_or_else(|| "Конвертировать".to_string());
                    if tool_button(
                        ui,
                        &palette,
                        icon::CONVERT,
                        Some(&convert_label),
                        self.convert_job.is_some() || self.show_convert,
                        "Перевести видео, музыку или фото в другой формат",
                    )
                    .clicked()
                    {
                        match self.current_path().cloned() {
                            Some(path) => self.open_converter(path),
                            None if self.convert_job.is_some() => self.show_convert = true,
                            None => {
                                self.status = "Сначала откройте файл для конвертации".to_string()
                            }
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if tool_button(
                            ui,
                            &palette,
                            icon::SETTINGS,
                            None,
                            self.show_settings,
                            "Настройки",
                        )
                        .clicked()
                        {
                            self.show_settings = true;
                        }
                        if tool_button(
                            ui,
                            &palette,
                            icon::LIST,
                            None,
                            self.settings.show_playlist,
                            "Показать или скрыть плейлист",
                        )
                        .clicked()
                        {
                            self.settings.show_playlist = !self.settings.show_playlist;
                            self.mark_dirty();
                        }
                        if tool_button(
                            ui,
                            &palette,
                            icon::PIN,
                            None,
                            self.settings.always_on_top,
                            "Поверх остальных окон",
                        )
                        .clicked()
                        {
                            self.settings.always_on_top = !self.settings.always_on_top;
                            self.mark_dirty();
                        }
                        if self.scanning_directory {
                            ui.add_space(6.0);
                            ui.add(egui::Spinner::new().size(16.0).color(palette.accent));
                        }
                        let free = ui.available_rect_before_wrap();
                        self.draw_toast(ui, free, &palette);
                    });
                });
            });
    }

    fn draw_toast(&mut self, ui: &egui::Ui, area: Rect, palette: &Palette) {
        let Some((message, shown_at)) = &self.toast else {
            return;
        };
        let elapsed = shown_at.elapsed().as_secs_f32();
        let error = is_error_message(message);
        let lifetime = if error { 7.0 } else { 3.5 };
        if elapsed > lifetime {
            self.toast = None;
            return;
        }
        let fade_in = (elapsed / 0.15).min(1.0);
        let fade_out = ((lifetime - elapsed) / 0.4).min(1.0);
        let alpha = fade_in.min(fade_out);
        if area.width() < 120.0 {
            return;
        }
        let text_color = if error { palette.danger } else { palette.text };
        let galley = truncated(
            ui,
            message,
            FontId::proportional(13.0),
            text_color,
            (area.width() - 60.0).min(560.0),
        );
        let size = galley.size() + Vec2::new(40.0, 14.0);
        let pill =
            Rect::from_center_size(area.center() + Vec2::new(0.0, (1.0 - fade_in) * -4.0), size);
        let painter = ui.painter();
        let fill = if error {
            palette.danger.gamma_multiply(0.14)
        } else {
            palette.card
        };
        painter.rect_filled(pill, 15.0, fill.gamma_multiply(alpha));
        painter.rect_stroke(
            pill,
            15.0,
            Stroke::new(
                1.0,
                (if error {
                    palette.danger.gamma_multiply(0.5)
                } else {
                    palette.outline_strong
                })
                .gamma_multiply(alpha),
            ),
            egui::StrokeKind::Inside,
        );
        painter.text(
            Pos2::new(pill.left() + 16.0, pill.center().y),
            Align2::CENTER_CENTER,
            if error { icon::ERROR } else { icon::CHECK },
            icon_font(12.0),
            (if error {
                palette.danger
            } else {
                palette.accent
            })
            .gamma_multiply(alpha),
        );
        painter.galley(
            Pos2::new(pill.left() + 30.0, pill.center().y - galley.size().y / 2.0),
            galley,
            text_color.gamma_multiply(alpha),
        );
        ui.ctx().request_repaint_after(Duration::from_millis(30));
    }

    fn control_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if !self.controls_visible() {
            return;
        }
        let palette = self.palette();
        let is_image = self.current_is_image();
        let has_file = self.current_index.is_some();
        let show_seek = has_file && !is_image;
        let height = if show_seek { 100.0 } else { 76.0 };
        egui::Panel::bottom("control_bar")
            .exact_size(height)
            .frame(
                Frame::new()
                    .fill(palette.surface)
                    .inner_margin(Margin::symmetric(18, 8)),
            )
            .show(ui, |ui| {
                if show_seek {
                    let seek_enabled = self.media_loaded && self.duration > 0.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        time_label(ui, &palette, self.current_time, Align2::RIGHT_CENTER);
                        let width = (ui.available_width() - 56.0).max(120.0);
                        let mut value = self.current_time.clamp(0.0, self.duration.max(0.0));
                        if seek_bar(ui, &palette, &mut value, self.duration, seek_enabled, width) {
                            self.seek_current(value);
                        }
                        time_label(ui, &palette, self.duration, Align2::LEFT_CENTER);
                    });
                }
                let available = ui.available_rect_before_wrap();
                let row = Rect::from_min_max(
                    Pos2::new(available.left(), available.top()),
                    Pos2::new(
                        available.right(),
                        available.bottom().max(available.top() + 52.0),
                    ),
                );
                ui.allocate_rect(row, Sense::hover());

                let center_width = if is_image { 372.0 } else { 236.0 };
                let center =
                    Rect::from_center_size(row.center(), Vec2::new(center_width, row.height()));
                let left = Rect::from_min_max(
                    row.min,
                    Pos2::new((center.left() - 16.0).max(row.left() + 40.0), row.max.y),
                );
                let right = Rect::from_min_max(
                    Pos2::new((center.right() + 16.0).min(row.right() - 40.0), row.min.y),
                    row.max,
                );

                self.now_playing(ui, left, &palette);
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(center)
                        .layout(Layout::left_to_right(Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if is_image {
                            self.image_controls(ui, &palette);
                        } else {
                            self.transport_controls(ui, &palette, has_file);
                        }
                    },
                );
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(right)
                        .layout(Layout::right_to_left(Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        self.right_controls(ui, ctx, &palette, is_image, has_file);
                    },
                );
            });
    }

    fn transport_controls(&mut self, ui: &mut egui::Ui, palette: &Palette, has_file: bool) {
        if icon_button(
            ui,
            palette,
            icon::SHUFFLE,
            36.0,
            self.settings.shuffle,
            true,
            "Случайный порядок",
        )
        .clicked()
        {
            self.settings.shuffle = !self.settings.shuffle;
            self.mark_dirty();
        }
        if icon_button(
            ui,
            palette,
            icon::PREVIOUS,
            40.0,
            false,
            has_file,
            "Предыдущий (PgUp)",
        )
        .clicked()
        {
            self.previous_track();
        }
        let playing = !self.paused && self.media_loaded;
        if play_button(ui, palette, playing, has_file && self.load_failed.is_none()).clicked() {
            self.toggle_playback();
        }
        if icon_button(
            ui,
            palette,
            icon::NEXT,
            40.0,
            false,
            has_file,
            "Следующий (PgDn)",
        )
        .clicked()
        {
            self.next_track();
        }
        if icon_button(
            ui,
            palette,
            icon::REPEAT_ONE,
            36.0,
            self.settings.loop_file,
            true,
            "Повтор файла",
        )
        .clicked()
        {
            self.settings.loop_file = !self.settings.loop_file;
            self.apply_loop();
            self.mark_dirty();
        }
    }

    fn image_controls(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        if icon_button(
            ui,
            palette,
            icon::PREVIOUS,
            38.0,
            false,
            true,
            "Предыдущее (←)",
        )
        .clicked()
        {
            self.navigate_images(false);
        }
        if icon_button(
            ui,
            palette,
            icon::ZOOM_OUT,
            38.0,
            false,
            true,
            "Уменьшить (−)",
        )
        .clicked()
        {
            self.zoom_image(1.0 / 1.25, None);
        }
        let percent = self.image_zoom_percent();
        let label = format!("{percent:.0}%");
        let response = pill_button(
            ui,
            palette,
            &label,
            "Щелчок — 100% / вписать в окно (1 / 0)",
        );
        if response.clicked() {
            if (self.image.zoom - 1.0).abs() < 0.001 && self.image.pan == Vec2::ZERO {
                self.image_actual_size();
            } else {
                self.image.reset_view();
            }
        }
        if icon_button(
            ui,
            palette,
            icon::ZOOM_IN,
            38.0,
            false,
            true,
            "Увеличить (+)",
        )
        .clicked()
        {
            self.zoom_image(1.25, None);
        }
        if icon_button(
            ui,
            palette,
            icon::ROTATE_LEFT,
            38.0,
            false,
            true,
            "Повернуть влево (Shift+R)",
        )
        .clicked()
        {
            self.rotate_image(false);
        }
        if icon_button(
            ui,
            palette,
            icon::ROTATE_RIGHT,
            38.0,
            false,
            true,
            "Повернуть вправо (R)",
        )
        .clicked()
        {
            self.rotate_image(true);
        }
        if icon_button(ui, palette, icon::NEXT, 38.0, false, true, "Следующее (→)").clicked()
        {
            self.navigate_images(true);
        }
    }

    fn right_controls(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        palette: &Palette,
        is_image: bool,
        has_file: bool,
    ) {
        let fullscreen_icon = if self.fullscreen {
            icon::EXIT_FULLSCREEN
        } else {
            icon::FULLSCREEN
        };
        if icon_button(
            ui,
            palette,
            fullscreen_icon,
            36.0,
            false,
            true,
            "Полный экран (F11, F)",
        )
        .clicked()
        {
            self.set_fullscreen(ctx, !self.fullscreen);
        }
        if is_image {
            if icon_button(
                ui,
                palette,
                icon::COPY,
                36.0,
                false,
                !self.image.rgba.is_empty(),
                "Копировать (Ctrl+C)",
            )
            .clicked()
            {
                self.copy_current_image();
            }
            if self.image.frames.len() > 1 {
                let glyph = if self.image.animation_paused {
                    icon::PLAY
                } else {
                    icon::PAUSE
                };
                if icon_button(
                    ui,
                    palette,
                    glyph,
                    36.0,
                    false,
                    true,
                    "Анимация: пауза (Пробел)",
                )
                .clicked()
                {
                    self.image.animation_paused = !self.image.animation_paused;
                }
            }
            return;
        }
        if self.media_zoom > 1.001 {
            let zoom = pill_button(
                ui,
                palette,
                &format!("{:.0}%", self.media_zoom * 100.0),
                "Масштаб. Щелчок — вернуть 100% (0)",
            );
            if zoom.clicked() {
                self.reset_media_zoom();
            }
        }
        let speed = pill_button(
            ui,
            palette,
            &format_speed(self.settings.speed),
            "Скорость: ЛКМ — быстрее, ПКМ — медленнее, колесо",
        );
        if speed.clicked() {
            self.step_speed(true);
        }
        if speed.secondary_clicked() {
            self.step_speed(false);
        }
        if speed.hovered() {
            let scroll = wheel_steps(ui);
            if scroll > 0.0 {
                self.step_speed(true);
            } else if scroll < 0.0 {
                self.step_speed(false);
            }
        }
        ui.add_space(4.0);
        let mut volume = self.settings.volume;
        if volume_slider(ui, palette, &mut volume, 104.0) {
            self.settings.volume = volume;
            if self.settings.muted && volume > 0.0 {
                self.settings.muted = false;
                self.apply_muted();
            }
            self.apply_volume();
            self.mark_dirty();
        }
        let volume_icon = if self.settings.muted || self.settings.volume <= 0.0 {
            icon::MUTE
        } else if self.settings.volume < 34.0 {
            icon::VOLUME_1
        } else if self.settings.volume < 67.0 {
            icon::VOLUME_2
        } else {
            icon::VOLUME_3
        };
        let _ = icon::VOLUME_0;
        if icon_button(ui, palette, volume_icon, 36.0, false, true, "Звук (M)").clicked() {
            self.settings.muted = !self.settings.muted;
            self.apply_muted();
            self.mark_dirty();
        }
        let _ = has_file;
    }

    fn now_playing(&mut self, ui: &mut egui::Ui, rect: Rect, palette: &Palette) {
        let Some(path) = self.current_path().cloned() else {
            ui.painter().text(
                Pos2::new(rect.left(), rect.center().y),
                Align2::LEFT_CENTER,
                "Ничего не открыто",
                FontId::proportional(13.0),
                palette.text_faint,
            );
            return;
        };
        let thumb = Rect::from_min_size(
            Pos2::new(rect.left(), rect.center().y - 24.0),
            Vec2::splat(48.0),
        );
        let texture = if self.current_kind == MediaKind::Audio {
            self.cover_texture
                .clone()
                .or_else(|| self.thumbnail_for(&path))
        } else {
            self.thumbnail_for(&path)
        };
        match texture {
            Some(texture) => paint_texture_cover(ui, &texture, thumb, 10),
            None => paint_kind_tile(ui.painter(), palette, thumb, self.current_kind),
        }
        let text_left = thumb.right() + 12.0;
        let max_width = (rect.right() - text_left).max(20.0);
        let title = if self.current_kind == MediaKind::Audio && !self.cover_title.trim().is_empty()
        {
            self.cover_title.clone()
        } else {
            path.file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or(self.current_title.as_str())
                .to_string()
        };
        let title_galley = truncated(ui, &title, semibold(14.5), palette.text, max_width);
        let subtitle = self.now_playing_subtitle(&path);
        let subtitle_galley = truncated(
            ui,
            &subtitle,
            FontId::proportional(12.5),
            palette.text_weak,
            max_width,
        );
        ui.painter().galley(
            Pos2::new(text_left, rect.center().y - title_galley.size().y + 1.0),
            title_galley,
            palette.text,
        );
        ui.painter().galley(
            Pos2::new(text_left, rect.center().y + 3.0),
            subtitle_galley,
            palette.text_weak,
        );
        let response = ui.interact(
            thumb.union(Rect::from_min_max(
                Pos2::new(text_left, rect.top()),
                Pos2::new(rect.right(), rect.bottom()),
            )),
            ui.id().with("now_playing"),
            Sense::click(),
        );
        if response.double_clicked() {
            reveal_in_explorer(&path);
        }
        response.on_hover_text(format!(
            "{}\nДвойной щелчок — показать в Проводнике",
            path.display()
        ));
    }

    fn now_playing_subtitle(&self, path: &Path) -> String {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_uppercase())
            .unwrap_or_default();
        let mut parts: Vec<String> = Vec::new();
        match self.current_kind {
            MediaKind::Image => {
                if self.image.original_size[0] > 0 {
                    parts.push(format!(
                        "{}×{}",
                        self.image.original_size[0], self.image.original_size[1]
                    ));
                }
                parts.push(if self.image.format.is_empty() {
                    extension
                } else {
                    self.image.format.clone()
                });
                if self.image.file_size > 0 {
                    parts.push(format_size(self.image.file_size));
                }
                if self.image.frames.len() > 1 {
                    parts.push(format!("анимация, {} кадр.", self.image.frames.len()));
                }
            }
            MediaKind::Audio => {
                if let Some(artist) = &self.cover_artist {
                    parts.push(artist.clone());
                }
                parts.push(extension);
                if self.backend != Backend::None {
                    parts.push(self.backend.label().to_string());
                }
            }
            MediaKind::Video | MediaKind::Unknown => {
                parts.push(self.current_kind.label().to_string());
                parts.push(extension);
                if self.backend == Backend::Mpv
                    && let Some((width, height)) = self.mpv.as_ref().and_then(MpvPlayer::video_size)
                {
                    parts.push(format!("{width}×{height}"));
                }
                if self.backend != Backend::None {
                    parts.push(self.backend.label().to_string());
                }
            }
        }
        parts.retain(|part| !part.is_empty());
        parts.join(" · ")
    }

    fn playlist_panel(&mut self, ui: &mut egui::Ui) {
        if !self.settings.show_playlist || self.fullscreen {
            return;
        }
        let palette = self.palette();
        egui::Panel::right("playlist_panel")
            .resizable(true)
            .default_size(340.0)
            .min_size(270.0)
            .max_size(600.0)
            .frame(Frame::new().fill(palette.surface).inner_margin(Margin {
                left: 14,
                right: 10,
                top: 14,
                bottom: 8,
            }))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Плейлист")
                            .font(semibold(17.0))
                            .color(palette.text),
                    );
                    let count = self.playlist.len().to_string();
                    let galley =
                        ui.painter()
                            .layout_no_wrap(count, semibold(11.5), palette.text_weak);
                    let (badge, _) = ui.allocate_exact_size(
                        Vec2::new(galley.size().x + 14.0, 20.0),
                        Sense::hover(),
                    );
                    ui.painter().rect_filled(badge, 10.0, palette.card);
                    ui.painter().galley(
                        badge.center() - galley.size() / 2.0,
                        galley,
                        palette.text_weak,
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(
                            ui,
                            &palette,
                            icon::DELETE,
                            32.0,
                            false,
                            !self.playlist.is_empty(),
                            "Очистить плейлист",
                        )
                        .clicked()
                        {
                            self.clear_playlist();
                        }
                        if icon_button(ui, &palette, icon::ADD, 32.0, false, true, "Добавить файлы")
                            .clicked()
                        {
                            self.add_files_dialog();
                        }
                    });
                });
                ui.add_space(6.0);
                search_field(ui, &palette, &mut self.playlist_filter);
                ui.add_space(8.0);

                let mut counts = [0usize; 3];
                for path in &self.playlist {
                    match kind_by_extension(path) {
                        MediaKind::Video | MediaKind::Unknown => counts[0] += 1,
                        MediaKind::Audio => counts[1] += 1,
                        MediaKind::Image => counts[2] += 1,
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                    let filters = [
                        (None, format!("Все {}", self.playlist.len())),
                        (Some(MediaKind::Video), format!("Видео {}", counts[0])),
                        (Some(MediaKind::Audio), format!("Музыка {}", counts[1])),
                        (Some(MediaKind::Image), format!("Фото {}", counts[2])),
                    ];
                    for (filter, label) in filters {
                        if filter.is_some() && label.ends_with(" 0") {
                            continue;
                        }
                        if chip(ui, &palette, &label, self.playlist_kind_filter == filter).clicked()
                        {
                            self.playlist_kind_filter = filter;
                        }
                    }
                });
                ui.add_space(8.0);

                let filter = self.playlist_filter.to_lowercase();
                let kind_filter = self.playlist_kind_filter;
                let rows = self
                    .playlist
                    .iter()
                    .enumerate()
                    .filter(|(_, path)| {
                        let kind = match kind_by_extension(path) {
                            MediaKind::Unknown => MediaKind::Video,
                            kind => kind,
                        };
                        kind_filter.is_none_or(|wanted| wanted == kind)
                            && (filter.is_empty()
                                || path
                                    .file_name()
                                    .map(|name| {
                                        name.to_string_lossy().to_lowercase().contains(&filter)
                                    })
                                    .unwrap_or(false))
                    })
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();

                if self.playlist.is_empty() {
                    self.empty_playlist(ui, &palette);
                    return;
                }
                if rows.is_empty() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Ничего не найдено").color(palette.text_faint));
                    });
                    return;
                }

                let mut selected = None;
                let mut remove = None;
                let mut reveal = None;
                let mut convert_request = None;
                let row_height = 60.0;
                let playing = !self.paused && self.media_loaded;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(ui, row_height, rows.len(), |ui, visible_rows| {
                        for row in visible_rows {
                            let index = rows[row];
                            let path = self.playlist[index].clone();
                            let width = ui.available_width() - 4.0;
                            let (outer, response) = ui
                                .allocate_exact_size(Vec2::new(width, row_height), Sense::click());
                            let rect = outer.shrink2(Vec2::new(0.0, 2.0));
                            let is_current = self.current_index == Some(index);
                            let hovered = response.hovered();
                            if is_current {
                                ui.painter().rect_filled(rect, 12.0, palette.accent_soft);
                            } else if hovered {
                                ui.painter().rect_filled(rect, 12.0, palette.card);
                            }

                            let kind = kind_by_extension(&path);
                            let thumb = Rect::from_min_size(
                                Pos2::new(rect.left() + 6.0, rect.center().y - 22.0),
                                Vec2::splat(44.0),
                            );
                            match self.thumbnail_for(&path) {
                                Some(texture) => {
                                    paint_texture_cover(ui, &texture, thumb, 9);
                                    if kind == MediaKind::Video {
                                        let badge = Rect::from_min_size(
                                            thumb.right_bottom() - Vec2::new(17.0, 17.0),
                                            Vec2::splat(15.0),
                                        );
                                        ui.painter().rect_filled(
                                            badge,
                                            4.0,
                                            Color32::from_black_alpha(170),
                                        );
                                        ui.painter().text(
                                            badge.center(),
                                            Align2::CENTER_CENTER,
                                            icon::PLAY,
                                            icon_font(8.0),
                                            Color32::WHITE,
                                        );
                                    }
                                }
                                None => paint_kind_tile(ui.painter(), &palette, thumb, kind),
                            }

                            let name = path
                                .file_stem()
                                .and_then(|value| value.to_str())
                                .unwrap_or("Файл");
                            let extension = path
                                .extension()
                                .and_then(|value| value.to_str())
                                .map(|value| value.to_ascii_uppercase())
                                .unwrap_or_default();
                            let subtitle = if extension.is_empty() {
                                kind.label().to_string()
                            } else {
                                format!("{} · {extension}", kind.label())
                            };
                            let text_left = thumb.right() + 12.0;
                            let text_width = (rect.right() - 36.0 - text_left).max(20.0);
                            let title_color = if is_current {
                                palette.accent
                            } else {
                                palette.text
                            };
                            let title = truncated(
                                ui,
                                name,
                                if is_current {
                                    semibold(13.5)
                                } else {
                                    FontId::proportional(13.5)
                                },
                                title_color,
                                text_width,
                            );
                            let sub = truncated(
                                ui,
                                &subtitle,
                                FontId::proportional(11.5),
                                palette.text_faint,
                                text_width,
                            );
                            ui.painter().galley(
                                Pos2::new(text_left, rect.center().y - title.size().y),
                                title,
                                title_color,
                            );
                            ui.painter().galley(
                                Pos2::new(text_left, rect.center().y + 2.0),
                                sub,
                                palette.text_faint,
                            );

                            let action = Rect::from_center_size(
                                Pos2::new(rect.right() - 18.0, rect.center().y),
                                Vec2::splat(26.0),
                            );
                            let remove_response = ui.interact(
                                action,
                                ui.id().with(("playlist_remove", index)),
                                Sense::click(),
                            );
                            if hovered || remove_response.hovered() {
                                if remove_response.hovered() {
                                    ui.painter().circle_filled(
                                        action.center(),
                                        13.0,
                                        palette.card_hover,
                                    );
                                }
                                ui.painter().text(
                                    action.center(),
                                    Align2::CENTER_CENTER,
                                    icon::CLOSE,
                                    icon_font(10.0),
                                    if remove_response.hovered() {
                                        palette.danger
                                    } else {
                                        palette.text_faint
                                    },
                                );
                            } else if is_current
                                && kind != MediaKind::Image
                                && self.load_failed.is_none()
                            {
                                paint_equalizer(ui, action.center(), palette.accent, playing);
                            }
                            if remove_response.clicked() {
                                remove = Some(index);
                            } else if response.clicked() && !is_current {
                                selected = Some(index);
                            }
                            let response = response.on_hover_text(path.display().to_string());
                            response.context_menu(|ui| {
                                if ui.button("Открыть").clicked() {
                                    selected = Some(index);
                                    ui.close();
                                }
                                if ui.button("Показать в Проводнике").clicked() {
                                    reveal = Some(path.clone());
                                    ui.close();
                                }
                                if ui.button("Конвертировать…").clicked() {
                                    convert_request = Some(path.clone());
                                    ui.close();
                                }
                                if ui.button("Копировать путь").clicked() {
                                    ui.ctx().copy_text(path.display().to_string());
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button("Убрать из плейлиста").clicked() {
                                    remove = Some(index);
                                    ui.close();
                                }
                            });
                        }
                    });
                if let Some(index) = selected {
                    self.select_index(index);
                }
                if let Some(index) = remove {
                    self.remove_playlist_index(index);
                }
                if let Some(path) = reveal {
                    reveal_in_explorer(&path);
                }
                if let Some(path) = convert_request {
                    self.open_converter(path);
                }
            });
    }

    fn empty_playlist(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
            ui.painter()
                .circle_filled(rect.center(), 28.0, palette.card);
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                icon::LIST,
                icon_font(20.0),
                palette.text_faint,
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new("Плейлист пуст")
                    .font(semibold(15.0))
                    .color(palette.text),
            );
            ui.label(
                RichText::new("Перетащите файлы или папку в окно")
                    .size(12.5)
                    .color(palette.text_faint),
            );
            ui.add_space(10.0);
            if action_button(ui, palette, Some(icon::ADD), "Добавить файлы", false).clicked()
            {
                self.add_files_dialog();
            }
        });
    }

    fn viewer(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let palette = self.palette();
        egui::CentralPanel::default()
            .frame(Frame::new().fill(palette.viewer))
            .show(ui, |ui| {
                let rect = ui.max_rect();
                let response = ui.allocate_rect(rect, Sense::click_and_drag());
                self.video_rect = Some(rect);

                if self.current_index.is_none() {
                    self.draw_empty_state(ui, rect, &palette);
                } else if let Some(error) = self.load_failed.clone() {
                    self.draw_failure(ui, rect, &palette, &error);
                } else if self.current_is_image() {
                    self.draw_image(ui, rect, &response, &palette);
                } else if self.current_kind == MediaKind::Audio
                    || (self.media_loaded && !self.has_video && self.backend_loaded_without_video())
                {
                    self.draw_audio(ui, rect, &palette);
                } else if !self.media_loaded {
                    draw_loading(ui, rect, &palette, &self.current_title);
                }

                let media_view = self.current_index.is_some()
                    && !self.current_is_image()
                    && self.load_failed.is_none();
                if media_view {
                    self.handle_media_zoom(ui, &response);
                }
                if response.double_clicked() && self.current_index.is_some() {
                    self.set_fullscreen(ctx, !self.fullscreen);
                } else if response.clicked()
                    && self.current_index.is_some()
                    && !self.current_is_image()
                    && self.load_failed.is_none()
                {
                    self.toggle_playback();
                }
            });
    }

    /// Колесо — масштаб к курсору, перетаскивание — сдвиг (как у фото).
    fn handle_media_zoom(&mut self, ui: &egui::Ui, response: &egui::Response) {
        if response.hovered() {
            let wheel = wheel_steps(ui);
            if wheel.abs() > 0.0 {
                let steps = (wheel / 50.0).clamp(-3.0, 3.0);
                let steps = if steps.abs() < 1.0 {
                    steps.signum()
                } else {
                    steps
                };
                self.zoom_media(1.15f32.powf(steps), response.hover_pos());
            }
        }
        if self.media_zoom > 1.0 {
            if response.dragged() {
                self.media_pan += response.drag_delta();
                self.clamp_media_pan();
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
        }
    }

    fn backend_loaded_without_video(&self) -> bool {
        match self.backend {
            Backend::Mpv => self.mpv.as_ref().is_some_and(MpvPlayer::is_loaded),
            Backend::Windows => self.windows_video_checked,
            _ => false,
        }
    }

    fn draw_empty_state(&mut self, ui: &mut egui::Ui, rect: Rect, palette: &Palette) {
        let center = rect.center();
        let logo_center = Pos2::new(center.x, center.y - 110.0);
        paint_glow(ui.painter(), logo_center, 120.0, palette.accent);
        paint_logo(
            ui.painter(),
            &self.logo,
            Rect::from_center_size(logo_center, Vec2::splat(96.0)),
        );
        let light = Color32::from_rgb(236, 238, 243);
        let weak = Color32::from_rgb(150, 157, 171);
        ui.painter().text(
            Pos2::new(center.x, center.y - 34.0),
            Align2::CENTER_CENTER,
            "Откройте видео, музыку или фото",
            semibold(26.0),
            light,
        );
        ui.painter().text(
            Pos2::new(center.x, center.y + 2.0),
            Align2::CENTER_CENTER,
            "Перетащите файлы или папку в окно — или выберите их",
            FontId::proportional(15.0),
            weak,
        );
        let buttons =
            Rect::from_center_size(Pos2::new(center.x, center.y + 56.0), Vec2::new(360.0, 44.0));
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(buttons)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let total = 174.0 + 166.0 + 10.0;
                ui.add_space(((buttons.width() - total) / 2.0).max(0.0));
                if action_button(ui, palette, Some(icon::OPEN_FILE), "Открыть файлы", true)
                    .clicked()
                {
                    self.open_files_dialog();
                }
                if action_button(ui, palette, Some(icon::FOLDER), "Открыть папку", false).clicked()
                {
                    self.open_folder_dialog();
                }
            },
        );
        let faint = Color32::from_rgb(105, 113, 129);
        ui.painter().text(
            Pos2::new(center.x, center.y + 112.0),
            Align2::CENTER_CENTER,
            "Ctrl+O — файлы   ·   Ctrl+Shift+O — папка   ·   Ctrl+V — картинка из буфера",
            FontId::proportional(12.5),
            faint,
        );
        let engine = if mpv::is_available() {
            "Все форматы: MP4 · MKV · AVI · MOV · WEBM · FLV · TS · MP3 · FLAC · OPUS · APE · JPG · PNG · HEIC · AVIF · RAW · PSD…"
        } else {
            "libmpv-2.dll не найдена — часть видео- и аудиоформатов недоступна"
        };
        ui.painter().text(
            Pos2::new(center.x, rect.bottom() - 26.0),
            Align2::CENTER_CENTER,
            engine,
            FontId::proportional(12.0),
            if mpv::is_available() {
                faint
            } else {
                palette.danger
            },
        );
    }

    fn draw_failure(&mut self, ui: &mut egui::Ui, rect: Rect, palette: &Palette, error: &str) {
        let card_width = (rect.width() - 80.0).clamp(280.0, 560.0);
        let painter = ui.painter();
        let detail = painter.layout(
            error.to_string(),
            FontId::proportional(12.5),
            Color32::from_rgb(160, 167, 181),
            card_width - 56.0,
        );
        let detail_height = detail.size().y.min(180.0);
        let card =
            Rect::from_center_size(rect.center(), Vec2::new(card_width, 222.0 + detail_height));
        let card_fill = Color32::from_rgb(22, 24, 30);
        painter.add(
            egui::Shadow {
                offset: [0, 16],
                blur: 48,
                spread: 0,
                color: Color32::from_black_alpha(120),
            }
            .as_shape(card, CornerRadius::same(18)),
        );
        painter.rect_filled(card, 18.0, card_fill);
        painter.rect_stroke(
            card,
            18.0,
            Stroke::new(1.0, Color32::from_rgb(40, 44, 54)),
            egui::StrokeKind::Inside,
        );
        let icon_center = Pos2::new(card.center().x, card.top() + 44.0);
        painter.circle_filled(icon_center, 24.0, palette.danger.gamma_multiply(0.16));
        painter.text(
            icon_center,
            Align2::CENTER_CENTER,
            if self.blocked {
                icon::SHIELD
            } else {
                icon::ERROR
            },
            icon_font(20.0),
            palette.danger,
        );
        painter.text(
            Pos2::new(card.center().x, card.top() + 90.0),
            Align2::CENTER_CENTER,
            if self.blocked {
                "Файл заблокирован защитой"
            } else {
                "Не удалось открыть файл"
            },
            semibold(18.0),
            Color32::from_rgb(236, 238, 243),
        );
        let name = truncated(
            ui,
            &self.current_title,
            FontId::proportional(13.0),
            Color32::from_rgb(160, 167, 181),
            card_width - 56.0,
        );
        painter.galley(
            Pos2::new(card.center().x - name.size().x / 2.0, card.top() + 106.0),
            name,
            Color32::WHITE,
        );
        let detail_rect = Rect::from_min_size(
            Pos2::new(card.left() + 28.0, card.top() + 134.0),
            Vec2::new(card_width - 56.0, detail_height),
        );
        painter
            .with_clip_rect(detail_rect)
            .galley(detail_rect.min, detail, Color32::WHITE);
        let buttons = Rect::from_min_max(
            Pos2::new(card.left() + 20.0, card.bottom() - 60.0),
            Pos2::new(card.right() - 20.0, card.bottom() - 16.0),
        );
        let path = self.current_path().cloned();
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(buttons)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add_space(((buttons.width() - 360.0) / 2.0).max(0.0));
                if self.playlist.len() > 1
                    && action_button(ui, palette, Some(icon::NEXT), "Следующий файл", true)
                        .clicked()
                {
                    self.next_track();
                }
                if let Some(path) = path
                    && action_button(
                        ui,
                        palette,
                        Some(icon::FOLDER_OPEN),
                        "Показать в папке",
                        false,
                    )
                    .clicked()
                {
                    reveal_in_explorer(&path);
                }
            },
        );
    }

    fn draw_image(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        response: &egui::Response,
        palette: &Palette,
    ) {
        if self.image.loading {
            draw_loading(ui, rect, palette, &self.current_title);
            return;
        }
        let Some(texture) = self.image.current_texture().cloned() else {
            return;
        };

        // Колесо — масштаб относительно курсора, перетаскивание — сдвиг.
        if response.hovered() {
            let wheel = ui.input(|input| {
                input
                    .events
                    .iter()
                    .filter_map(|event| match event {
                        egui::Event::MouseWheel {
                            unit,
                            delta,
                            modifiers,
                            ..
                        } if !modifiers.ctrl => Some(match unit {
                            egui::MouseWheelUnit::Point => delta.y / 60.0,
                            egui::MouseWheelUnit::Line => delta.y,
                            egui::MouseWheelUnit::Page => delta.y * 3.0,
                        }),
                        _ => None,
                    })
                    .sum::<f32>()
            });
            if wheel.abs() > 0.0 {
                let anchor = response.hover_pos();
                self.zoom_image(1.18f32.powf(wheel), anchor);
            }
        }
        if response.dragged() {
            self.image.pan += response.drag_delta();
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if response.hovered() && self.image.zoom > 1.0 {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        let fit = self.image_fit_scale(rect);
        let scale = fit * self.image.zoom;
        let source = Vec2::new(self.image.size[0] as f32, self.image.size[1] as f32);
        let rotated = self.rotated_image_size() * scale;
        let center = rect.center() + self.image.pan;
        let bounds = Rect::from_center_size(center, rotated);
        let painter = ui.painter().with_clip_rect(rect);
        if self.image.has_alpha {
            paint_checkerboard(&painter, bounds.intersect(rect), 12.0);
        } else {
            painter.add(
                egui::Shadow {
                    offset: [0, 10],
                    blur: 40,
                    spread: 0,
                    color: Color32::from_black_alpha(110),
                }
                .as_shape(bounds, CornerRadius::ZERO),
            );
        }
        let unrotated = Rect::from_center_size(center, source * scale);
        let angle = self.image.rotation as f32 * std::f32::consts::FRAC_PI_2;
        let mut ui_clip = ui.new_child(UiBuilder::new().max_rect(rect));
        ui_clip.set_clip_rect(rect);
        egui::Image::new(&texture)
            .rotate(angle, Vec2::splat(0.5))
            .paint_at(&ui_clip, unrotated);
    }

    fn draw_audio(&mut self, ui: &mut egui::Ui, rect: Rect, palette: &Palette) {
        let painter = ui.painter().with_clip_rect(rect);
        if let Some(backdrop) = &self.cover_backdrop {
            let side = rect.width().max(rect.height()) * 1.2;
            let backdrop_rect = Rect::from_center_size(rect.center(), Vec2::splat(side));
            painter.image(
                backdrop.id(),
                backdrop_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::from_white_alpha(150),
            );
        } else {
            paint_glow(
                &painter,
                rect.center(),
                rect.height() * 0.6,
                palette.audio_tint,
            );
        }
        paint_vertical_gradient(
            &painter,
            rect,
            palette.viewer.gamma_multiply(0.55),
            palette.viewer.gamma_multiply(0.92),
        );

        let side = (rect.height() * 0.52)
            .min(rect.width() * 0.5)
            .clamp(96.0, 440.0)
            * self.media_zoom.clamp(1.0, 2.0);
        let cover = Rect::from_center_size(
            Pos2::new(rect.center().x, rect.center().y - 40.0),
            Vec2::splat(side),
        );
        painter.add(
            egui::Shadow {
                offset: [0, 18],
                blur: 60,
                spread: 0,
                color: Color32::from_black_alpha(150),
            }
            .as_shape(cover, CornerRadius::same(18)),
        );
        match &self.cover_texture {
            Some(texture) => {
                let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
                child.set_clip_rect(rect);
                paint_texture_cover(&child, texture, cover, 18);
            }
            None => {
                painter.rect_filled(cover, 18.0, Color32::from_rgb(28, 31, 38));
                painter.text(
                    cover.center(),
                    Align2::CENTER_CENTER,
                    icon::MUSIC,
                    icon_font((side * 0.3).round()),
                    palette.audio_tint.gamma_multiply(0.85),
                );
                if self.cover_loading {
                    painter.text(
                        Pos2::new(cover.center().x, cover.bottom() - 24.0),
                        Align2::CENTER_CENTER,
                        "Поиск обложки…",
                        FontId::proportional(12.0),
                        Color32::from_rgb(150, 157, 171),
                    );
                }
            }
        }
        paint_brightness_overlay(&painter, cover, self.settings.brightness, 18.0);

        let title = if self.current_kind == MediaKind::Audio && !self.cover_title.trim().is_empty()
        {
            self.cover_title.clone()
        } else {
            self.current_title.clone()
        };
        let width = (rect.width() - 80.0).max(100.0);
        let title_galley = truncated(ui, &title, semibold(24.0), Color32::WHITE, width);
        painter.galley(
            Pos2::new(
                rect.center().x - title_galley.size().x / 2.0,
                cover.bottom() + 26.0,
            ),
            title_galley,
            Color32::WHITE,
        );
        if let Some(artist) = &self.cover_artist {
            let artist_galley = truncated(
                ui,
                artist,
                FontId::proportional(16.0),
                Color32::from_rgb(190, 196, 208),
                width,
            );
            painter.galley(
                Pos2::new(
                    rect.center().x - artist_galley.size().x / 2.0,
                    cover.bottom() + 62.0,
                ),
                artist_galley,
                Color32::WHITE,
            );
        }
        if let Some(source) = &self.cover_source {
            painter.text(
                Pos2::new(rect.center().x, rect.bottom() - 16.0),
                Align2::CENTER_BOTTOM,
                source,
                FontId::proportional(11.5),
                Color32::from_rgb(120, 127, 141),
            );
        }
        if !self.media_loaded {
            ui.put(
                Rect::from_center_size(
                    Pos2::new(rect.center().x, cover.bottom() + 100.0),
                    Vec2::splat(22.0),
                ),
                egui::Spinner::new().size(20.0).color(palette.accent),
            );
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let palette = self.palette();
        let card = |ui: &mut egui::Ui, title: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
            ui.label(
                RichText::new(title)
                    .font(semibold(13.0))
                    .color(palette.text_weak),
            );
            ui.add_space(2.0);
            Frame::new()
                .fill(palette.card)
                .corner_radius(14)
                .inner_margin(Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    add(ui);
                });
            ui.add_space(14.0);
        };

        let screen = ctx.content_rect();
        let response = egui::Modal::new(egui::Id::new("settings_modal"))
            .backdrop_color(Color32::from_black_alpha(140))
            .frame(
                Frame::new()
                    .fill(palette.surface)
                    .corner_radius(20)
                    .stroke(Stroke::new(1.0, palette.outline))
                    .inner_margin(Margin::same(22))
                    .shadow(ctx.global_style().visuals.window_shadow),
            )
            .show(ctx, |ui| {
                ui.set_width(540.0_f32.min(screen.width() - 80.0));
                let mut close = false;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Настройки").font(semibold(22.0)).color(palette.text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, &palette, icon::CLOSE, 34.0, false, true, "Закрыть (Esc)").clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(12.0);
                egui::ScrollArea::vertical()
                    .max_height((screen.height() - 200.0).max(240.0))
                    .min_scrolled_height((screen.height() - 220.0).clamp(240.0, 600.0))
                    .show(ui, |ui| {
                        card(ui, "ОФОРМЛЕНИЕ", &mut |ui| {
                            ui.label(RichText::new("Тема").color(palette.text_weak).size(12.5));
                            if segmented(
                                ui,
                                &palette,
                                &mut self.settings.theme,
                                &[
                                    (ThemeMode::System, "Как в Windows"),
                                    (ThemeMode::Dark, "Тёмная"),
                                    (ThemeMode::Light, "Светлая"),
                                ],
                            ) {
                                self.mark_dirty();
                            }
                            ui.add_space(8.0);
                            ui.label(RichText::new("Акцентный цвет").color(palette.text_weak).size(12.5));
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 10.0;
                                for (index, accent) in ACCENTS.iter().enumerate() {
                                    let color = if palette.dark { accent.dark } else { accent.light };
                                    let (rect, response) =
                                        ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
                                    let selected = self.settings.accent == index;
                                    ui.painter().circle_filled(rect.center(), 13.0, color);
                                    if selected {
                                        ui.painter().circle_stroke(
                                            rect.center(),
                                            15.5,
                                            Stroke::new(2.0, palette.text),
                                        );
                                        ui.painter().text(
                                            rect.center(),
                                            Align2::CENTER_CENTER,
                                            icon::CHECK,
                                            icon_font(11.0),
                                            Color32::WHITE,
                                        );
                                    } else if response.hovered() {
                                        ui.painter().circle_stroke(
                                            rect.center(),
                                            15.5,
                                            Stroke::new(1.5, palette.outline_strong),
                                        );
                                    }
                                    if response.on_hover_text(accent.name).clicked() {
                                        self.settings.accent = index;
                                        self.mark_dirty();
                                    }
                                }
                            });
                            ui.add_space(6.0);
                            if toggle(ui, &palette, &mut self.settings.always_on_top, "Поверх остальных окон").changed() {
                                self.mark_dirty();
                            }
                            if toggle(ui, &palette, &mut self.settings.show_playlist, "Показывать плейлист").changed() {
                                self.mark_dirty();
                            }
                        });
                        card(ui, "ВОСПРОИЗВЕДЕНИЕ", &mut |ui| {
                            if toggle(ui, &palette, &mut self.settings.autoplay_next, "Автоматически открывать следующий файл").changed() {
                                self.mark_dirty();
                            }
                            if toggle(ui, &palette, &mut self.settings.shuffle, "Случайный порядок").changed() {
                                self.mark_dirty();
                            }
                            if toggle(ui, &palette, &mut self.settings.remember_position, "Запоминать позицию видео и музыки").changed() {
                                self.mark_dirty();
                            }
                            if toggle(ui, &palette, &mut self.settings.remember_playlist, "Запоминать плейлист").changed() {
                                self.mark_dirty();
                            }
                        });
                        card(ui, "ДВИЖОК ВИДЕО", &mut |ui| {
                            if segmented(
                                ui,
                                &palette,
                                &mut self.settings.video_engine,
                                &[
                                    (VideoEngine::Mpv, "mpv (FFmpeg)"),
                                    (VideoEngine::MediaFoundation, "Кодеки Windows"),
                                ],
                            ) {
                                self.mark_dirty();
                            }
                            ui.add_space(6.0);
                            let available = mpv::is_available();
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(if available { icon::CHECK } else { icon::ERROR })
                                        .font(icon_font(12.0))
                                        .color(if available { palette.accent } else { palette.danger }),
                                );
                                ui.label(RichText::new(mpv::describe()).size(12.0).color(palette.text_weak));
                            });
                            ui.label(
                                RichText::new(
                                    "Если выбранный движок не открыл файл, AnMedia автоматически \
                                     пробует второй. Музыка сначала играет встроенным декодером.",
                                )
                                .size(12.0)
                                .color(palette.text_faint),
                            );
                        });
                        card(ui, "ФОРМАТЫ И WINDOWS", &mut |ui| {
                            ui.label(
                                RichText::new("Дополнительные расширения через запятую")
                                    .size(12.5)
                                    .color(palette.text_weak),
                            );
                            let edit = egui::TextEdit::singleline(&mut self.settings.custom_extensions)
                                .hint_text("например: m4p, bik")
                                .desired_width(f32::INFINITY);
                            if ui.add(edit).changed() {
                                self.mark_dirty();
                            }
                            ui.add_space(6.0);
                            ui.horizontal_wrapped(|ui| {
                                if ui.button("Зарегистрировать в Windows").clicked() {
                                    self.register_associations();
                                }
                                if ui.button("Приложения по умолчанию").clicked()
                                    && let Err(error) = open_default_apps_settings()
                                {
                                    self.status = error;
                                }
                                if ui.button("Обновить превью Проводника").clicked() {
                                    match rebuild_shell_preview_cache() {
                                        Ok(()) => {
                                            self.status =
                                                "Кэш иконок и миниатюр Windows обновлён".to_string();
                                        }
                                        Err(error) => self.status = error,
                                    }
                                }
                            });
                        });
                        card(ui, "БЕЗОПАСНОСТЬ", &mut |ui| {
                            if toggle(
                                ui,
                                &palette,
                                &mut self.settings.block_suspicious,
                                "Блокировать программы, замаскированные под медиа",
                            )
                            .changed()
                            {
                                self.mark_dirty();
                            }
                            if toggle(
                                ui,
                                &palette,
                                &mut self.settings.online_covers,
                                "Искать обложки в интернете",
                            )
                            .changed()
                            {
                                self.mark_dirty();
                            }
                            ui.add_space(4.0);
                            for line in security::applied_protections() {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(icon::SHIELD)
                                            .font(icon_font(12.0))
                                            .color(palette.accent),
                                    );
                                    ui.label(RichText::new(*line).size(12.5).color(palette.text_weak));
                                });
                            }
                        });
                        card(ui, "СБРОС", &mut |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("Вернуть все настройки по умолчанию")
                                        .color(palette.text),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let button = egui::Button::new(
                                        RichText::new("Сбросить").color(palette.danger),
                                    )
                                    .stroke(Stroke::new(1.0, palette.danger.gamma_multiply(0.5)))
                                    .fill(Color32::TRANSPARENT);
                                    if ui.add(button).clicked() {
                                        self.reset_settings();
                                    }
                                });
                            });
                        });
                        ui.label(
                            RichText::new("AnMedia 2.0 · Rust + egui · libmpv / Media Foundation / Symphonia / WIC")
                                .size(11.5)
                                .color(palette.text_faint),
                        );
                    });
                close
            });
        if response.inner || response.should_close() {
            self.show_settings = false;
        }
    }

    fn association_prompt(&mut self, ctx: &egui::Context) {
        if !self.show_association_prompt || self.show_settings {
            return;
        }
        let palette = self.palette();
        egui::Modal::new(egui::Id::new("association_prompt"))
            .backdrop_color(Color32::from_black_alpha(140))
            .frame(
                Frame::new()
                    .fill(palette.surface)
                    .corner_radius(20)
                    .stroke(Stroke::new(1.0, palette.outline))
                    .inner_margin(Margin::same(24)),
            )
            .show(ctx, |ui| {
                ui.set_width(440.0);
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(52.0), Sense::hover());
                paint_logo(ui.painter(), &self.logo, rect);
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Открывать медиафайлы в AnMedia?")
                        .font(semibold(19.0))
                        .color(palette.text),
                );
                ui.label(
                    RichText::new(
                        "AnMedia появится в списке приложений Windows для фото, видео и музыки. \
                         Выбрать её по умолчанию можно в настройках Windows.",
                    )
                    .color(palette.text_weak),
                );
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if action_button(ui, &palette, None, "Открыть настройки", true).clicked()
                    {
                        self.register_associations();
                        let _ = open_default_apps_settings();
                        self.settings.association_prompt_done = true;
                        self.show_association_prompt = false;
                        self.mark_dirty();
                    }
                    if action_button(ui, &palette, None, "Не сейчас", false).clicked() {
                        self.settings.association_prompt_done = true;
                        self.show_association_prompt = false;
                        self.mark_dirty();
                    }
                });
                ui.add_space(6.0);
                if ui
                    .add(
                        egui::Label::new(
                            RichText::new("Больше не спрашивать")
                                .size(12.5)
                                .color(palette.text_faint),
                        )
                        .sense(Sense::click()),
                    )
                    .clicked()
                {
                    self.settings.association_prompt_disabled = true;
                    self.show_association_prompt = false;
                    self.mark_dirty();
                }
            });
    }

    fn open_converter(&mut self, path: PathBuf) {
        if self.convert_job.is_some() {
            self.show_convert = true;
            return;
        }
        let presets = convert::presets_for(detect_kind(&path));
        if !presets
            .iter()
            .any(|preset| preset.id == self.convert_preset)
        {
            self.convert_preset = presets.first().map(|preset| preset.id).unwrap_or("mp4");
        }
        self.convert_source = Some(path);
        self.convert_output = None;
        self.convert_result = None;
        self.show_convert = true;
    }

    fn poll_convert(&mut self) {
        let Some(result) = self.convert_job.as_ref().and_then(Job::try_result) else {
            return;
        };
        self.convert_job = None;
        match result {
            JobResult::Done(path) => {
                self.status = format!(
                    "Готово: {}",
                    path.file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or("файл")
                );
                self.convert_result = Some(Ok(path));
            }
            JobResult::Cancelled => {
                self.status = "Конвертация отменена".to_string();
                self.convert_result = None;
            }
            JobResult::Failed(error) => {
                self.status = format!("Не удалось конвертировать: {error}");
                self.convert_result = Some(Err(error));
            }
        }
    }

    fn convert_window(&mut self, ctx: &egui::Context) {
        if !self.show_convert {
            return;
        }
        let Some(source) = self.convert_source.clone() else {
            self.show_convert = false;
            return;
        };
        let palette = self.palette();
        let kind = detect_kind(&source);
        let presets = convert::presets_for(kind);
        let Some(preset) = convert::preset(self.convert_preset) else {
            self.show_convert = false;
            return;
        };
        let output = self
            .convert_output
            .clone()
            .unwrap_or_else(|| convert::default_output(&source, preset));
        let source_name = source
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("файл")
            .to_string();
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("convert_modal"))
            .backdrop_color(Color32::from_black_alpha(140))
            .frame(
                Frame::new()
                    .fill(palette.surface)
                    .corner_radius(20)
                    .stroke(Stroke::new(1.0, palette.outline))
                    .inner_margin(Margin::same(22))
                    .shadow(ctx.global_style().visuals.window_shadow),
            )
            .show(ctx, |ui| {
                ui.set_width(600.0_f32.min(ctx.content_rect().width() - 80.0));
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Конвертация")
                            .font(semibold(22.0))
                            .color(palette.text),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(
                            ui,
                            &palette,
                            icon::CLOSE,
                            34.0,
                            false,
                            true,
                            "Закрыть (Esc)",
                        )
                        .clicked()
                        {
                            close = true;
                        }
                    });
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let (tile, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                    paint_kind_tile(ui.painter(), &palette, tile, kind);
                    ui.vertical(|ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&source_name)
                                    .font(semibold(14.0))
                                    .color(palette.text),
                            )
                            .truncate(),
                        );
                        ui.label(
                            RichText::new(format!("→ {} · {}", preset.label, preset.description))
                                .size(12.5)
                                .color(palette.text_weak),
                        );
                    });
                });
                ui.add_space(12.0);

                if let Some(job) = &self.convert_job {
                    let progress = job.progress();
                    ui.label(
                        RichText::new(format!(
                            "Конвертация в {}… {:.0}%",
                            job.preset_label,
                            progress * 100.0
                        ))
                        .color(palette.text),
                    );
                    ui.add_space(6.0);
                    let (bar, _) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 10.0), Sense::hover());
                    ui.painter().rect_filled(bar, 5.0, palette.seek_track);
                    ui.painter().rect_filled(
                        Rect::from_min_size(
                            bar.min,
                            Vec2::new(bar.width() * progress.max(0.02), bar.height()),
                        ),
                        5.0,
                        palette.accent,
                    );
                    ui.add_space(6.0);
                    let output_text = job.output.display().to_string();
                    ui.label(
                        RichText::new(output_text)
                            .size(12.0)
                            .color(palette.text_faint),
                    );
                    ui.add_space(12.0);
                    let mut cancel = false;
                    ui.horizontal(|ui| {
                        if action_button(ui, &palette, Some(icon::CLOSE), "Остановить", false)
                            .clicked()
                        {
                            cancel = true;
                        }
                        ui.label(
                            RichText::new("Окно можно закрыть — конвертация продолжится")
                                .size(12.0)
                                .color(palette.text_faint),
                        );
                    });
                    if cancel {
                        job.cancel();
                    }
                    return;
                }

                if let Some(Ok(path)) = self.convert_result.clone() {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(icon::CHECK)
                                .font(icon_font(16.0))
                                .color(palette.accent),
                        );
                        ui.label(
                            RichText::new("Файл сохранён")
                                .font(semibold(16.0))
                                .color(palette.text),
                        );
                    });
                    ui.label(
                        RichText::new(path.display().to_string())
                            .size(12.5)
                            .color(palette.text_weak),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if action_button(ui, &palette, Some(icon::PLAY), "Открыть", true).clicked()
                        {
                            self.add_paths(vec![path.clone()], false);
                            if let Some(index) = self.playlist.iter().position(|candidate| {
                                path_identity(candidate) == path_identity(&path)
                            }) {
                                self.select_index(index);
                            }
                            close = true;
                        }
                        if action_button(
                            ui,
                            &palette,
                            Some(icon::FOLDER_OPEN),
                            "Показать в папке",
                            false,
                        )
                        .clicked()
                        {
                            reveal_in_explorer(&path);
                        }
                        if action_button(ui, &palette, Some(icon::CONVERT), "Другой формат", false)
                            .clicked()
                        {
                            self.convert_result = None;
                            self.convert_output = None;
                        }
                    });
                    return;
                }

                let groups = [
                    (TargetKind::Video, "ВИДЕО"),
                    (
                        TargetKind::Audio,
                        if kind == MediaKind::Audio {
                            "МУЗЫКА"
                        } else {
                            "ТОЛЬКО ЗВУК"
                        },
                    ),
                    (TargetKind::Image, "ИЗОБРАЖЕНИЕ"),
                ];
                for (group, title) in groups {
                    let items = presets
                        .iter()
                        .filter(|preset| preset.kind == group)
                        .collect::<Vec<_>>();
                    if items.is_empty() {
                        continue;
                    }
                    ui.label(
                        RichText::new(title)
                            .font(semibold(12.5))
                            .color(palette.text_weak),
                    );
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                        for item in items {
                            if format_card(
                                ui,
                                &palette,
                                item.label,
                                item.description,
                                item.id == self.convert_preset,
                            )
                            .clicked()
                            {
                                self.convert_preset = item.id;
                                self.convert_output = None;
                            }
                        }
                    });
                    ui.add_space(10.0);
                }

                let lossless = matches!(
                    preset.id,
                    "flac"
                        | "wav"
                        | "aiff"
                        | "png"
                        | "bmp"
                        | "tiff"
                        | "tga"
                        | "qoi"
                        | "webp"
                        | "ico"
                        | "gif-image"
                );
                if !lossless {
                    ui.label(
                        RichText::new("КАЧЕСТВО")
                            .font(semibold(12.5))
                            .color(palette.text_weak),
                    );
                    segmented(
                        ui,
                        &palette,
                        &mut self.convert_quality,
                        &[
                            (Quality::High, "Высокое"),
                            (Quality::Balanced, "Обычное"),
                            (Quality::Small, "Меньше размер"),
                        ],
                    );
                    ui.add_space(10.0);
                }

                ui.label(
                    RichText::new("СОХРАНИТЬ КАК")
                        .font(semibold(12.5))
                        .color(palette.text_weak),
                );
                ui.horizontal(|ui| {
                    let width = ui.available_width() - 110.0;
                    let galley = truncated(
                        ui,
                        &output.display().to_string(),
                        FontId::proportional(13.0),
                        palette.text,
                        width - 20.0,
                    );
                    let (field, _) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::hover());
                    ui.painter().rect_filled(field, 10.0, palette.field);
                    ui.painter().rect_stroke(
                        field,
                        10.0,
                        Stroke::new(1.0, palette.outline),
                        egui::StrokeKind::Inside,
                    );
                    ui.painter().galley(
                        Pos2::new(
                            field.left() + 10.0,
                            field.center().y - galley.size().y / 2.0,
                        ),
                        galley,
                        palette.text,
                    );
                    if ui
                        .add_sized([100.0, 34.0], egui::Button::new("Изменить…"))
                        .clicked()
                    {
                        let mut dialog = rfd::FileDialog::new()
                            .set_title("Сохранить как")
                            .add_filter(preset.label, &[preset.extension]);
                        if let Some(directory) = output.parent() {
                            dialog = dialog.set_directory(directory);
                        }
                        if let Some(name) = output.file_name().and_then(|value| value.to_str()) {
                            dialog = dialog.set_file_name(name);
                        }
                        if let Some(mut chosen) = dialog.save_file() {
                            if chosen.extension().is_none() {
                                chosen.set_extension(preset.extension);
                            }
                            self.convert_output = Some(chosen);
                        }
                    }
                });
                if let Some(Err(error)) = &self.convert_result {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!("Ошибка: {error}"))
                            .size(12.5)
                            .color(palette.danger),
                    );
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if action_button(ui, &palette, Some(icon::CONVERT), "Конвертировать", true)
                        .clicked()
                    {
                        self.convert_result = None;
                        self.convert_job = Some(convert::start(
                            &source,
                            &output,
                            preset,
                            self.convert_quality,
                        ));
                        self.convert_output = None;
                    }
                    if action_button(ui, &palette, None, "Отмена", false).clicked() {
                        close = true;
                    }
                });
            });
        if close || response.should_close() {
            self.show_convert = false;
            if self.convert_job.is_none() {
                self.convert_result = None;
            }
        }
    }

    fn register_associations(&mut self) {
        let Some(executable) = std::env::current_exe().ok() else {
            self.status = "Не удалось определить путь AnMedia.exe".to_string();
            return;
        };
        match register_file_associations(&executable, &self.settings.custom_extensions) {
            Ok(path) => self.status = format!("AnMedia зарегистрирована: {}", path.display()),
            Err(error) => self.status = format!("Ошибка регистрации: {error}"),
        }
    }

    fn update_media_host(&mut self, ctx: &egui::Context) {
        let hovering_files = ctx.input(|input| !input.raw.hovered_files.is_empty());
        let visible = matches!(self.backend, Backend::Mpv | Backend::Windows)
            && self.has_video
            && self.media_loaded
            && self.load_failed.is_none()
            && !self.show_settings
            && !self.show_convert
            && !(self.show_association_prompt && !self.settings.association_prompt_done)
            && !hovering_files;
        let Some(host) = self.media_host.as_mut() else {
            return;
        };
        host.set_visible(visible);
        if !visible {
            return;
        }
        let Some(rect) = self.video_rect else {
            return;
        };
        let scale = ctx.pixels_per_point();
        host.set_zoom(self.media_zoom);
        host.set_pan(
            (self.media_pan.x * scale).round() as i32,
            (self.media_pan.y * scale).round() as i32,
        );
        host.set_brightness(self.settings.brightness);
        host.set_bounds(
            (rect.min.x * scale).round() as i32,
            (rect.min.y * scale).round() as i32,
            (rect.width() * scale).round() as i32,
            (rect.height() * scale).round() as i32,
        );
        let surface_size = host.surface_size();
        if self.last_video_surface_size != Some(surface_size) {
            if self.backend == Backend::Windows
                && let Some(engine) = &self.native_media
            {
                engine.update_video_rect(surface_size.0, surface_size.1);
            }
            self.last_video_surface_size = Some(surface_size);
        }
    }

    fn store_current_position(&mut self) {
        if !self.settings.remember_position || self.current_is_image() || !self.media_loaded {
            return;
        }
        let Some(path) = self.current_path().cloned() else {
            return;
        };
        let position = self.current_time;
        if position > 1.0 && (self.duration <= 0.0 || position + 5.0 < self.duration) {
            self.settings
                .positions
                .insert(position_key(&path), position);
        } else {
            self.settings.positions.remove(&position_key(&path));
        }
        self.dirty = true;
    }

    fn clear_current_position(&mut self) {
        if let Some(path) = self.current_path() {
            self.settings.positions.remove(&position_key(path));
            self.dirty = true;
        }
    }

    fn sync_playlist_to_settings(&mut self) {
        if self.settings.remember_playlist {
            self.settings.playlist = self.playlist.clone();
            self.settings.playlist_index = self.current_index;
        } else {
            self.settings.playlist.clear();
            self.settings.playlist_index = None;
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn save_if_needed(&mut self) {
        if !self.dirty || self.last_save.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.sync_playlist_to_settings();
        match self.settings.save() {
            Ok(()) => self.dirty = false,
            Err(error) => self.status = format!("Не удалось сохранить настройки: {error}"),
        }
        self.last_save = Instant::now();
    }

    fn update_toast(&mut self) {
        if self.status == self.shown_status {
            return;
        }
        self.shown_status = self.status.clone();
        let quiet = ["Открыто:", "Загрузка", "Открытие:", "Готово", "Буферизация"];
        if quiet.iter().any(|prefix| self.status.starts_with(prefix)) {
            return;
        }
        self.toast = Some((self.status.clone(), Instant::now()));
    }

    fn draw_drop_overlay(&self, ctx: &egui::Context) {
        let hovering = ctx.input(|input| !input.raw.hovered_files.is_empty());
        if !hovering {
            return;
        }
        let palette = self.palette();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop_overlay"),
        ));
        let screen = ctx.content_rect();
        painter.rect_filled(screen, 0.0, palette.background.gamma_multiply(0.85));
        let zone = screen.shrink(18.0);
        painter.rect_filled(zone, 22.0, palette.accent.gamma_multiply(0.08));
        painter.rect_stroke(
            zone,
            22.0,
            Stroke::new(2.0, palette.accent),
            egui::StrokeKind::Inside,
        );
        painter.circle_filled(zone.center() - Vec2::new(0.0, 40.0), 34.0, palette.accent);
        painter.text(
            zone.center() - Vec2::new(0.0, 40.0),
            Align2::CENTER_CENTER,
            icon::ADD,
            icon_font(24.0),
            palette.on_accent,
        );
        painter.text(
            zone.center() + Vec2::new(0.0, 22.0),
            Align2::CENTER_CENTER,
            "Отпустите, чтобы открыть",
            semibold(22.0),
            palette.text,
        );
        painter.text(
            zone.center() + Vec2::new(0.0, 52.0),
            Align2::CENTER_CENTER,
            "Файлы и папки заменят текущий плейлист",
            FontId::proportional(13.5),
            palette.text_weak,
        );
    }

    fn sync_window_title(&mut self, ctx: &egui::Context) {
        let desired = if self.current_title.is_empty() {
            "AnMedia".to_string()
        } else {
            format!("{} — AnMedia", self.current_title)
        };
        if self.applied_title.as_deref() != Some(desired.as_str()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(desired.clone()));
            self.applied_title = Some(desired);
        }
    }

    fn update_system_theme(&mut self) {
        if self.settings.theme != ThemeMode::System
            || self.last_theme_check.elapsed() < Duration::from_secs(2)
        {
            return;
        }
        self.last_theme_check = Instant::now();
        self.system_dark = detect_system_dark();
    }
}

impl eframe::App for AnMediaApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.texture_side_known {
            self.texture_side_known = true;
            set_max_texture_side(ctx.input(|input| input.max_texture_side));
        }
        self.update_system_theme();
        self.sync_theme(&ctx);
        self.ensure_host(frame);
        if self.pending_initial_load {
            self.pending_initial_load = false;
            self.load_current();
        }
        self.process_mpv_events();
        self.process_native_events();
        self.poll_playback();
        self.process_image_results(&ctx);
        self.process_cover_results(&ctx);
        self.process_thumbnail_results(&ctx);
        self.process_instance_requests();
        self.process_folder_scan();
        self.process_dropped_files(&ctx);
        self.track_pointer_activity(&ctx);
        self.handle_keyboard(&ctx);
        self.advance_animation(&ctx);
        self.poll_convert();
        crate::cover_art::ONLINE_COVERS.store(
            self.settings.online_covers,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.update_toast();

        if !self.fullscreen {
            self.top_bar(ui);
        }
        self.control_bar(ui, &ctx);
        self.playlist_panel(ui);
        self.viewer(ui, &ctx);
        self.settings_window(&ctx);
        self.association_prompt(&ctx);
        self.convert_window(&ctx);
        self.update_media_host(&ctx);
        self.draw_drop_overlay(&ctx);
        self.sync_window_title(&ctx);

        if self.fullscreen && !self.controls_visible() {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }
        if self.applied_always_on_top != Some(self.settings.always_on_top) {
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                if self.settings.always_on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                },
            ));
            self.applied_always_on_top = Some(self.settings.always_on_top);
        }

        self.save_if_needed();
        let fast_repaint = self.pending_initial_load
            || self.image.loading
            || self.cover_loading
            || self.scanning_directory
            || self.toast.is_some()
            || self.convert_job.is_some()
            || (self.backend == Backend::Mpv && !self.media_loaded)
            || (self.media_loaded && !self.paused && !self.current_is_image());
        let delay = if fast_repaint {
            33
        } else if self.fullscreen {
            100
        } else {
            200
        };
        ctx.request_repaint_after(Duration::from_millis(delay));
    }
}

impl Drop for AnMediaApp {
    fn drop(&mut self) {
        self.store_current_position();
        self.sync_playlist_to_settings();
        let _ = self.settings.save();
        // mpv должен закрыться раньше окна, в которое он рисует.
        self.mpv = None;
        self.native_media = None;
    }
}

// ─────────────────────────── вспомогательные функции ───────────────────────────

fn load_logo(ctx: &egui::Context) -> TextureHandle {
    let image = image::load_from_memory(include_bytes!("../assets/anmedia.png"))
        .map(|value| value.to_rgba8())
        .unwrap_or_else(|_| image::RgbaImage::new(1, 1));
    let size = [image.width() as usize, image.height() as usize];
    ctx.load_texture(
        "anmedia-logo",
        ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
        TEXTURE_OPTIONS,
    )
}

fn paint_logo(painter: &egui::Painter, texture: &TextureHandle, rect: Rect) {
    painter.image(
        texture.id(),
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
}

/// Карточка формата в окне конвертации.
fn format_card(
    ui: &mut egui::Ui,
    palette: &Palette,
    label: &str,
    description: &str,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(134.0, 58.0), Sense::click());
    let fill = if selected {
        palette.accent_soft
    } else if response.hovered() {
        palette.card_hover
    } else {
        palette.card
    };
    ui.painter().rect_filled(rect, 12.0, fill);
    ui.painter().rect_stroke(
        rect,
        12.0,
        Stroke::new(
            if selected { 1.5 } else { 1.0 },
            if selected {
                palette.accent
            } else {
                palette.outline
            },
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        Pos2::new(rect.left() + 12.0, rect.top() + 18.0),
        Align2::LEFT_CENTER,
        label,
        semibold(15.0),
        if selected {
            palette.accent
        } else {
            palette.text
        },
    );
    let galley = truncated(
        ui,
        description,
        FontId::proportional(11.0),
        palette.text_faint,
        rect.width() - 20.0,
    );
    ui.painter().galley(
        Pos2::new(rect.left() + 12.0, rect.top() + 32.0),
        galley,
        palette.text_faint,
    );
    response.on_hover_text(description)
}

fn wheel_steps(ui: &egui::Ui) -> f32 {
    ui.input(|input| {
        input
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                _ => None,
            })
            .sum::<f32>()
    })
}

fn time_label(ui: &mut egui::Ui, palette: &Palette, seconds: f64, align: Align2) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(46.0, 20.0), Sense::hover());
    let x = if align == Align2::RIGHT_CENTER {
        rect.right()
    } else {
        rect.left()
    };
    ui.painter().text(
        Pos2::new(x, rect.center().y),
        align,
        format_time(seconds),
        FontId::proportional(12.5),
        palette.text_weak,
    );
}

fn search_field(ui: &mut egui::Ui, palette: &Palette, value: &mut String) {
    Frame::new()
        .fill(palette.field)
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icon::SEARCH)
                        .font(icon_font(13.0))
                        .color(palette.text_faint),
                );
                let clear_width = if value.is_empty() { 0.0 } else { 28.0 };
                ui.add(
                    egui::TextEdit::singleline(value)
                        .frame(Frame::NONE)
                        .hint_text("Поиск в плейлисте")
                        .desired_width(ui.available_width() - clear_width),
                );
                if !value.is_empty()
                    && ui
                        .add(
                            egui::Label::new(
                                RichText::new(icon::CLOSE)
                                    .font(icon_font(10.0))
                                    .color(palette.text_faint),
                            )
                            .sense(Sense::click()),
                        )
                        .clicked()
                {
                    value.clear();
                }
            });
        });
}

fn draw_loading(ui: &mut egui::Ui, rect: Rect, palette: &Palette, title: &str) {
    ui.put(
        Rect::from_center_size(rect.center() - Vec2::new(0.0, 16.0), Vec2::splat(34.0)),
        egui::Spinner::new().size(32.0).color(palette.accent),
    );
    let galley = truncated(
        ui,
        title,
        FontId::proportional(14.0),
        Color32::from_rgb(160, 167, 181),
        (rect.width() - 80.0).max(60.0),
    );
    ui.painter().galley(
        Pos2::new(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y + 20.0,
        ),
        galley,
        Color32::WHITE,
    );
}

fn truncated(
    ui: &egui::Ui,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping {
        max_width: max_width.max(10.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.painter().layout_job(job)
}

fn paint_vertical_gradient(painter: &egui::Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(mesh);
}

fn format_speed(speed: f64) -> String {
    let text = format!("{speed:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text}×")
}

fn is_error_message(message: &str) -> bool {
    let lower = message.to_lowercase();
    [
        "не удалось",
        "ошибка",
        "недоступ",
        "не найден",
        "не поддерж",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn format_cover_error(error: &str) -> String {
    let normalized = error.to_lowercase();
    if normalized.contains("503")
        || normalized.contains("429")
        || normalized.contains("service unavailable")
        || normalized.contains("too many requests")
        || normalized.contains("musicbrainz")
        || normalized.contains("itunes")
        || normalized.contains("http")
        || normalized.contains("url")
    {
        "Обложка не загружена: сервис временно недоступен".to_string()
    } else if normalized.contains("размер") {
        "Обложка не загружена: файл слишком большой".to_string()
    } else if normalized.contains("декод") || normalized.contains("изображ") {
        "Обложка получена, но её формат не поддерживается".to_string()
    } else {
        "Обложка не загружена".to_string()
    }
}

fn reveal_in_explorer(path: &Path) {
    let _ = std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", path.display()))
        .spawn();
}

fn detect_system_dark() -> bool {
    matches!(dark_light::detect(), Ok(dark_light::Mode::Dark))
}

fn position_key(path: &Path) -> String {
    path_identity(path)
}

fn path_identity(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    absolute.to_string_lossy().replace('/', "\\").to_lowercase()
}
