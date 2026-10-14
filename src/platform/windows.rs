use eframe::Frame;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, LWA_ALPHA, MoveWindow, SW_HIDE, SW_SHOW,
    SetLayeredWindowAttributes, ShowWindow, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
    WS_EX_LAYERED, WS_EX_TRANSPARENT, WS_VISIBLE,
};

const SS_BLACKRECT: u32 = 0x0004;
const SS_WHITERECT: u32 = 0x0006;

pub struct MediaHostWindow {
    container: HWND,
    surface: HWND,
    dark_overlay: HWND,
    light_overlay: HWND,
    visible: bool,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    zoom: f32,
    pan_x: i32,
    pan_y: i32,
    brightness: f32,
}

impl MediaHostWindow {
    pub fn create(parent: HWND) -> Result<Self, String> {
        let class_name = wide("STATIC");
        let instance = unsafe { GetModuleHandleW(null()) } as HINSTANCE;
        let container = create_child(
            parent,
            instance,
            0,
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN | SS_BLACKRECT,
            &class_name,
        );
        if container.is_null() {
            return Err("Не удалось создать область вывода мультимедиа".to_string());
        }
        let surface = create_child(
            container,
            instance,
            0,
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN | SS_BLACKRECT,
            &class_name,
        );
        if surface.is_null() {
            unsafe {
                DestroyWindow(container);
            }
            return Err("Не удалось создать окно видеодекодера".to_string());
        }
        let overlay_style = WS_EX_LAYERED | WS_EX_TRANSPARENT;
        let dark_overlay = create_child(
            container,
            instance,
            overlay_style,
            WS_CHILD | SS_BLACKRECT,
            &class_name,
        );
        let light_overlay = create_child(
            container,
            instance,
            overlay_style,
            WS_CHILD | SS_WHITERECT,
            &class_name,
        );
        if dark_overlay.is_null() || light_overlay.is_null() {
            unsafe {
                DestroyWindow(container);
            }
            return Err("Не удалось создать слой яркости видео".to_string());
        }
        unsafe {
            // Отключённое окно пропускает мышь к окну AnMedia: клики, колесо и
            // двойной щелчок по видео обрабатывает интерфейс, а не mpv.
            EnableWindow(container, 0);
            SetLayeredWindowAttributes(dark_overlay, 0, 0, LWA_ALPHA);
            SetLayeredWindowAttributes(light_overlay, 0, 0, LWA_ALPHA);
            ShowWindow(dark_overlay, SW_HIDE);
            ShowWindow(light_overlay, SW_HIDE);
        }
        Ok(Self {
            container,
            surface,
            dark_overlay,
            light_overlay,
            visible: true,
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            zoom: 1.0,
            pan_x: 0,
            pan_y: 0,
            brightness: 1.0,
        })
    }

    pub fn hwnd_value(&self) -> u64 {
        self.surface as usize as u64
    }

    pub fn set_bounds(&mut self, x: i32, y: i32, width: i32, height: i32) {
        let x = x.max(0);
        let y = y.max(0);
        let width = width.max(1);
        let height = height.max(1);
        if self.x == x && self.y == y && self.width == width && self.height == height {
            return;
        }
        self.x = x;
        self.y = y;
        self.width = width;
        self.height = height;
        unsafe {
            MoveWindow(self.container, self.x, self.y, self.width, self.height, 1);
        }
        self.layout_children();
    }

    /// Сдвиг увеличенного кадра в пикселях (как у фото).
    pub fn set_pan(&mut self, x: i32, y: i32) {
        if self.pan_x == x && self.pan_y == y {
            return;
        }
        self.pan_x = x;
        self.pan_y = y;
        self.layout_children();
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        let zoom = zoom.clamp(0.25, 8.0);
        if (self.zoom - zoom).abs() < f32::EPSILON {
            return;
        }
        self.zoom = zoom;
        self.layout_children();
    }

    pub fn set_brightness(&mut self, brightness: f32) {
        let brightness = brightness.clamp(0.25, 2.0);
        if (self.brightness - brightness).abs() < f32::EPSILON {
            return;
        }
        self.brightness = brightness;
        self.apply_brightness();
    }

    pub fn surface_size(&self) -> (i32, i32) {
        (
            ((self.width as f32) * self.zoom).round().max(1.0) as i32,
            ((self.height as f32) * self.zoom).round().max(1.0) as i32,
        )
    }

    pub fn set_visible(&mut self, visible: bool) {
        if self.visible == visible {
            return;
        }
        unsafe {
            ShowWindow(self.container, if visible { SW_SHOW } else { SW_HIDE });
        }
        self.visible = visible;
    }

    fn layout_children(&self) {
        let (surface_width, surface_height) = self.surface_size();
        let surface_x = (self.width - surface_width) / 2 + self.pan_x;
        let surface_y = (self.height - surface_height) / 2 + self.pan_y;
        unsafe {
            MoveWindow(
                self.surface,
                surface_x,
                surface_y,
                surface_width,
                surface_height,
                1,
            );
            MoveWindow(self.dark_overlay, 0, 0, self.width, self.height, 1);
            MoveWindow(self.light_overlay, 0, 0, self.width, self.height, 1);
        }
        self.apply_brightness();
    }

    fn apply_brightness(&self) {
        unsafe {
            ShowWindow(self.dark_overlay, SW_HIDE);
            ShowWindow(self.light_overlay, SW_HIDE);
        }
        if self.brightness < 0.995 {
            let alpha = (((1.0 - self.brightness) / 0.75) * 190.0)
                .round()
                .clamp(1.0, 220.0) as u8;
            unsafe {
                SetLayeredWindowAttributes(self.dark_overlay, 0, alpha, LWA_ALPHA);
                ShowWindow(self.dark_overlay, SW_SHOW);
            }
        } else if self.brightness > 1.005 {
            let alpha = (((self.brightness - 1.0) / 1.0) * 145.0)
                .round()
                .clamp(1.0, 170.0) as u8;
            unsafe {
                SetLayeredWindowAttributes(self.light_overlay, 0, alpha, LWA_ALPHA);
                ShowWindow(self.light_overlay, SW_SHOW);
            }
        }
    }
}

impl Drop for MediaHostWindow {
    fn drop(&mut self) {
        if !self.container.is_null() {
            unsafe {
                DestroyWindow(self.container);
            }
        }
    }
}

pub fn parent_hwnd(frame: &Frame) -> Option<HWND> {
    let handle = frame.window_handle().ok()?.as_raw();
    match handle {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as HWND),
        _ => None,
    }
}

fn create_child(
    parent: HWND,
    instance: HINSTANCE,
    extended_style: u32,
    style: u32,
    class_name: &[u16],
) -> HWND {
    unsafe {
        CreateWindowExW(
            extended_style,
            class_name.as_ptr(),
            null(),
            style,
            0,
            0,
            1,
            1,
            parent,
            null_mut(),
            instance,
            null::<c_void>(),
        )
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
