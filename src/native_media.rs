use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use url::Url;
use windows::Win32::Foundation::RECT;
use windows::Win32::Media::MediaFoundation::{
    CLSID_MFMediaEngineClassFactory, IMFAttributes, IMFByteStream, IMFMediaEngine,
    IMFMediaEngineClassFactory, IMFMediaEngineEx, IMFMediaEngineNotify, IMFMediaEngineNotify_Impl,
    MF_ACCESSMODE_READ, MF_FILEFLAGS_NONE, MF_MEDIA_ENGINE_CALLBACK, MF_MEDIA_ENGINE_PLAYBACK_HWND,
    MF_MEDIA_ENGINE_PRELOAD_AUTOMATIC, MF_OPENMODE_FAIL_IF_NOT_EXIST, MF_VERSION,
    MFCreateAttributes, MFCreateFile, MFShutdown, MFStartup,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows_core::{BSTR, Error, HRESULT, Interface, PCWSTR, Result, implement};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeMediaEvent {
    LoadStarted,
    MetadataLoaded,
    DataLoaded,
    Ready,
    Playing,
    Paused,
    Waiting,
    Seeking,
    Seeked,
    TimeUpdated,
    DurationChanged,
    VolumeChanged,
    RateChanged,
    FormatChanged,
    Ended,
    Error(String),
}

#[derive(Clone, Copy, Default)]
struct MediaState {
    current_time: f64,
    duration: f64,
    paused: bool,
    has_video: bool,
    has_audio: bool,
}

struct LoadedSource {
    _stream: Option<IMFByteStream>,
    _source: BSTR,
}

#[implement(IMFMediaEngineNotify)]
struct MediaEngineCallback {
    sender: Sender<NativeMediaEvent>,
}

impl IMFMediaEngineNotify_Impl for MediaEngineCallback_Impl {
    fn EventNotify(&self, event: u32, param1: usize, param2: u32) -> Result<()> {
        let mapped = match event {
            1 => Some(NativeMediaEvent::LoadStarted),
            5 if param1 != 1 => Some(NativeMediaEvent::Error(format_media_engine_error(
                param1, param2,
            ))),
            5 => None,
            8 | 13 => Some(NativeMediaEvent::Playing),
            9 => Some(NativeMediaEvent::Paused),
            10 => Some(NativeMediaEvent::MetadataLoaded),
            11 => Some(NativeMediaEvent::DataLoaded),
            12 | 1005 => Some(NativeMediaEvent::Waiting),
            14 | 15 | 1006 | 1009 => Some(NativeMediaEvent::Ready),
            16 => Some(NativeMediaEvent::Seeking),
            17 => Some(NativeMediaEvent::Seeked),
            18 => Some(NativeMediaEvent::TimeUpdated),
            19 => Some(NativeMediaEvent::Ended),
            20 => Some(NativeMediaEvent::RateChanged),
            21 => Some(NativeMediaEvent::DurationChanged),
            22 => Some(NativeMediaEvent::VolumeChanged),
            1000 => Some(NativeMediaEvent::FormatChanged),
            1014 => Some(NativeMediaEvent::Error(format!(
                "Ошибка вывода медиапотока: {}",
                format_hresult(param2)
            ))),
            _ => None,
        };
        if let Some(event) = mapped {
            let _ = self.sender.send(event);
        }
        Ok(())
    }
}

enum MediaCommand {
    Load(PathBuf, Sender<std::result::Result<(), String>>),
    Play(Sender<std::result::Result<(), String>>),
    Pause(Sender<std::result::Result<(), String>>),
    Stop(Sender<std::result::Result<(), String>>),
    Seek(f64, Sender<std::result::Result<(), String>>),
    Volume(f64, Sender<std::result::Result<(), String>>),
    Muted(bool, Sender<std::result::Result<(), String>>),
    Speed(f64, Sender<std::result::Result<(), String>>),
    Loop(bool, Sender<std::result::Result<(), String>>),
    VideoRect(i32, i32),
    Shutdown,
}

pub struct NativeMediaEngine {
    command_tx: Sender<MediaCommand>,
    event_rx: Receiver<NativeMediaEvent>,
    state: Arc<RwLock<MediaState>>,
    worker: Option<JoinHandle<()>>,
}

impl NativeMediaEngine {
    pub fn create(hwnd: u64) -> std::result::Result<Self, String> {
        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = unbounded();
        let (ready_tx, ready_rx) = bounded(1);
        let state = Arc::new(RwLock::new(MediaState {
            paused: true,
            ..MediaState::default()
        }));
        let worker_state = Arc::clone(&state);

        let worker = thread::Builder::new()
            .name("AnMedia-MediaFoundation".to_string())
            .spawn(move || {
                let _ = run_media_thread(hwnd, command_rx, event_tx, worker_state, ready_tx);
            })
            .map_err(|error| format!("Не удалось запустить поток Media Foundation: {error}"))?;

        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(Self {
                command_tx,
                event_rx,
                state,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                let _ = command_tx.send(MediaCommand::Shutdown);
                let _ = worker.join();
                Err("Media Foundation не завершила инициализацию".to_string())
            }
        }
    }

    pub fn events(&self) -> &Receiver<NativeMediaEvent> {
        &self.event_rx
    }

    pub fn load(&self, path: &Path) -> std::result::Result<(), String> {
        self.request(|reply| MediaCommand::Load(path.to_path_buf(), reply))
    }

    pub fn play(&self) -> std::result::Result<(), String> {
        self.request(MediaCommand::Play)
    }

    pub fn pause(&self) -> std::result::Result<(), String> {
        self.request(MediaCommand::Pause)
    }

    pub fn stop(&self) -> std::result::Result<(), String> {
        self.request(MediaCommand::Stop)
    }

    pub fn seek(&self, seconds: f64) -> std::result::Result<(), String> {
        if !seconds.is_finite() {
            return Err("Некорректная позиция перемотки".to_string());
        }
        self.request(|reply| MediaCommand::Seek(seconds, reply))
    }

    pub fn set_volume(&self, volume: f64) -> std::result::Result<(), String> {
        self.request(|reply| MediaCommand::Volume(volume, reply))
    }

    pub fn set_muted(&self, muted: bool) -> std::result::Result<(), String> {
        self.request(|reply| MediaCommand::Muted(muted, reply))
    }

    pub fn set_speed(&self, speed: f64) -> std::result::Result<(), String> {
        self.request(|reply| MediaCommand::Speed(speed, reply))
    }

    pub fn set_loop(&self, enabled: bool) -> std::result::Result<(), String> {
        self.request(|reply| MediaCommand::Loop(enabled, reply))
    }

    pub fn current_time(&self) -> f64 {
        self.read_state().current_time
    }

    pub fn duration(&self) -> f64 {
        self.read_state().duration
    }

    pub fn is_paused(&self) -> bool {
        self.read_state().paused
    }

    pub fn has_video(&self) -> bool {
        self.read_state().has_video
    }

    pub fn has_audio(&self) -> bool {
        self.read_state().has_audio
    }

    pub fn update_video_rect(&self, width: i32, height: i32) {
        let _ = self.command_tx.send(MediaCommand::VideoRect(width, height));
    }

    fn request<F>(&self, build: F) -> std::result::Result<(), String>
    where
        F: FnOnce(Sender<std::result::Result<(), String>>) -> MediaCommand,
    {
        let (reply_tx, reply_rx) = bounded(1);
        self.command_tx
            .send(build(reply_tx))
            .map_err(|_| "Поток Media Foundation остановлен".to_string())?;
        reply_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "Media Foundation не ответила вовремя".to_string())?
    }

    fn read_state(&self) -> MediaState {
        self.state.read().map(|value| *value).unwrap_or_default()
    }
}

impl Drop for NativeMediaEngine {
    fn drop(&mut self) {
        let _ = self.command_tx.send(MediaCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_media_thread(
    hwnd: u64,
    command_rx: Receiver<MediaCommand>,
    event_tx: Sender<NativeMediaEvent>,
    state: Arc<RwLock<MediaState>>,
    ready_tx: Sender<std::result::Result<(), String>>,
) -> std::result::Result<(), String> {
    unsafe {
        let com_result = CoInitializeEx(None, COINIT_MULTITHREADED);
        if com_result.is_err() {
            let message =
                format_windows_error("Инициализация COM MTA", Error::from_hresult(com_result));
            let _ = ready_tx.send(Err(message.clone()));
            return Err(message);
        }
        if let Err(error) = MFStartup(MF_VERSION, 0) {
            CoUninitialize();
            let message = format_windows_error("Инициализация Media Foundation", error);
            let _ = ready_tx.send(Err(message.clone()));
            return Err(message);
        }

        let result = create_media_engine(hwnd, event_tx);
        let (engine, engine_ex, callback) = match result {
            Ok(value) => value,
            Err(error) => {
                let _ = MFShutdown();
                CoUninitialize();
                let _ = ready_tx.send(Err(error.clone()));
                return Err(error);
            }
        };
        let _ = ready_tx.send(Ok(()));

        let mut current_source: Option<LoadedSource> = None;
        let mut poll_interval = Duration::from_millis(200);
        loop {
            match command_rx.recv_timeout(poll_interval) {
                Ok(MediaCommand::Load(path, reply)) => {
                    let _ = engine.Pause();
                    let result = match load_engine(&engine, engine_ex.as_ref(), &path) {
                        Ok(source) => {
                            current_source = Some(source);
                            Ok(())
                        }
                        Err(error) => Err(error),
                    };
                    let _ = reply.send(result);
                }
                Ok(MediaCommand::Play(reply)) => {
                    let result = engine
                        .Play()
                        .map_err(|error| format_windows_error("Воспроизведение", error));
                    let _ = reply.send(result);
                }
                Ok(MediaCommand::Pause(reply)) => {
                    let result = engine
                        .Pause()
                        .map_err(|error| format_windows_error("Пауза", error));
                    let _ = reply.send(result);
                }
                Ok(MediaCommand::Stop(reply)) => {
                    let result = engine
                        .Pause()
                        .and_then(|_| engine.SetCurrentTime(0.0))
                        .map_err(|error| format_windows_error("Остановка", error));
                    let _ = reply.send(result);
                }
                Ok(MediaCommand::Seek(seconds, reply)) => {
                    let result = engine
                        .SetCurrentTime(seconds.max(0.0))
                        .map_err(|error| format_windows_error("Перемотка", error));
                    let _ = reply.send(result);
                }
                Ok(MediaCommand::Volume(volume, reply)) => {
                    let result = engine
                        .SetVolume((volume / 100.0).clamp(0.0, 1.0))
                        .map_err(|error| format_windows_error("Громкость", error));
                    let _ = reply.send(result);
                }
                Ok(MediaCommand::Muted(muted, reply)) => {
                    let result = engine
                        .SetMuted(muted)
                        .map_err(|error| format_windows_error("Отключение звука", error));
                    let _ = reply.send(result);
                }
                Ok(MediaCommand::Speed(speed, reply)) => {
                    let result = engine
                        .SetPlaybackRate(speed.clamp(0.25, 4.0))
                        .map_err(|error| format_windows_error("Скорость", error));
                    let _ = reply.send(result);
                }
                Ok(MediaCommand::Loop(enabled, reply)) => {
                    let result = engine
                        .SetLoop(enabled)
                        .map_err(|error| format_windows_error("Повтор", error));
                    let _ = reply.send(result);
                }
                Ok(MediaCommand::VideoRect(width, height)) => {
                    if let Some(engine_ex) = &engine_ex {
                        let destination = RECT {
                            left: 0,
                            top: 0,
                            right: width.max(1),
                            bottom: height.max(1),
                        };
                        let _ = engine_ex.UpdateVideoStream(
                            None,
                            Some(&destination as *const RECT),
                            None,
                        );
                    }
                }
                Ok(MediaCommand::Shutdown)
                | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    break;
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            }

            let duration = engine.GetDuration();
            let snapshot = MediaState {
                current_time: engine.GetCurrentTime().max(0.0),
                duration: if duration.is_finite() {
                    duration.max(0.0)
                } else {
                    0.0
                },
                paused: engine.IsPaused().as_bool(),
                has_video: engine.HasVideo().as_bool(),
                has_audio: engine.HasAudio().as_bool(),
            };
            poll_interval = if snapshot.paused {
                Duration::from_millis(200)
            } else {
                Duration::from_millis(33)
            };
            if let Ok(mut value) = state.write() {
                *value = snapshot;
            }
        }

        let _ = engine.Shutdown();
        drop(current_source);
        drop(engine_ex);
        drop(engine);
        drop(callback);
        let _ = MFShutdown();
        CoUninitialize();
        Ok(())
    }
}

fn create_media_engine(
    hwnd: u64,
    event_tx: Sender<NativeMediaEvent>,
) -> std::result::Result<
    (
        IMFMediaEngine,
        Option<IMFMediaEngineEx>,
        IMFMediaEngineNotify,
    ),
    String,
> {
    unsafe {
        let callback: IMFMediaEngineNotify = MediaEngineCallback { sender: event_tx }.into();
        let mut attributes: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attributes, 4)
            .map_err(|error| format_windows_error("Создание атрибутов медиадвижка", error))?;
        let attributes =
            attributes.ok_or_else(|| "Media Foundation не создала набор атрибутов".to_string())?;

        attributes
            .SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &callback)
            .map_err(|error| format_windows_error("Подключение обратного вызова", error))?;
        attributes
            .SetUINT64(&MF_MEDIA_ENGINE_PLAYBACK_HWND, hwnd)
            .map_err(|error| format_windows_error("Подключение окна вывода", error))?;

        let factory: IMFMediaEngineClassFactory =
            CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| format_windows_error("Создание Media Foundation Engine", error))?;
        let engine = factory
            .CreateInstance(0, &attributes)
            .map_err(|error| format_windows_error("Запуск Media Foundation Engine", error))?;
        let _ = engine.SetPreload(MF_MEDIA_ENGINE_PRELOAD_AUTOMATIC);
        let engine_ex = engine.cast::<IMFMediaEngineEx>().ok();
        Ok((engine, engine_ex, callback))
    }
}

fn load_engine(
    engine: &IMFMediaEngine,
    engine_ex: Option<&IMFMediaEngineEx>,
    path: &Path,
) -> std::result::Result<LoadedSource, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("Не удалось определить рабочую папку: {error}"))?
            .join(path)
    };
    let metadata = fs::metadata(&absolute)
        .map_err(|error| format!("Файл недоступен {}: {error}", absolute.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "Указанный путь не является файлом: {}",
            absolute.display()
        ));
    }
    if metadata.len() == 0 {
        return Err(format!("Файл пуст: {}", absolute.display()));
    }

    unsafe {
        if let Some(engine_ex) = engine_ex {
            let wide_path = absolute.as_os_str().encode_wide().collect::<Vec<_>>();
            let mut nul_terminated_path = Vec::with_capacity(wide_path.len() + 1);
            nul_terminated_path.extend_from_slice(&wide_path);
            nul_terminated_path.push(0);

            let stream = MFCreateFile(
                MF_ACCESSMODE_READ,
                MF_OPENMODE_FAIL_IF_NOT_EXIST,
                MF_FILEFLAGS_NONE,
                PCWSTR(nul_terminated_path.as_ptr()),
            )
            .map_err(|error| format_windows_error("Открытие локального файла", error))?;
            // Media Foundation выбирает обработчик по расширению в URL; если
            // расширение не совпадает с содержимым, подсказываем настоящее.
            let source = match crate::formats::sniff_file(&absolute) {
                Some((_, real))
                    if !absolute
                        .extension()
                        .and_then(|value| value.to_str())
                        .is_some_and(|value| value.eq_ignore_ascii_case(real)) =>
                {
                    let mut hinted = wide_path.clone();
                    hinted.extend(format!(".{real}").encode_utf16());
                    BSTR::from_wide(&hinted)
                }
                _ => BSTR::from_wide(&wide_path),
            };
            engine_ex
                .SetSourceFromByteStream(&stream, &source)
                .map_err(|error| format_windows_error("Передача файла медиадвижку", error))?;
            return Ok(LoadedSource {
                _stream: Some(stream),
                _source: source,
            });
        }

        let url = Url::from_file_path(&absolute)
            .map_err(|_| format!("Не удалось преобразовать путь: {}", absolute.display()))?;
        let source = BSTR::from(url.as_str());
        engine
            .SetSource(&source)
            .map_err(|error| format_windows_error("Открытие файла", error))?;
        Ok(LoadedSource {
            _stream: None,
            _source: source,
        })
    }
}

fn format_media_engine_error(code: usize, hresult: u32) -> String {
    let reason = match code {
        1 => "операция была прервана",
        2 => "сетевая ошибка",
        3 => "ошибка декодирования",
        4 => "источник или формат не поддерживается",
        5 => "защищённый или зашифрованный источник не поддерживается",
        _ => "неизвестная ошибка медиадвижка",
    };
    format!("{reason}; {}", format_hresult(hresult))
}

fn format_hresult(hresult: u32) -> String {
    match hresult {
        0 => "HRESULT не указан".to_string(),
        0x80070002 => "файл не найден (HRESULT 0x80070002)".to_string(),
        0x80070003 => "системе не удалось найти указанный путь (HRESULT 0x80070003)".to_string(),
        _ => {
            let error = Error::from_hresult(HRESULT(hresult as i32));
            format!("{} (HRESULT 0x{hresult:08X})", error.message())
        }
    }
}

fn format_windows_error(operation: &str, error: Error) -> String {
    format!(
        "{operation}: {} (0x{:08X})",
        error.message(),
        error.code().0 as u32
    )
}
