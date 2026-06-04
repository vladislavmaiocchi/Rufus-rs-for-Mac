use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use eframe::egui::{self, Color32};
use rfd::FileDialog;
use rufus_rs::disk::{self, DiskInfo};
use rufus_rs::pipeline::{self, CreateUsbOptions, FileSystem};
use rufus_rs::{iso, validation};

enum JobMessage {
    Log(String),
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
    receiver: Option<Receiver<JobMessage>>,
    output_log: String,
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
            receiver: None,
            output_log: String::new(),
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
        match disk::list_disks() {
            Ok(disks) => {
                self.disks = disks;
                if let Some(selected) = self.selected_disk.as_ref() {
                    if !self.disks.iter().any(|disk| &disk.identifier == selected) {
                        self.selected_disk = None;
                    }
                }
                self.append_log("Disk list refreshed.");
            }
            Err(error) => {
                self.append_log(format!("Unable to refresh disk list: {error:#}"));
            }
        }
    }

    fn start_job(&mut self, execute: bool) {
        if self.busy {
            return;
        }

        let Some(disk_identifier) = self.selected_disk.clone() else {
            self.append_log("Select a target disk first.");
            return;
        };

        let iso_input = self.iso_path.trim();
        if iso_input.is_empty() {
            self.append_log("Select a Windows ISO first.");
            return;
        }

        let iso_path = PathBuf::from(iso_input);
        let iso_path = match iso_path.canonicalize() {
            Ok(path) => path,
            Err(_) => {
                self.append_log(format!("ISO not found: {}", iso_path.display()));
                return;
            }
        };

        let max_split_size_mb = match self.max_split_size_mb.trim().parse::<u32>() {
            Ok(value) if value >= 512 => value,
            _ => {
                self.append_log("max_split_size_mb must be a number >= 512.");
                return;
            }
        };

        let label = match validation::normalize_volume_label(&self.label) {
            Ok(value) => value,
            Err(error) => {
                self.append_log(format!("Invalid volume label: {error:#}"));
                return;
            }
        };

        if execute && !self.acknowledge_erase {
            self.append_log("Check the erase confirmation before executing.");
            return;
        }

        let options = CreateUsbOptions {
            iso_path,
            disk_identifier: disk::normalize_disk_identifier(&disk_identifier),
            label,
            fs: self.fs,
            max_split_size_mb,
        };

        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.busy = true;
        self.append_log(if execute {
            "Starting USB creation..."
        } else {
            "Starting dry-run..."
        });

        thread::spawn(move || {
            let result = run_create_usb_job(options, execute, &sender);
            let completion = result.map(|_| {
                if execute {
                    "USB creation finished successfully.".to_string()
                } else {
                    "Dry-run completed successfully.".to_string()
                }
            });
            let _ = sender.send(JobMessage::Finished(completion.map_err(|e| format!("{e:#}"))));
        });
    }

    fn poll_worker_messages(&mut self) {
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
                JobMessage::Finished(result) => {
                    finished = true;
                    match result {
                        Ok(done) => self.append_log(done),
                        Err(error) => {
                            self.append_log(format!("Operation failed: {error:#}"));
                            let err_str = error.to_string();
                            if err_str.contains("Permission denied") || err_str.contains("Read-only file system") {
                                self.append_log("\n--- [!] ACTION REQUIRED: FULL DISK ACCESS ---");
                                self.append_log("macOS is blocking direct disk access for NTFS formatting.");
                                self.append_log("Please grant 'Full Disk Access' to this app in:");
                                self.append_log("System Settings > Privacy & Security > Full Disk Access");
                                self.append_log("Click 'Open System Settings' below to go there now.");
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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker_messages();
        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("rufus-rs GUI");
            ui.label("Create Windows bootable USB drives on macOS.");
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("ISO:");
                ui.text_edit_singleline(&mut self.iso_path);
                if ui.button("Browse").clicked() {
                    if let Some(path) = FileDialog::new()
                        .add_filter("Windows ISO", &["iso"])
                        .pick_file()
                    {
                        self.iso_path = path.display().to_string();
                    }
                }
            });

            ui.horizontal(|ui| {
                let selected_text = self
                    .selected_disk
                    .as_deref()
                    .map(|disk| format!("/dev/{disk}"))
                    .unwrap_or_else(|| "Select disk".to_string());
                egui::ComboBox::from_label("Target disk")
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        for disk in &self.disks {
                            let row = format!(
                                "/dev/{} | internal={} | removable={} | {} | {}",
                                disk.identifier,
                                disk.internal,
                                disk.removable,
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
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Refresh disks"))
                    .clicked()
                {
                    self.refresh_disks();
                }
            });

            if let Some(disk) = self.selected_disk_info() {
                if disk.internal {
                    ui.colored_label(
                        Color32::RED,
                        "Selected disk is internal: execution will be blocked.",
                    );
                }
            }

            ui.horizontal(|ui| {
                ui.label("Volume label:");
                ui.text_edit_singleline(&mut self.label);

                ui.label("Filesystem:");
                egui::ComboBox::from_id_source("fs_combo")
                    .selected_text(self.fs.to_string())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.fs, FileSystem::Fat32, "FAT32 (Standard)");
                        ui.selectable_value(&mut self.fs, FileSystem::ExFat, "ExFAT (Linux/Compatibility)");
                        ui.selectable_value(&mut self.fs, FileSystem::Ntfs, "NTFS (Windows Only)");
                    });

                if self.fs == FileSystem::Fat32 {
                    ui.label("Max split size MB:");
                    ui.text_edit_singleline(&mut self.max_split_size_mb);
                }
            });

            ui.add_space(4.0);
            match self.fs {
                FileSystem::Fat32 => {
                    ui.label("FAT32: High compatibility (UEFI), but requires WIM splitting for large files.");
                }
                FileSystem::ExFat => {
                    ui.label("ExFAT: Recommended for Linux compatibility and large files (>4GB).");
                }
                FileSystem::Ntfs => {
                    ui.label("NTFS: Windows only. Now supported natively with built-in drivers.");
                }
            }

            ui.separator();

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Dry-run"))
                    .clicked()
                {
                    self.start_job(false);
                }
                ui.checkbox(
                    &mut self.acknowledge_erase,
                    "I understand the target disk will be erased",
                );
                if ui
                    .add_enabled(
                        !self.busy && self.acknowledge_erase,
                        egui::Button::new("Create USB"),
                    )
                    .clicked()
                {
                    self.start_job(true);
                }
            });

            if self.output_log.contains("FULL DISK ACCESS") {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.visuals_mut().override_text_color = Some(Color32::from_rgb(255, 165, 0));
                    if ui.button("⚙ Open System Settings (Privacy)").clicked() {
                        let _ = Command::new("open")
                            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
                            .spawn();
                    }
                    ui.label("After granting access, you MUST restart the app.");
                });
                ui.add_space(8.0);
            }

            ui.separator();
            ui.label("Log");
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.output_log)
                        .desired_rows(20)
                        .interactive(true) // Allow selection/copying
                );
            });
        });
    }
}

fn run_create_usb_job(
    options: CreateUsbOptions,
    execute: bool,
    sender: &Sender<JobMessage>,
) -> Result<()> {
    let _ = sender.send(JobMessage::Log(format!(
        "Inspecting target disk /dev/{}",
        options.disk_identifier
    )));
    let target_disk = disk::find_disk(&options.disk_identifier)
        .with_context(|| format!("Target disk not found: {}", options.disk_identifier))?;
    if target_disk.internal {
        bail!(
            "Refusing to use /dev/{} because it is an internal disk",
            target_disk.identifier
        );
    }

    let _ = sender.send(JobMessage::Log(format!(
        "Inspecting ISO: {}",
        options.iso_path.display()
    )));
    let inspection = iso::inspect_iso(&options.iso_path, options.max_split_size_mb)
        .context("Could not inspect ISO content")?;
    let plan = pipeline::build_plan(&options, &target_disk, &inspection);
    for (index, step) in plan.steps.iter().enumerate() {
        let _ = sender.send(JobMessage::Log(format!("{}. {}", index + 1, step)));
    }

    if !execute {
        return Ok(());
    }

    let _ = sender.send(JobMessage::Log(
        "Executing disk partition + file copy pipeline...".to_string(),
    ));
    pipeline::execute_create_usb(&options, &inspection)
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "rufus-rs GUI",
        options,
        Box::new(|_cc| Ok(Box::new(RufusGuiApp::default()))),
    )
}
