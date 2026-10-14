use directories::ProjectDirs;
use std::fs;
use std::path::{Path, PathBuf};

pub fn ensure_stable_executable(source: &Path) -> Result<PathBuf, String> {
    let source = source
        .canonicalize()
        .unwrap_or_else(|_| source.to_path_buf());
    if !source.is_file() {
        return Err("Исполняемый файл AnMedia не найден".to_string());
    }

    let project_dirs = ProjectDirs::from("", "Anton4k272", "AnMedia")
        .ok_or_else(|| "Windows не предоставила каталог LocalAppData".to_string())?;
    let install_dir = project_dirs.data_local_dir();
    fs::create_dir_all(install_dir)
        .map_err(|error| format!("Не удалось создать каталог установки AnMedia: {error}"))?;

    let destination = install_dir.join("AnMedia.exe");
    install_mpv_library(&source, install_dir);
    if same_path(&source, &destination) {
        cleanup_old_builds(install_dir, &destination);
        return Ok(destination);
    }

    let temporary = install_dir.join(format!("AnMedia.update.{}.exe", std::process::id()));
    let backup = install_dir.join(format!("AnMedia.backup.{}.exe", std::process::id()));
    let _ = fs::remove_file(&temporary);
    let _ = fs::remove_file(&backup);
    fs::copy(&source, &temporary)
        .map_err(|error| format!("Не удалось установить AnMedia: {error}"))?;

    let had_previous = destination.is_file();
    if had_previous {
        fs::rename(&destination, &backup)
            .map_err(|error| format!("Не удалось подготовить обновление AnMedia: {error}"))?;
    }

    if let Err(error) = fs::rename(&temporary, &destination) {
        if had_previous {
            let _ = fs::rename(&backup, &destination);
        }
        let _ = fs::remove_file(&temporary);
        return Err(format!("Не удалось завершить установку AnMedia: {error}"));
    }

    let _ = fs::remove_file(&backup);
    cleanup_old_builds(install_dir, &destination);
    refresh_icon_cache();
    Ok(destination)
}

/// Копирует libmpv-2.dll рядом с установленным AnMedia.exe — без неё
/// установленная копия открывала бы только форматы с кодеками Windows.
fn install_mpv_library(source: &Path, install_dir: &Path) {
    let Some(library) = crate::mpv::library_near(source) else {
        return;
    };
    let destination = install_dir.join("libmpv-2.dll");
    if same_path(&library, &destination) {
        return;
    }
    let size = |path: &Path| fs::metadata(path).map(|value| value.len()).ok();
    if size(&library).is_some() && size(&library) == size(&destination) {
        return;
    }
    let temporary = install_dir.join(format!("libmpv-2.update.{}.dll", std::process::id()));
    if fs::copy(&library, &temporary).is_ok() {
        // Если установленная копия сейчас запущена, DLL занята — обновим в следующий раз.
        if fs::rename(&temporary, &destination).is_err() {
            let _ = fs::remove_file(&temporary);
        }
    }
}

/// После обновления exe Проводник и панель задач могут показывать старую
/// иконку из кэша — просим Windows обновить его.
fn refresh_icon_cache() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let tool = Path::new(&system_root)
        .join("System32")
        .join("ie4uinit.exe");
    if tool.is_file() {
        let _ = std::process::Command::new(tool)
            .arg("-show")
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
}

fn cleanup_old_builds(directory: &Path, current: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if same_path(&path, current) {
            continue;
        }
        let is_old_build = path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| {
                value.eq_ignore_ascii_case("AnMedia.exe")
                    || (value.starts_with("AnMedia-") && value.ends_with(".exe"))
                    || value.starts_with("AnMedia.update.")
                    || value.starts_with("AnMedia.backup.")
            });
        if is_old_build {
            let _ = fs::remove_file(path);
        }
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = left
        .canonicalize()
        .unwrap_or_else(|_| left.to_path_buf())
        .to_string_lossy()
        .replace('/', "\\");
    let right = right
        .canonicalize()
        .unwrap_or_else(|_| right.to_path_buf())
        .to_string_lossy()
        .replace('/', "\\");
    left.eq_ignore_ascii_case(&right)
}
