use crate::formats::{
    is_openable_path, is_supported_path_with_extensions, parse_custom_extensions,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn collect_media_from_directory(directory: &Path, custom_extensions: &str) -> Vec<PathBuf> {
    let custom_extensions = parse_custom_extensions(custom_extensions);
    collect_media_from_directory_with_extensions(directory, &custom_extensions)
}

fn collect_media_from_directory_with_extensions(
    directory: &Path,
    custom_extensions: &[String],
) -> Vec<PathBuf> {
    let mut files = WalkDir::new(directory)
        .follow_links(false)
        .max_depth(8)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| is_supported_path_with_extensions(path, custom_extensions))
        .collect::<Vec<_>>();
    files.sort_by_cached_key(|path| path.to_string_lossy().to_lowercase());
    files
}

pub fn collect_media_from_parent_directory(file: &Path, custom_extensions: &str) -> Vec<PathBuf> {
    let Some(directory) = file.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let custom_extensions = parse_custom_extensions(custom_extensions);
    let mut files = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && is_supported_path_with_extensions(path, &custom_extensions)
        })
        .collect::<Vec<_>>();
    files.sort_by_cached_key(|path| {
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_lowercase()
    });
    files
}

pub fn normalize_input_paths(paths: Vec<PathBuf>, custom_extensions: &str) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let custom_extensions = parse_custom_extensions(custom_extensions);
    for path in paths.into_iter().map(make_absolute) {
        if path.is_dir() {
            for file in collect_media_from_directory_with_extensions(&path, &custom_extensions) {
                push_unique(&mut result, &mut seen, file);
            }
        } else if path.is_file() && is_openable_path(&path, &custom_extensions) {
            push_unique(&mut result, &mut seen, path);
        }
    }
    result
}

fn make_absolute(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }
    std::env::current_dir()
        .map(|directory| directory.join(&path))
        .unwrap_or(path)
}

fn push_unique(result: &mut Vec<PathBuf>, seen: &mut HashSet<String>, path: PathBuf) {
    let key = path.to_string_lossy().replace('/', "\\").to_lowercase();
    if seen.insert(key) {
        result.push(path);
    }
}
