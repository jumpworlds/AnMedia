use std::io::Read;
use std::path::Path;

pub const IMAGE_EXTENSIONS: &[&str] = &[
    // Встроенный декодер (crate image)
    "apng", "bmp", "dds", "dib", "exr", "ff", "gif", "hdr", "ico", "jfif", "jpe", "jpeg", "jpg",
    "pam", "pbm", "pgm", "pjp", "pjpeg", "png", "pnm", "ppm", "qoi", "rgbe", "tga", "tif", "tiff",
    "webp", // Векторные (resvg)
    "svg", "svgz", // JPEG XL (jxl-oxide)
    "jxl",  // Photoshop
    "psd", "psb",
    // Системные кодеки Windows (WIC): HEIF/AVIF, JPEG XR, JPEG 2000
    "avci", "avif", "avifs", "heic", "heics", "heif", "hif", "hdp", "jxr", "wdp", "jp2", "j2c",
    "j2k", "jpf", "jpx", "jpm", "cur", // Через FFmpeg (libmpv)
    "pcx", "sgi", "rgb", "ras", "sun", "xbm", "xpm", "xwd", "pic", "jls", "pfm", "phm",
    // RAW камер (Microsoft Raw Image Extension через WIC)
    "3fr", "arw", "cr2", "cr3", "crw", "dcr", "dng", "erf", "iiq", "kdc", "mef", "mos", "mrw",
    "nef", "nrw", "orf", "ori", "pef", "raf", "raw", "rw2", "rwl", "sr2", "srf", "srw", "x3f",
];

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "264", "265", "3g2", "3gp", "3gp2", "3gpp", "amv", "asf", "avi", "av1", "avs", "bik", "dav",
    "divx", "drc", "dv", "dvr-ms", "evo", "f4b", "f4p", "f4v", "flv", "gifv", "h264", "h265",
    "hevc", "ivf", "m1v", "m2p", "m2t", "m2ts", "m2v", "m4v", "mjpeg", "mjpg", "mk3d", "mkv",
    "mod", "mov", "mp2v", "mp4", "mp4v", "mpe", "mpeg", "mpg", "mpv", "mts", "mxf", "nsv", "nut",
    "obu", "ogm", "ogv", "ogx", "qt", "rec", "rm", "rmvb", "roq", "smk", "swf", "tod", "tp", "trp",
    "ts", "vob", "vp8", "vp9", "vro", "webm", "wm", "wmv", "wtv", "xvid", "y4m", "yuv",
];

pub const AUDIO_EXTENSIONS: &[&str] = &[
    "3ga", "8svx", "aa3", "aac", "ac3", "ac4", "adts", "aif", "aifc", "aiff", "alac", "amr", "ape",
    "apl", "au", "awb", "caf", "dff", "dsf", "dts", "dtshd", "eac3", "ec3", "flac", "gsm", "it",
    "loas", "m4a", "m4b", "m4r", "mka", "mlp", "mo3", "mod", "mp1", "mp2", "mp3", "mpa", "mpc",
    "mptm", "oga", "ogg", "oma", "opus", "ofr", "ofs", "ra", "rf64", "s3m", "shn", "snd", "spx",
    "tak", "thd", "tta", "umx", "voc", "vqf", "w64", "wav", "wave", "weba", "wma", "wv", "xm",
];

pub const PLAYLIST_EXTENSIONS: &[&str] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Unknown,
}

impl MediaKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Image => "Изображение",
            Self::Video => "Видео",
            Self::Audio => "Аудио",
            Self::Unknown => "Файл",
        }
    }
}

fn normalized_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.trim_start_matches('.').to_ascii_lowercase())
}

/// Тип файла только по расширению — быстро, без чтения диска.
pub fn kind_by_extension(path: &Path) -> MediaKind {
    let Some(extension) = normalized_extension(path) else {
        return MediaKind::Unknown;
    };
    if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        MediaKind::Image
    } else if AUDIO_EXTENSIONS.contains(&extension.as_str()) {
        MediaKind::Audio
    } else if VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        MediaKind::Video
    } else {
        MediaKind::Unknown
    }
}

/// Тип файла: сначала расширение, затем сигнатура содержимого.
pub fn detect_kind(path: &Path) -> MediaKind {
    match kind_by_extension(path) {
        MediaKind::Unknown => sniff_file(path)
            .map(|(kind, _)| kind)
            .unwrap_or(MediaKind::Unknown),
        kind => kind,
    }
}

/// Определяет формат по первым байтам файла.
/// Возвращает тип и «настоящее» расширение для подсказок декодерам.
pub fn sniff_file(path: &Path) -> Option<(MediaKind, &'static str)> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buffer = [0u8; 512];
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(_) => return None,
        }
    }
    sniff(&buffer[..filled])
}

pub fn sniff(data: &[u8]) -> Option<(MediaKind, &'static str)> {
    use MediaKind::{Audio, Image, Video};
    let starts = |prefix: &[u8]| data.starts_with(prefix);
    let at = |offset: usize, value: &[u8]| {
        data.len() >= offset + value.len() && &data[offset..offset + value.len()] == value
    };

    if at(4, b"ftyp") {
        let brand = data.get(8..12)?;
        return Some(match brand {
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"mif1" | b"msf1" => {
                (Image, "heic")
            }
            b"avif" | b"avis" => (Image, "avif"),
            b"crx " => (Image, "cr3"),
            b"M4A " | b"M4B " | b"M4P " | b"F4A " => (Audio, "m4a"),
            b"qt  " => (Video, "mov"),
            b"3gp4" | b"3gp5" | b"3gp6" | b"3g2a" => (Video, "3gp"),
            _ => (Video, "mp4"),
        });
    }
    if at(4, b"moov") || at(4, b"mdat") || at(4, b"wide") || at(4, b"free") || at(4, b"skip") {
        return Some((Video, "mov"));
    }
    if starts(b"RIFF") && data.len() >= 12 {
        return match &data[8..12] {
            b"WEBP" => Some((Image, "webp")),
            b"AVI " | b"AVIX" => Some((Video, "avi")),
            b"WAVE" => Some((Audio, "wav")),
            b"CDXA" => Some((Video, "mpg")),
            _ => None,
        };
    }
    if starts(b"OggS") {
        let header = data.get(28..40).unwrap_or_default();
        return Some(
            if header.starts_with(b"\x80theora") || header.starts_with(b"\x80kate") {
                (Video, "ogv")
            } else if header.starts_with(b"OpusHead") {
                (Audio, "opus")
            } else {
                (Audio, "ogg")
            },
        );
    }
    if starts(&[0x1A, 0x45, 0xDF, 0xA3]) {
        let webm = data.windows(4).take(64).any(|window| window == b"webm");
        return Some((Video, if webm { "webm" } else { "mkv" }));
    }

    let image = [
        (&b"\x89PNG\r\n\x1a\n"[..], "png"),
        (&[0xFF, 0xD8, 0xFF][..], "jpg"),
        (b"GIF87a", "gif"),
        (b"GIF89a", "gif"),
        (b"8BPS", "psd"),
        (&[0xFF, 0x0A][..], "jxl"),
        (b"\x00\x00\x00\x0cJXL \r\n\x87\n", "jxl"),
        (b"\x00\x00\x00\x0cjP  \r\n\x87\n", "jp2"),
        (&[0xFF, 0x4F, 0xFF, 0x51][..], "j2k"),
        (b"qoif", "qoi"),
        (b"DDS ", "dds"),
        (&[0x76, 0x2F, 0x31, 0x01][..], "exr"),
        (b"#?RADIANCE", "hdr"),
        (b"#?RGBE", "hdr"),
        (b"FUJIFILMCCD-RAW", "raf"),
        (b"IIRO", "orf"),
        (b"IIRS", "orf"),
        (b"IIU\0", "rw2"),
        (b"FOVb", "x3f"),
        (b"II*\0", "tif"),
        (b"MM\0*", "tif"),
        (b"II\xBC", "jxr"),
        (b"farbfeld", "ff"),
        (&[0x00, 0x00, 0x01, 0x00][..], "ico"),
        (&[0x00, 0x00, 0x02, 0x00][..], "cur"),
    ];
    for (signature, extension) in image {
        if starts(signature) {
            return Some((Image, extension));
        }
    }
    if starts(b"BM") && data.len() >= 26 && data[14] >= 12 && data[15] == 0 {
        return Some((Image, "bmp"));
    }

    let video = [
        (&b"FLV\x01"[..], "flv"),
        (&[0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11][..], "wmv"),
        (&[0x00, 0x00, 0x01, 0xBA][..], "mpg"),
        (&[0x00, 0x00, 0x01, 0xB3][..], "m2v"),
        (b".RMF", "rm"),
        (b"DKIF", "ivf"),
        (b"YUV4MPEG2", "y4m"),
        (b"nut/multimedia", "nut"),
        (b"BIKi", "bik"),
        (b"BIKb", "bik"),
        (b"SMK2", "smk"),
        (b"SMK4", "smk"),
        (b"FWS", "swf"),
        (b"CWS", "swf"),
        (&[0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01][..], "mxf"),
    ];
    for (signature, extension) in video {
        if starts(signature) {
            return Some((Video, extension));
        }
    }
    if data.len() > 376 && data[0] == 0x47 && data[188] == 0x47 && data[376] == 0x47 {
        return Some((Video, "ts"));
    }
    if data.len() > 388 && data[4] == 0x47 && data[196] == 0x47 && data[388] == 0x47 {
        return Some((Video, "m2ts"));
    }

    let audio = [
        (&b"fLaC"[..], "flac"),
        (b"ID3", "mp3"),
        (b"MAC ", "ape"),
        (b"wvpk", "wv"),
        (b"TTA1", "tta"),
        (b"MPCK", "mpc"),
        (b"MP+", "mpc"),
        (b"tBaK", "tak"),
        (b"#!AMR", "amr"),
        (b"DSD ", "dsf"),
        (b"FRM8", "dff"),
        (b"caff", "caf"),
        (b".snd", "au"),
        (b"OFR ", "ofr"),
        (b"ajkg", "shn"),
        (b"RF64", "rf64"),
        (b"Extended Module: ", "xm"),
        (b"IMPM", "it"),
        (&[0x0B, 0x77][..], "ac3"),
        (&[0x7F, 0xFE, 0x80, 0x01][..], "dts"),
        (b"Creative Voice File", "voc"),
    ];
    for (signature, extension) in audio {
        if starts(signature) {
            return Some((Audio, extension));
        }
    }
    if starts(b"FORM") && data.len() >= 12 {
        return match &data[8..12] {
            b"AIFF" | b"AIFC" => Some((Audio, "aiff")),
            b"8SVX" => Some((Audio, "8svx")),
            _ => None,
        };
    }
    if at(44, b"SCRM") {
        return Some((Audio, "s3m"));
    }
    if data.len() >= 2 && data[0] == 0xFF {
        match data[1] & 0xF6 {
            0xF0 => return Some((Audio, "aac")),
            _ if data[1] & 0xE0 == 0xE0 => return Some((Audio, "mp3")),
            _ => {}
        }
    }

    // SVG и другие XML-документы
    let text_start = data
        .iter()
        .position(|byte| {
            !byte.is_ascii_whitespace() && *byte != 0xEF && *byte != 0xBB && *byte != 0xBF
        })
        .unwrap_or(0);
    let text = &data[text_start..];
    if text.starts_with(b"<svg") || (text.starts_with(b"<?xml") && contains(data, b"<svg")) {
        return Some((Image, "svg"));
    }
    if starts(&[0x1F, 0x8B]) {
        // Сжатый SVG (svgz) — уточняется декодером.
        return None;
    }
    None
}

fn contains(data: &[u8], needle: &[u8]) -> bool {
    data.windows(needle.len()).any(|window| window == needle)
}

pub fn extension_kind(extension: &str) -> &'static str {
    let extension = extension.trim_start_matches('.').to_ascii_lowercase();
    if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        "image"
    } else if AUDIO_EXTENSIONS.contains(&extension.as_str()) {
        "audio"
    } else if VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        "video"
    } else if PLAYLIST_EXTENSIONS.contains(&extension.as_str()) {
        "document"
    } else {
        "media"
    }
}

/// Проверка для сканирования папок: только известные и пользовательские расширения.
pub fn is_supported_path_with_extensions(path: &Path, custom_extensions: &[String]) -> bool {
    if kind_by_extension(path) != MediaKind::Unknown {
        return true;
    }
    normalized_extension(path)
        .is_some_and(|ext| custom_extensions.iter().any(|value| value == &ext))
}

/// Проверка для явно открытых файлов: дополнительно распознаёт содержимое,
/// поэтому открываются файлы без расширения или с чужим расширением.
pub fn is_openable_path(path: &Path, custom_extensions: &[String]) -> bool {
    is_supported_path_with_extensions(path, custom_extensions) || sniff_file(path).is_some()
}

pub fn all_extensions(custom_extensions: &str) -> Vec<String> {
    let mut result = Vec::new();
    for value in IMAGE_EXTENSIONS
        .iter()
        .chain(VIDEO_EXTENSIONS.iter())
        .chain(AUDIO_EXTENSIONS.iter())
    {
        let value = (*value).to_string();
        if !result.contains(&value) {
            result.push(value);
        }
    }
    for value in parse_custom_extensions(custom_extensions) {
        if !result.contains(&value) {
            result.push(value);
        }
    }
    result.sort();
    result
}

pub fn parse_custom_extensions(value: &str) -> Vec<String> {
    let mut result = value
        .split([',', ';', ' ', '\n', '\r', '\t'])
        .map(|item| item.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|item| {
            !item.is_empty()
                && item
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
        .collect::<Vec<_>>();
    result.sort();
    result.dedup();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_common_signatures() {
        assert_eq!(
            sniff(b"\x89PNG\r\n\x1a\n...."),
            Some((MediaKind::Image, "png"))
        );
        assert_eq!(
            sniff(b"\0\0\0\x18ftypheic\0\0\0\0"),
            Some((MediaKind::Image, "heic"))
        );
        assert_eq!(
            sniff(b"\0\0\0\x18ftypisom\0\0\0\0"),
            Some((MediaKind::Video, "mp4"))
        );
        assert_eq!(
            sniff(b"\0\0\0\x18ftypM4A \0\0\0\0"),
            Some((MediaKind::Audio, "m4a"))
        );
        assert_eq!(sniff(b"fLaC\0\0\0\x22"), Some((MediaKind::Audio, "flac")));
        assert_eq!(sniff(b"ID3\x04\0\0"), Some((MediaKind::Audio, "mp3")));
        assert_eq!(
            sniff(b"RIFF\0\0\0\0WAVEfmt "),
            Some((MediaKind::Audio, "wav"))
        );
        assert_eq!(
            sniff(b"RIFF\0\0\0\0AVI LIST"),
            Some((MediaKind::Video, "avi"))
        );
        assert_eq!(sniff(b"FLV\x01\x05"), Some((MediaKind::Video, "flv")));
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?><svg xmlns="),
            Some((MediaKind::Image, "svg"))
        );
        assert_eq!(sniff(b"hello world"), None);
    }
}
