use crate::formats::{
    AUDIO_EXTENSIONS, IMAGE_EXTENSIONS, VIDEO_EXTENSIONS, all_extensions, extension_kind,
};
use crate::installation::ensure_stable_executable;
use std::env;
use std::ffi::c_void;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::{null, null_mut};
use std::thread;
use std::time::Duration;
use windows_sys::Win32::UI::Shell::{
    SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify, ShellExecuteW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};

const APP_NAME: &str = "AnMedia";
const LEGACY_PROG_ID: &str = "AnMedia.MediaFile";
const IMAGE_PROG_ID: &str = "AnMedia.ImageFile";
const VIDEO_PROG_ID: &str = "AnMedia.VideoFile";
const AUDIO_PROG_ID: &str = "AnMedia.AudioFile";
const PLAYLIST_PROG_ID: &str = "AnMedia.PlaylistFile";
const CAPABILITIES_PATH: &str = "Software\\Anton4k272\\AnMedia\\Capabilities";
const THUMBNAIL_HANDLER_IID: &str = "{e357fccd-a995-4576-b01f-234630154e96}";
const PHOTO_THUMBNAIL_PROVIDER: &str = "{C7657C4A-9F68-40fa-A4DF-96BC08EB3551}";
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub fn register_file_associations(
    executable: &Path,
    custom_extensions: &str,
) -> Result<PathBuf, String> {
    let executable = ensure_stable_executable(executable)?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let executable_text = executable.to_string_lossy().into_owned();
    let open_command = format!("\"{}\" \"%1\"", executable_text);
    let icon_command = format!("\"{}\",0", executable_text);

    let classes = hkcu
        .open_subkey_with_flags("Software\\Classes", KEY_WRITE)
        .map_err(|error| error.to_string())?;

    register_prog_id(
        &classes,
        LEGACY_PROG_ID,
        "Мультимедийный файл AnMedia",
        &open_command,
        None,
        None,
    )?;
    register_prog_id(
        &classes,
        IMAGE_PROG_ID,
        "Изображение AnMedia",
        &open_command,
        Some("image"),
        None,
    )?;
    register_prog_id(
        &classes,
        VIDEO_PROG_ID,
        "Видео AnMedia",
        &open_command,
        Some("video"),
        None,
    )?;
    register_prog_id(
        &classes,
        AUDIO_PROG_ID,
        "Аудио AnMedia",
        &open_command,
        Some("audio"),
        None,
    )?;
    register_prog_id(
        &classes,
        PLAYLIST_PROG_ID,
        "Плейлист AnMedia",
        &open_command,
        Some("document"),
        Some(&icon_command),
    )?;

    let image_prog_id = classes
        .open_subkey_with_flags(IMAGE_PROG_ID, KEY_WRITE)
        .map_err(|error| error.to_string())?;
    let (image_thumbnail_handler, _) = image_prog_id
        .create_subkey(format!("ShellEx\\{THUMBNAIL_HANDLER_IID}"))
        .map_err(|error| error.to_string())?;
    image_thumbnail_handler
        .set_value("", &PHOTO_THUMBNAIL_PROVIDER)
        .map_err(|error| error.to_string())?;
    configure_thumbnail_metadata(&image_prog_id)?;

    let (system_image, _) = classes
        .create_subkey("SystemFileAssociations\\image")
        .map_err(|error| error.to_string())?;
    let (system_image_thumbnail_handler, _) = system_image
        .create_subkey(format!("ShellEx\\{THUMBNAIL_HANDLER_IID}"))
        .map_err(|error| error.to_string())?;
    system_image_thumbnail_handler
        .set_value("", &PHOTO_THUMBNAIL_PROVIDER)
        .map_err(|error| error.to_string())?;
    configure_thumbnail_metadata(&system_image)?;

    let (system_video, _) = classes
        .create_subkey("SystemFileAssociations\\video")
        .map_err(|error| error.to_string())?;
    configure_thumbnail_metadata(&system_video)?;

    let (system_audio, _) = classes
        .create_subkey("SystemFileAssociations\\audio")
        .map_err(|error| error.to_string())?;
    configure_thumbnail_metadata(&system_audio)?;

    register_application(&classes, &open_command, &icon_command, &executable)?;

    let (capabilities, _) = hkcu
        .create_subkey(CAPABILITIES_PATH)
        .map_err(|error| error.to_string())?;
    capabilities
        .set_value("ApplicationName", &APP_NAME)
        .map_err(|error| error.to_string())?;
    capabilities
        .set_value(
            "ApplicationDescription",
            &"Открытие изображений, видео, музыки и плейлистов",
        )
        .map_err(|error| error.to_string())?;
    capabilities
        .set_value("ApplicationIcon", &icon_command)
        .map_err(|error| error.to_string())?;

    let (file_associations, _) = capabilities
        .create_subkey("FileAssociations")
        .map_err(|error| error.to_string())?;
    let application = classes
        .open_subkey_with_flags("Applications\\AnMedia.exe", KEY_WRITE)
        .map_err(|error| error.to_string())?;
    let (supported_types, _) = application
        .create_subkey("SupportedTypes")
        .map_err(|error| error.to_string())?;

    for extension in all_extensions(custom_extensions) {
        let dotted_extension = format!(".{extension}");
        let prog_id = prog_id_for_extension(&extension);
        file_associations
            .set_value(&dotted_extension, &prog_id)
            .map_err(|error| error.to_string())?;
        supported_types
            .set_value(&dotted_extension, &"")
            .map_err(|error| error.to_string())?;
        register_extension_metadata(&classes, &extension, prog_id)?;
    }

    let (registered_applications, _) = hkcu
        .create_subkey("Software\\RegisteredApplications")
        .map_err(|error| error.to_string())?;
    registered_applications
        .set_value(APP_NAME, &CAPABILITIES_PATH)
        .map_err(|error| error.to_string())?;

    let (app_path, _) = hkcu
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\AnMedia.exe")
        .map_err(|error| error.to_string())?;
    app_path
        .set_value("", &executable_text)
        .map_err(|error| error.to_string())?;
    if let Some(parent) = executable.parent() {
        let parent_text = parent.to_string_lossy().into_owned();
        app_path
            .set_value("Path", &parent_text)
            .map_err(|error| error.to_string())?;
    }

    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            null::<c_void>(),
            null::<c_void>(),
        );
    }
    Ok(executable)
}

pub fn rebuild_shell_preview_cache() -> Result<(), String> {
    let _ = Command::new("taskkill.exe")
        .args(["/F", "/IM", "explorer.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    thread::sleep(Duration::from_millis(450));

    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        let explorer_cache = PathBuf::from(&local_app_data)
            .join("Microsoft")
            .join("Windows")
            .join("Explorer");
        if let Ok(entries) = fs::read_dir(explorer_cache) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy().to_ascii_lowercase();
                if (name.starts_with("thumbcache_") || name.starts_with("iconcache_"))
                    && name.ends_with(".db")
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        let _ = fs::remove_file(PathBuf::from(local_app_data).join("IconCache.db"));
    }

    Command::new("explorer.exe")
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| format!("Не удалось перезапустить Проводник Windows: {error}"))?;
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            null::<c_void>(),
            null::<c_void>(),
        );
    }
    Ok(())
}

fn register_prog_id(
    classes: &RegKey,
    prog_id: &str,
    description: &str,
    open_command: &str,
    perceived_type: Option<&str>,
    default_icon: Option<&str>,
) -> Result<(), String> {
    let (key, _) = classes
        .create_subkey(prog_id)
        .map_err(|error| error.to_string())?;
    key.set_value("", &description)
        .map_err(|error| error.to_string())?;
    key.set_value("FriendlyTypeName", &description)
        .map_err(|error| error.to_string())?;
    if let Some(perceived_type) = perceived_type {
        key.set_value("PerceivedType", &perceived_type)
            .map_err(|error| error.to_string())?;
    }

    let (command, _) = key
        .create_subkey("shell\\open\\command")
        .map_err(|error| error.to_string())?;
    command
        .set_value("", &open_command)
        .map_err(|error| error.to_string())?;

    match default_icon {
        Some(default_icon) => {
            let (icon, _) = key
                .create_subkey("DefaultIcon")
                .map_err(|error| error.to_string())?;
            icon.set_value("", &default_icon)
                .map_err(|error| error.to_string())?;
        }
        None => {
            let _ = classes.delete_subkey_all(format!("{prog_id}\\DefaultIcon"));
        }
    }
    Ok(())
}

fn register_application(
    classes: &RegKey,
    open_command: &str,
    icon_command: &str,
    executable: &Path,
) -> Result<(), String> {
    let (application, _) = classes
        .create_subkey("Applications\\AnMedia.exe")
        .map_err(|error| error.to_string())?;
    application
        .set_value("FriendlyAppName", &APP_NAME)
        .map_err(|error| error.to_string())?;
    application
        .set_value("ApplicationCompany", &"Anton4k272")
        .map_err(|error| error.to_string())?;
    application
        .set_value("ApplicationIcon", &icon_command)
        .map_err(|error| error.to_string())?;
    application
        .set_value("ApplicationName", &APP_NAME)
        .map_err(|error| error.to_string())?;
    let executable_text = executable.to_string_lossy().into_owned();
    application
        .set_value("ExecutablePath", &executable_text)
        .map_err(|error| error.to_string())?;
    let (application_command, _) = application
        .create_subkey("shell\\open\\command")
        .map_err(|error| error.to_string())?;
    application_command
        .set_value("", &open_command)
        .map_err(|error| error.to_string())?;
    let (icon, _) = application
        .create_subkey("DefaultIcon")
        .map_err(|error| error.to_string())?;
    icon.set_value("", &icon_command)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn register_extension_metadata(
    classes: &RegKey,
    extension: &str,
    prog_id: &str,
) -> Result<(), String> {
    let dotted_extension = format!(".{extension}");
    let perceived_type = extension_kind(extension);
    let (extension_key, _) = classes
        .create_subkey(&dotted_extension)
        .map_err(|error| error.to_string())?;
    extension_key
        .set_value("PerceivedType", &perceived_type)
        .map_err(|error| error.to_string())?;
    if let Some(content_type) = content_type(extension) {
        extension_key
            .set_value("Content Type", &content_type)
            .map_err(|error| error.to_string())?;
    }
    if is_shell_preview_extension(extension) {
        configure_thumbnail_metadata(&extension_key)?;
        let _ = extension_key.delete_subkey_all("DefaultIcon");
    }
    if IMAGE_EXTENSIONS.contains(&extension) {
        let (thumbnail_handler, _) = extension_key
            .create_subkey(format!("ShellEx\\{THUMBNAIL_HANDLER_IID}"))
            .map_err(|error| error.to_string())?;
        thumbnail_handler
            .set_value("", &PHOTO_THUMBNAIL_PROVIDER)
            .map_err(|error| error.to_string())?;
    }

    let (open_with, _) = extension_key
        .create_subkey("OpenWithProgids")
        .map_err(|error| error.to_string())?;
    open_with
        .set_value(prog_id, &"")
        .map_err(|error| error.to_string())?;
    open_with
        .set_value(LEGACY_PROG_ID, &"")
        .map_err(|error| error.to_string())?;

    let (system_association, _) = classes
        .create_subkey(format!("SystemFileAssociations\\{dotted_extension}"))
        .map_err(|error| error.to_string())?;
    system_association
        .set_value("PerceivedType", &perceived_type)
        .map_err(|error| error.to_string())?;
    if let Some(content_type) = content_type(extension) {
        system_association
            .set_value("Content Type", &content_type)
            .map_err(|error| error.to_string())?;
    }
    if is_shell_preview_extension(extension) {
        configure_thumbnail_metadata(&system_association)?;
    }
    if IMAGE_EXTENSIONS.contains(&extension) {
        let (thumbnail_handler, _) = system_association
            .create_subkey(format!("ShellEx\\{THUMBNAIL_HANDLER_IID}"))
            .map_err(|error| error.to_string())?;
        thumbnail_handler
            .set_value("", &PHOTO_THUMBNAIL_PROVIDER)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn is_shell_preview_extension(extension: &str) -> bool {
    IMAGE_EXTENSIONS.contains(&extension)
        || VIDEO_EXTENSIONS.contains(&extension)
        || AUDIO_EXTENSIONS.contains(&extension)
}

fn configure_thumbnail_metadata(key: &RegKey) -> Result<(), String> {
    key.set_value("ThumbnailCutoff", &1u32)
        .map_err(|error| error.to_string())?;
    key.set_value("Treatment", &0u32)
        .map_err(|error| error.to_string())?;
    key.set_value("TypeOverlay", &"")
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn prog_id_for_extension(extension: &str) -> &'static str {
    match extension_kind(extension) {
        "image" => IMAGE_PROG_ID,
        "video" => VIDEO_PROG_ID,
        "audio" => AUDIO_PROG_ID,
        "document" => PLAYLIST_PROG_ID,
        _ => LEGACY_PROG_ID,
    }
}

fn content_type(extension: &str) -> Option<&'static str> {
    match extension {
        "apng" | "png" => Some("image/png"),
        "avif" => Some("image/avif"),
        "bmp" | "dib" => Some("image/bmp"),
        "gif" => Some("image/gif"),
        "heic" | "heif" => Some("image/heif"),
        "ico" => Some("image/x-icon"),
        "jfif" | "jpe" | "jpeg" | "jpg" => Some("image/jpeg"),
        "jxl" => Some("image/jxl"),
        "svg" => Some("image/svg+xml"),
        "tif" | "tiff" => Some("image/tiff"),
        "webp" => Some("image/webp"),
        "avi" => Some("video/x-msvideo"),
        "mkv" => Some("video/x-matroska"),
        "mov" | "qt" => Some("video/quicktime"),
        "mp4" | "m4v" => Some("video/mp4"),
        "mpeg" | "mpg" => Some("video/mpeg"),
        "webm" => Some("video/webm"),
        "wmv" => Some("video/x-ms-wmv"),
        "aac" => Some("audio/aac"),
        "flac" => Some("audio/flac"),
        "m4a" | "m4b" => Some("audio/mp4"),
        "mp3" => Some("audio/mpeg"),
        "oga" | "ogg" => Some("audio/ogg"),
        "opus" => Some("audio/opus"),
        "wav" => Some("audio/wav"),
        "wma" => Some("audio/x-ms-wma"),
        "m3u" | "m3u8" => Some("audio/x-mpegurl"),
        "pls" => Some("audio/x-scpls"),
        _ => None,
    }
}

pub fn open_default_apps_settings() -> Result<(), String> {
    let operation = wide("open");
    let uri = wide("ms-settings:defaultapps?registeredAppUser=AnMedia");
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            operation.as_ptr(),
            uri.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        Err("Windows не открыла параметры приложений по умолчанию".to_string())
    } else {
        Ok(())
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
