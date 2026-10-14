//! Тема оформления AnMedia.
//!
//! Тёмная и светлая палитры с выбираемым акцентом, шрифты Segoe UI и
//! системный набор иконок Windows (Segoe Fluent Icons / Segoe MDL2 Assets).

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
    Vec2,
};
use std::fs;

pub struct Accent {
    pub name: &'static str,
    pub dark: Color32,
    pub light: Color32,
}

pub const ACCENTS: &[Accent] = &[
    Accent {
        name: "Индиго",
        dark: Color32::from_rgb(132, 118, 255),
        light: Color32::from_rgb(98, 80, 232),
    },
    Accent {
        name: "Синий",
        dark: Color32::from_rgb(76, 151, 255),
        light: Color32::from_rgb(28, 112, 230),
    },
    Accent {
        name: "Бирюзовый",
        dark: Color32::from_rgb(38, 200, 182),
        light: Color32::from_rgb(0, 150, 136),
    },
    Accent {
        name: "Розовый",
        dark: Color32::from_rgb(255, 99, 146),
        light: Color32::from_rgb(222, 52, 108),
    },
    Accent {
        name: "Оранжевый",
        dark: Color32::from_rgb(255, 146, 64),
        light: Color32::from_rgb(226, 108, 22),
    },
    Accent {
        name: "Зелёный",
        dark: Color32::from_rgb(76, 205, 110),
        light: Color32::from_rgb(30, 150, 70),
    },
];

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_active: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub background: Color32,
    pub surface: Color32,
    pub card: Color32,
    pub card_hover: Color32,
    pub field: Color32,
    pub outline: Color32,
    pub outline_strong: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    pub text_faint: Color32,
    pub danger: Color32,
    pub viewer: Color32,
    pub seek_track: Color32,
    pub video_tint: Color32,
    pub audio_tint: Color32,
    pub image_tint: Color32,
}

pub fn palette(dark: bool, accent_index: usize) -> Palette {
    let accent_preset = &ACCENTS[accent_index.min(ACCENTS.len() - 1)];
    if dark {
        let accent = accent_preset.dark;
        Palette {
            dark,
            accent,
            accent_hover: accent.lerp_to_gamma(Color32::WHITE, 0.14),
            accent_active: accent.lerp_to_gamma(Color32::BLACK, 0.12),
            accent_soft: accent.gamma_multiply(0.20),
            on_accent: Color32::WHITE,
            background: Color32::from_rgb(13, 14, 18),
            surface: Color32::from_rgb(19, 21, 26),
            card: Color32::from_rgb(28, 31, 38),
            card_hover: Color32::from_rgb(37, 41, 50),
            field: Color32::from_rgb(24, 27, 33),
            outline: Color32::from_rgb(36, 40, 49),
            outline_strong: Color32::from_rgb(54, 60, 72),
            text: Color32::from_rgb(236, 238, 243),
            text_weak: Color32::from_rgb(160, 167, 181),
            text_faint: Color32::from_rgb(105, 113, 129),
            danger: Color32::from_rgb(255, 107, 107),
            viewer: Color32::from_rgb(8, 9, 11),
            seek_track: Color32::from_rgb(48, 53, 64),
            video_tint: Color32::from_rgb(92, 160, 255),
            audio_tint: Color32::from_rgb(255, 112, 160),
            image_tint: Color32::from_rgb(80, 210, 140),
        }
    } else {
        let accent = accent_preset.light;
        Palette {
            dark,
            accent,
            accent_hover: accent.lerp_to_gamma(Color32::WHITE, 0.12),
            accent_active: accent.lerp_to_gamma(Color32::BLACK, 0.12),
            accent_soft: accent.gamma_multiply(0.13),
            on_accent: Color32::WHITE,
            background: Color32::from_rgb(240, 242, 246),
            surface: Color32::from_rgb(251, 251, 253),
            card: Color32::from_rgb(240, 242, 246),
            card_hover: Color32::from_rgb(229, 232, 239),
            field: Color32::WHITE,
            outline: Color32::from_rgb(224, 227, 234),
            outline_strong: Color32::from_rgb(200, 205, 215),
            text: Color32::from_rgb(22, 25, 32),
            text_weak: Color32::from_rgb(88, 96, 112),
            text_faint: Color32::from_rgb(146, 153, 167),
            danger: Color32::from_rgb(214, 54, 54),
            viewer: Color32::from_rgb(14, 15, 18),
            seek_track: Color32::from_rgb(214, 218, 227),
            video_tint: Color32::from_rgb(40, 120, 235),
            audio_tint: Color32::from_rgb(222, 52, 120),
            image_tint: Color32::from_rgb(20, 160, 90),
        }
    }
}

/// Символы системного шрифта иконок Windows.
pub mod icon {
    pub const PLAY: &str = "\u{E768}";
    pub const PAUSE: &str = "\u{E769}";
    pub const PREVIOUS: &str = "\u{E892}";
    pub const NEXT: &str = "\u{E893}";
    pub const SHUFFLE: &str = "\u{E8B1}";
    pub const REPEAT_ONE: &str = "\u{E8ED}";
    pub const VOLUME_0: &str = "\u{E992}";
    pub const VOLUME_1: &str = "\u{E993}";
    pub const VOLUME_2: &str = "\u{E994}";
    pub const VOLUME_3: &str = "\u{E995}";
    pub const MUTE: &str = "\u{E74F}";
    pub const FULLSCREEN: &str = "\u{E740}";
    pub const EXIT_FULLSCREEN: &str = "\u{E73F}";
    pub const SETTINGS: &str = "\u{E713}";
    pub const OPEN_FILE: &str = "\u{E8E5}";
    pub const FOLDER: &str = "\u{E8B7}";
    pub const ADD: &str = "\u{E710}";
    pub const LIST: &str = "\u{E8FD}";
    pub const SEARCH: &str = "\u{E721}";
    pub const CLOSE: &str = "\u{E711}";
    pub const DELETE: &str = "\u{E74D}";
    pub const ZOOM_IN: &str = "\u{E8A3}";
    pub const ZOOM_OUT: &str = "\u{E71F}";
    pub const ROTATE_LEFT: &str = "\u{E80C}";
    pub const ROTATE_RIGHT: &str = "\u{E80D}";
    pub const COPY: &str = "\u{E8C8}";
    pub const PHOTO: &str = "\u{E91B}";
    pub const MUSIC: &str = "\u{EC4F}";
    pub const VIDEO: &str = "\u{E714}";
    pub const ERROR: &str = "\u{E783}";
    pub const PIN: &str = "\u{E718}";
    pub const CHECK: &str = "\u{E73E}";
    pub const FOLDER_OPEN: &str = "\u{E838}";
    pub const CONVERT: &str = "\u{E8AB}";
    pub const SHIELD: &str = "\u{EA18}";
}

pub fn icons_family() -> FontFamily {
    FontFamily::Name("icons".into())
}

pub fn semibold_family() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, icons_family())
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, semibold_family())
}

pub fn install_fonts(ctx: &egui::Context) {
    let read_first = |paths: &[&str]| paths.iter().find_map(|path| fs::read(path).ok());
    let mut fonts = FontDefinitions::default();
    let defaults = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();

    let mut proportional = defaults.clone();
    if let Some(data) = read_first(&[
        r"C:\Windows\Fonts\segoeui.ttf",
        r"C:\Windows\Fonts\arial.ttf",
    ]) {
        fonts
            .font_data
            .insert("ui".to_string(), FontData::from_owned(data).into());
        proportional.insert(0, "ui".to_string());
    }
    let mut semibold = proportional.clone();
    if let Some(data) = read_first(&[
        r"C:\Windows\Fonts\seguisb.ttf",
        r"C:\Windows\Fonts\segoeuib.ttf",
    ]) {
        fonts
            .font_data
            .insert("ui_semibold".to_string(), FontData::from_owned(data).into());
        semibold.insert(0, "ui_semibold".to_string());
    }
    let mut icons = defaults;
    if let Some(data) = read_first(&[
        r"C:\Windows\Fonts\SegoeIcons.ttf",
        r"C:\Windows\Fonts\segmdl2.ttf",
    ]) {
        fonts
            .font_data
            .insert("ui_icons".to_string(), FontData::from_owned(data).into());
        icons.insert(0, "ui_icons".to_string());
    }
    fonts
        .families
        .insert(FontFamily::Proportional, proportional.clone());
    fonts.families.insert(semibold_family(), semibold);
    fonts.families.insert(icons_family(), icons);
    if let Some(monospace) = fonts.families.get_mut(&FontFamily::Monospace) {
        monospace.push("ui".to_string());
    }
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context, palette: &Palette) {
    let mut visuals = if palette.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    let radius = CornerRadius::same(8);
    visuals.override_text_color = Some(palette.text);
    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.surface;
    visuals.window_stroke = Stroke::new(1.0, palette.outline);
    visuals.window_corner_radius = CornerRadius::same(16);
    visuals.window_shadow = egui::Shadow {
        offset: [0, 12],
        blur: 40,
        spread: 0,
        color: Color32::from_black_alpha(if palette.dark { 140 } else { 50 }),
    };
    visuals.popup_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 20,
        spread: 0,
        color: Color32::from_black_alpha(if palette.dark { 120 } else { 40 }),
    };
    visuals.menu_corner_radius = CornerRadius::same(10);
    visuals.extreme_bg_color = palette.field;
    visuals.faint_bg_color = palette.card;
    visuals.code_bg_color = palette.field;
    visuals.hyperlink_color = palette.accent;
    visuals.error_fg_color = palette.danger;
    visuals.warn_fg_color = Color32::from_rgb(232, 170, 70);
    visuals.slider_trailing_fill = true;
    visuals.handle_shape = egui::style::HandleShape::Circle;
    visuals.text_cursor.stroke = Stroke::new(2.0, palette.accent);

    visuals.selection.bg_fill =
        palette
            .accent
            .gamma_multiply(if palette.dark { 0.45 } else { 0.28 });
    visuals.selection.stroke = Stroke::new(1.0, palette.accent);

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = palette.surface;
    widgets.noninteractive.weak_bg_fill = palette.surface;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.outline);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text_weak);
    widgets.noninteractive.corner_radius = radius;

    widgets.inactive.bg_fill = palette.card;
    widgets.inactive.weak_bg_fill = palette.card;
    widgets.inactive.bg_stroke = Stroke::NONE;
    widgets.inactive.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.inactive.corner_radius = radius;

    widgets.hovered.bg_fill = palette.card_hover;
    widgets.hovered.weak_bg_fill = palette.card_hover;
    widgets.hovered.bg_stroke = Stroke::new(1.0, palette.outline_strong);
    widgets.hovered.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.hovered.corner_radius = radius;
    widgets.hovered.expansion = 0.0;

    widgets.active.bg_fill = palette.accent;
    widgets.active.weak_bg_fill = palette.card_hover;
    widgets.active.bg_stroke = Stroke::new(1.0, palette.accent);
    widgets.active.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.active.corner_radius = radius;
    widgets.active.expansion = 0.0;

    widgets.open.bg_fill = palette.card_hover;
    widgets.open.weak_bg_fill = palette.card_hover;
    widgets.open.bg_stroke = Stroke::new(1.0, palette.accent);
    widgets.open.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.open.corner_radius = radius;

    ctx.set_visuals(visuals);
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.interact_size = Vec2::new(36.0, 30.0);
        style.spacing.slider_width = 160.0;
        style.spacing.window_margin = egui::Margin::same(20);
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.scroll.bar_width = 6.0;
        style.spacing.scroll.floating = true;
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(14.0));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(14.0));
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(12.0));
        style.text_styles.insert(TextStyle::Heading, semibold(20.0));
        style.interaction.selectable_labels = false;
    });
}
