use std::collections::BTreeSet;
use std::io::Cursor;
use std::process::Command;

use anyhow::{bail, Context, Result};
use plist::{Dictionary, Value};

#[derive(Debug, Clone)]
pub struct DiskInfo {
    pub identifier: String,
    pub size_bytes: u64,
    pub model: String,
    pub internal: bool,
    pub removable: bool,
}

pub fn normalize_disk_identifier(raw: &str) -> String {
    raw.trim().trim_start_matches("/dev/").to_string()
}

pub fn list_disks() -> Result<Vec<DiskInfo>> {
    let identifiers = list_whole_disk_identifiers()?;
    let mut disks = Vec::with_capacity(identifiers.len());
    for identifier in identifiers {
        if let Ok(info) = disk_info(&identifier) {
            disks.push(info);
        }
    }
    disks.sort_by(|a, b| a.identifier.cmp(&b.identifier));
    Ok(disks)
}

pub fn find_disk(identifier: &str) -> Result<DiskInfo> {
    let normalized = normalize_disk_identifier(identifier);
    list_disks()?
        .into_iter()
        .find(|d| d.identifier == normalized)
        .ok_or_else(|| anyhow::anyhow!("Disk not found: {}", normalized))
}

pub fn format_bytes(size: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = size as f64;
    let mut unit_index = 0usize;
    while value >= 1024.0 && unit_index < units.len() - 1 {
        value /= 1024.0;
        unit_index += 1;
    }
    if unit_index == 0 {
        format!("{} {}", size, units[unit_index])
    } else {
        format!("{value:.1} {}", units[unit_index])
    }
}

fn list_whole_disk_identifiers() -> Result<Vec<String>> {
    let plist = run_diskutil_plist(&["list", "-plist"])?;
    let root = plist
        .as_dictionary()
        .ok_or_else(|| anyhow::anyhow!("Unexpected diskutil plist structure"))?;

    let mut disk_ids: BTreeSet<String> = BTreeSet::new();

    if let Some(entries) = root.get("AllDisksAndPartitions").and_then(Value::as_array) {
        for entry in entries {
            if let Some(dict) = entry.as_dictionary() {
                if let Some(id) = get_string(dict, "DeviceIdentifier") {
                    disk_ids.insert(id.to_string());
                }
            }
        }
    }

    if disk_ids.is_empty() {
        if let Some(entries) = root.get("WholeDisks").and_then(Value::as_array) {
            for entry in entries {
                if let Some(id) = entry.as_string() {
                    disk_ids.insert(id.to_string());
                }
            }
        }
    }

    if disk_ids.is_empty() {
        if let Some(entries) = root.get("AllDisks").and_then(Value::as_array) {
            for entry in entries {
                if let Some(id) = entry.as_string() {
                    if !id.contains('s') {
                        disk_ids.insert(id.to_string());
                    }
                }
            }
        }
    }

    if disk_ids.is_empty() {
        bail!("No whole disks found in diskutil output");
    }

    Ok(disk_ids.into_iter().collect())
}

fn disk_info(identifier: &str) -> Result<DiskInfo> {
    let dev = format!("/dev/{identifier}");
    let plist = run_diskutil_plist(&["info", "-plist", &dev])?;
    let info = plist
        .as_dictionary()
        .ok_or_else(|| anyhow::anyhow!("Unexpected diskutil info plist structure"))?;

    let size_bytes = get_u64(info, "TotalSize")
        .or_else(|| get_u64(info, "DiskSize"))
        .unwrap_or(0);
    let internal = get_bool(info, "Internal").unwrap_or(false);
    let removable = get_bool(info, "RemovableMedia")
        .or_else(|| get_bool(info, "Ejectable"))
        .unwrap_or(!internal);
    let model = get_string(info, "DeviceModel")
        .or_else(|| get_string(info, "MediaName"))
        .or_else(|| get_string(info, "IORegistryEntryName"))
        .unwrap_or("Unknown")
        .to_string();

    Ok(DiskInfo {
        identifier: identifier.to_string(),
        size_bytes,
        model,
        internal,
        removable,
    })
}

fn run_diskutil_plist(args: &[&str]) -> Result<Value> {
    let output = Command::new("diskutil")
        .args(args)
        .output()
        .with_context(|| format!("Failed to execute: diskutil {}", args.join(" ")))?;

    if !output.status.success() {
        bail!(
            "diskutil {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Value::from_reader_xml(Cursor::new(output.stdout))
        .context("Failed to parse diskutil XML plist output")
}

fn get_string<'a>(dict: &'a Dictionary, key: &str) -> Option<&'a str> {
    dict.get(key).and_then(Value::as_string)
}

fn get_bool(dict: &Dictionary, key: &str) -> Option<bool> {
    dict.get(key).and_then(Value::as_boolean)
}

fn get_u64(dict: &Dictionary, key: &str) -> Option<u64> {
    match dict.get(key) {
        Some(Value::Integer(number)) => number
            .as_unsigned()
            .or_else(|| number.as_signed().and_then(|v| u64::try_from(v).ok())),
        Some(Value::Real(number)) => Some(*number as u64),
        Some(Value::String(number)) => number.parse::<u64>().ok(),
        _ => None,
    }
}
