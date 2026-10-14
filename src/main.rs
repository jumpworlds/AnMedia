#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod associations;
#[cfg(windows)]
mod audio_engine;
#[cfg(windows)]
mod clipboard;
#[cfg(windows)]
mod config;
#[cfg(windows)]
mod convert;
#[cfg(windows)]
mod cover_art;
#[cfg(windows)]
mod formats;
#[cfg(windows)]
mod image_decode;
#[cfg(windows)]
mod image_loader;
#[cfg(windows)]
mod installation;
#[cfg(windows)]
mod media;
#[cfg(windows)]
mod mpv;
#[cfg(windows)]
mod native_media;
#[cfg(windows)]
mod platform;
#[cfg(windows)]
mod security;
#[cfg(windows)]
mod single_instance;
#[cfg(windows)]
mod theme;
#[cfg(windows)]
mod thumbnail;
#[cfg(windows)]
mod ui_kit;

#[cfg(windows)]
fn main() -> eframe::Result<()> {
    security::harden_process();
    let arguments = std::env::args_os()
        .skip(1)
        .map(std::path::PathBuf::from)
        .collect::<Vec<_>>();
    let single_instance = match single_instance::initialize(arguments) {
        single_instance::InstanceMode::Primary(instance) => instance,
        single_instance::InstanceMode::Secondary => return Ok(()),
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("AnMedia 2.0")
        .with_inner_size([1280.0, 760.0])
        .with_min_inner_size([900.0, 560.0]);
    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "AnMedia",
        native_options,
        Box::new(move |creation_context| {
            Ok(Box::new(app::AnMediaApp::new(
                creation_context,
                single_instance,
            )))
        }),
    )
}

#[cfg(windows)]
fn load_app_icon() -> Option<egui::viewport::IconData> {
    let image = image::load_from_memory(include_bytes!("../assets/anmedia.png"))
        .ok()?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::viewport::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

#[cfg(not(windows))]
fn main() {
    eprintln!("AnMedia предназначена для Windows 11");
}
