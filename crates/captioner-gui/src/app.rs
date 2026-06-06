use crossbeam_channel::Receiver;
use eframe::egui::{
    Area, Color32, DragValue, FontId, Frame, Key,
    Order, Pos2, Rect, RichText, ScrollArea,
    Stroke, Vec2,
};
use eframe::{egui, App, NativeOptions};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::debug;

use captioner_core::{
    AppCommand, HotkeyEvent, TranscriptionResult,
};

pub struct DuplicateDetector;

impl DuplicateDetector {
    pub fn new(_threshold: f32, _recent_count: usize) -> Self { Self }
    pub fn add_line(&mut self, _line: &str) {}
    pub fn is_duplicate(&self, _text: &str) -> bool { false }
    pub fn clear(&mut self) {}
}

impl Default for DuplicateDetector {
    fn default() -> Self { Self }
}

pub fn process_text_with_furigana(text: &str) -> String {
    text.to_string()
}

const MODEL_OPTIONS: [&str; 11] = [
    "tiny", "base", "small", "medium", "large", "large-v2", "large-v3",
    "tiny.en", "base.en", "small.en", "medium.en",
];

pub struct CaptionerGUI {
    lines: Vec<String>,
    font_size: f32,
    alpha: u8,
    monitor: usize,
    top: bool,
    fullscreen: bool,
    fullwidth: bool,
    fullheight: bool,
    last_geometry: Option<Rect>,
    window_width: f32,
    window_height: f32,
    pos_x: f32,
    pos_y: f32,
    recording_enabled: bool,
    language: Option<String>,
    transcription_rx: tokio::sync::broadcast::Receiver<TranscriptionResult>,
    hotkey_rx: Receiver<HotkeyEvent>,
    duplicate_detector: DuplicateDetector,
    show_settings: bool,
    stop_flag: Arc<AtomicBool>,
    command_tx: tokio::sync::broadcast::Sender<AppCommand>,
    current_model: String,
    model_index: usize,
    show_toolbar: bool,
    hovered_toolbar: bool,
}

impl CaptionerGUI {
    pub fn new(
        stop_flag: Arc<AtomicBool>,
        command_tx: tokio::sync::broadcast::Sender<AppCommand>,
        transcription_rx: tokio::sync::broadcast::Receiver<TranscriptionResult>,
        hotkey_rx: Receiver<HotkeyEvent>,
        language: Option<String>,
    ) -> Self {
        Self {
            lines: Vec::new(),
            font_size: 55.0,
            alpha: 128,
            monitor: 2,
            top: false,
            fullscreen: false,
            fullwidth: false,
            fullheight: false,
            last_geometry: None,
            window_width: 1200.0,
            window_height: 210.0,
            pos_x: 0.0,
            pos_y: 870.0,
            recording_enabled: true,
            language,
            transcription_rx,
            hotkey_rx,
            duplicate_detector: DuplicateDetector,
            show_settings: false,
            stop_flag,
            command_tx,
            current_model: "base".to_string(),
            model_index: 1,
            show_toolbar: false,
            hovered_toolbar: false,
        }
    }

    fn handle_hotkey(&mut self, event: HotkeyEvent) {
        match event {
            HotkeyEvent::ToggleRecording => {
                self.recording_enabled = !self.recording_enabled;
                let _ = self.command_tx.send(AppCommand::ToggleRecording);
                debug!("Toggle recording: {}", self.recording_enabled);
            }
            HotkeyEvent::IncreaseFontSize => {
                self.font_size = (self.font_size + 2.0).min(200.0);
            }
            HotkeyEvent::DecreaseFontSize => {
                self.font_size = (self.font_size - 2.0).max(10.0);
            }
            HotkeyEvent::ClearText => {
                self.lines.clear();
                let _ = self.command_tx.send(AppCommand::Clear);
            }
            HotkeyEvent::IncreaseTransparency => {
                self.alpha = (self.alpha as i32 - 12).clamp(20, 255) as u8;
            }
            HotkeyEvent::DecreaseTransparency => {
                self.alpha = (self.alpha as i32 + 12).clamp(20, 255) as u8;
            }
            HotkeyEvent::IncreaseWindowWidth => {
                self.window_width = (self.window_width + 50.0).min(3000.0);
            }
            HotkeyEvent::DecreaseWindowWidth => {
                self.window_width = (self.window_width - 50.0).max(200.0);
            }
            HotkeyEvent::IncreaseWindowHeight => {
                self.window_height = (self.window_height + 50.0).min(2000.0);
            }
            HotkeyEvent::DecreaseWindowHeight => {
                self.window_height = (self.window_height - 50.0).max(100.0);
            }
            HotkeyEvent::MoveMonitor => {
                self.monitor = if self.monitor == 1 { 2 } else { 1 };
            }
            HotkeyEvent::ToggleTop => {
                self.top = !self.top;
            }
            HotkeyEvent::ShowSettings => {
                self.show_settings = !self.show_settings;
            }
            HotkeyEvent::ToggleFullscreen => {
                self.toggle_fullscreen();
            }
            HotkeyEvent::ToggleFullwidth => {
                self.toggle_fullwidth();
            }
            HotkeyEvent::ToggleFullheight => {
                self.toggle_fullheight();
            }
        }
    }

    fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
        if self.fullscreen {
            self.last_geometry = Some(Rect::from_min_size(
                Pos2::new(self.pos_x, self.pos_y),
                Vec2::new(self.window_width, self.window_height),
            ));
        } else if let Some(g) = self.last_geometry.take() {
            self.pos_x = g.min.x;
            self.pos_y = g.min.y;
            self.window_width = g.width();
            self.window_height = g.height();
        }
    }

    fn toggle_fullwidth(&mut self) {
        self.fullwidth = !self.fullwidth;
        if self.fullwidth {
            self.last_geometry = Some(Rect::from_min_size(
                Pos2::new(self.pos_x, self.pos_y),
                Vec2::new(self.window_width, self.window_height),
            ));
            self.window_width = 1920.0;
        } else if let Some(g) = self.last_geometry.take() {
            self.pos_x = g.min.x;
            self.pos_y = g.min.y;
            self.window_width = g.width();
            self.window_height = g.height();
        }
    }

    fn toggle_fullheight(&mut self) {
        self.fullheight = !self.fullheight;
        if self.fullheight {
            self.last_geometry = Some(Rect::from_min_size(
                Pos2::new(self.pos_x, self.pos_y),
                Vec2::new(self.window_width, self.window_height),
            ));
            self.window_height = 1080.0;
            self.pos_y = 0.0;
        } else if let Some(g) = self.last_geometry.take() {
            self.pos_x = g.min.x;
            self.pos_y = g.min.y;
            self.window_width = g.width();
            self.window_height = g.height();
        }
    }

    fn add_line(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        if self.duplicate_detector.is_duplicate(trimmed) {
            return;
        }
        let processed = process_text_with_furigana(trimmed);
        self.duplicate_detector.add_line(trimmed);
        self.lines.push(processed);
    }
}

impl App for CaptionerGUI {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(result) = self.transcription_rx.try_recv() {
            if !self.recording_enabled {
                continue;
            }
            self.add_line(&result.text);
        }

        while let Ok(event) = self.hotkey_rx.try_recv() {
            self.handle_hotkey(event);
        }

        let input = ctx.input(|i| {
            let mut events = Vec::new();
            for e in &i.events {
                if let egui::Event::Key { key, pressed, modifiers, .. } = e {
                    if *pressed {
                        events.push((*key, *modifiers));
                    }
                }
            }
            events
        });

        for (key, mods) in input {
            let no_mods = !mods.ctrl && !mods.alt;
            let ctrl_alt = mods.ctrl && mods.alt;

            if (key == Key::Equals || key == Key::Plus) && (no_mods || ctrl_alt) {
                self.font_size = (self.font_size + 2.0).min(200.0);
            }
            if key == Key::Minus && (no_mods || ctrl_alt) {
                self.font_size = (self.font_size - 2.0).max(10.0);
            }
            if key == Key::Num9 && !mods.ctrl && !mods.alt {
                self.alpha = (self.alpha as i32 - 12).clamp(20, 255) as u8;
            }
            if key == Key::Num0 && !mods.ctrl && !mods.alt {
                self.alpha = (self.alpha as i32 + 12).clamp(20, 255) as u8;
            }
            if key == Key::X && ((!mods.ctrl && !mods.alt) || (mods.ctrl && mods.alt)) {
                self.lines.clear();
                let _ = self.command_tx.send(AppCommand::Clear);
            }
            if key == Key::F && !mods.ctrl && !mods.alt {
                self.toggle_fullscreen();
            }
            if key == Key::W && !mods.ctrl && !mods.alt {
                self.toggle_fullwidth();
            }
            if key == Key::H && !mods.ctrl && !mods.alt {
                self.toggle_fullheight();
            }
            if key == Key::B && !mods.ctrl && !mods.alt {
                self.top = !self.top;
            }
            if key == Key::Space && !mods.ctrl && !mods.alt {
                self.monitor = if self.monitor == 1 { 2 } else { 1 };
            }
            if key == Key::T && !mods.ctrl && !mods.alt {
                self.show_settings = !self.show_settings;
            }
            if key == Key::Q && mods.ctrl && mods.alt {
                self.stop_flag.store(true, Ordering::SeqCst);
            }
            if mods.alt && key == Key::ArrowLeft {
                self.pos_x -= 10.0;
            }
            if mods.alt && key == Key::ArrowRight {
                self.pos_x += 10.0;
            }
            if mods.alt && key == Key::ArrowUp {
                self.pos_y -= 10.0;
            }
            if mods.alt && key == Key::ArrowDown {
                self.pos_y += 10.0;
            }
        }

        ctx.set_visuals(egui::Visuals::dark());

        let screen_size = ctx.available_rect().size();
        let frame_rect = if self.fullscreen || (self.fullwidth && self.fullheight) {
            Rect::from_min_size(Pos2::ZERO, screen_size)
        } else if self.fullwidth {
            Rect::from_min_size(Pos2::new(0.0, self.pos_y), Vec2::new(screen_size.x, self.window_height))
        } else if self.fullheight {
            Rect::from_min_size(Pos2::new(self.pos_x, 0.0), Vec2::new(self.window_width, screen_size.y))
        } else {
            Rect::from_min_size(Pos2::new(self.pos_x, self.pos_y), Vec2::new(self.window_width, self.window_height))
        };

        let mouse_pos = ctx.input(|i| i.pointer.hover_pos());
        let toolbar_zone_top = frame_rect.min.y + 40.0;
        self.show_toolbar = mouse_pos.is_some_and(|p| p.y < toolbar_zone_top && p.x >= frame_rect.min.x && p.x <= frame_rect.max.x);
        self.hovered_toolbar = self.show_toolbar;

        Area::new(egui::Id::new("captioner_overlay"))
        .fixed_pos(frame_rect.min)
        .order(Order::Foreground)
        .show(ctx, |ui| {
            let bg_alpha = self.alpha.max(40);
            Frame::none()
            .fill(Color32::from_rgba_unmultiplied(0, 0, 0, bg_alpha))
            .show(ui, |ui| {
                        ui.set_min_size(frame_rect.size());

                        if self.show_toolbar {
                            self.draw_toolbar(ui, &frame_rect);
                        }

                        let font_id = FontId::monospace(self.font_size);

                        ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .stick_to_bottom(true)
                            .show(ui, |scroll_ui| {
                                let mut text = self.lines.join("\n");
                                if !text.is_empty() && !self.recording_enabled {
                                    text.push_str("\n[Recording Paused]");
                                }

                                scroll_ui.label(
                                    RichText::new(&text)
                                        .color(Color32::WHITE)
                                        .font(font_id),
                                );
                            });

                        if !self.recording_enabled {
                            let dot_radius = 10.0;
                            let margin = 5.0;
                            let dot_x = frame_rect.max.x - dot_radius * 2.0 - margin;
                            let dot_y = frame_rect.min.y + margin;
                            let painter = ui.painter();
                            painter.circle(
                                Pos2::new(dot_x, dot_y + dot_radius),
                                dot_radius,
                                Color32::from_rgb(200, 50, 50),
                                Stroke::default(),
                            );
                        }
                    });
            });

        if self.show_settings {
            self.show_settings_dialog(ctx, &frame_rect);
        }

        ctx.request_repaint();
    }
}

impl CaptionerGUI {
    fn draw_toolbar(&mut self, ui: &mut egui::Ui, frame_rect: &Rect) {
        let toolbar_alpha = 200u8;

        Frame::none()
            .fill(Color32::from_rgba_unmultiplied(20, 20, 30, toolbar_alpha))
            .inner_margin(egui::Margin::symmetric(8.0, 4.0))
            .show(ui, |ui| {
                ui.set_min_width(frame_rect.width());
                ui.horizontal(|row| {
                    row.spacing_mut().item_spacing.x = 6.0;

                    let rec_color = if self.recording_enabled {
                        Color32::from_rgb(60, 160, 60)
                    } else {
                        Color32::from_rgb(180, 50, 50)
                    };
                    let rec_text = if self.recording_enabled { "\u{2B24} REC" } else { "\u{25CB} PAUSED" };
                    if row.button(RichText::new(rec_text).color(rec_color).size(13.0)).clicked() {
                        self.recording_enabled = !self.recording_enabled;
                        let _ = self.command_tx.send(AppCommand::ToggleRecording);
                    }

                    row.separator();

                    let clear_resp = row.button(RichText::new("Clear").size(13.0));
                    if clear_resp.clicked() {
                        self.lines.clear();
                        self.duplicate_detector.clear();
                        let _ = self.command_tx.send(AppCommand::Clear);
                    }

                    row.separator();

                    row.label(RichText::new("Model:").size(12.0).color(Color32::from_rgb(180, 180, 180)));
                    let model_combo = egui::ComboBox::from_id_salt("toolbar_model")
                        .width(90.0)
                        .selected_text(MODEL_OPTIONS[self.model_index]);
                    model_combo.show_ui(row, |cb_ui| {
                        for (i, model) in MODEL_OPTIONS.iter().enumerate() {
                            cb_ui.selectable_value(&mut self.model_index, i, *model);
                        }
                    });
                    if MODEL_OPTIONS.get(self.model_index).copied() != Some(self.current_model.as_str()) {
                        if let Some(&new_model) = MODEL_OPTIONS.get(self.model_index) {
                            self.current_model = new_model.to_string();
                            let _ = self.command_tx.send(AppCommand::SetModel(new_model.to_string()));
                        }
                    }

                    row.separator();

                    if row.button(RichText::new("\u{2699} Settings").size(13.0)).clicked() {
                        self.show_settings = !self.show_settings;
                    }

                    row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |right| {
                        if right.button(RichText::new("\u{2715}").size(14.0)).clicked() {
                            self.stop_flag.store(true, Ordering::SeqCst);
                        }
                    });
                });
            });
    }
}

impl CaptionerGUI {
    fn show_settings_dialog(&mut self, ctx: &egui::Context, frame_rect: &Rect) {
        let dialog_width = 400.0;
        let dialog_height = 300.0;
        let dialog_x = frame_rect.min.x + (frame_rect.width() - dialog_width) / 2.0;
        let dialog_y = frame_rect.min.y + (frame_rect.height() - dialog_height) / 2.0;

        let mut lang_input = self.language.clone().unwrap_or_default();
        let mut font_size = self.font_size;

        Area::new(egui::Id::new("settings_dialog"))
            .fixed_pos(Pos2::new(dialog_x, dialog_y))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                Frame::none()
                    .fill(Color32::from_rgba_unmultiplied(30, 30, 50, 230))
                    .inner_margin(20.0)
                    .show(ui, |ui| {
                        ui.set_min_size(Vec2::new(dialog_width, dialog_height));
                        ui.heading("Settings");
                        ui.separator();
                        ui.add_space(8.0);

                        ui.horizontal(|row_ui| {
                            row_ui.label("Model:");
                            egui::ComboBox::from_id_salt("settings_model")
                                .width(200.0)
                                .selected_text(MODEL_OPTIONS[self.model_index])
                                .show_ui(row_ui, |cb_ui| {
                                    for (i, model) in MODEL_OPTIONS.iter().enumerate() {
                                        cb_ui.selectable_value(&mut self.model_index, i, *model);
                                    }
                                });
                        });

                        ui.horizontal(|row_ui| {
                            row_ui.label("Language:");
                            row_ui.text_edit_singleline(&mut lang_input);
                        });

                        ui.horizontal(|row_ui| {
                            row_ui.label("Font Size:");
                            row_ui.add(DragValue::new(&mut font_size).range(10.0..=200.0).speed(1.0));
                        });

                        ui.separator();

                        ui.horizontal(|btn_ui| {
                            if btn_ui.button("OK").clicked() {
                                self.show_settings = false;
                                let new_lang = if lang_input.is_empty() {
                                    None
                                } else {
                                    Some(lang_input.clone())
                                };
                                self.language = new_lang.clone();
                                self.font_size = font_size;
                                if let Some(&new_model) = MODEL_OPTIONS.get(self.model_index) {
                                    self.current_model = new_model.to_string();
                                    let _ = self.command_tx.send(AppCommand::SetModel(new_model.to_string()));
                                }
                                let _ = self.command_tx.send(AppCommand::SetLanguage(new_lang));
                            }
                            if btn_ui.button("Cancel").clicked() {
                                self.show_settings = false;
                            }
                        });
                    });
            });
    }
}

impl CaptionerGUI {
    pub fn run(
        stop_flag: Arc<AtomicBool>,
        command_tx: tokio::sync::broadcast::Sender<AppCommand>,
        transcription_rx: tokio::sync::broadcast::Receiver<TranscriptionResult>,
        hotkey_rx: Receiver<HotkeyEvent>,
        language: Option<String>,
    ) -> eframe::Result<()> {
        let viewport = egui::ViewportBuilder::default()
            .with_decorations(false)
            .with_always_on_top()
            .with_inner_size(Vec2::new(1200.0, 210.0))
            .with_resizable(true);

        let options = NativeOptions {
            viewport,
            ..Default::default()
        };

        let app = Self::new(
            stop_flag,
            command_tx,
            transcription_rx,
            hotkey_rx,
            language,
        );

        eframe::run_native(
            "Captioner",
            options,
            Box::new(|_cc| Ok(Box::new(app))),
        )
    }
}
