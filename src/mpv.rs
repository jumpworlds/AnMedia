//! Универсальный движок на libmpv (FFmpeg).
//!
//! `libmpv-2.dll` загружается динамически из папки AnMedia.exe. Если
//! библиотеки нет, AnMedia продолжает работать на системных кодеках Windows.
//! Видео выводится в дочернее окно AnMedia через опцию `wid`.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
    LoadLibraryExW,
};

const LIBRARY_NAMES: &[&str] = &["libmpv-2.dll", "mpv-2.dll", "mpv-1.dll"];

const FORMAT_FLAG: c_int = 3;
const FORMAT_INT64: c_int = 4;
const FORMAT_DOUBLE: c_int = 5;

const EVENT_NONE: c_int = 0;
const EVENT_SHUTDOWN: c_int = 1;
const EVENT_END_FILE: c_int = 7;
const EVENT_FILE_LOADED: c_int = 8;
const EVENT_PLAYBACK_RESTART: c_int = 21;

const END_FILE_REASON_EOF: c_int = 0;
const END_FILE_REASON_ERROR: c_int = 4;

/// Защита: mpv читает только локальный файл. Сеть, ссылки на другие
/// файлы, плейлисты внутри медиа, скрипты и профили отключены.
const HARDENING: &[(&str, &str)] = &[
    ("config", "no"),
    ("load-scripts", "no"),
    ("ytdl", "no"),
    ("load-auto-profiles", "no"),
    ("load-stats-overlay", "no"),
    ("load-osd-console", "no"),
    ("load-select", "no"),
    ("load-positioning", "no"),
    ("load-commands", "no"),
    ("load-context-menu", "no"),
    ("access-references", "no"),
    ("load-unsafe-playlists", "no"),
    ("demuxer-lavf-o", "protocol_whitelist=[file,crypto,data]"),
    ("input-ipc-server", ""),
    ("network-timeout", "1"),
];

#[repr(C)]
struct RawEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct RawEndFile {
    reason: c_int,
    error: c_int,
    playlist_entry_id: i64,
    playlist_insert_id: i64,
    playlist_insert_num_entries: c_int,
}

type Handle = *mut c_void;

struct Library {
    path: PathBuf,
    client_api_version: unsafe extern "C" fn() -> u64,
    create: unsafe extern "C" fn() -> Handle,
    initialize: unsafe extern "C" fn(Handle) -> c_int,
    terminate_destroy: unsafe extern "C" fn(Handle),
    set_option_string: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    set_property_string: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    set_property: unsafe extern "C" fn(Handle, *const c_char, c_int, *mut c_void) -> c_int,
    get_property: unsafe extern "C" fn(Handle, *const c_char, c_int, *mut c_void) -> c_int,
    command: unsafe extern "C" fn(Handle, *mut *const c_char) -> c_int,
    wait_event: unsafe extern "C" fn(Handle, f64) -> *mut RawEvent,
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
}

// Указатели на функции и путь не меняются после загрузки.
unsafe impl Send for Library {}
unsafe impl Sync for Library {}

static LIBRARY: OnceLock<Result<Library, String>> = OnceLock::new();

fn library() -> Result<&'static Library, String> {
    LIBRARY
        .get_or_init(load_library)
        .as_ref()
        .map_err(Clone::clone)
}

/// Доступен ли движок mpv (библиотека найдена и загружена).
pub fn is_available() -> bool {
    library().is_ok()
}

/// Человекочитаемое описание состояния движка для настроек.
pub fn describe() -> String {
    match library() {
        Ok(library) => {
            let version = unsafe { (library.client_api_version)() };
            format!(
                "libmpv {}.{} — {}",
                version >> 16,
                version & 0xffff,
                library.path.display()
            )
        }
        Err(error) => error,
    }
}

fn candidate_directories() -> Vec<PathBuf> {
    let mut result = Vec::new();
    if let Some(directory) = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
    {
        result.push(directory.join("mpv"));
        result.push(directory.clone());
        // При запуске из target\…\release ищем рядом с проектом.
        for ancestor in directory.ancestors().skip(1).take(4) {
            result.push(ancestor.join("mpv"));
        }
    }
    // Папка установки (туда AnMedia копирует себя при регистрации в Windows).
    if let Some(dirs) = directories::ProjectDirs::from("", "Anton4k272", "AnMedia") {
        result.push(dirs.data_local_dir().to_path_buf());
    }
    result
}

/// Ищет файл libmpv рядом с указанным exe (для копирования при установке).
pub fn library_near(executable: &Path) -> Option<PathBuf> {
    let directory = executable.parent()?;
    let mut directories = vec![directory.to_path_buf(), directory.join("mpv")];
    for ancestor in directory.ancestors().skip(1).take(4) {
        directories.push(ancestor.join("mpv"));
    }
    directories
        .into_iter()
        .flat_map(|directory| LIBRARY_NAMES.iter().map(move |name| directory.join(name)))
        .find(|path| path.is_file())
}

fn load_library() -> Result<Library, String> {
    for directory in candidate_directories() {
        for name in LIBRARY_NAMES {
            let path = directory.join(name);
            if !path.is_file() {
                continue;
            }
            let wide = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>();
            let module = unsafe {
                LoadLibraryExW(
                    wide.as_ptr(),
                    std::ptr::null_mut(),
                    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
                )
            };
            if module.is_null() {
                continue;
            }
            return unsafe { bind(module, path) };
        }
    }
    Err("libmpv-2.dll не найдена рядом с AnMedia.exe — работают только кодеки Windows".to_string())
}

unsafe fn bind(module: HMODULE, path: PathBuf) -> Result<Library, String> {
    macro_rules! symbol {
        ($name:literal) => {{
            let address = unsafe { GetProcAddress(module, concat!($name, "\0").as_ptr()) }
                .ok_or_else(|| format!("в {} нет функции {}", path.display(), $name))?;
            unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, _>(address) }
        }};
    }
    Ok(Library {
        client_api_version: symbol!("mpv_client_api_version"),
        create: symbol!("mpv_create"),
        initialize: symbol!("mpv_initialize"),
        terminate_destroy: symbol!("mpv_terminate_destroy"),
        set_option_string: symbol!("mpv_set_option_string"),
        set_property_string: symbol!("mpv_set_property_string"),
        set_property: symbol!("mpv_set_property"),
        get_property: symbol!("mpv_get_property"),
        command: symbol!("mpv_command"),
        wait_event: symbol!("mpv_wait_event"),
        error_string: symbol!("mpv_error_string"),
        path,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum MpvEvent {
    Loaded,
    Restarted,
    Ended,
    Error(String),
}

/// Экземпляр mpv. Все функции libmpv потокобезопасны.
pub(crate) struct Instance {
    library: &'static Library,
    handle: Handle,
}

unsafe impl Send for Instance {}

impl Instance {
    pub(crate) fn new(options: &[(&str, &str)]) -> Result<Self, String> {
        let library = library()?;
        let handle = unsafe { (library.create)() };
        if handle.is_null() {
            return Err("mpv: не удалось создать экземпляр".to_string());
        }
        let instance = Self { library, handle };
        for (name, value) in HARDENING {
            instance.set_option(name, value);
        }
        for (name, value) in options {
            let code = instance.set_option(name, value);
            if code < 0 && *name == "wid" {
                return Err(format!("mpv: опция {name}: {}", instance.error_text(code)));
            }
        }
        let code = unsafe { (library.initialize)(handle) };
        if code < 0 {
            return Err(format!("mpv: инициализация: {}", instance.error_text(code)));
        }
        Ok(instance)
    }

    fn set_option(&self, name: &str, value: &str) -> c_int {
        let (Ok(name), Ok(value)) = (CString::new(name), CString::new(value)) else {
            return -4;
        };
        unsafe { (self.library.set_option_string)(self.handle, name.as_ptr(), value.as_ptr()) }
    }

    pub(crate) fn set_string(&self, name: &str, value: &str) -> Result<(), String> {
        let name_c = CString::new(name).map_err(|error| error.to_string())?;
        let value_c = CString::new(value).map_err(|error| error.to_string())?;
        let code = unsafe {
            (self.library.set_property_string)(self.handle, name_c.as_ptr(), value_c.as_ptr())
        };
        self.check(code, name)
    }

    fn set_double(&self, name: &str, mut value: f64) -> Result<(), String> {
        let name_c = CString::new(name).map_err(|error| error.to_string())?;
        let code = unsafe {
            (self.library.set_property)(
                self.handle,
                name_c.as_ptr(),
                FORMAT_DOUBLE,
                (&mut value as *mut f64).cast(),
            )
        };
        self.check(code, name)
    }

    fn set_flag(&self, name: &str, value: bool) -> Result<(), String> {
        let name_c = CString::new(name).map_err(|error| error.to_string())?;
        let mut flag: c_int = value.into();
        let code = unsafe {
            (self.library.set_property)(
                self.handle,
                name_c.as_ptr(),
                FORMAT_FLAG,
                (&mut flag as *mut c_int).cast(),
            )
        };
        self.check(code, name)
    }

    pub(crate) fn get_double(&self, name: &str) -> Option<f64> {
        let name_c = CString::new(name).ok()?;
        let mut value = 0f64;
        let code = unsafe {
            (self.library.get_property)(
                self.handle,
                name_c.as_ptr(),
                FORMAT_DOUBLE,
                (&mut value as *mut f64).cast(),
            )
        };
        (code >= 0 && value.is_finite()).then_some(value)
    }

    fn get_flag(&self, name: &str) -> Option<bool> {
        let name_c = CString::new(name).ok()?;
        let mut value: c_int = 0;
        let code = unsafe {
            (self.library.get_property)(
                self.handle,
                name_c.as_ptr(),
                FORMAT_FLAG,
                (&mut value as *mut c_int).cast(),
            )
        };
        (code >= 0).then_some(value != 0)
    }

    fn get_int(&self, name: &str) -> Option<i64> {
        let name_c = CString::new(name).ok()?;
        let mut value = 0i64;
        let code = unsafe {
            (self.library.get_property)(
                self.handle,
                name_c.as_ptr(),
                FORMAT_INT64,
                (&mut value as *mut i64).cast(),
            )
        };
        (code >= 0).then_some(value)
    }

    pub(crate) fn command(&self, args: &[&str]) -> Result<(), String> {
        let owned = args
            .iter()
            .map(|value| CString::new(*value).map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut pointers = owned
            .iter()
            .map(|value| value.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect::<Vec<_>>();
        let code = unsafe { (self.library.command)(self.handle, pointers.as_mut_ptr()) };
        self.check(code, args.first().copied().unwrap_or("command"))
    }

    pub(crate) fn next_event(
        &self,
        timeout: f64,
    ) -> Option<(c_int, c_int, Option<(c_int, c_int)>)> {
        let event = unsafe { (self.library.wait_event)(self.handle, timeout) };
        if event.is_null() {
            return None;
        }
        let event = unsafe { &*event };
        if event.event_id == EVENT_NONE {
            return None;
        }
        let end_file = (event.event_id == EVENT_END_FILE && !event.data.is_null()).then(|| {
            let data = unsafe { &*(event.data as *const RawEndFile) };
            (data.reason, data.error)
        });
        Some((event.event_id, event.error, end_file))
    }

    pub(crate) fn error_text(&self, code: c_int) -> String {
        let text = unsafe { (self.library.error_string)(code) };
        if text.is_null() {
            return format!("код {code}");
        }
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    }

    fn check(&self, code: c_int, operation: &str) -> Result<(), String> {
        if code >= 0 {
            Ok(())
        } else {
            Err(format!("mpv: {operation}: {}", self.error_text(code)))
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe { (self.library.terminate_destroy)(self.handle) };
    }
}

pub struct MpvPlayer {
    instance: Instance,
    loaded: bool,
}

impl MpvPlayer {
    pub fn create(hwnd: u64) -> Result<Self, String> {
        let wid = hwnd.to_string();
        let instance = Instance::new(&[
            ("wid", wid.as_str()),
            ("config", "no"),
            ("terminal", "no"),
            ("idle", "yes"),
            ("keep-open", "yes"),
            ("force-window", "no"),
            ("input-default-bindings", "no"),
            ("input-vo-keyboard", "no"),
            ("input-cursor", "no"),
            ("cursor-autohide", "no"),
            ("osc", "no"),
            ("osd-level", "0"),
            ("load-scripts", "no"),
            ("ytdl", "no"),
            ("audio-display", "no"),
            ("hwdec", "auto-safe"),
            ("sub-auto", "fuzzy"),
            ("audio-file-auto", "no"),
            ("volume-max", "100"),
            ("image-display-duration", "inf"),
            ("background-color", "#000000"),
        ])?;
        Ok(Self {
            instance,
            loaded: false,
        })
    }

    pub fn load(&mut self, path: &Path, start: Option<f64>, paused: bool) -> Result<(), String> {
        let path = path
            .to_str()
            .ok_or_else(|| "mpv: путь к файлу содержит недопустимые символы".to_string())?;
        let start = start
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(|value| format!("{value:.3}"))
            .unwrap_or_else(|| "none".to_string());
        let _ = self.instance.set_string("start", &start);
        let _ = self.instance.set_flag("pause", paused);
        self.loaded = false;
        self.instance.command(&["loadfile", path, "replace"])
    }

    pub fn play(&self) -> Result<(), String> {
        if self.instance.get_flag("eof-reached") == Some(true) {
            let _ = self.instance.command(&["seek", "0", "absolute"]);
        }
        self.instance.set_flag("pause", false)
    }

    pub fn pause(&self) -> Result<(), String> {
        self.instance.set_flag("pause", true)
    }

    pub fn stop(&mut self) {
        self.loaded = false;
        let _ = self.instance.command(&["stop"]);
    }

    pub fn rewind(&self) -> Result<(), String> {
        self.instance.set_flag("pause", true)?;
        self.seek(0.0)
    }

    pub fn seek(&self, seconds: f64) -> Result<(), String> {
        if !seconds.is_finite() {
            return Err("Некорректная позиция перемотки".to_string());
        }
        let target = format!("{:.3}", seconds.max(0.0));
        self.instance.command(&["seek", &target, "absolute+exact"])
    }

    pub fn set_volume(&self, volume: f64) -> Result<(), String> {
        self.instance.set_double("volume", volume.clamp(0.0, 100.0))
    }

    pub fn set_muted(&self, muted: bool) -> Result<(), String> {
        self.instance.set_flag("mute", muted)
    }

    pub fn set_speed(&self, speed: f64) -> Result<(), String> {
        self.instance.set_double("speed", speed.clamp(0.25, 4.0))
    }

    pub fn set_loop(&self, enabled: bool) -> Result<(), String> {
        self.instance
            .set_string("loop-file", if enabled { "inf" } else { "no" })
    }

    pub fn current_time(&self) -> f64 {
        self.instance.get_double("time-pos").unwrap_or(0.0).max(0.0)
    }

    pub fn duration(&self) -> f64 {
        self.instance.get_double("duration").unwrap_or(0.0).max(0.0)
    }

    pub fn is_paused(&self) -> bool {
        self.instance.get_flag("pause").unwrap_or(true)
            || self.instance.get_flag("eof-reached").unwrap_or(false)
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn has_video(&self) -> bool {
        self.loaded
            && self.instance.get_int("current-tracks/video/id").is_some()
            && self.instance.get_flag("current-tracks/video/albumart") != Some(true)
    }

    pub fn has_audio(&self) -> bool {
        self.loaded && self.instance.get_int("current-tracks/audio/id").is_some()
    }

    pub fn video_size(&self) -> Option<(i64, i64)> {
        Some((
            self.instance.get_int("video-params/w")?,
            self.instance.get_int("video-params/h")?,
        ))
    }

    pub fn poll_events(&mut self) -> Vec<MpvEvent> {
        let mut result = Vec::new();
        let mut eof_reached = false;
        while let Some((id, _error, end_file)) = self.instance.next_event(0.0) {
            match id {
                EVENT_FILE_LOADED => {
                    self.loaded = true;
                    result.push(MpvEvent::Loaded);
                }
                EVENT_PLAYBACK_RESTART => result.push(MpvEvent::Restarted),
                EVENT_END_FILE => match end_file {
                    Some((END_FILE_REASON_ERROR, code)) => {
                        self.loaded = false;
                        result.push(MpvEvent::Error(describe_load_error(
                            &self.instance.error_text(code),
                        )));
                    }
                    Some((END_FILE_REASON_EOF, _)) => eof_reached = true,
                    _ => {}
                },
                EVENT_SHUTDOWN => {
                    self.loaded = false;
                    result.push(MpvEvent::Error("mpv завершил работу".to_string()));
                }
                _ => {}
            }
        }
        if eof_reached {
            result.push(MpvEvent::Ended);
        }
        result
    }

    /// keep-open=yes: в конце файла mpv ставит паузу и выставляет eof-reached.
    pub fn eof_reached(&self) -> bool {
        self.loaded && self.instance.get_flag("eof-reached") == Some(true)
    }
}

fn describe_load_error(error: &str) -> String {
    match error {
        "unrecognized file format" => "mpv: формат файла не распознан".to_string(),
        "loading failed" => "mpv: файл повреждён или не читается".to_string(),
        "no audio or video data played" => "mpv: в файле нет воспроизводимых дорожек".to_string(),
        other => format!("mpv: {other}"),
    }
}

/// Извлекает один кадр файла в PNG через `vo=image`.
///
/// Используется как последний резерв для изображений (AVIF/HEIC без
/// расширений Windows, PSD, PCX и т.п.) и для миниатюр видео.
pub fn grab_frame(
    path: &Path,
    start: Option<&str>,
    max_side: Option<u32>,
) -> Result<Vec<u8>, String> {
    library()?;
    let path_text = path
        .to_str()
        .ok_or_else(|| "mpv: путь к файлу содержит недопустимые символы".to_string())?
        .to_string();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let directory =
        std::env::temp_dir().join(format!("anmedia-frame-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let result = grab_frame_into(&directory, &path_text, start, max_side);
    let _ = std::fs::remove_dir_all(&directory);
    result
}

fn grab_frame_into(
    directory: &Path,
    path: &str,
    start: Option<&str>,
    max_side: Option<u32>,
) -> Result<Vec<u8>, String> {
    let outdir = directory.to_string_lossy().into_owned();
    let filter = max_side.map(|side| {
        format!("lavfi=[scale=w={side}:h={side}:force_original_aspect_ratio=decrease]")
    });
    let mut options = vec![
        ("config", "no"),
        ("terminal", "no"),
        ("idle", "no"),
        ("keep-open", "no"),
        ("load-scripts", "no"),
        ("ytdl", "no"),
        ("vo", "image"),
        ("vo-image-format", "png"),
        ("vo-image-png-compression", "1"),
        ("vo-image-outdir", outdir.as_str()),
        ("ao", "null"),
        ("aid", "no"),
        ("sid", "no"),
        ("hwdec", "no"),
        ("frames", "1"),
        ("audio-display", "no"),
        ("image-display-duration", "0"),
    ];
    if let Some(start) = start {
        options.push(("start", start));
    }
    if let Some(filter) = filter.as_deref() {
        options.push(("vf", filter));
    }
    let instance = Instance::new(&options)?;
    instance.command(&["loadfile", path])?;

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut error = None;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            error = Some("mpv: превышено время ожидания кадра".to_string());
            break;
        }
        match instance.next_event(remaining.as_secs_f64().min(0.5)) {
            Some((EVENT_END_FILE, _, end_file)) => {
                if let Some((END_FILE_REASON_ERROR, code)) = end_file {
                    error = Some(describe_load_error(&instance.error_text(code)));
                }
                break;
            }
            Some((EVENT_SHUTDOWN, _, _)) => break,
            _ => {}
        }
    }
    drop(instance);

    let frame = std::fs::read_dir(directory).ok().and_then(|entries| {
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|value| value.eq_ignore_ascii_case("png"))
            })
            .min()
    });
    match frame {
        Some(frame) => std::fs::read(frame).map_err(|error| error.to_string()),
        None => Err(error.unwrap_or_else(|| "mpv: кадр не получен".to_string())),
    }
}

#[cfg(test)]
pub fn probe(path: &Path) -> Result<(f64, bool, bool), String> {
    let instance = Instance::new(&[
        ("config", "no"),
        ("terminal", "no"),
        ("vo", "null"),
        ("ao", "null"),
        ("pause", "yes"),
        ("audio-display", "no"),
    ])?;
    instance.command(&["loadfile", path.to_str().unwrap_or_default()])?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match instance.next_event(0.5) {
            Some((EVENT_FILE_LOADED, _, _)) => {
                return Ok((
                    instance.get_double("duration").unwrap_or(0.0),
                    instance.get_int("current-tracks/audio/id").is_some(),
                    instance.get_int("current-tracks/video/id").is_some(),
                ));
            }
            Some((EVENT_END_FILE, _, Some((END_FILE_REASON_ERROR, code)))) => {
                return Err(instance.error_text(code));
            }
            _ => {}
        }
    }
    Err("timeout".to_string())
}
