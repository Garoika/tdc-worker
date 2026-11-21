#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use eframe::egui;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
struct GameStateInfo {
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
    games: HashMap<String, GameStateInfo>,
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

        // 1. Check true connection status (must be fresh within 6s)
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
                ui.heading(egui::RichText::new("⚡ Twitch Drops Cluster").strong().color(egui::Color32::from_rgb(145, 70, 255)));
                ui.label(egui::RichText::new("• Worker Dashboard").color(egui::Color32::from_rgb(180, 180, 190)));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if is_online {
                        ui.label(egui::RichText::new("● ONLINE").strong().color(egui::Color32::from_rgb(16, 185, 129)));
                    } else {
                        ui.label(egui::RichText::new("● OFFLINE (НЕТ СВЯЗИ)").strong().color(egui::Color32::from_rgb(239, 68, 68)));
                    }
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            if let Some(ref st) = self.state {
                // STATS PILLS ROW
                ui.horizontal_wrapped(|ui| {
                    // Node pill
                    draw_metric_pill(ui, "Нода", &st.node_name, egui::Color32::from_rgb(220, 220, 230));
                    ui.add_space(10.0);

                    // Accounts pill
                    let acc_str = format!("{} / {}", st.total_accounts, st.max_containers);
                    draw_metric_pill(ui, "Аккаунтов", &acc_str, egui::Color32::from_rgb(145, 70, 255));
                    ui.add_space(10.0);

                    // Active campaigns pill
                    let camp_str = format!("{}", st.games.len());
                    draw_metric_pill(ui, "Кампаний", &camp_str, egui::Color32::from_rgb(59, 130, 246));
                    ui.add_space(10.0);

                    // RAM pill
                    let ram_str = if st.ram_total_mb > 0.0 {
                        format!("{:.1} GB", st.ram_used_mb / 1024.0)
                    } else {
                        format!("{:.0} MB", st.ram_used_mb)
                    };
                    draw_metric_pill(ui, "RAM", &ram_str, egui::Color32::from_rgb(234, 179, 8));
                    ui.add_space(10.0);

                    // Uptime pill
                    let uptime_str = format_uptime(st.uptime_seconds);
                    draw_metric_pill(ui, "Аптайм", &uptime_str, egui::Color32::from_rgb(156, 163, 175));
                });

                ui.add_space(14.0);

                // GAME CAMPAIGNS CARDS
                ui.label(egui::RichText::new("АКТИВНЫЕ КАМПАНИИ (СГРУППИРОВАНО ПО ИГРАМ)").small().strong().color(egui::Color32::from_rgb(156, 163, 175)));
                ui.add_space(6.0);

                egui::ScrollArea::vertical().show(ui, |ui| {
                    if st.games.is_empty() {
                        ui.group(|ui| {
                            ui.set_width(ui.available_width());
                            ui.vertical_centered(|ui| {
                                ui.add_space(20.0);
                                ui.label(egui::RichText::new("💤 Нет активных задач фарма").size(15.0).color(egui::Color32::GRAY));
                                ui.label(egui::RichText::new("Ожидание назначения кампаний от главного сервера...").small().color(egui::Color32::DARK_GRAY));
                                ui.add_space(20.0);
                            });
                        });
                    } else {
                        for (game_name, g) in &st.games {
                            draw_game_card(ui, game_name, g);
                            ui.add_space(10.0);
                        }
                    }
                });

            } else {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(egui::RichText::new("⏳ Поиск запущенного воркера...").size(16.0).color(egui::Color32::YELLOW));
                    ui.label(egui::RichText::new("Ожидание файла состояния .worker_state.json").small().color(egui::Color32::GRAY));
                });
            }

            // FOOTER BAR
            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("TDC Native Dashboard").small().color(egui::Color32::DARK_GRAY));
                    ui.label(egui::RichText::new("•").small().color(egui::Color32::DARK_GRAY));
                    let sync_text = if updated_sec_ago < 2 {
                        "Синхронизация: живая (1с)".to_string()
                    } else {
                        format!("Синхронизация: {}с назад", updated_sec_ago)
                    };
                    ui.label(egui::RichText::new(sync_text).small().color(egui::Color32::GRAY));

                    if let Some(ref st) = self.state {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let url_disp = st.master_url.replace("ws://", "").replace("wss://", "");
                            ui.label(egui::RichText::new(format!("Сервер: {}", url_disp)).small().color(egui::Color32::DARK_GRAY));
                        });
                    }
                });
            });
        });
    }
}

fn draw_metric_pill(ui: &mut egui::Ui, label: &str, value: &str, val_color: egui::Color32) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{}:", label)).small().color(egui::Color32::from_rgb(140, 140, 150)));
            ui.label(egui::RichText::new(value).strong().color(val_color));
        });
    });
}

fn draw_game_card(ui: &mut egui::Ui, game_name: &str, g: &GameStateInfo) {
    let frame = egui::Frame::group(ui.style())
        .fill(egui::Color32::from_rgb(24, 25, 30))
        .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(45, 48, 58)))
        .inner_margin(12.0)
        .rounding(8.0);

    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());

        // Game Header
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("🎮").size(18.0));
            ui.label(egui::RichText::new(game_name).strong().size(16.0).color(egui::Color32::WHITE));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let badge_text = format!("{} аккаунтов", g.count);
                ui.label(egui::RichText::new(badge_text).strong().size(13.0).color(egui::Color32::from_rgb(145, 70, 255)));
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
            ui.label(egui::RichText::new("📺 Стример:").color(egui::Color32::GRAY));
            ui.label(egui::RichText::new(streamers_str).strong().color(egui::Color32::from_rgb(220, 220, 240)));

            ui.add_space(16.0);

            ui.label(egui::RichText::new("🎯 Предмет:").color(egui::Color32::GRAY));
            let drop_str = if g.drop_total > 1 {
                format!("Дроп {}/{}", g.drop_index, g.drop_total)
            } else {
                "Дроп".to_string()
            };
            ui.label(egui::RichText::new(drop_str).strong().color(egui::Color32::from_rgb(234, 179, 8)));
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
            .text(egui::RichText::new(progress_label).strong().color(egui::Color32::WHITE))
            .fill(egui::Color32::from_rgb(16, 185, 129))
            .animate(g.is_active);

        ui.add(progress_bar);

        ui.add_space(6.0);

        // Sub-details: Accounts breakdown & Time left
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("● Фармят:").color(egui::Color32::from_rgb(16, 185, 129)));
            ui.label(egui::RichText::new(format!("{}", g.active_count)).strong().color(egui::Color32::WHITE));

            ui.add_space(12.0);

            ui.label(egui::RichText::new("○ В очереди:").color(egui::Color32::from_rgb(234, 179, 8)));
            ui.label(egui::RichText::new(format!("{}", g.waiting_count)).strong().color(egui::Color32::WHITE));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let time_str = format!("⏱ Осталось: ~{}", g.time_left);
                ui.label(egui::RichText::new(time_str).strong().color(egui::Color32::from_rgb(180, 180, 195)));
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
            // Apply dark theme style
            let mut style = (*cc.egui_ctx.style()).clone();
            style.visuals = egui::Visuals::dark();
            style.visuals.panel_fill = egui::Color32::from_rgb(18, 19, 23);
            style.visuals.override_text_color = Some(egui::Color32::from_rgb(230, 230, 240));
            cc.egui_ctx.set_style(style);

            Box::new(TdcDashboardApp::default())
        }),
    )
}
