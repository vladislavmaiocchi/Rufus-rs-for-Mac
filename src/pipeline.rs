use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs::{self, OpenOptions};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use plist::Value;

use crate::copy;
use crate::disk::{self, DiskInfo};
use crate::i18n::Language;
use crate::iso::{IsoInspection, MountedIso};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSystem {
    Fat32,
    ExFat,
    Ntfs,
}

impl std::fmt::Display for FileSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileSystem::Fat32 => write!(f, "FAT32"),
            FileSystem::ExFat => write!(f, "ExFAT"),
            FileSystem::Ntfs => write!(f, "NTFS"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreateUsbOptions {
    pub iso_path: Option<PathBuf>,
    pub disk_identifier: String,
    pub label: String,
    pub fs: FileSystem,
    pub max_split_size_mb: u32,
    pub lang: Language,
}

#[derive(Debug, Clone)]
pub struct CreateUsbPlan {
    pub steps: Vec<String>,
}

pub fn build_plan(
    options: &CreateUsbOptions,
    target_disk: &DiskInfo,
    inspection: Option<&IsoInspection>,
) -> CreateUsbPlan {
    let t = options.lang.translations();
    let mut steps = Vec::new();

    steps.push(t.step_validate_disk
        .replace("/dev/{}", &format!("/dev/{}", target_disk.identifier))
        .replace("{};", &format!("{};", target_disk.model))
        .replace("={})", &format!("={})", target_disk.internal))
    );

    steps.push(t.step_repartition
        .replace("/dev/{}", &format!("/dev/{}", options.disk_identifier))
        .replace("{}", &options.fs.to_string())
        .replace("{}", &options.label)
    );

    if let (Some(iso_path), Some(inspection)) = (&options.iso_path, inspection) {
        steps.push(t.step_attach_iso
            .replace("{}", &iso_path.display().to_string())
            .replace("{}", &disk::format_bytes(inspection.iso_size_bytes))
        );
        
        if let Some(install_wim_size) = inspection.install_wim_size {
            let actual_needs_split = options.fs == FileSystem::Fat32 && inspection.needs_wim_split;
            if actual_needs_split {
                steps.push(t.step_copy_split
                    .replace("{}", &disk::format_bytes(install_wim_size))
                    .replace("{}", &options.max_split_size_mb.to_string())
                );
            } else {
                steps.push(t.step_copy_full
                    .replace("{}", &disk::format_bytes(install_wim_size))
                );
            }
        } else {
            steps.push(t.step_copy_no_wim.to_string());
        }
    }
    
    steps.push(t.step_sync.to_string());
    steps.push(t.step_done.to_string());

    CreateUsbPlan { steps }
}

pub fn execute_create_usb<F>(options: &CreateUsbOptions, inspection: Option<&IsoInspection>, mut progress: F) -> Result<()> 
where F: FnMut(f32) {
    let t = options.lang.translations();
    let device_node = format!("/dev/{}", options.disk_identifier);
    let log_path = PathBuf::from("rufus-rs-log.txt");

    // Initialize log file
    let _ = fs::write(&log_path, format!("--- Rufus-rs Log Started at {:?} ---
", Instant::now()));
    let log_fn = |msg: &str| {
        let _ = OpenOptions::new().append(true).open(&log_path)
            .map(|mut f| writeln!(f, "[{}] {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg));
    };

    log_fn(&format!("Starting USB creation for disk: {}", device_node));
    log_fn(&format!("Options: {:?}", options));

    progress(0.05);
    for i in 1..=3 {
        log_fn(&format!("Attempting to unmount disk {} (attempt {})", device_node, i));
        let output = Command::new("diskutil")
            .arg("unmountDisk")
            .arg("force")
            .arg(&device_node)
            .output();
        
        match output {
            Ok(out) if out.status.success() => log_fn(&format!("Successfully unmounted {} (attempt {})", device_node, i)),
            Ok(out) => log_fn(&format!("Unmount attempt {} failed: {}", i, String::from_utf8_lossy(&out.stderr))),
            Err(e) => log_fn(&format!("Unmount attempt {} error: {}", i, e)),
        }
        thread::sleep(Duration::from_millis(500));
        if i == 3 { break; }
    }

    progress(0.1);
    let (fs_str, is_ntfs) = match options.fs {
        FileSystem::Fat32 => ("MS-DOS", false),
        FileSystem::ExFat => ("ExFAT", false),
        FileSystem::Ntfs => ("MS-DOS", true),
    };

    log_fn(&format!("Partitioning disk {} with GPT and {} (label: {})", device_node, fs_str, options.label));
    let mut partition_cmd = Command::new("diskutil");
    partition_cmd
        .arg("partitionDisk")
        .arg(&device_node)
        .arg("GPT")
        .arg("FAT32")
        .arg("EFI")
        .arg("200M")
        .arg(fs_str)
        .arg(&options.label)
        .arg("Remainder");

    run_command(&mut partition_cmd, t.step_repartition.split(" /dev/").next().unwrap_or("partitioning"), &log_path)
        .context(t.partition_error)?;

    // Control Fix: Validate that we have at least two partitions (EFI and Data)
    log_fn("Validating partition structure...");
    let partitions = list_disk_partitions(&options.disk_identifier)?;
    if partitions.len() < 2 {
        let err_msg = format!("Partitioning failed: Expected at least 2 partitions (EFI and Data), but found {}. Please check if the disk is correctly connected.", partitions.len());
        log_fn(&format!("[ERROR] {}", err_msg));
        bail!("{}", err_msg);
    }
    log_fn(&format!("Partition validation successful. Found {} partitions.", partitions.len()));

    progress(0.2);
    if is_ntfs {
        log_fn("Formatting partition as NTFS...");
        let tools = crate::ntfs::NtfsTools::new()?;
        let mut target_part = None;
        for part_id in &partitions {
            if part_id.ends_with("s1") { continue; }
            if let Some((_, part_label)) = read_partition_info(part_id)? {
                if part_label.to_uppercase() == options.label.to_uppercase() || !part_label.is_empty() {
                    target_part = Some(part_id.clone());
                    break;
                }
            }
        }
        let part_id = target_part.or_else(|| partitions.get(1).cloned()).ok_or_else(|| {
            anyhow::anyhow!("Could not find a suitable partition to format as NTFS.")
        })?;

        let part_node = format!("/dev/r{}", part_id);
        for _i in 1..=3 {
            log_fn(&format!("Attempting forced unmount of {} for NTFS format (attempt {})", part_node, _i));
            let _ = Command::new("diskutil").arg("unmountDisk").arg("force").arg(&device_node).output();
            thread::sleep(Duration::from_millis(500));
        }

        log_fn(&format!("Zeroing out first 4MB of {} to clear partition headers...", part_node));
        let zero_out = Command::new("dd")
            .arg("if=/dev/zero")
            .arg(format!("of={}", part_node))
            .arg("bs=1m")
            .arg("count=4")
            .output();
        
        match zero_out {
            Ok(out) if out.status.success() => log_fn("Zeroing out successful."),
            Ok(out) => log_fn(&format!("Zeroing out failed: {}", String::from_utf8_lossy(&out.stderr))),
            Err(e) => log_fn(&format!("Zeroing out error: {}", e)),
        }
        
        thread::sleep(Duration::from_secs(2));
        let _ = Command::new("diskutil").arg("unmountDisk").arg("force").arg(&device_node).output();

        log_fn(&format!("Starting NTFS format on {} with label {}", part_node, options.label));
        tools.format(&part_node, &options.label).context(t.ntfs_error)?;
        log_fn("NTFS format completed successfully.");
    }

    if let (Some(iso_path), Some(inspection)) = (&options.iso_path, inspection) {
        log_fn(&format!("Starting file copy phase from {} to {}", iso_path.display(), options.disk_identifier));
        progress(0.3);
        let target_mount = wait_for_partition_mount(&options.disk_identifier, &options.label, Duration::from_secs(45), &t)
            .context(t.timeout_waiting_mount)?;

        log_fn(&format!("Target partition mounted at: {}", target_mount.display()));

        progress(0.35);
        let mut mounted_iso = MountedIso::attach(iso_path).context("Unable to attach ISO for copy phase")?;
        let needs_split = options.fs == FileSystem::Fat32 && inspection.needs_wim_split;

        let p_start = 0.4;
        let p_end = 0.95;
        copy::copy_iso_to_usb(
            mounted_iso.mount_point(),
            &target_mount,
            needs_split,
            options.max_split_size_mb,
            |p| progress(p_start + p * (p_end - p_start)),
        )?;

        progress(0.96);
        log_fn("ISO detachment complete.");
        mounted_iso.detach()?;
    } else {
        progress(0.9);
    }

    log_fn("Running final sync...");
    let mut sync_command = Command::new("sync");
    run_command(&mut sync_command, t.step_sync, &log_path)?;

    log_fn("USB creation process finished successfully.");
    progress(1.0);
    Ok(())
}

fn wait_for_partition_mount(disk_identifier: &str, label: &str, timeout: Duration, t: &crate::i18n::Translations) -> Result<PathBuf> {
    let deadline = Instant::now() + timeout;
    while Instant::now() <= deadline {
        let partitions = list_disk_partitions(disk_identifier)?;
        for part_id in partitions {
            if let Some((mount_point, part_label)) = read_partition_info(&part_id)? {
                if part_label.to_uppercase() == label.to_uppercase() {
                    if let Some(mp) = mount_point { return Ok(mp); }
                }
            }
        }
        thread::sleep(Duration::from_millis(1000));
    }
    bail!("{}", t.mount_not_found.replace("'{}'", &format!("'{}'", label)).replace("{}", disk_identifier));
}

fn list_disk_partitions(disk_identifier: &str) -> Result<Vec<String>> {
    let output = Command::new("diskutil").arg("list").arg("-plist").arg(format!("/dev/{}", disk_identifier)).output()?;
    let value = Value::from_reader_xml(Cursor::new(output.stdout))?;
    let mut ids = Vec::new();
    if let Some(dict) = value.as_dictionary() {
        if let Some(all_disks) = dict.get("AllDisks").and_then(Value::as_array) {
            for disk in all_disks {
                if let Some(s) = disk.as_string() {
                    if s.starts_with(disk_identifier) && s != disk_identifier { ids.push(s.to_string()); }
                }
            }
        }
    }
    Ok(ids)
}

fn read_partition_info(partition_identifier: &str) -> Result<Option<(Option<PathBuf>, String)>> {
    let device_node = format!("/dev/{}", partition_identifier);
    let output = Command::new("diskutil").arg("info").arg("-plist").arg(&device_node).output()?;
    if !output.status.success() { return Ok(None); }
    let value = Value::from_reader_xml(Cursor::new(output.stdout))?;
    let Some(dict) = value.as_dictionary() else { return Ok(None); };
    let mount_point = dict.get("MountPoint").and_then(Value::as_string).map(PathBuf::from);
    let volume_name = dict.get("VolumeName").and_then(Value::as_string).unwrap_or("").to_string();
    Ok(Some((mount_point, volume_name)))
}

fn run_command(command: &mut Command, context_label: &str, log_path: &PathBuf) -> Result<()> {
    let cmd_str = format!("{:?}", command);
    
    let _ = OpenOptions::new().append(true).open(log_path)
        .map(|mut f| writeln!(f, "[INFO] Executing: {} (Context: {})", cmd_str, context_label));

    let output = command.output().with_context(|| format!("Failed to execute command for {context_label}"))?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let _ = OpenOptions::new().append(true).open(log_path)
            .map(|mut f| writeln!(f, "[ERROR] Command failed: {}
STDOUT: {}
STDERR: {}", cmd_str, stdout, stderr));
        
        bail!(
            "Command failed during {context_label}: {}
{}",
            format!("{command:?}"),
            stderr
        );
    } else {
        let _ = OpenOptions::new().append(true).open(log_path)
            .map(|mut f| writeln!(f, "[SUCCESS] Command completed: {}", cmd_str));
    }
    Ok(())
}
