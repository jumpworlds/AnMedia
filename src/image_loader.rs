use crate::image_decode::{AnimationFrame, decode_for_view};
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::path::{Path, PathBuf};
use std::thread;

pub struct ImageData {
    pub path: PathBuf,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    pub original_size: [usize; 2],
    pub frames: Vec<AnimationFrame>,
    pub format: String,
    pub file_size: u64,
}

pub struct ImageResult {
    pub path: PathBuf,
    pub data: Result<ImageData, String>,
}

pub fn start_worker() -> (Sender<PathBuf>, Receiver<ImageResult>) {
    let (request_tx, request_rx) = unbounded::<PathBuf>();
    let (result_tx, result_rx) = unbounded::<ImageResult>();
    thread::spawn(move || {
        while let Ok(mut path) = request_rx.recv() {
            while let Ok(next) = request_rx.try_recv() {
                path = next;
            }
            let data = decode(&path);
            if result_tx.send(ImageResult { path, data }).is_err() {
                break;
            }
        }
    });
    (request_tx, result_rx)
}

fn decode(path: &Path) -> Result<ImageData, String> {
    let decoded = decode_for_view(path)?;
    Ok(ImageData {
        path: path.to_path_buf(),
        width: decoded.image.width,
        height: decoded.image.height,
        rgba: decoded.image.rgba,
        original_size: decoded.original_size,
        frames: decoded.frames,
        format: decoded.format,
        file_size: std::fs::metadata(path)
            .map(|value| value.len())
            .unwrap_or(0),
    })
}
