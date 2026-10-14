use crossbeam_channel::{Receiver, Sender, unbounded};
use directories::ProjectDirs;
use lofty::file::TaggedFileExt;
use lofty::picture::PictureType;
use lofty::tag::Accessor;
use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde_json::Value;
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

/// Разрешён ли поиск обложек в интернете (настройка «Безопасность»).
pub static ONLINE_COVERS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

const MAX_REMOTE_IMAGE_BYTES: u64 = 12 * 1024 * 1024;
const COVER_SIDE: u32 = 1200;
const MUSICBRAINZ_MINIMUM_INTERVAL: Duration = Duration::from_secs(1);

pub struct CoverData {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub struct CoverResult {
    pub path: PathBuf,
    pub title: String,
    pub artist: Option<String>,
    pub source: Option<String>,
    pub data: Option<CoverData>,
    pub error: Option<String>,
}

pub fn start_worker() -> (Sender<PathBuf>, Receiver<CoverResult>) {
    let (request_tx, request_rx) = unbounded::<PathBuf>();
    let (result_tx, result_rx) = unbounded::<CoverResult>();

    thread::Builder::new()
        .name("AnMedia-CoverArt".to_string())
        .spawn(move || {
            let client = Client::builder()
                .user_agent("AnMedia/2.0 (desktop media player; Anton4k272)")
                .timeout(Duration::from_secs(12))
                .connect_timeout(Duration::from_secs(5))
                .build()
                .ok();

            let mut last_musicbrainz_request = Instant::now() - MUSICBRAINZ_MINIMUM_INTERVAL;
            let mut musicbrainz_retry_after = None;
            while let Ok(mut path) = request_rx.recv() {
                while let Ok(next) = request_rx.try_recv() {
                    path = next;
                }
                let result = resolve_cover(
                    &path,
                    client.as_ref(),
                    &mut last_musicbrainz_request,
                    &mut musicbrainz_retry_after,
                );
                if result_tx.send(result).is_err() {
                    break;
                }
            }
        })
        .ok();

    (request_tx, result_rx)
}

/// Встроенная обложка аудиофайла (для миниатюр плейлиста).
pub fn embedded_cover_bytes(path: &Path) -> Option<Vec<u8>> {
    let tagged = lofty::read_from_path(path).ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    tag.get_picture_type(PictureType::CoverFront)
        .or_else(|| tag.pictures().first())
        .map(|picture| picture.data().to_vec())
}

fn resolve_cover(
    path: &Path,
    client: Option<&Client>,
    last_musicbrainz_request: &mut Instant,
    musicbrainz_retry_after: &mut Option<Instant>,
) -> CoverResult {
    let mut title = fallback_title(path);
    let mut artist = None;
    let mut embedded = None;

    if let Ok(tagged) = lofty::read_from_path(path) {
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            if let Some(value) = tag.title() {
                let value = value.trim();
                if !value.is_empty() {
                    title = value.to_string();
                }
            }
            if let Some(value) = tag.artist() {
                let value = value.trim();
                if !value.is_empty() {
                    artist = Some(value.to_string());
                }
            }
            embedded = tag
                .get_picture_type(PictureType::CoverFront)
                .or_else(|| tag.pictures().first())
                .map(|picture| picture.data().to_vec());
        }
    }

    if artist.is_none() {
        let parsed = split_artist_title(&title);
        if let Some(parsed_artist) = parsed.0 {
            artist = Some(parsed_artist);
            title = parsed.1;
        }
    }

    if let Some(bytes) = embedded {
        if let Ok(data) = decode_cover(&bytes) {
            return CoverResult {
                path: path.to_path_buf(),
                title,
                artist,
                source: Some("Обложка из файла".to_string()),
                data: Some(data),
                error: None,
            };
        }
    }

    if !ONLINE_COVERS.load(std::sync::atomic::Ordering::Relaxed) {
        return CoverResult {
            path: path.to_path_buf(),
            title,
            artist,
            source: None,
            data: None,
            error: None,
        };
    }

    let remote = resolve_remote(
        &title,
        artist.as_deref(),
        client,
        last_musicbrainz_request,
        musicbrainz_retry_after,
    );

    match remote {
        Ok(Some((data, source))) => CoverResult {
            path: path.to_path_buf(),
            title,
            artist,
            source: Some(source),
            data: Some(data),
            error: None,
        },
        Ok(None) => CoverResult {
            path: path.to_path_buf(),
            title,
            artist,
            source: None,
            data: None,
            error: None,
        },
        Err(error) => CoverResult {
            path: path.to_path_buf(),
            title,
            artist,
            source: None,
            data: None,
            error: Some(error),
        },
    }
}

fn resolve_remote(
    title: &str,
    artist: Option<&str>,
    client: Option<&Client>,
    last_musicbrainz_request: &mut Instant,
    musicbrainz_retry_after: &mut Option<Instant>,
) -> Result<Option<(CoverData, String)>, String> {
    let Some(client) = client else {
        return Ok(None);
    };
    if title.trim().is_empty() {
        return Ok(None);
    }

    let cache_path = cover_cache_path(title, artist);
    if cache_path.is_file() {
        if let Ok(bytes) = fs::read(&cache_path) {
            if let Ok(data) = decode_cover(&bytes) {
                return Ok(Some((data, "Кэш обложек".to_string())));
            }
        }
        let _ = fs::remove_file(&cache_path);
    }

    let mut providers_unavailable = 0usize;

    match resolve_deezer(client, title, artist) {
        Ok(Some(data)) => {
            save_cache(&cache_path, &data);
            return Ok(Some((data, "Обложка найдена через Deezer".to_string())));
        }
        Ok(None) => {}
        Err(_) => providers_unavailable += 1,
    }

    match resolve_itunes(client, title, artist) {
        Ok(Some(data)) => {
            save_cache(&cache_path, &data);
            return Ok(Some((
                data,
                "Обложка найдена через Apple Music".to_string(),
            )));
        }
        Ok(None) => {}
        Err(_) => providers_unavailable += 1,
    }

    match resolve_musicbrainz(
        client,
        title,
        artist,
        last_musicbrainz_request,
        musicbrainz_retry_after,
    ) {
        Ok(Some(data)) => {
            save_cache(&cache_path, &data);
            return Ok(Some((
                data,
                "Обложка найдена через MusicBrainz".to_string(),
            )));
        }
        Ok(None) => {}
        Err(_) => providers_unavailable += 1,
    }

    if providers_unavailable >= 3 {
        Err("Сервисы поиска обложек временно недоступны".to_string())
    } else {
        Ok(None)
    }
}

fn resolve_deezer(
    client: &Client,
    title: &str,
    artist: Option<&str>,
) -> Result<Option<CoverData>, String> {
    let queries = cover_search_queries(title, artist);
    let mut had_transport_error = false;

    for query in queries {
        for endpoint in [
            "https://api.deezer.com/search/track",
            "https://api.deezer.com/search/album",
        ] {
            let response = match client
                .get(endpoint)
                .query(&[("q", query.as_str()), ("limit", "50")])
                .send()
            {
                Ok(response) => response,
                Err(_) => {
                    had_transport_error = true;
                    continue;
                }
            };

            if response.status().is_server_error()
                || response.status() == StatusCode::TOO_MANY_REQUESTS
            {
                had_transport_error = true;
                continue;
            }
            if !response.status().is_success() {
                continue;
            }

            let Ok(json) = response.json::<Value>() else {
                had_transport_error = true;
                continue;
            };
            if json.get("error").is_some() {
                had_transport_error = true;
                continue;
            }
            let Some(results) = json.get("data").and_then(Value::as_array) else {
                continue;
            };

            let mut best: Option<(i32, String)> = None;
            for item in results {
                let candidate_title = item
                    .get("title")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("title_short").and_then(Value::as_str))
                    .unwrap_or_default();
                let candidate_artist = item
                    .get("artist")
                    .and_then(|value| value.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let candidate_album = item
                    .get("album")
                    .and_then(|value| value.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or(candidate_title);
                let artwork = item
                    .get("album")
                    .and_then(deezer_artwork_url)
                    .or_else(|| deezer_artwork_url(item));
                let Some(artwork) = artwork else {
                    continue;
                };

                let mut score = cover_match_score(title, artist, candidate_title, candidate_artist);
                score += text_match_score(
                    &normalize_for_match(title),
                    &normalize_for_match(candidate_album),
                ) / 4;

                if best
                    .as_ref()
                    .is_none_or(|(best_score, _)| score > *best_score)
                {
                    best = Some((score, artwork.to_string()));
                }
            }

            if let Some((score, artwork)) = best {
                if score < 45 {
                    continue;
                }
                match download_cover(client, &artwork) {
                    Ok(Some(data)) => return Ok(Some(data)),
                    Ok(None) => {}
                    Err(_) => had_transport_error = true,
                }
            }
        }
    }

    if had_transport_error {
        Err("Сервис Deezer временно недоступен".to_string())
    } else {
        Ok(None)
    }
}

fn deezer_artwork_url(value: &Value) -> Option<&str> {
    value
        .get("cover_xl")
        .and_then(Value::as_str)
        .or_else(|| value.get("cover_big").and_then(Value::as_str))
        .or_else(|| value.get("cover_medium").and_then(Value::as_str))
        .or_else(|| value.get("cover").and_then(Value::as_str))
}

fn resolve_itunes(
    client: &Client,
    title: &str,
    artist: Option<&str>,
) -> Result<Option<CoverData>, String> {
    let queries = cover_search_queries(title, artist);
    let mut had_transport_error = false;

    for term in queries {
        for country in ["RU", "US", "KZ", "UA", "UZ", "MX", "GB"] {
            for entity in ["song", "album"] {
                let response = match client
                    .get("https://itunes.apple.com/search")
                    .query(&[
                        ("term", term.as_str()),
                        ("media", "music"),
                        ("entity", entity),
                        ("limit", "50"),
                        ("country", country),
                    ])
                    .send()
                {
                    Ok(response) => response,
                    Err(_) => {
                        had_transport_error = true;
                        continue;
                    }
                };

                if !response.status().is_success() {
                    if response.status().is_server_error()
                        || response.status() == StatusCode::TOO_MANY_REQUESTS
                    {
                        had_transport_error = true;
                    }
                    continue;
                }

                let Ok(json) = response.json::<Value>() else {
                    had_transport_error = true;
                    continue;
                };
                let Some(results) = json.get("results").and_then(Value::as_array) else {
                    continue;
                };

                let mut best: Option<(i32, String)> = None;
                for item in results {
                    let Some(artwork) = item.get("artworkUrl100").and_then(Value::as_str) else {
                        continue;
                    };
                    let candidate_title = item
                        .get("trackName")
                        .and_then(Value::as_str)
                        .or_else(|| item.get("collectionName").and_then(Value::as_str))
                        .unwrap_or_default();
                    let candidate_artist = item
                        .get("artistName")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let score = cover_match_score(title, artist, candidate_title, candidate_artist);
                    if best
                        .as_ref()
                        .is_none_or(|(best_score, _)| score > *best_score)
                    {
                        best = Some((score, artwork.to_string()));
                    }
                }

                if let Some((score, artwork)) = best {
                    if score < 45 {
                        continue;
                    }
                    let large = upscale_itunes_artwork(&artwork);
                    if let Ok(Some(data)) = download_cover(client, &large) {
                        return Ok(Some(data));
                    }
                    if large != artwork {
                        if let Ok(Some(data)) = download_cover(client, &artwork) {
                            return Ok(Some(data));
                        }
                    }
                }
            }
        }
    }

    if had_transport_error {
        Err("Сервис Apple Music временно недоступен".to_string())
    } else {
        Ok(None)
    }
}

fn cover_search_queries(title: &str, artist: Option<&str>) -> Vec<String> {
    let clean_title = clean_search_text(title);
    let clean_artist = artist.map(clean_search_text).unwrap_or_default();
    let mut queries = Vec::new();

    if !clean_artist.is_empty() {
        queries.push(format!("artist:\"{clean_artist}\" track:\"{clean_title}\""));
        queries.push(format!("{clean_artist} {clean_title}"));
        queries.push(format!("{clean_title} {clean_artist}"));
    }
    queries.push(clean_title.clone());
    if clean_title != title.trim() {
        queries.push(title.trim().to_string());
    }

    let mut unique = HashSet::new();
    queries
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .filter(|value| unique.insert(normalize_for_match(value)))
        .collect()
}

fn clean_search_text(value: &str) -> String {
    let mut result = value
        .replace('_', " ")
        .replace("SkySound.cc", " ")
        .replace("skysound.cc", " ")
        .replace("SkySound", " ")
        .replace("skysound", " ");
    for separator in [" | ", " :: "] {
        if let Some((left, _)) = result.split_once(separator) {
            result = left.to_string();
        }
    }
    result
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|character: char| {
            matches!(character, '-' | '–' | '—' | '(' | ')' | '[' | ']')
        })
        .trim()
        .to_string()
}

fn resolve_musicbrainz(
    client: &Client,
    title: &str,
    artist: Option<&str>,
    last_musicbrainz_request: &mut Instant,
    retry_after: &mut Option<Instant>,
) -> Result<Option<CoverData>, String> {
    if retry_after.is_some_and(|deadline| Instant::now() < deadline) {
        return Ok(None);
    }

    let query = build_musicbrainz_query(title, artist);
    let mut json = None;

    for attempt in 0..2u64 {
        let elapsed = last_musicbrainz_request.elapsed();
        if elapsed < MUSICBRAINZ_MINIMUM_INTERVAL {
            thread::sleep(MUSICBRAINZ_MINIMUM_INTERVAL - elapsed);
        }
        *last_musicbrainz_request = Instant::now();

        let response = client
            .get("https://musicbrainz.org/ws/2/recording/")
            .query(&[("query", query.as_str()), ("fmt", "json"), ("limit", "8")])
            .send()
            .map_err(|_| "MusicBrainz недоступен".to_string())?;

        if response.status() == StatusCode::SERVICE_UNAVAILABLE
            || response.status() == StatusCode::TOO_MANY_REQUESTS
        {
            *retry_after = Some(Instant::now() + Duration::from_secs(10 * 60));
            if attempt == 0 {
                thread::sleep(Duration::from_millis(250));
            }
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err("MusicBrainz отклонил запрос".to_string());
        }
        json = Some(
            response
                .json::<Value>()
                .map_err(|_| "MusicBrainz вернул некорректный ответ".to_string())?,
        );
        *retry_after = None;
        break;
    }

    let Some(json) = json else {
        return Ok(None);
    };

    let mut release_groups = Vec::new();
    let mut releases = Vec::new();
    let mut unique_groups = HashSet::new();
    let mut unique_releases = HashSet::new();

    if let Some(recordings) = json.get("recordings").and_then(Value::as_array) {
        for recording in recordings {
            let score = recording.get("score").and_then(Value::as_u64).unwrap_or(0);
            if score < 65 {
                continue;
            }
            if let Some(items) = recording.get("releases").and_then(Value::as_array) {
                for release in items {
                    if let Some(group_id) = release
                        .get("release-group")
                        .and_then(|value| value.get("id"))
                        .and_then(Value::as_str)
                    {
                        if unique_groups.insert(group_id.to_string()) {
                            release_groups.push(group_id.to_string());
                        }
                    }
                    if let Some(release_id) = release.get("id").and_then(Value::as_str) {
                        if unique_releases.insert(release_id.to_string()) {
                            releases.push(release_id.to_string());
                        }
                    }
                }
            }
        }
    }

    for id in release_groups.iter().take(5) {
        let url = format!("https://coverartarchive.org/release-group/{id}/front-500");
        if let Ok(Some(data)) = download_cover(client, &url) {
            return Ok(Some(data));
        }
    }

    for id in releases.iter().take(5) {
        let url = format!("https://coverartarchive.org/release/{id}/front-500");
        if let Ok(Some(data)) = download_cover(client, &url) {
            return Ok(Some(data));
        }
    }

    Ok(None)
}

fn download_cover(client: &Client, url: &str) -> Result<Option<CoverData>, String> {
    let response = client
        .get(url)
        .send()
        .map_err(|_| "не удалось загрузить обложку".to_string())?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err("сервер обложек временно недоступен".to_string());
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_REMOTE_IMAGE_BYTES)
    {
        return Err("обложка превышает допустимый размер".to_string());
    }
    let bytes = response
        .bytes()
        .map_err(|_| "не удалось прочитать обложку".to_string())?;
    if bytes.len() as u64 > MAX_REMOTE_IMAGE_BYTES {
        return Err("обложка превышает допустимый размер".to_string());
    }
    decode_cover(&bytes).map(Some)
}

fn cover_match_score(
    title: &str,
    artist: Option<&str>,
    candidate_title: &str,
    candidate_artist: &str,
) -> i32 {
    let title = normalize_for_match(title);
    let candidate_title = normalize_for_match(candidate_title);
    let artist = artist.map(normalize_for_match).unwrap_or_default();
    let candidate_artist = normalize_for_match(candidate_artist);

    let mut score = text_match_score(&title, &candidate_title);
    if !artist.is_empty() {
        score += text_match_score(&artist, &candidate_artist) * 4 / 5;
    }
    score
}

fn text_match_score(expected: &str, actual: &str) -> i32 {
    if expected.is_empty() || actual.is_empty() {
        return 0;
    }
    if expected == actual {
        return 100;
    }
    if expected.contains(actual) || actual.contains(expected) {
        return 72;
    }

    let expected_tokens = expected.split_whitespace().collect::<HashSet<_>>();
    let actual_tokens = actual.split_whitespace().collect::<HashSet<_>>();
    if expected_tokens.is_empty() || actual_tokens.is_empty() {
        return 0;
    }
    let common = expected_tokens.intersection(&actual_tokens).count() as i32;
    let total = expected_tokens.union(&actual_tokens).count() as i32;
    common * 65 / total.max(1)
}

fn normalize_for_match(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn upscale_itunes_artwork(url: &str) -> String {
    for token in ["100x100bb", "100x100-75", "100x100"] {
        if url.contains(token) {
            return url.replace(token, "1200x1200bb");
        }
    }
    url.to_string()
}

fn decode_cover(bytes: &[u8]) -> Result<CoverData, String> {
    let image = image::load_from_memory(bytes)
        .map_err(|error| format!("декодирование обложки: {error}"))?;
    let image = image.thumbnail(COVER_SIDE, COVER_SIDE).to_rgba8();
    let (width, height) = image.dimensions();
    Ok(CoverData {
        width: width as usize,
        height: height as usize,
        rgba: image.into_raw(),
    })
}

fn save_cache(path: &Path, data: &CoverData) {
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Some(image) =
        image::RgbaImage::from_raw(data.width as u32, data.height as u32, data.rgba.clone())
    else {
        return;
    };
    let temporary = path.with_extension("tmp.png");
    if image.save(&temporary).is_ok() {
        let _ = fs::remove_file(path);
        let _ = fs::rename(temporary, path);
    }
}

fn cover_cache_path(title: &str, artist: Option<&str>) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    title.to_lowercase().hash(&mut hasher);
    artist.unwrap_or_default().to_lowercase().hash(&mut hasher);
    let file_name = format!("{:016x}.png", hasher.finish());
    ProjectDirs::from("", "Anton4k272", "AnMedia")
        .map(|directories| directories.cache_dir().join("covers").join(&file_name))
        .unwrap_or_else(|| PathBuf::from("covers").join(&file_name))
}

fn build_musicbrainz_query(title: &str, artist: Option<&str>) -> String {
    let title = escape_lucene(title);
    match artist.filter(|value| !value.trim().is_empty()) {
        Some(artist) => format!(
            "recording:\"{}\" AND artist:\"{}\"",
            title,
            escape_lucene(artist)
        ),
        None => format!("recording:\"{title}\""),
    }
}

fn escape_lucene(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn fallback_title(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Аудиофайл")
        .replace('_', " ");
    let mut value = stem.split_whitespace().collect::<Vec<_>>().join(" ");
    value = remove_leading_track_number(&value);
    for suffix in [
        "official audio",
        "official music video",
        "official video",
        "lyrics",
        "lyric video",
        "audio",
        "hq",
    ] {
        value = remove_suffix_group(&value, suffix);
    }
    value.trim().to_string()
}

fn remove_leading_track_number(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index == 0 {
        return value.to_string();
    }
    while index < bytes.len() && matches!(bytes[index], b'.' | b'-' | b'_' | b' ' | b')' | b']') {
        index += 1;
    }
    value[index..].trim().to_string()
}

fn remove_suffix_group(value: &str, suffix: &str) -> String {
    let lower = value.to_lowercase();
    for (open, close) in [('(', ')'), ('[', ']')] {
        let token = format!("{open}{suffix}{close}");
        if lower.ends_with(&token) {
            return value[..value.len().saturating_sub(token.len())]
                .trim()
                .to_string();
        }
    }
    value.to_string()
}

fn split_artist_title(value: &str) -> (Option<String>, String) {
    for separator in [" - ", " – ", " — "] {
        if let Some((artist, title)) = value.split_once(separator) {
            let artist = artist.trim();
            let title = title.trim();
            if !artist.is_empty() && !title.is_empty() {
                return (Some(artist.to_string()), title.to_string());
            }
        }
    }
    (None, value.trim().to_string())
}
