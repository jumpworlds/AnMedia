//! Виджеты интерфейса AnMedia: кнопки, переключатели, сикбар, плитки.

use crate::formats::MediaKind;
use crate::theme::{Palette, icon, icon_font, semibold};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke,
    TextureHandle, Ui, Vec2, epaint::CircleShape,
};

/// Плоская кнопка панели инструментов: иконка и необязательная подпись.
pub fn tool_button(
    ui: &mut Ui,
    palette: &Palette,
    glyph: &str,
    label: Option<&str>,
    active: bool,
    tooltip: &str,
) -> Response {
    let height = 34.0;
    let label_galley = label.map(|text| {
        ui.painter()
            .layout_no_wrap(text.to_string(), FontId::proportional(13.5), palette.text)
    });
    let width = match &label_galley {
        Some(galley) => 12.0 + 16.0 + 8.0 + galley.size().x + 14.0,
        None => height,
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let hovered = response.hovered();
    let pressed = response.is_pointer_button_down_on();
    let fill = if active {
        palette.accent_soft
    } else if pressed {
        palette.card_hover
    } else if hovered {
        palette.card
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 9.0, fill);
    let color = if active { palette.accent } else { palette.text };
    match label_galley {
        Some(galley) => {
            let icon_center = Pos2::new(rect.left() + 12.0 + 8.0, rect.center().y);
            ui.painter().text(
                icon_center,
                Align2::CENTER_CENTER,
                glyph,
                icon_font(15.0),
                color,
            );
            ui.painter().galley(
                Pos2::new(rect.left() + 36.0, rect.center().y - galley.size().y / 2.0),
                galley,
                color,
            );
        }
        None => {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                glyph,
                icon_font(15.0),
                color,
            );
        }
    }
    response.on_hover_text(tooltip)
}

/// Круглая кнопка с иконкой (транспорт, громкость, полный экран).
pub fn icon_button(
    ui: &mut Ui,
    palette: &Palette,
    glyph: &str,
    size: f32,
    active: bool,
    enabled: bool,
    tooltip: &str,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::splat(size),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hovered = enabled && response.hovered();
    let pressed = enabled && response.is_pointer_button_down_on();
    let fill = if pressed {
        palette.card_hover
    } else if hovered {
        palette.card
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().circle_filled(rect.center(), size / 2.0, fill);
    let color = if !enabled {
        palette.text_faint.gamma_multiply(0.6)
    } else if active {
        palette.accent
    } else if hovered {
        palette.text
    } else {
        palette.text_weak
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        icon_font((size * 0.44).round()),
        color,
    );
    if active {
        ui.painter().circle_filled(
            Pos2::new(rect.center().x, rect.bottom() - 3.0),
            2.0,
            palette.accent,
        );
    }
    response.on_hover_text(tooltip)
}

/// Главная кнопка воспроизведения.
pub fn play_button(ui: &mut Ui, palette: &Palette, playing: bool, enabled: bool) -> Response {
    let diameter = 46.0;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::splat(diameter),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hovered = enabled && response.hovered();
    let fill = if !enabled {
        palette.card
    } else if response.is_pointer_button_down_on() {
        palette.accent_active
    } else if hovered {
        palette.accent_hover
    } else {
        palette.accent
    };
    if enabled {
        ui.painter().circle_filled(
            rect.center() + Vec2::new(0.0, 3.0),
            diameter / 2.0 + 2.0,
            palette
                .accent
                .gamma_multiply(if hovered { 0.30 } else { 0.18 }),
        );
    }
    ui.painter()
        .circle_filled(rect.center(), diameter / 2.0, fill);
    let glyph = if playing { icon::PAUSE } else { icon::PLAY };
    let offset = if playing { 0.0 } else { 1.5 };
    ui.painter().text(
        rect.center() + Vec2::new(offset, 0.0),
        Align2::CENTER_CENTER,
        glyph,
        icon_font(18.0),
        if enabled {
            palette.on_accent
        } else {
            palette.text_faint
        },
    );
    response.on_hover_text(if playing {
        "Пауза (Пробел)"
    } else {
        "Воспроизвести (Пробел)"
    })
}

/// Обычная кнопка с заливкой (основная или второстепенная).
pub fn action_button(
    ui: &mut Ui,
    palette: &Palette,
    glyph: Option<&str>,
    label: &str,
    primary: bool,
) -> Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        semibold(14.0),
        if primary {
            palette.on_accent
        } else {
            palette.text
        },
    );
    let icon_width = if glyph.is_some() { 24.0 } else { 0.0 };
    let size = Vec2::new(galley.size().x + icon_width + 36.0, 40.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    let fill = match (primary, hovered) {
        (true, true) => palette.accent_hover,
        (true, false) => palette.accent,
        (false, true) => palette.card_hover,
        (false, false) => palette.card,
    };
    ui.painter().rect_filled(rect, 11.0, fill);
    if !primary {
        ui.painter().rect_stroke(
            rect,
            11.0,
            Stroke::new(1.0, palette.outline),
            egui::StrokeKind::Inside,
        );
    }
    let text_color = if primary {
        palette.on_accent
    } else {
        palette.text
    };
    let mut x = rect.left() + 18.0;
    if let Some(glyph) = glyph {
        ui.painter().text(
            Pos2::new(x + 8.0, rect.center().y),
            Align2::CENTER_CENTER,
            glyph,
            icon_font(15.0),
            text_color,
        );
        x += icon_width;
    }
    ui.painter().galley(
        Pos2::new(x, rect.center().y - galley.size().y / 2.0),
        galley,
        text_color,
    );
    response
}

/// Переключатель в стиле Windows 11.
pub fn toggle(ui: &mut Ui, palette: &Palette, value: &mut bool, label: &str) -> Response {
    let width = ui.available_width();
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    let progress = ui.ctx().animate_bool_with_time(response.id, *value, 0.12);
    let track = Rect::from_center_size(
        Pos2::new(rect.right() - 22.0, rect.center().y),
        Vec2::new(40.0, 20.0),
    );
    let track_fill = if *value {
        palette.accent
    } else if response.hovered() {
        palette.card_hover
    } else {
        palette.field
    };
    ui.painter().rect_filled(track, 10.0, track_fill);
    if !*value {
        ui.painter().rect_stroke(
            track,
            10.0,
            Stroke::new(1.0, palette.outline_strong),
            egui::StrokeKind::Inside,
        );
    }
    let knob_x = egui::lerp(track.left() + 10.0..=track.right() - 10.0, progress);
    let knob_radius = if response.hovered() { 7.0 } else { 6.0 };
    ui.painter().circle_filled(
        Pos2::new(knob_x, track.center().y),
        knob_radius,
        if *value {
            palette.on_accent
        } else {
            palette.text_weak
        },
    );
    ui.painter().text(
        Pos2::new(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        palette.text,
    );
    response
}

/// Сегментированный переключатель вариантов.
pub fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    palette: &Palette,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    let width = ui.available_width();
    let height = 34.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    ui.painter().rect_filled(rect, 10.0, palette.field);
    ui.painter().rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0, palette.outline),
        egui::StrokeKind::Inside,
    );
    let segment = (rect.width() - 6.0) / options.len().max(1) as f32;
    let mut changed = false;
    for (index, (option, label)) in options.iter().enumerate() {
        let segment_rect = Rect::from_min_size(
            Pos2::new(rect.left() + 3.0 + segment * index as f32, rect.top() + 3.0),
            Vec2::new(segment, height - 6.0),
        );
        let response = ui.interact(
            segment_rect,
            ui.id().with(("segment", index, label)),
            Sense::click(),
        );
        let selected = *value == *option;
        if selected {
            ui.painter().rect_filled(segment_rect, 8.0, palette.accent);
        } else if response.hovered() {
            ui.painter()
                .rect_filled(segment_rect, 8.0, palette.card_hover);
        }
        ui.painter().text(
            segment_rect.center(),
            Align2::CENTER_CENTER,
            *label,
            if selected {
                semibold(13.0)
            } else {
                FontId::proportional(13.0)
            },
            if selected {
                palette.on_accent
            } else {
                palette.text_weak
            },
        );
        if response.clicked() && !selected {
            *value = *option;
            changed = true;
        }
    }
    changed
}

/// Сикбар: тонкая дорожка, растёт при наведении, показывает время под курсором.
pub fn seek_bar(
    ui: &mut Ui,
    palette: &Palette,
    value: &mut f64,
    duration: f64,
    enabled: bool,
    width: f32,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width.max(60.0), 20.0),
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let active = enabled && (response.hovered() || response.dragged());
    let grow = ui.ctx().animate_bool_with_time(response.id, active, 0.12);
    let track_height = egui::lerp(4.0..=7.0, grow);
    let track = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), track_height));
    let painter = ui.painter();
    painter.rect_filled(track, track_height / 2.0, palette.seek_track);

    let duration = duration.max(0.000_001);
    let mut changed = false;
    if enabled
        && (response.clicked() || response.dragged())
        && let Some(position) = response.interact_pointer_pos()
    {
        let fraction = ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        *value = fraction as f64 * duration;
        changed = true;
    }

    if enabled && let Some(position) = response.hover_pos() {
        let hover_x = position.x.clamp(track.left(), track.right());
        painter.rect_filled(
            Rect::from_min_max(track.min, Pos2::new(hover_x, track.bottom())),
            track_height / 2.0,
            palette.text_faint.gamma_multiply(0.35),
        );
    }

    let fraction = (*value / duration).clamp(0.0, 1.0) as f32;
    if enabled && fraction > 0.0 {
        let filled = Rect::from_min_max(
            track.min,
            Pos2::new(track.left() + track.width() * fraction, track.bottom()),
        );
        painter.rect_filled(filled, track_height / 2.0, palette.accent);
    }
    if enabled {
        let knob = Pos2::new(track.left() + track.width() * fraction, track.center().y);
        let radius = egui::lerp(0.0..=7.0, grow);
        if radius > 0.5 {
            painter.circle_filled(knob, radius + 2.0, palette.accent.gamma_multiply(0.25));
            painter.circle_filled(knob, radius, Color32::WHITE);
        }
    }
    if active && let Some(position) = response.hover_pos() {
        let pointer_fraction = ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64;
        let text = format_time(pointer_fraction * duration);
        let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), palette.text);
        let bubble = Rect::from_center_size(
            Pos2::new(
                position.x.clamp(
                    rect.left() + galley.size().x / 2.0 + 8.0,
                    rect.right() - galley.size().x / 2.0 - 8.0,
                ),
                rect.top() - 14.0,
            ),
            galley.size() + Vec2::new(14.0, 6.0),
        );
        let tooltip_painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            response.id.with("seek_tip"),
        ));
        tooltip_painter.rect_filled(bubble, 7.0, palette.card);
        tooltip_painter.rect_stroke(
            bubble,
            7.0,
            Stroke::new(1.0, palette.outline_strong),
            egui::StrokeKind::Inside,
        );
        tooltip_painter.galley(bubble.center() - galley.size() / 2.0, galley, palette.text);
    }
    changed
}

/// Компактный слайдер громкости.
pub fn volume_slider(ui: &mut Ui, palette: &Palette, value: &mut f64, width: f32) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::click_and_drag());
    let track = Rect::from_center_size(rect.center(), Vec2::new(rect.width() - 10.0, 4.0));
    let painter = ui.painter();
    painter.rect_filled(track, 2.0, palette.seek_track);
    let mut changed = false;
    if (response.clicked() || response.dragged())
        && let Some(position) = response.interact_pointer_pos()
    {
        let fraction = ((position.x - track.left()) / track.width()).clamp(0.0, 1.0);
        *value = (fraction * 100.0).round() as f64;
        changed = true;
    }
    if response.hovered() {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll.abs() > 0.0 {
            *value = (*value + (scroll / 12.0) as f64).clamp(0.0, 100.0);
            changed = true;
        }
    }
    let fraction = (*value / 100.0).clamp(0.0, 1.0) as f32;
    painter.rect_filled(
        Rect::from_min_max(
            track.min,
            Pos2::new(track.left() + track.width() * fraction, track.bottom()),
        ),
        2.0,
        if response.hovered() || response.dragged() {
            palette.accent
        } else {
            palette.text_weak
        },
    );
    let knob = Pos2::new(track.left() + track.width() * fraction, track.center().y);
    let radius = if response.hovered() || response.dragged() {
        7.0
    } else {
        5.5
    };
    painter.circle_filled(knob, radius, Color32::WHITE);
    painter.circle_stroke(knob, radius, Stroke::new(1.0, palette.outline_strong));
    response.on_hover_text(format!("Громкость: {:.0}%", *value));
    changed
}

/// Пилюля с текстом (скорость, масштаб). Возвращает ответ для меню.
pub fn pill_button(ui: &mut Ui, palette: &Palette, text: &str, tooltip: &str) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), semibold(12.5), palette.text);
    let size = Vec2::new((galley.size().x + 20.0).max(48.0), 28.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if response.hovered() {
        palette.card_hover
    } else {
        palette.card
    };
    ui.painter().rect_filled(rect, 14.0, fill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, palette.text);
    response.on_hover_text(tooltip)
}

/// Маленькая метка-чип (фильтры плейлиста).
pub fn chip(ui: &mut Ui, palette: &Palette, text: &str, selected: bool) -> Response {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_string(), FontId::proportional(12.5), palette.text);
    let size = Vec2::new(galley.size().x + 20.0, 26.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let (fill, color) = if selected {
        (palette.accent_soft, palette.accent)
    } else if response.hovered() {
        (palette.card_hover, palette.text)
    } else {
        (palette.card, palette.text_weak)
    };
    ui.painter().rect_filled(rect, 13.0, fill);
    if selected {
        ui.painter().rect_stroke(
            rect,
            13.0,
            Stroke::new(1.0, palette.accent.gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    response
}

pub fn kind_color(palette: &Palette, kind: MediaKind) -> Color32 {
    match kind {
        MediaKind::Image => palette.image_tint,
        MediaKind::Audio => palette.audio_tint,
        MediaKind::Video | MediaKind::Unknown => palette.video_tint,
    }
}

pub fn kind_glyph(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => icon::PHOTO,
        MediaKind::Audio => icon::MUSIC,
        MediaKind::Video | MediaKind::Unknown => icon::VIDEO,
    }
}

/// Плитка-заглушка с иконкой типа файла.
pub fn paint_kind_tile(painter: &egui::Painter, palette: &Palette, rect: Rect, kind: MediaKind) {
    let color = kind_color(palette, kind);
    painter.rect_filled(
        rect,
        9.0,
        color.gamma_multiply(if palette.dark { 0.16 } else { 0.12 }),
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        kind_glyph(kind),
        icon_font((rect.height() * 0.40).round()),
        color,
    );
}

/// Текстура с обрезкой по квадрату и скруглением (миниатюры, обложки).
pub fn paint_texture_cover(ui: &Ui, texture: &TextureHandle, rect: Rect, radius: u8) {
    let size = texture.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let target_aspect = rect.width() / rect.height();
    let aspect = size.x / size.y;
    let uv = if aspect > target_aspect {
        let visible = target_aspect / aspect;
        Rect::from_min_max(
            Pos2::new((1.0 - visible) / 2.0, 0.0),
            Pos2::new((1.0 + visible) / 2.0, 1.0),
        )
    } else {
        let visible = aspect / target_aspect;
        Rect::from_min_max(
            Pos2::new(0.0, (1.0 - visible) / 2.0),
            Pos2::new(1.0, (1.0 + visible) / 2.0),
        )
    };
    egui::Image::new(texture)
        .uv(uv)
        .corner_radius(CornerRadius::same(radius))
        .paint_at(ui, rect);
}

/// Анимированный эквалайзер у играющего трека.
pub fn paint_equalizer(ui: &Ui, center: Pos2, color: Color32, animate: bool) {
    let time = ui.input(|input| input.time) as f32;
    let painter = ui.painter();
    for index in 0..3 {
        let phase = time * 6.0 + index as f32 * 1.7;
        let height = if animate {
            5.0 + (phase.sin() * 0.5 + 0.5) * 9.0
        } else {
            4.0 + index as f32 * 2.0
        };
        let x = center.x - 5.0 + index as f32 * 5.0;
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(x - 1.5, center.y + 7.0 - height),
                Pos2::new(x + 1.5, center.y + 7.0),
            ),
            1.5,
            color,
        );
    }
    if animate {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
}

/// Мягкое свечение позади элемента.
pub fn paint_glow(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32) {
    for step in 0..8 {
        let t = step as f32 / 8.0;
        painter.add(CircleShape::filled(
            center,
            radius * (1.0 - t * 0.6),
            color.gamma_multiply(0.035 + t * 0.02),
        ));
    }
}

pub fn paint_checkerboard(painter: &egui::Painter, rect: Rect, cell: f32) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 || cell <= 0.0 {
        return;
    }
    let light = Color32::from_rgb(58, 60, 66);
    let dark = Color32::from_rgb(44, 46, 51);
    painter.rect_filled(rect, 0.0, light);
    let columns = (rect.width() / cell).ceil() as i32;
    let rows = (rect.height() / cell).ceil() as i32;
    if columns * rows > 40_000 {
        return;
    }
    for row in 0..rows {
        for column in 0..columns {
            if (row + column) % 2 == 0 {
                continue;
            }
            let min = Pos2::new(
                rect.left() + column as f32 * cell,
                rect.top() + row as f32 * cell,
            );
            let max = Pos2::new(
                (min.x + cell).min(rect.right()),
                (min.y + cell).min(rect.bottom()),
            );
            painter.rect_filled(Rect::from_min_max(min, max), 0.0, dark);
        }
    }
}

pub fn paint_brightness_overlay(
    painter: &egui::Painter,
    rect: Rect,
    brightness: f32,
    rounding: f32,
) {
    if brightness < 0.995 {
        let alpha = (((1.0 - brightness.clamp(0.25, 1.0)) / 0.75) * 210.0)
            .round()
            .clamp(0.0, 220.0) as u8;
        painter.rect_filled(rect, rounding, Color32::from_black_alpha(alpha));
    } else if brightness > 1.005 {
        let alpha = (((brightness.clamp(1.0, 2.0) - 1.0) / 1.0) * 150.0)
            .round()
            .clamp(0.0, 170.0) as u8;
        painter.rect_filled(rect, rounding, Color32::from_white_alpha(alpha));
    }
}

pub fn format_time(seconds: f64) -> String {
    let seconds = seconds.max(0.0).round() as u64;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["Б", "КБ", "МБ", "ГБ"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
