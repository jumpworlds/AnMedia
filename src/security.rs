//! Защита от вредоносных файлов.
//!
//! 1. Жёсткие настройки процесса: DLL не подгружаются из папки с медиафайлом,
//!    запрещены сторонние «точки расширения» (AppInit, глобальные хуки),
//!    DLL с низким уровнем целостности и несистемные шрифты.
//! 2. Проверка файла перед открытием: программы, ярлыки и скрипты,
//!    замаскированные под фото/видео/музыку, не открываются.
//! 3. Декодеры ограничены: mpv работает только с локальным файлом (без сети и
//!    ссылок на другие файлы), SVG не читает внешние файлы, у изображений есть
//!    лимиты на размер и память (защита от «zip-бомб»).

use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;
use windows_sys::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, SetDefaultDllDirectories, SetDllDirectoryW,
};
use windows_sys::Win32::System::Threading::SetProcessMitigationPolicy;

static APPLIED: OnceLock<Vec<&'static str>> = OnceLock::new();

// Значения PROCESS_MITIGATION_POLICY.
const POLICY_EXTENSION_POINT_DISABLE: i32 = 6;
const POLICY_FONT_DISABLE: i32 = 9;
const POLICY_IMAGE_LOAD: i32 = 10;

/// Вызывается первым делом в `main`.
pub fn harden_process() {
    let mut applied = Vec::new();
    unsafe {
        // Папка, из которой открыт файл, больше не участвует в поиске DLL:
        // подброшенная рядом с видео DLL не загрузится.
        let empty = [0u16];
        if SetDllDirectoryW(empty.as_ptr()) != 0
            && SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) != 0
        {
            applied.push("DLL не загружаются из папок с медиафайлами");
        }
        let mut flags: u32 = 1; // DisableExtensionPoints
        if set_policy(POLICY_EXTENSION_POINT_DISABLE, &mut flags) {
            applied.push("Запрещено внедрение чужих DLL (AppInit, глобальные хуки)");
        }
        let mut flags: u32 = 0b110; // NoLowMandatoryLabelImages | PreferSystem32Images
        if set_policy(POLICY_IMAGE_LOAD, &mut flags) {
            applied.push("Системные DLL берутся только из System32");
        }
        let mut flags: u32 = 1; // DisableNonSystemFonts
        if set_policy(POLICY_FONT_DISABLE, &mut flags) {
            applied.push("Шрифты из файлов не загружаются в систему");
        }
    }
    applied.push("Плееру запрещены сеть и ссылки на другие файлы внутри медиа");
    applied.push("SVG не читает внешние файлы, у изображений есть лимиты памяти");
    let _ = APPLIED.set(applied);
}

unsafe fn set_policy(policy: i32, flags: &mut u32) -> bool {
    unsafe {
        SetProcessMitigationPolicy(
            policy,
            (flags as *mut u32).cast(),
            std::mem::size_of::<u32>(),
        ) != 0
    }
}

pub fn applied_protections() -> &'static [&'static str] {
    APPLIED.get().map(Vec::as_slice).unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Safe,
    /// Файл скачан из интернета (Mark of the Web) — открывается, но с пометкой.
    FromInternet,
    Blocked(String),
}

/// Проверяет файл перед открытием.
pub fn inspect(path: &Path) -> Verdict {
    if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
        // Символы смены направления текста прячут настоящее расширение:
        // «фото\u{202E}gpj.exe» выглядит как «фотоexe.jpg».
        if name
            .chars()
            .any(|character| matches!(character, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
        {
            return Verdict::Blocked(
                "в имени файла скрыто настоящее расширение (символы смены направления текста)"
                    .to_string(),
            );
        }
    }
    let mut header = [0u8; 4096];
    let read = std::fs::File::open(path)
        .and_then(|mut file| file.read(&mut header))
        .unwrap_or(0);
    if let Some(reason) = dangerous_content(&header[..read]) {
        return Verdict::Blocked(reason.to_string());
    }
    if from_internet(path) {
        Verdict::FromInternet
    } else {
        Verdict::Safe
    }
}

fn dangerous_content(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"MZ") {
        return Some("это программа Windows (EXE/DLL), замаскированная под медиафайл");
    }
    if data.starts_with(&[0x4C, 0x00, 0x00, 0x00, 0x01, 0x14, 0x02, 0x00]) {
        return Some("это ярлык Windows (LNK), который может запустить программу");
    }
    if data.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        return Some("это документ Office или установщик MSI, а не медиафайл");
    }
    if data.starts_with(b"\x7FELF") {
        return Some("это исполняемый файл Linux, а не медиафайл");
    }
    let text = String::from_utf8_lossy(&data[..data.len().min(1024)]).to_lowercase();
    let text = text.trim_start_matches('\u{feff}').trim_start();
    let script_markers = [
        "<script",
        "<hta:",
        "<!doctype html",
        "<html",
        "@echo off",
        "powershell",
        "wscript.",
        "createobject(",
        "[internetshortcut]",
    ];
    if !text.contains("<svg")
        && script_markers
            .iter()
            .any(|marker| text.starts_with(marker) || (marker.len() > 8 && text.contains(marker)))
    {
        return Some("внутри скрипт или веб-страница, а не медиафайл");
    }
    None
}

/// Файл скачан из интернета: в потоке Zone.Identifier указана зона 3 или 4.
fn from_internet(path: &Path) -> bool {
    let mut stream = path.as_os_str().to_os_string();
    stream.push(":Zone.Identifier");
    std::fs::read_to_string(stream)
        .map(|value| {
            value
                .lines()
                .any(|line| matches!(line.trim(), "ZoneId=3" | "ZoneId=4"))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::dangerous_content;

    #[test]
    fn blocks_disguised_programs() {
        assert!(dangerous_content(b"MZ\x90\0\x03").is_some());
        assert!(dangerous_content(b"<html><script>alert(1)</script>").is_some());
        assert!(dangerous_content(b"@echo off\r\ndel *").is_some());
        assert!(dangerous_content(b"<?xml version='1.0'?><svg><script/></svg>").is_none());
        assert!(dangerous_content(b"\x89PNG\r\n\x1a\n").is_none());
        assert!(dangerous_content(b"ID3\x04\0\0").is_none());
    }
}
