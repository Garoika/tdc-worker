#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use eframe::egui;
use serde::Deserialize;

const SEMIBOLD_FAMILY: &str = "semibold";

fn semibold_family() -> egui::FontFamily {
    egui::FontFamily::Name(SEMIBOLD_FAMILY.into())
}

fn semibold_text(text: impl Into<String>, size: f32, color: egui::Color32) -> egui::RichText {
    egui::RichText::new(text)
        .font(egui::FontId::new(size, semibold_family()))
        .color(color)
}

#[derive(Debug, Clone, Deserialize)]
struct GameStateInfo {
    #[serde(default)]
    order: u64,
    count: usize,
    active_count: usize,
    waiting_count: usize,
    is_active: bool,
    streamers: Vec<String>,
    min_watched: i32,
    required_minutes: i32,
    #[allow(dead_code)]
    left_minutes: i32,
    time_left: String,
    drop_index: i32,
    drop_total: i32,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkerStateFile {
    updated_at: u64,
    #[allow(dead_code)]
    start_time: u64,
    uptime_seconds: u64,
    node_name: String,
    master_url: String,
    master_connected: bool,
    #[allow(dead_code)]
    parent_pid: u32,
    #[allow(dead_code)]
    cpu_percent: f64,
    ram_used_mb: f64,
    ram_total_mb: f64,
    total_accounts: usize,
    max_containers: usize,
    games: BTreeMap<String, GameStateInfo>,
}

struct TdcDashboardApp {
    state: Option<WorkerStateFile>,
    last_read: SystemTime,
}

impl Default for TdcDashboardApp {
    fn default() -> Self {
        Self {
            state: None,
            last_read: SystemTime::now(),
        }
    }
}

impl TdcDashboardApp {
    fn reload_state(&mut self) {
        let candidates = [
            Path::new(".worker_state.json"),
            Path::new("../.worker_state.json"),
        ];

        for p in &candidates {
            if p.exists() {
                if let Ok(content) = fs::read_to_string(p) {
                    if let Ok(st) = serde_json::from_str::<WorkerStateFile>(&content) {
                        self.state = Some(st);
                        self.last_read = SystemTime::now();
                        return;
                    }
                }
            }
        }
    }
}

impl eframe::App for TdcDashboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll state file every 500ms
        if self.last_read.elapsed().unwrap_or(Duration::from_secs(10)) > Duration::from_millis(500) {
            self.reload_state();
        }

        ctx.request_repaint_after(Duration::from_millis(500));

        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Check true connection status (must be fresh within 6s)
        let (is_online, updated_sec_ago) = if let Some(ref st) = self.state {
            let diff = now_secs.saturating_sub(st.updated_at);
            let online = st.master_connected && diff <= 6;
            (online, diff)
        } else {
            (false, 999)
        };

        egui::CentralPanel::default().show(ctx, |ui| {
            // HEADER BAR
            ui.horizontal(|ui| {
                ui.label(semibold_text("⚡ Twitch Drops Cluster", 20.0, egui::Color32::from_rgb(155, 95, 255)));
                ui.label(egui::RichText::new("• Worker Dashboard").size(14.5).color(egui::Color32::from_rgb(175, 180, 195)));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if is_online {
                        ui.label(semibold_text("● ONLINE", 13.5, egui::Color32::from_rgb(16, 185, 129)));
                    } else {
                        ui.label(semibold_text("● OFFLINE (НЕТ СВЯЗИ)", 13.5, egui::Color32::from_rgb(239, 68, 68)));
                    }
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            if let Some(ref st) = self.state {
                // STATS PILLS ROW
                ui.horizontal_wrapped(|ui| {
                    draw_metric_pill(ui, "Нода", &st.node_name, egui::Color32::from_rgb(230, 232, 240));
                    ui.add_space(6.0);

                    let acc_str = format!("{} / {}", st.total_accounts, st.max_containers);
                    draw_metric_pill(ui, "Аккаунтов", &acc_str, egui::Color32::from_rgb(155, 95, 255));
                    ui.add_space(6.0);

                    let camp_str = format!("{}", st.games.len());
                    draw_metric_pill(ui, "Кампаний", &camp_str, egui::Color32::from_rgb(59, 130, 246));
                    ui.add_space(6.0);

                    let ram_str = if st.ram_total_mb > 0.0 {
                        format!("{:.1} GB", st.ram_used_mb / 1024.0)
                    } else {
                        format!("{:.0} MB", st.ram_used_mb)
                    };
                    draw_metric_pill(ui, "RAM", &ram_str, egui::Color32::from_rgb(234, 179, 8));
                    ui.add_space(6.0);

                    let uptime_str = format_uptime(st.uptime_seconds);
                    draw_metric_pill(ui, "Аптайм", &uptime_str, egui::Color32::from_rgb(175, 182, 195));
                });

                ui.add_space(14.0);

                // GAME CAMPAIGNS CARDS
                ui.label(semibold_text(
                    "АКТИВНЫЕ КАМПАНИИ (СГРУППИРОВАНО ПО ИГРАМ)",
                    11.5,
                    egui::Color32::from_rgb(148, 155, 170),
                ));
                ui.add_space(6.0);

                egui::ScrollArea::vertical().show(ui, |ui| {
                    if st.games.is_empty() {
                        ui.group(|ui| {
                            ui.set_width(ui.available_width());
                            ui.vertical_centered(|ui| {
                                ui.add_space(20.0);
                                ui.label(semibold_text("💤 Нет активных задач фарма", 15.0, egui::Color32::GRAY));
                                ui.label(egui::RichText::new("Ожидание назначения кампаний от главного сервера...").size(12.5).color(egui::Color32::from_rgb(130, 136, 150)));
                                ui.add_space(20.0);
                            });
                        });
                    } else {
                        let mut ordered_games: Vec<(&String, &GameStateInfo)> = st.games.iter().collect();
                        ordered_games.sort_by(|(name_a, a), (name_b, b)| {
                            a.order.cmp(&b.order).then_with(|| name_a.cmp(name_b))
                        });
                        for (game_name, g) in ordered_games {
                            draw_game_card(ui, game_name, g);
                            ui.add_space(10.0);
                        }
                    }
                });
            } else {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(semibold_text("⏳ Поиск запущенного воркера...", 16.0, egui::Color32::YELLOW));
                    ui.label(egui::RichText::new("Ожидание файла состояния .worker_state.json").size(12.5).color(egui::Color32::GRAY));
                });
            }

            // FOOTER BAR
            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("TDC Native Dashboard").small().color(egui::Color32::from_rgb(120, 126, 140)));
                    ui.label(egui::RichText::new("•").small().color(egui::Color32::from_rgb(100, 105, 118)));
                    let sync_text = if updated_sec_ago < 2 {
                        "Синхронизация: живая (1с)".to_string()
                    } else {
                        format!("Синхронизация: {}с назад", updated_sec_ago)
                    };
                    ui.label(egui::RichText::new(sync_text).small().color(egui::Color32::from_rgb(156, 163, 175)));

                    if let Some(ref st) = self.state {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let url_disp = st.master_url.replace("ws://", "").replace("wss://", "");
                            ui.label(egui::RichText::new(format!("Сервер: {}", url_disp)).small().color(egui::Color32::from_rgb(130, 136, 150)));
                        });
                    }
                });
            });
        });
    }
}

fn draw_metric_pill(ui: &mut egui::Ui, label: &str, value: &str, val_color: egui::Color32) {
    let frame = egui::Frame::none()
        .fill(egui::Color32::from_rgb(24, 26, 33))
        .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(42, 46, 58)))
        .inner_margin(egui::Margin::symmetric(10.0, 6.0))
        .rounding(6.0);

    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{}:", label)).size(12.5).color(egui::Color32::from_rgb(148, 155, 170)));
            ui.label(semibold_text(value, 13.5, val_color));
        });
    });
}

fn draw_game_card(ui: &mut egui::Ui, game_name: &str, g: &GameStateInfo) {
    let frame = egui::Frame::group(ui.style())
        .fill(egui::Color32::from_rgb(24, 25, 31))
        .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(45, 49, 61)))
        .inner_margin(12.0)
        .rounding(8.0);

    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());

        // Game Header
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("🎮").size(17.0).color(egui::Color32::from_rgb(165, 135, 255)));
            ui.label(semibold_text(game_name, 16.5, egui::Color32::WHITE));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let badge_text = format!("{} аккаунтов", g.count);
                ui.label(semibold_text(badge_text, 13.5, egui::Color32::from_rgb(155, 95, 255)));
            });
        });

        ui.add_space(6.0);

        // Streamers and Drop details
        ui.horizontal(|ui| {
            let streamers_str = if g.streamers.is_empty() {
                "Поиск стримеров...".to_string()
            } else {
                g.streamers.join(", ")
            };
            ui.label(egui::RichText::new("📺 Стример:").size(13.5).color(egui::Color32::from_rgb(148, 155, 170)));
            ui.label(semibold_text(streamers_str, 13.5, egui::Color32::from_rgb(225, 228, 242)));

            ui.add_space(16.0);

            ui.label(egui::RichText::new("🎯 Предмет:").size(13.5).color(egui::Color32::from_rgb(148, 155, 170)));
            let drop_str = if g.drop_total > 1 {
                format!("Дроп {}/{}", g.drop_index, g.drop_total)
            } else {
                "Дроп".to_string()
            };
            ui.label(semibold_text(drop_str, 13.5, egui::Color32::from_rgb(234, 179, 8)));
        });

        ui.add_space(8.0);

        // Progress Bar
        let pct = if g.required_minutes > 0 {
            (g.min_watched as f32 / g.required_minutes as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let progress_label = format!("{}/{} мин ({:.1}%)", g.min_watched, g.required_minutes, pct * 100.0);
        let progress_bar = egui::ProgressBar::new(pct)
            .desired_height(20.0)
            .rounding(5.0)
            .text(semibold_text(progress_label, 12.5, egui::Color32::WHITE))
            .fill(egui::Color32::from_rgb(16, 185, 129))
            .animate(g.is_active);

        ui.add(progress_bar);

        ui.add_space(6.0);

        // Sub-details: Accounts breakdown & Time left
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("● Фармят:").size(13.5).color(egui::Color32::from_rgb(16, 185, 129)));
            ui.label(semibold_text(format!("{}", g.active_count), 13.5, egui::Color32::WHITE));

            ui.add_space(12.0);

            ui.label(egui::RichText::new("○ В очереди:").size(13.5).color(egui::Color32::from_rgb(234, 179, 8)));
            ui.label(semibold_text(format!("{}", g.waiting_count), 13.5, egui::Color32::WHITE));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let time_str = format!("⏱ Осталось: ~{}", g.time_left);
                ui.label(semibold_text(time_str, 13.5, egui::Color32::from_rgb(185, 190, 205)));
            });
        });
    });
}

fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86400;
    let rem = seconds % 86400;
    let hours = rem / 3600;
    let mins = (rem % 3600) / 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{}д", days));
    }
    if hours > 0 || days > 0 {
        parts.push(format!("{}ч", hours));
    }
    parts.push(format!("{}м", mins));
    parts.join(" ")
}

fn load_first_font(candidates: &[PathBuf], tweak: egui::FontTweak) -> Option<egui::FontData> {
    for path in candidates {
        if let Ok(bytes) = fs::read(path) {
            return Some(egui::FontData::from_owned(bytes).tweak(tweak));
        }
    }
    None
}

fn system_font_candidates(names: &[&str]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    #[cfg(windows)]
    {
        let windir = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let fonts_dir = windir.join("Fonts");
        for name in names {
            paths.push(fonts_dir.join(name));
        }
    }
    #[cfg(not(windows))]
    {
        let dirs = [
            "/usr/share/fonts/truetype/noto",
            "/usr/share/fonts/truetype/dejavu",
            "/usr/share/fonts/TTF",
            "/usr/share/fonts/noto",
        ];
        for dir in dirs {
            for name in names {
                paths.push(Path::new(dir).join(name));
            }
        }
    }
    paths
}

fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Segoe UI and Segoe UI Symbol share identical em metrics (upem 2048, asc 2210, desc -514).
    // Slight negative baseline offset centers caps vertically inside metric pills and progress bars.
    let ui_tweak = egui::FontTweak {
        baseline_offset_factor: -0.05,
        ..Default::default()
    };
    let symbol_tweak = egui::FontTweak {
        scale: 0.96,
        baseline_offset_factor: -0.05,
        ..Default::default()
    };

    let mut prop_chain = Vec::new();
    let mut semibold_chain = Vec::new();
    let mut mono_chain = Vec::new();

    if let Some(regular) = load_first_font(
        &system_font_candidates(&["segoeui.ttf", "tahoma.ttf", "NotoSans-Regular.ttf", "DejaVuSans.ttf"]),
        ui_tweak,
    ) {
        fonts.font_data.insert("SystemUI-Regular".to_owned(), regular);
        prop_chain.push("SystemUI-Regular".to_owned());
    }

    if let Some(semibold) = load_first_font(
        &system_font_candidates(&["seguisb.ttf", "segoeuib.ttf", "tahomabd.ttf", "NotoSans-SemiBold.ttf", "DejaVuSans-Bold.ttf"]),
        ui_tweak,
    ) {
        fonts.font_data.insert("SystemUI-Semibold".to_owned(), semibold);
        semibold_chain.push("SystemUI-Semibold".to_owned());
    } else if prop_chain.contains(&"SystemUI-Regular".to_owned()) {
        semibold_chain.push("SystemUI-Regular".to_owned());
    }

    if let Some(mono) = load_first_font(
        &system_font_candidates(&["consola.ttf", "JetBrainsMono-Regular.ttf", "DejaVuSansMono.ttf"]),
        Default::default(),
    ) {
        fonts.font_data.insert("SystemUI-Mono".to_owned(), mono);
        mono_chain.push("SystemUI-Mono".to_owned());
    }

    // Monochrome vector symbol/emoji font covers geometric bullets (●, ○) and UI emojis without square fallbacks.
    if let Some(symbols) = load_first_font(
        &system_font_candidates(&["seguisym.ttf", "NotoSansSymbols2-Regular.ttf", "DejaVuSans.ttf"]),
        symbol_tweak,
    ) {
        fonts.font_data.insert("SystemUI-Symbols".to_owned(), symbols);
        prop_chain.push("SystemUI-Symbols".to_owned());
        semibold_chain.push("SystemUI-Symbols".to_owned());
        mono_chain.push("SystemUI-Symbols".to_owned());
    }

    let existing_prop = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    let existing_mono = fonts.families.get(&egui::FontFamily::Monospace).cloned().unwrap_or_default();

    prop_chain.extend(existing_prop.iter().cloned());
    semibold_chain.extend(existing_prop);
    mono_chain.extend(existing_mono);

    // Last-resort outline fallback from Segoe UI Emoji for newer Unicode blocks.
    if let Some(emoji) = load_first_font(&system_font_candidates(&["seguiemj.ttf"]), symbol_tweak) {
        fonts.font_data.insert("SystemUI-Emoji".to_owned(), emoji);
        prop_chain.push("SystemUI-Emoji".to_owned());
        semibold_chain.push("SystemUI-Emoji".to_owned());
        mono_chain.push("SystemUI-Emoji".to_owned());
    }

    fonts.families.insert(egui::FontFamily::Proportional, prop_chain);
    fonts.families.insert(semibold_family(), semibold_chain);
    fonts.families.insert(egui::FontFamily::Monospace, mono_chain);

    ctx.set_fonts(fonts);
}

fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = egui::Color32::from_rgb(16, 18, 23);
    style.visuals.extreme_bg_color = egui::Color32::from_rgb(12, 14, 18);
    style.visuals.faint_bg_color = egui::Color32::from_rgb(24, 26, 33);
    style.visuals.override_text_color = Some(egui::Color32::from_rgb(230, 233, 242));

    style.text_styles = [
        (egui::TextStyle::Heading, egui::FontId::new(20.0, semibold_family())),
        (egui::TextStyle::Body, egui::FontId::new(14.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Monospace, egui::FontId::new(13.5, egui::FontFamily::Monospace)),
        (egui::TextStyle::Button, egui::FontId::new(14.0, egui::FontFamily::Proportional)),
        (egui::TextStyle::Small, egui::FontId::new(12.0, egui::FontFamily::Proportional)),
    ]
    .into();

    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    ctx.set_style(style);
}

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Twitch Drops Cluster • Worker Dashboard")
            .with_inner_size([720.0, 520.0])
            .with_min_inner_size([580.0, 380.0])
            .with_active(true),
        ..Default::default()
    };

    eframe::run_native(
        "Twitch Drops Cluster Dashboard",
        native_options,
        Box::new(|cc| {
            configure_fonts(&cc.egui_ctx);
            configure_style(&cc.egui_ctx);

            Box::new(TdcDashboardApp::default())
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_dashboard_symbols_and_cyrillic_have_non_empty_glyphs() {
        let ctx = egui::Context::default();
        configure_fonts(&ctx);
        configure_style(&ctx);
        let _ = ctx.run(Default::default(), |_| {});

        let sample = "⚡•●○💤⏳🎮📺🎯⏱АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя";

        ctx.fonts(|fonts| {
            for family in [egui::FontFamily::Proportional, semibold_family(), egui::FontFamily::Monospace] {
                let font_id = egui::FontId::new(16.0, family.clone());
                for ch in sample.chars() {
                    assert!(
                        fonts.has_glyph(&font_id, ch),
                        "Missing glyph for {:?} (U+{:04X}) in family {:?}",
                        ch,
                        ch as u32,
                        family
                    );
                    let galley = fonts.layout_no_wrap(ch.to_string(), font_id.clone(), egui::Color32::WHITE);
                    let glyph = &galley.rows[0].glyphs[0];
                    assert!(
                        !glyph.uv_rect.is_nothing() && glyph.uv_rect.size.x > 0.0 && glyph.uv_rect.size.y > 0.0,
                        "Empty outline for {:?} (U+{:04X}) in family {:?}",
                        ch,
                        ch as u32,
                        family
                    );
                }
            }
        });
    }
}
