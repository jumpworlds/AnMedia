<div align="center">

<img src="docs/logo.png" width="128" alt="AnMedia">

# AnMedia

**Быстрый медиаплеер для Windows: видео, музыка и фото в одном окне**

Открывает практически любой формат, конвертирует файлы и защищает систему от вредоносных «медиафайлов».

![Windows](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D6?logo=windows&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-1.97-000000?logo=rust)
![egui](https://img.shields.io/badge/UI-egui-8A2BE2)
![libmpv](https://img.shields.io/badge/engine-libmpv%20%2F%20FFmpeg-5C2D91)
![License](https://img.shields.io/badge/license-MIT-green)

<img src="docs/screenshots/music.png" alt="AnMedia — музыка" width="900">

</div>

---

## Возможности

- 🎬 **Любое видео** — MP4, MKV, AVI, MOV, WebM, FLV, TS, WMV, RM/RMVB, ProRes, HEVC, AV1, VP9 и десятки других через встроенный движок **libmpv (FFmpeg)**. Субтитры рядом с видео подхватываются сами.
- 🎵 **Любая музыка** — MP3, FLAC, Opus, OGG, AAC/M4A, ALAC, WMA, WAV, APE, WavPack, TTA, AC3/DTS, AMR, трекерная музыка. Обложки из тегов или из интернета, фон окна окрашивается под обложку.
- 🖼️ **Любые фото** — JPEG, PNG, WebP, HEIC, AVIF, JPEG XL, RAW камер (CR2/CR3, NEF, ARW, DNG…), PSD, SVG, TIFF, EXR, HDR и другие. Анимированные GIF / WebP / APNG, учёт EXIF-поворота, огромные снимки.
- 🔍 **Формат по содержимому** — файлы без расширения или с неправильным расширением всё равно открываются.
- 🔄 **Конвертер** — видео, музыку и фото в другой формат в пару кликов, с выбором качества и прогрессом.
- 🔎 **Масштаб как в просмотрщике фото** — колесо мыши приближает к курсору, перетаскивание двигает кадр, и для видео тоже.
- 🛡️ **Защита** — программы, ярлыки и скрипты, замаскированные под видео/фото/музыку, не открываются; плеер работает в «песочнице» из ограничений Windows.
- 🎨 **Современный интерфейс** — тёмная и светлая темы, 6 акцентных цветов, плейлист с миниатюрами, фильтры, поиск, полноэкранный режим с автоскрытием панели.
- ⚡ **Лёгкий и быстрый** — нативное приложение на Rust, один экземпляр, запоминание плейлиста и позиции.

## Скриншоты

| Видео | Фото |
|:---:|:---:|
| <img src="docs/screenshots/video.png" alt="Видео"> | <img src="docs/screenshots/photo.png" alt="Фото"> |
| **Конвертер** | **Защита от замаскированных файлов** |
| <img src="docs/screenshots/convert.png" alt="Конвертер"> | <img src="docs/screenshots/protection.png" alt="Защита"> |
| **Настройки** | **Стартовый экран** |
| <img src="docs/screenshots/settings.png" alt="Настройки"> | <img src="docs/screenshots/empty.png" alt="Стартовый экран"> |

## Скачать

Готовая сборка — в разделе [**Releases**](../../releases): распакуйте архив и запустите `AnMedia.exe`.

> `AnMedia.exe` и `libmpv-2.dll` должны лежать в одной папке. Без DLL плеер тоже работает, но только с кодеками, установленными в Windows.

При первом запуске AnMedia предложит добавить себя в список приложений Windows для фото, видео и музыки.

## Конвертер

Кнопка **«Конвертировать»**, `Ctrl+E` или правый клик по файлу в плейлисте.

| Из | В |
|---|---|
| Видео | MP4 (H.264), MKV (H.265), WebM (VP9), MOV, AVI, GIF — или только звук |
| Музыка | MP3, M4A (AAC), FLAC, WAV, Opus, OGG, WMA, AIFF |
| Фото | PNG, JPG, WebP, AVIF, BMP, TIFF, ICO, GIF, TGA, QOI |

Три уровня качества, отмена в любой момент, исходный файл никогда не перезаписывается.

## Безопасность

Сам по себе медиафайл не может «запуститься», но злоумышленники маскируют программы под видео и пользуются ошибками в декодерах. AnMedia защищается от этого:

- **Проверка содержимого.** EXE/DLL, ярлыки LNK, установщики MSI, HTML/HTA и скрипты, переименованные в `.mp4`/`.jpg`/`.mp3`, блокируются. Ловится и подмена расширения невидимыми символами (`фото‮gpj.exe`).
- **Ограничения процесса Windows.** DLL не подгружаются из папки с медиафайлом (защита от DLL-подмены), запрещено внедрение чужих DLL и загрузка несистемных шрифтов.
- **Изолированный движок.** mpv читает только сам файл: без сети, без ссылок на другие файлы и плейлистов внутри медиа, без скриптов и конфигов.
- **Безопасные декодеры.** SVG не может читать файлы с диска, у изображений есть лимиты памяти против «бомб».
- Файлы, скачанные из интернета (Mark of the Web), помечаются уведомлением. Поиск обложек в сети можно отключить.

> Это не антивирус: защита закрывает типичные способы атаки через медиафайлы, но не заменяет антивирусную проверку.

## Горячие клавиши

| Клавиша | Действие |
|---|---|
| `Ctrl+O` / `Ctrl+Shift+O` | Открыть файлы / папку |
| `Ctrl+V` / `Ctrl+C` | Открыть картинку из буфера / скопировать фото |
| `Ctrl+E` | Конвертировать |
| `Пробел` / `S` | Пауза / стоп |
| `←` `→` | Перемотка 5 с (`Ctrl` — 30 с); для фото — предыдущее/следующее |
| `PgUp` / `PgDn` | Предыдущий / следующий файл |
| `↑` `↓` / `M` | Громкость / без звука |
| `[` `]` | Скорость воспроизведения |
| Колесо, `+` `−`, `0` | Масштаб, сброс масштаба |
| `1`, `R` / `Shift+R` | Фото: 100%, поворот |
| `F11`, `F`, двойной щелчок | Полный экран |

## Сборка из исходников

Требования: Windows 10/11 x64, [Rust](https://rustup.rs) (версия подтянется из `rust-toolchain.toml`), Visual Studio Build Tools с MSVC и Windows SDK.

```powershell
git clone https://github.com/<user>/AnMedia.git
cd AnMedia
.\build.bat
```

`build.bat` соберёт релиз, при необходимости сам скачает `libmpv-2.dll` (скрипт `scripts\fetch-libmpv.ps1`) и упакует всё в `dist\AnMedia-2.0-Windows-x64.zip`.

Для разработки:

```powershell
.\scripts\fetch-libmpv.ps1        # один раз: libmpv-2.dll → mpv\
cargo run --release --target x86_64-pc-windows-msvc
cargo test --release --target x86_64-pc-windows-msvc
```

## Как устроено

```
src/
├── app.rs            интерфейс и логика плеера
├── theme.rs          палитры, шрифты Segoe UI, иконки Segoe Fluent
├── ui_kit.rs         собственные виджеты (кнопки, сикбар, переключатели)
├── mpv.rs            динамическая загрузка libmpv, вывод видео в окно
├── native_media.rs   резервный движок Windows Media Foundation
├── audio_engine.rs   встроенный аудиодекодер (Rodio / Symphonia)
├── image_decode.rs   цепочка декодеров изображений
├── formats.rs        определение формата по сигнатуре
├── convert.rs        конвертер (libmpv --o + crate image)
├── security.rs       проверка файлов и ограничения процесса
└── …                 обложки, миниатюры, ассоциации, один экземпляр
```

Цепочки декодеров с автоматическим резервом:

- **Музыка:** Symphonia → mpv → Media Foundation
- **Видео:** mpv → Media Foundation (порядок меняется в настройках)
- **Фото:** `image` → resvg / jxl-oxide / PSD → WIC (кодеки Windows) → FFmpeg

## Лицензия

Код AnMedia распространяется по лицензии [MIT](LICENSE).

Сторонние компоненты:
- [mpv / libmpv](https://github.com/mpv-player/mpv) и [FFmpeg](https://ffmpeg.org) — GPL/LGPL. Используемая сборка libmpv: [shinchiro/mpv-winbuild-cmake](https://github.com/shinchiro/mpv-winbuild-cmake). DLL не входит в репозиторий и скачивается при сборке.
- [egui / eframe](https://github.com/emilk/egui), [image](https://github.com/image-rs/image), [resvg](https://github.com/linebender/resvg), [jxl-oxide](https://github.com/tirr-c/jxl-oxide), [Symphonia](https://github.com/pdeljanov/Symphonia), [rodio](https://github.com/RustAudio/rodio), [lofty](https://github.com/Serial-ATA/lofty-rs) — MIT / Apache-2.0.
