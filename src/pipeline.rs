use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use plist::Value;

use crate::copy;
use crate::disk::{self, DiskInfo};
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
    let mut steps = Vec::new();
    steps.push(format!(
        "Validate target disk /dev/{} ({}; internal={})",
        target_disk.identifier, target_disk.model, target_disk.internal
    ));
    steps.push(format!(
        "Unmount and repartition /dev/{} as GPT + {} label {}",
        options.disk_identifier, options.fs, options.label
    ));
    steps.push(format!(
        "Attach ISO read-only: {} (size {})",
        inspection.iso_path.display(),
        disk::format_bytes(inspection.iso_size_bytes)
    ));
    
    if let Some(install_wim_size) = inspection.install_wim_size {
        let actual_needs_split = options.fs == FileSystem::Fat32 && inspection.needs_wim_split;
        if actual_needs_split {
            steps.push(format!(
                "Copy ISO content excluding install.wim, then split install.wim ({}) into SWM chunks ({} MB each)",
                disk::format_bytes(install_wim_size),
                options.max_split_size_mb
            ));
        } else {
            steps.push(format!(
                "Copy ISO content including install.wim ({})",
                disk::format_bytes(install_wim_size)
            ));
        }
    } else {
        steps.push("Copy ISO content (install.wim not present)".to_string());
    }
    steps.push("Sync filesystem buffers to USB".to_string());
    steps.push("Done".to_string());

    CreateUsbPlan { steps }
}

pub fn execute_create_usb(options: &CreateUsbOptions, inspection: &IsoInspection) -> Result<()> {
    let device_node = format!("/dev/{}", options.disk_identifier);
    
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

    run_command(&mut erase_cmd, "partition and format target disk (eraseDisk)")
        .context("Failed to partition the USB disk. Ensure no other applications (like Finder or Terminal) are using it.")?;

    if is_ntfs {
        println!("Applying NTFS filesystem...");
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
let block_node = format!("/dev/{}", part_id);

// macOS is very aggressive with auto-mounting. We unmount multiple times.
for i in 1..=3 {
    let _ = Command::new("diskutil")
        .arg("unmountDisk")
        .arg("force")
        .arg(&device_node)
        .output();
    thread::sleep(Duration::from_millis(500));
}

println!("Clearing partition headers...");
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

println!("Running mkntfs on {}...", part_node);
tools.format(&part_node, &options.label)
    .context("Failed to format partition as NTFS. macOS is still locking the device. Try giving 'Full Disk Access' to the app, or use FAT32 (which supports large ISOs via splitting).")?;
}

    let target_mount = wait_for_partition_mount(&options.disk_identifier, &options.label, Duration::from_secs(45))
        .context("Timed out waiting for target partition to mount after formatting")?;

    let mut mounted_iso =
        MountedIso::attach(&options.iso_path).context("Unable to attach ISO for copy phase")?;

    let needs_split = options.fs == FileSystem::Fat32 && inspection.needs_wim_split;

    copy::copy_iso_to_usb(
        mounted_iso.mount_point(),
        &target_mount,
        needs_split,
        options.max_split_size_mb,
    )?;

    mounted_iso.detach()?;
    let mut sync_command = Command::new("sync");
    run_command(&mut sync_command, "sync filesystem buffers")?;

    println!("USB creation completed successfully.");
    println!("Target mount path: {}", target_mount.display());
    Ok(())
}

fn wait_for_partition_mount(disk_identifier: &str, label: &str, timeout: Duration) -> Result<PathBuf> {
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
        "Could not find a mounted partition with label '{}' on disk {} before timeout",
        label,
        disk_identifier
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
