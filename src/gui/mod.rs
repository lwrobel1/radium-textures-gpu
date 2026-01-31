/// GUI module for Radium Textures optimization
use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;
use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use crate::game::Game;

mod worker;
pub use worker::OptimizationWorker;

/// Application settings that persist between runs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub profile_path: String,
    pub mods_path: String,
    pub data_path: String,
    pub output_path: String,
    pub preset: String,
    // Game selection
    #[serde(default)]
    pub game: Game,
    // Custom preset resolutions
    #[serde(default = "default_diffuse")]
    pub custom_diffuse: u32,
    #[serde(default = "default_normal")]
    pub custom_normal: u32,
    #[serde(default = "default_parallax")]
    pub custom_parallax: u32,
    #[serde(default = "default_material")]
    pub custom_material: u32,
    // Thread configuration
    #[serde(default = "default_thread_count")]
    pub thread_count: usize,
    // Compression backend (nvtt3 = fast CUDA, texconv = original Wine)
    #[serde(default = "default_backend")]
    pub backend: String,
}

fn default_backend() -> String {
    "auto".to_string()  // Auto-detect best available
}

fn default_thread_count() -> usize {
    num_cpus::get()
}

fn default_diffuse() -> u32 { 2048 }
fn default_normal() -> u32 { 1024 }
fn default_parallax() -> u32 { 512 }
fn default_material() -> u32 { 512 }

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            profile_path: String::new(),
            mods_path: String::new(),
            data_path: String::new(),
            output_path: String::new(),
            preset: "Optimum".to_string(),
            game: Game::default(),
            custom_diffuse: default_diffuse(),
            custom_normal: default_normal(),
            custom_parallax: default_parallax(),
            custom_material: default_material(),
            thread_count: default_thread_count(),
            backend: default_backend(),
        }
    }
}

impl AppSettings {
    /// Load settings from file
    pub fn load() -> Self {
        let config_path = Self::config_path();
        if let Ok(content) = std::fs::read_to_string(&config_path) {
            if let Ok(mut settings) = serde_json::from_str::<Self>(&content) {
                // Normalize all paths to ensure they're absolute
                settings.profile_path = Self::normalize_path(&settings.profile_path);
                settings.mods_path = Self::normalize_path(&settings.mods_path);
                settings.data_path = Self::normalize_path(&settings.data_path);
                settings.output_path = Self::normalize_path(&settings.output_path);
                return settings;
            }
        }
        Self::default()
    }

    /// Normalize a path string to ensure it's absolute
    fn normalize_path(path_str: &str) -> String {
        if path_str.is_empty() {
            return String::new();
        }

        let path = PathBuf::from(path_str);

        // If already absolute, return as-is
        if path.is_absolute() {
            return path_str.to_string();
        }

        // If it's a relative path that looks like it should be absolute
        // (starts with "home/" or "mnt/" etc), add the leading /
        if path_str.starts_with("home/") ||
           path_str.starts_with("mnt/") ||
           path_str.starts_with("usr/") ||
           path_str.starts_with("opt/") ||
           path_str.starts_with("var/") {
            return format!("/{}", path_str);
        }

        // Otherwise, make it relative to current directory
        if let Ok(cwd) = std::env::current_dir() {
            cwd.join(path).display().to_string()
        } else {
            path_str.to_string()
        }
    }

    /// Save settings to file
    pub fn save(&self) -> anyhow::Result<()> {
        let config_path = Self::config_path();
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&config_path, content)?;
        Ok(())
    }

    /// Get the config file path
    fn config_path() -> PathBuf {
        let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push("radium-textures");
        path.push("settings.json");
        path
    }
}

/// Message sent from worker thread to GUI
#[derive(Debug, Clone)]
pub enum WorkerMessage {
    Progress { current: usize, total: usize, message: String },
    Log(String),
    Complete { success: bool, message: String },
}

/// Message sent from GUI to worker thread
#[derive(Debug, Clone)]
pub enum ControlMessage {
    Cancel,
}

/// Main GUI application
pub struct VramrApp {
    /// Application settings
    settings: AppSettings,

    /// Worker thread communication
    worker_tx: Option<Sender<ControlMessage>>,
    worker_rx: Option<Receiver<WorkerMessage>>,

    /// UI state
    is_running: bool,
    progress_current: usize,
    progress_total: usize,
    progress_message: String,
    log_messages: Vec<String>,

    /// Scroll to bottom of log
    scroll_to_bottom: bool,

    /// Text input buffers for custom resolutions
    custom_diffuse_text: String,
    custom_normal_text: String,
    custom_parallax_text: String,
    custom_material_text: String,
}

impl Default for VramrApp {
    fn default() -> Self {
        let settings = AppSettings::load();
        Self {
            custom_diffuse_text: settings.custom_diffuse.to_string(),
            custom_normal_text: settings.custom_normal.to_string(),
            custom_parallax_text: settings.custom_parallax.to_string(),
            custom_material_text: settings.custom_material.to_string(),
            settings,
            worker_tx: None,
            worker_rx: None,
            is_running: false,
            progress_current: 0,
            progress_total: 0,
            progress_message: String::from("Ready"),
            log_messages: Vec::new(),
            scroll_to_bottom: false,
        }
    }
}

impl VramrApp {
    /// Create a new GUI application
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Set up custom fonts if needed
        Self::default()
    }

    /// Start optimization in background thread
    fn start_optimization(&mut self) {
        if self.is_running {
            return;
        }

        // Create channels for communication
        let (worker_tx, worker_rx_control) = crossbeam_channel::unbounded();
        let (worker_tx_msg, worker_rx) = crossbeam_channel::unbounded();

        self.worker_tx = Some(worker_tx);
        self.worker_rx = Some(worker_rx);
        self.is_running = true;
        self.progress_current = 0;
        self.progress_total = 0;
        self.progress_message = "Starting...".to_string();
        self.log_messages.clear();

        // Clone settings for worker thread
        let settings = self.settings.clone();

        // Spawn worker thread
        std::thread::spawn(move || {
            OptimizationWorker::run(settings, worker_tx_msg, worker_rx_control);
        });
    }

    /// Cancel running optimization
    fn cancel_optimization(&mut self) {
        if let Some(tx) = &self.worker_tx {
            let _ = tx.send(ControlMessage::Cancel);
        }
    }

    /// Process messages from worker thread
    fn process_worker_messages(&mut self) {
        let mut should_complete = false;

        if let Some(rx) = &self.worker_rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    WorkerMessage::Progress { current, total, message } => {
                        self.progress_current = current;
                        self.progress_total = total;
                        self.progress_message = message;
                    }
                    WorkerMessage::Log(log) => {
                        self.log_messages.push(log);
                        self.scroll_to_bottom = true;
                    }
                    WorkerMessage::Complete { success: _, message } => {
                        self.is_running = false;
                        self.progress_message = message.clone();
                        self.log_messages.push(format!("=== {} ===", message));
                        self.scroll_to_bottom = true;
                        should_complete = true;
                    }
                }
            }
        }

        // Clean up after borrow ends
        if should_complete {
            self.worker_tx = None;
            self.worker_rx = None;
        }
    }
}

impl eframe::App for VramrApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process worker messages
        self.process_worker_messages();

        // Request repaint if worker is running
        if self.is_running {
            ctx.request_repaint();
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Radium Textures");
            ui.add_space(10.0);

            // Game info (hardcoded to Skyrim SE for now, FO4 support coming later)
            ui.horizontal(|ui| {
                ui.label("Game:");
                ui.label(self.settings.game.display_name());
            });
            ui.add_space(10.0);

            // Settings section
            egui::Grid::new("settings_grid")
                .num_columns(3)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    // Profile path
                    ui.label("Profile:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.profile_path)
                            .desired_width(400.0)
                    );
                    if ui.button("Browse...").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            let abs_path = if path.is_absolute() {
                                path
                            } else {
                                std::env::current_dir().unwrap_or_default().join(&path)
                            };
                            self.settings.profile_path = abs_path.display().to_string();
                            log::debug!("GUI: Profile path set to: {}", self.settings.profile_path);
                        }
                    }
                    ui.end_row();

                    // Mods path
                    ui.label("Mods:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.mods_path)
                            .desired_width(400.0)
                    );
                    if ui.button("Browse...").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            let abs_path = if path.is_absolute() {
                                path
                            } else {
                                std::env::current_dir().unwrap_or_default().join(&path)
                            };
                            self.settings.mods_path = abs_path.display().to_string();
                            log::debug!("GUI: Mods path set to: {}", self.settings.mods_path);
                        }
                    }
                    ui.end_row();

                    // Data path
                    ui.label("Data:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.data_path)
                            .desired_width(400.0)
                    );
                    if ui.button("Browse...").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            let abs_path = if path.is_absolute() {
                                path
                            } else {
                                std::env::current_dir().unwrap_or_default().join(&path)
                            };
                            self.settings.data_path = abs_path.display().to_string();
                            log::debug!("GUI: Data path set to: {}", self.settings.data_path);
                        }
                    }
                    ui.end_row();

                    // Output path
                    ui.label("Output:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.output_path)
                            .desired_width(400.0)
                    );
                    if ui.button("Browse...").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            // Ensure we get the absolute path
                            let abs_path = if path.is_absolute() {
                                path
                            } else {
                                std::env::current_dir().unwrap_or_default().join(&path)
                            };
                            self.settings.output_path = abs_path.display().to_string();
                            log::debug!("GUI: Output path set to: {}", self.settings.output_path);
                        }
                    }
                    ui.end_row();

                    // Preset selector
                    ui.label("Preset:");
                    egui::ComboBox::from_id_salt("preset_selector")
                        .selected_text(&self.settings.preset)
                        .width(400.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.settings.preset, "HighQuality".to_string(), "High Quality");
                            ui.selectable_value(&mut self.settings.preset, "Quality".to_string(), "Quality");
                            ui.selectable_value(&mut self.settings.preset, "Optimum".to_string(), "Optimum");
                            ui.selectable_value(&mut self.settings.preset, "Performance".to_string(), "Performance");
                            ui.selectable_value(&mut self.settings.preset, "Vanilla".to_string(), "Vanilla");
                            ui.selectable_value(&mut self.settings.preset, "Custom".to_string(), "Custom");
                        });
                    ui.label(""); // Empty cell for alignment
                    ui.end_row();

                    // Thread count selector
                    ui.label("Threads:");
                    let max_threads = num_cpus::get();
                    ui.add(egui::Slider::new(&mut self.settings.thread_count, 1..=max_threads)
                        .text("threads")
                        .show_value(true));
                    ui.label(format!("(max: {})", max_threads));
                    ui.end_row();

                    // Compression backend selector
                    ui.label("Backend:");
                    egui::ComboBox::from_id_salt("backend_selector")
                        .selected_text(match self.settings.backend.as_str() {
                            "nvtt3" => "NVTT3 (CUDA) - Fast",
                            "texconv" => "texconv (Wine)",
                            _ => "Auto (Best Available)",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.settings.backend, "auto".to_string(), "Auto (Best Available)");
                            ui.selectable_value(&mut self.settings.backend, "nvtt3".to_string(), "NVTT3 (CUDA) - Fast");
                            ui.selectable_value(&mut self.settings.backend, "texconv".to_string(), "texconv (Wine)");
                        });
                    ui.label("");  // Empty label for alignment
                    ui.end_row();
                });

            // Show preset resolution info
            ui.add_space(5.0);
            ui.group(|ui| {
                ui.label(egui::RichText::new("Preset Resolutions:").strong());

                let (diffuse, normal, parallax, material) = match self.settings.preset.as_str() {
                    "HighQuality" => (2048, 2048, 1024, 1024),
                    "Quality" => (2048, 1024, 1024, 1024),
                    "Optimum" => (2048, 1024, 512, 512),
                    "Performance" => (2048, 512, 512, 512),
                    "Vanilla" => (512, 512, 512, 512),
                    "Custom" => (self.settings.custom_diffuse, self.settings.custom_normal,
                                 self.settings.custom_parallax, self.settings.custom_material),
                    _ => (2048, 1024, 512, 512), // Default to Optimum
                };

                if self.settings.preset == "Custom" {
                    // Show editable custom resolution fields (text input only)
                    egui::Grid::new("custom_resolutions")
                        .num_columns(3)
                        .spacing([10.0, 5.0])
                        .show(ui, |ui| {
                            // Diffuse
                            ui.label("Diffuse:");
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut self.custom_diffuse_text)
                                    .desired_width(80.0)
                            );
                            if response.changed() {
                                if let Ok(val) = self.custom_diffuse_text.parse::<u32>() {
                                    self.settings.custom_diffuse = val.clamp(128, 8192);
                                }
                            }
                            if response.lost_focus() {
                                // Reset to valid value if invalid input
                                self.custom_diffuse_text = self.settings.custom_diffuse.to_string();
                            }
                            ui.label("px");
                            ui.end_row();

                            // Normal
                            ui.label("Normal:");
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut self.custom_normal_text)
                                    .desired_width(80.0)
                            );
                            if response.changed() {
                                if let Ok(val) = self.custom_normal_text.parse::<u32>() {
                                    self.settings.custom_normal = val.clamp(128, 8192);
                                }
                            }
                            if response.lost_focus() {
                                self.custom_normal_text = self.settings.custom_normal.to_string();
                            }
                            ui.label("px");
                            ui.end_row();

                            // Parallax
                            ui.label("Parallax:");
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut self.custom_parallax_text)
                                    .desired_width(80.0)
                            );
                            if response.changed() {
                                if let Ok(val) = self.custom_parallax_text.parse::<u32>() {
                                    self.settings.custom_parallax = val.clamp(128, 8192);
                                }
                            }
                            if response.lost_focus() {
                                self.custom_parallax_text = self.settings.custom_parallax.to_string();
                            }
                            ui.label("px");
                            ui.end_row();

                            // Material
                            ui.label("Material:");
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut self.custom_material_text)
                                    .desired_width(80.0)
                            );
                            if response.changed() {
                                if let Ok(val) = self.custom_material_text.parse::<u32>() {
                                    self.settings.custom_material = val.clamp(128, 8192);
                                }
                            }
                            if response.lost_focus() {
                                self.custom_material_text = self.settings.custom_material.to_string();
                            }
                            ui.label("px");
                            ui.end_row();
                        });
                } else {
                    // Show read-only preset info
                    ui.horizontal(|ui| {
                        ui.label(format!("Diffuse: {}px", diffuse));
                        ui.label("|");
                        ui.label(format!("Normal: {}px", normal));
                        ui.label("|");
                        ui.label(format!("Parallax: {}px", parallax));
                        ui.label("|");
                        ui.label(format!("Material: {}px", material));
                    });
                }
            });

            ui.add_space(10.0);

            // Control buttons
            ui.horizontal(|ui| {
                let start_enabled = !self.is_running
                    && !self.settings.profile_path.is_empty()
                    && !self.settings.mods_path.is_empty()
                    && !self.settings.data_path.is_empty()
                    && !self.settings.output_path.is_empty();

                if ui.add_enabled(start_enabled, egui::Button::new("Start Optimization")).clicked() {
                    // Save settings
                    let _ = self.settings.save();
                    self.start_optimization();
                }

                if ui.add_enabled(self.is_running, egui::Button::new("Cancel")).clicked() {
                    self.cancel_optimization();
                }

                if ui.button("Save Settings").clicked() {
                    match self.settings.save() {
                        Ok(_) => self.log_messages.push("Settings saved successfully".to_string()),
                        Err(e) => self.log_messages.push(format!("Failed to save settings: {}", e)),
                    }
                    self.scroll_to_bottom = true;
                }
            });

            ui.add_space(10.0);

            // Progress section
            if self.is_running || self.progress_total > 0 {
                ui.group(|ui| {
                    ui.label("Progress:");

                    let progress = if self.progress_total > 0 {
                        self.progress_current as f32 / self.progress_total as f32
                    } else {
                        0.0
                    };

                    let progress_bar = egui::ProgressBar::new(progress)
                        .show_percentage()
                        .text(format!("{} / {}", self.progress_current, self.progress_total));

                    ui.add(progress_bar);
                    ui.label(&self.progress_message);
                });
                ui.add_space(10.0);
            }

            // Log viewer
            ui.group(|ui| {
                ui.label("Log:");

                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .stick_to_bottom(self.scroll_to_bottom)
                    .show(ui, |ui| {
                        for log in &self.log_messages {
                            ui.label(log);
                        }
                    });

                if self.scroll_to_bottom {
                    self.scroll_to_bottom = false;
                }
            });
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Clean up and cancel any running operation
        if self.is_running {
            self.cancel_optimization();
        }
    }
}

/// Run the GUI application
pub fn run() -> Result<(), eframe::Error> {
    log::info!("Starting Radium Textures GUI...");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 700.0])
            .with_min_inner_size([600.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Radium Textures",
        options,
        Box::new(|cc| Ok(Box::new(VramrApp::new(cc)))),
    )
}
