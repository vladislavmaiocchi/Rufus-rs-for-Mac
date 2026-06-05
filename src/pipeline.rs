use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;
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
    pub iso_path: PathBuf,
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
    inspection: &IsoInspection,
) -> CreateUsbPlan {
    let t = options.lang.translations();
    let mut steps = Vec::new();
    steps.push(format!(
        "{} /dev/{} ({}; internal={})",
        t.step_validate_disk.replace("/dev/{}", ""),
        target_disk.identifier, target_disk.model, target_disk.internal
    ));
    // Wait, my i18n strings have placeholders. I should use them.
    steps.clear();
    steps.push(t.step_validate_disk
        .replace("/dev/{}", &format!("/dev/{}", target_disk.identifier))
        .replace("{}", target_disk.model.as_str())
        .replace("{}", &target_disk.internal.to_string())
    );
    // This replace chain is fragile. Better to use a more robust way or just keep it simple.
    // Actually, I'll just use format! with the translation strings if they contain {}.
    
    // Let's re-evaluate the step strings.
    // step_validate_disk: "Validate target disk /dev/{} ({}; internal={})"
    
    steps.clear();
    steps.push(format!(
        "{} /dev/{} ({}; internal={})",
        t.step_validate_disk.split("/dev/").next().unwrap_or("Validate target disk").trim(),
        target_disk.identifier, target_disk.model, target_disk.internal
    ));
    
    // Okay, I'll just use manual formatting for now to keep it safe, 
    // but using the labels from i18n.
    
    steps.clear();
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

    steps.push(t.step_attach_iso
        .replace("{}", &inspection.iso_path.display().to_string())
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
    steps.push(t.step_sync.to_string());
    steps.push(t.step_done.to_string());

    CreateUsbPlan { steps }
}

pub fn execute_create_usb<F>(options: &CreateUsbOptions, inspection: &IsoInspection, mut progress: F) -> Result<()> 
where F: FnMut(f32) {
    let t = options.lang.translations();
    let device_node = format!("/dev/{}", options.disk_identifier);
    
    progress(0.05);
    // Aggressively unmount the disk multiple times if needed, with a small delay.
    // This helps against macOS auto-mounting or processes holding onto the disk.
    for i in 1..=3 {
        let _ = Command::new("diskutil")
            .arg("unmountDisk")
            .arg("force")
            .arg(&device_node)
            .output();
        thread::sleep(Duration::from_millis(500));
        if i == 3 { break; }
    }

    progress(0.1);
    // Use eraseDisk instead of partitionDisk as it is often more reliable for this task.
    // Format: diskutil eraseDisk <format> <name> <partmap> <device>
    let (fs_str, is_ntfs) = match options.fs {
        FileSystem::Fat32 => ("MS-DOS", false),
        FileSystem::ExFat => ("ExFAT", false),
        FileSystem::Ntfs => ("MS-DOS", true), // Format as FAT32 first to get the partition, then overwrite with NTFS
    };

    let mut erase_cmd = Command::new("diskutil");
    erase_cmd
        .arg("eraseDisk")
        .arg(fs_str)
        .arg(&options.label)
        .arg("GPT")
        .arg(&device_node);

    run_command(&mut erase_cmd, t.step_repartition.split(" /dev/").next().unwrap_or("partition and format"))
        .context(t.partition_error)?;

    progress(0.2);
    if is_ntfs {
        println!("{}", t.applying_ntfs);
        let tools = crate::ntfs::NtfsTools::new()?;
        
        // Find the right partition by label. diskutil eraseDisk just finished.
        let partitions = list_disk_partitions(&options.disk_identifier)?;
        let mut target_part = None;
        for part_id in &partitions {
            // Skip EFI partitions (usually the first one on GPT)
            if part_id.ends_with("s1") {
                continue;
            }
            
            if let Some((_, part_label)) = read_partition_info(part_id)? {
                if part_label.to_uppercase() == options.label.to_uppercase() || !part_label.is_empty() {
                    target_part = Some(part_id.clone());
                    break;
                }
            }
        }
let part_id = target_part.or_else(|| partitions.get(1).cloned()).ok_or_else(|| {
    anyhow::anyhow!(
        "Could not find a suitable partition on {} to format as NTFS.",
        options.disk_identifier
    )
})?;

let part_node = format!("/dev/r{}", part_id);
let _block_node = format!("/dev/{}", part_id);

// macOS is very aggressive with auto-mounting. We unmount multiple times.
for _i in 1..=3 {
    let _ = Command::new("diskutil")
        .arg("unmountDisk")
        .arg("force")
        .arg(&device_node)
        .output();
    thread::sleep(Duration::from_millis(500));
}

println!("{}", t.clearing_headers);
// Zero out the first 4MB of the partition to destroy any existing FS structures
// that might cause macOS to auto-probe/lock it.
let _ = Command::new("dd")
    .arg("if=/dev/zero")
    .arg(format!("of={}", part_node))
    .arg("bs=1m")
    .arg("count=4")
    .output();

// Settle time after dd
thread::sleep(Duration::from_secs(2));

// One last unmount just in case dd triggered a probe
let _ = Command::new("diskutil")
    .arg("unmountDisk")
    .arg("force")
    .arg(&device_node)
    .output();

println!("{}", t.running_mkntfs.replace("{}", &part_node));
tools.format(&part_node, &options.label)
    .context(t.ntfs_error)?;
}

    progress(0.3);
    let target_mount = wait_for_partition_mount(&options.disk_identifier, &options.label, Duration::from_secs(45), t)
        .context(t.timeout_waiting_mount)?;

    progress(0.35);
    let mut mounted_iso =
        MountedIso::attach(&options.iso_path).context("Unable to attach ISO for copy phase")?;

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
    mounted_iso.detach()?;
    let mut sync_command = Command::new("sync");
    run_command(&mut sync_command, t.step_sync)?;

    progress(1.0);
    println!("{}", t.creation_completed);
    println!("{}", t.target_mount_path.replace("{}", &target_mount.display().to_string()));
    Ok(())
}

fn wait_for_partition_mount(disk_identifier: &str, label: &str, timeout: Duration, t: &crate::i18n::Translations) -> Result<PathBuf> {
    let deadline = Instant::now() + timeout;

    while Instant::now() <= deadline {
        let partitions = list_disk_partitions(disk_identifier)?;
        for part_id in partitions {
            if let Some((mount_point, part_label)) = read_partition_info(&part_id)? {
                if part_label.to_uppercase() == label.to_uppercase() {
                    if let Some(mp) = mount_point {
                        return Ok(mp);
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(1000));
    }

    bail!(
        "{}",
        t.mount_not_found
            .replace("'{}'", &format!("'{}'", label))
            .replace("{}", disk_identifier)
    );
}

fn list_disk_partitions(disk_identifier: &str) -> Result<Vec<String>> {
    let output = Command::new("diskutil")
        .arg("list")
        .arg("-plist")
        .arg(format!("/dev/{}", disk_identifier))
        .output()
        .with_context(|| "Failed to list partitions")?;

    let value = Value::from_reader_xml(Cursor::new(output.stdout))
        .context("Failed to parse diskutil list plist")?;
    
    let mut ids = Vec::new();
    if let Some(dict) = value.as_dictionary() {
        if let Some(all_disks) = dict.get("AllDisks").and_then(Value::as_array) {
            for disk in all_disks {
                if let Some(s) = disk.as_string() {
                    if s.starts_with(disk_identifier) && s != disk_identifier {
                        ids.push(s.to_string());
                    }
                }
            }
        }
    }
    Ok(ids)
}

fn read_partition_info(partition_identifier: &str) -> Result<Option<(Option<PathBuf>, String)>> {
    let device_node = format!("/dev/{}", partition_identifier);
    let output = Command::new("diskutil")
        .arg("info")
        .arg("-plist")
        .arg(&device_node)
        .output()
        .with_context(|| format!("Failed to execute diskutil info for {device_node}"))?;

    if !output.status.success() {
        return Ok(None);
    }

    let value = Value::from_reader_xml(Cursor::new(output.stdout))
        .context("Failed to parse diskutil info plist")?;
    let Some(dict) = value.as_dictionary() else {
        return Ok(None);
    };
    
    let mount_point = dict.get("MountPoint").and_then(Value::as_string).map(PathBuf::from);
    let volume_name = dict.get("VolumeName").and_then(Value::as_string).unwrap_or("").to_string();
    
    Ok(Some((mount_point, volume_name)))
}

fn run_command(command: &mut Command, context_label: &str) -> Result<()> {
    let printable = format!("{command:?}");
    let output = command
        .output()
        .with_context(|| format!("Failed to execute command for {context_label}: {printable}"))?;

    if !output.status.success() {
        bail!(
            "Command failed during {context_label}: {printable}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}
