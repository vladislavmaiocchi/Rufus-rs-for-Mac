use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use eframe::egui::{self, Color32};
use rfd::FileDialog;
use rufus_rs::disk::{self, DiskInfo};
use rufus_rs::i18n::Language;
use rufus_rs::pipeline::{self, CreateUsbOptions, FileSystem};
use rufus_rs::{iso, validation};

enum JobMessage {
    Log(String),
    Progress(f32),
    Finished(Result<String, String>),
}

struct RufusGuiApp {
    iso_path: String,
    label: String,
    fs: FileSystem,
    max_split_size_mb: String,
    disks: Vec<DiskInfo>,
    selected_disk: Option<String>,
    acknowledge_erase: bool,
    busy: bool,
    progress: f32,
    receiver: Option<Receiver<JobMessage>>,
    output_log: String,
    lang: Language,
}

impl Default for RufusGuiApp {
    fn default() -> Self {
        let mut app = Self {
            iso_path: String::new(),
            label: "WININSTALL".to_string(),
            fs: FileSystem::Fat32,
            max_split_size_mb: "3800".to_string(),
            disks: Vec::new(),
            selected_disk: None,
            acknowledge_erase: false,
            busy: false,
            progress: 0.0,
            receiver: None,
            output_log: String::new(),
            lang: Language::English,
        };
        app.refresh_disks();
        app
    }
}

impl RufusGuiApp {
    fn append_log(&mut self, line: impl AsRef<str>) {
        if !self.output_log.is_empty() {
            self.output_log.push('\n');
        }
        self.output_log.push_str(line.as_ref());
    }

    fn refresh_disks(&mut self) {
        let t = self.lang.translations();
        match disk::list_disks() {
            Ok(disks) => {
                self.disks = disks;
                if let Some(selected) = self.selected_disk.as_ref() {
                    if !self.disks.iter().any(|disk| &disk.identifier == selected) {
                        self.selected_disk = None;
                    }
                }
                self.append_log(t.refresh_disks);
            }
            Err(error) => {
                self.append_log(format!("{}: {error:#}", t.operation_failed.replace(": {}", "")));
            }
        }
    }

    fn start_job(&mut self, execute: bool, format_only: bool) {
        if self.busy {
            return;
        }

        let t = self.lang.translations();

        let Some(disk_identifier) = self.selected_disk.clone() else {
            self.append_log(t.select_disk_error);
            return;
        };

        let mut iso_path = None;
        if !format_only {
            let iso_input = self.iso_path.trim();
            if iso_input.is_empty() {
                self.append_log(t.select_iso_error);
                return;
            }

            let path = PathBuf::from(iso_input);
            match path.canonicalize() {
                Ok(path) => iso_path = Some(path),
                Err(_) => {
                    self.append_log(format!("ISO not found: {}", path.display()));
                    return;
                }
            };
        }

        let max_split_size_mb = match self.max_split_size_mb.trim().parse::<u32>() {
            Ok(value) if value >= 512 => value,
            _ => {
                self.append_log(t.max_split_error);
                return;
            }
        };

        let label = match validation::normalize_volume_label(&self.label) {
            Ok(value) => value,
            Err(error) => {
                self.append_log(t.invalid_label_error.replace("{}", &format!("{error:#}")));
                return;
            }
        };

        if execute && !self.acknowledge_erase {
            self.append_log(t.acknowledge_error);
            return;
        }

        let options = CreateUsbOptions {
            iso_path,
            disk_identifier: disk::normalize_disk_identifier(&disk_identifier),
            label,
            fs: self.fs,
            max_split_size_mb,
            lang: self.lang,
        };

        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.busy = true;
        self.progress = 0.0;
        
        if format_only {
            self.append_log(t.start_format_only);
        } else {
            self.append_log(if execute {
                t.start_usb_creation
            } else {
                t.start_dry_run
            });
        }

        let lang_for_thread = self.lang;
        thread::spawn(move || {
            let t = lang_for_thread.translations();
            let result = run_create_usb_job(options, execute, format_only, &sender);
            let completion = result.map(|_| {
                if format_only {
                    t.success_format.to_string()
                } else if execute {
                    t.success_usb.to_string()
                } else {
                    t.success_dry_run.to_string()
                }
            });
            let _ = sender.send(JobMessage::Finished(completion.map_err(|e| format!("{e:#}"))));
        });
    }

    fn poll_worker_messages(&mut self) {
        let t = self.lang.translations();
        let mut messages = Vec::new();
        if let Some(receiver) = self.receiver.as_ref() {
            while let Ok(message) = receiver.try_recv() {
                messages.push(message);
            }
        }

        let mut finished = false;
        for message in messages {
            match message {
                JobMessage::Log(line) => self.append_log(line),
                JobMessage::Progress(p) => self.progress = p,
                JobMessage::Finished(result) => {
                    finished = true;
                    match result {
                        Ok(done) => {
                            self.append_log(done);
                            self.progress = 1.0;
                        }
                        Err(error) => {
                            self.append_log(t.operation_failed.replace("{}", &format!("{error:#}")));
                            let err_str = error.to_string();
                            if err_str.contains("Permission denied") || err_str.contains("Read-only file system") {
                                self.append_log(t.full_disk_access_header);
                                self.append_log(t.full_disk_access_msg);
                                self.append_log(t.full_disk_access_grant);
                                self.append_log(t.full_disk_access_path);
                                self.append_log(t.open_settings);
                            }
                        }
                    }
                }
            }
        }

        if finished {
            self.busy = false;
            self.receiver = None;
        }
    }

    fn selected_disk_info(&self) -> Option<&DiskInfo> {
        let selected = self.selected_disk.as_ref()?;
        self.disks.iter().find(|disk| &disk.identifier == selected)
    }
}

impl eframe::App for RufusGuiApp {

                egui::Grid::new("drive_grid")
                    .num_columns(2)
                    .spacing([8.0, 8.0])
                    .min_col_width(120.0)
                    .show(ui, |ui| {
                        // Device
                        ui.label(t.target_disk);
                        ui.horizontal(|ui| {
                            let selected_text = self
                                .selected_disk
                                .as_deref()
                                .map(|disk| format!("/dev/{disk}"))
                                .unwrap_or_else(|| t.target_disk.to_string());
                            
                            ui.add_enabled_ui(!self.busy, |ui| {
                                egui::ComboBox::from_id_salt("disk_combo")
                                    .selected_text(selected_text)
                                    .width(ui.available_width() - 30.0)
                                    .show_ui(ui, |ui| {
                                        for disk in &self.disks {
                                            let row = format!(
                                                "/dev/{} | {} | {}",
                                                disk.identifier,
                                                disk::format_bytes(disk.size_bytes),
                                                disk.model
                                            );
                                            ui.selectable_value(
                                                &mut self.selected_disk,
                                                Some(disk.identifier.clone()),
                                                row,
                                            );
                                        }
                                    });
                                if ui.button("🔄").on_hover_text(t.refresh_disks).clicked() {
                                    self.refresh_disks();
                                }
                            });
                        });
                        ui.end_row();

                        // Boot selection
                        ui.label(t.boot_selection);
                        ui.horizontal(|ui| {
                            ui.add_enabled_ui(false, |ui| {
                                egui::ComboBox::from_id_salt("boot_combo")
                                    .selected_text(t.boot_selection_iso)
                                    .width(ui.available_width() - 80.0)
                                    .show_ui(ui, |_| {});
                            });
                            ui.add_enabled_ui(!self.busy, |ui| {
                                if ui.button(t.select_button).clicked() {
                                    if let Some(path) = FileDialog::new()
                                        .add_filter("Windows ISO", &["iso"])
                                        .pick_file()
                                    {
                                        self.iso_path = path.display().to_string();
                                    }
                                }
                            });
                        });
                        ui.end_row();
                        
                        // ISO Path (shown if selected)
                        if !self.iso_path.is_empty() {
                            ui.label("");
                            ui.label(egui::RichText::new(&self.iso_path).small().weak());
                            ui.end_row();
                        }

                        // Partition scheme and Target system
                        ui.label(t.partition_scheme);
                        ui.horizontal(|ui| {
                            ui.add_enabled_ui(false, |ui| {
                                egui::ComboBox::from_id_salt("part_combo")
                                    .selected_text(t.partition_scheme_gpt)
                                    .width(ui.available_width() / 2.0 - 4.0)
                                    .show_ui(ui, |_| {});
                            });
                            ui.label(t.target_system);
                            ui.add_enabled_ui(false, |ui| {
                                egui::ComboBox::from_id_salt("target_combo")
                                    .selected_text(t.target_system_uefi)
                                    .width(ui.available_width())
                                    .show_ui(ui, |_| {});
                            });
                        });
                        ui.end_row();
                    });
            });

            if let Some(disk) = self.selected_disk_info() {
                if disk.internal {
                    ui.colored_label(Color32::RED, t.selected_internal_error);
                }
            }
            ui.add_space(4.0);

            // FORMAT OPTIONS
            ui.group(|ui| {
                ui.label(egui::RichText::new(t.format_options).strong());
                ui.add_space(2.0);

                egui::Grid::new("format_grid")
                    .num_columns(2)
                    .spacing([8.0, 8.0])
                    .min_col_width(120.0)
                    .show(ui, |ui| {
                        // Volume label
                        ui.label(t.volume_label);
                        ui.add_enabled_ui(!self.busy, |ui| {
                            ui.text_edit_singleline(&mut self.label);
                        });
                        ui.end_row();

                        // File system
                        ui.label(t.filesystem);
                        ui.add_enabled_ui(!self.busy, |ui| {
                            egui::ComboBox::from_id_salt("fs_combo")
                                .selected_text(match self.fs {
                                    FileSystem::Fat32 => t.fat32_label,
                                    FileSystem::ExFat => t.exfat_label,
                                    FileSystem::Ntfs => t.ntfs_label,
                                })
                                .width(ui.available_width())
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.fs, FileSystem::Fat32, t.fat32_label);
                                    ui.selectable_value(&mut self.fs, FileSystem::ExFat, t.exfat_label);
                                    ui.selectable_value(&mut self.fs, FileSystem::Ntfs, t.ntfs_label);
                                });
                        });
                        ui.end_row();

                        // Cluster size
                        ui.label(t.cluster_size);
                        ui.add_enabled_ui(false, |ui| {
                            egui::ComboBox::from_id_salt("cluster_combo")
                                .selected_text(t.cluster_size_default)
                                .width(ui.available_width())
                                .show_ui(ui, |_| {});
                        });
                        ui.end_row();
                    });

                ui.add_space(4.0);
                ui.collapsing(t.advanced_format_options, |ui| {
                    if self.fs == FileSystem::Fat32 {
                        ui.horizontal(|ui| {
                            ui.label(t.max_split_size);
                            ui.text_edit_singleline(&mut self.max_split_size_mb);
                        });
                        ui.label(egui::RichText::new(t.fat32_info).small().weak());
                    } else if self.fs == FileSystem::ExFat {
                        ui.label(egui::RichText::new(t.exfat_info).small().weak());
                    } else {
                        ui.label(egui::RichText::new(t.ntfs_info).small().weak());
                    }

                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(!self.busy, |ui| {
                            if ui.button(t.dry_run).clicked() {
                                self.start_job(false, false);
                            }
                            if ui.button(t.format_only).clicked() {
                                if self.acknowledge_erase {
                                    self.start_job(true, true);
                                } else {
                                    self.append_log(t.acknowledge_error);
                                }
                            }
                        });
                    });
                });
            });

            // Status Section
            ui.add_space(4.0);
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(t.status_label);
                    let status_text = if self.busy {
                        // Try to find the last line of log for status
                        self.output_log.lines().last().unwrap_or(t.ready_label)
                    } else {
                        t.ready_label
                    };
                    ui.label(egui::RichText::new(status_text).strong());
                });
                ui.add_space(2.0);
                ui.add(egui::ProgressBar::new(self.progress).show_percentage());
            });

            // Action Buttons
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.acknowledge_erase, t.acknowledge_erase);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_sized([100.0, 30.0], egui::Button::new(t.start_button)).clicked() {
                        self.start_job(true, false);
                    }
                    if ui.add_sized([100.0, 30.0], egui::Button::new(t.close_button)).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });

            // Log / Alerts
            if self.output_log.contains("FULL DISK ACCESS") || self.output_log.contains("ACCESSO COMPLETO AL DISCO") {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.visuals_mut().override_text_color = Some(Color32::from_rgb(255, 165, 0));
                    if ui.button(t.open_settings).clicked() {
                        let _ = Command::new("open")
                            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
                            .spawn();
                    }
                    ui.label(t.restart_required);
                });
            }

            ui.add_space(4.0);
            egui::CollapsingHeader::new(t.log_label).default_open(false).show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut self.output_log).desired_rows(10).interactive(true).desired_width(ui.available_width()));
                });
            });
        });
    }
}

fn run_create_usb_job(
    options: CreateUsbOptions,
    execute: bool,
    format_only: bool,
    sender: &Sender<JobMessage>,
) -> Result<()> {
    let t = options.lang.translations();
    let _ = sender.send(JobMessage::Log(t.inspecting_disk.replace("{}", &options.disk_identifier)));
    let target_disk = disk::find_disk(&options.disk_identifier)
        .with_context(|| format!("Target disk not found: {}", options.disk_identifier))?;
    if target_disk.internal {
        bail!("Refusing to use /dev/{} because it is an internal disk", target_disk.identifier);
    }

    let mut inspection = None;
    if !format_only {
        if let Some(iso_path) = &options.iso_path {
            let _ = sender.send(JobMessage::Log(t.inspecting_iso.replace("{}", &iso_path.display().to_string())));
            inspection = Some(iso::inspect_iso(iso_path, options.max_split_size_mb).context("Could not inspect ISO content")?);
        }
    }

    let plan = pipeline::build_plan(&options, &target_disk, inspection.as_ref());
    for (index, step) in plan.steps.iter().enumerate() {
        let _ = sender.send(JobMessage::Log(format!("{}. {}", index + 1, step)));
    }

    if !execute {
        return Ok(());
    }

    let _ = sender.send(JobMessage::Log(t.executing_pipeline.to_string()));
    pipeline::execute_create_usb(&options, inspection.as_ref(), |p| {
        let _ = sender.send(JobMessage::Progress(p));
    })
}

fn main() -> eframe::Result {
    let mut options = eframe::NativeOptions::default();
    options.viewport = options.viewport.with_inner_size([420.0, 700.0]);
    
    eframe::run_native(
        "rufus-rs GUI",
        options,
        Box::new(|_cc| Ok(Box::new(RufusGuiApp::default()))),
    )
}
