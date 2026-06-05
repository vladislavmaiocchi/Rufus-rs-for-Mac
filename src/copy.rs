use std::ffi::CString;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use walkdir::WalkDir;
use wimlib::{OpenFlags, WimLib, WriteFlags};

pub fn copy_iso_to_usb<F>(
    source_mount: &Path,
    target_mount: &Path,
    split_install_wim: bool,
    max_split_size_mb: u32,
    mut progress: F,
) -> Result<()> 
where F: FnMut(f32) {
    let source_install_wim = source_mount.join("sources").join("install.wim");
    let target_sources_dir = target_mount.join("sources");

    // Calculate total size for progress reporting
    let mut total_size = 0u64;
    let mut entries = Vec::new();
    for entry in WalkDir::new(source_mount) {
        let entry = entry.context("Directory traversal failed during size calculation")?;
        if entry.file_type().is_file() {
            if split_install_wim && entry.path() == source_install_wim {
                // We'll handle WIM separately as it takes a lot of time
                continue;
            }
            total_size += entry.metadata()?.len();
        }
        entries.push(entry);
    }

    let mut copied_size = 0u64;
    let wim_weight = 0.5; // Give WIM splitting 50% of the progress if it happens
    let file_weight = if split_install_wim { 1.0 - wim_weight } else { 1.0 };

    for entry in entries {
        let source_path = entry.path();
        let relative = source_path
            .strip_prefix(source_mount)
            .context("Unable to compute relative path during copy")?;
        if relative.as_os_str().is_empty() {
            continue;
        }

        let destination_path = target_mount.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&destination_path).with_context(|| {
                format!("Unable to create destination directory {}", destination_path.display())
            })?;
            continue;
        }

        if entry.file_type().is_symlink() {
            continue;
        }

        if split_install_wim && source_path == source_install_wim {
            continue;
        }

        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Unable to create destination directory {}", parent.display())
            })?;
        }

        let file_size = entry.metadata()?.len();
        fs::copy(source_path, &destination_path).with_context(|| {
            format!(
                "Unable to copy {} to {}",
                source_path.display(),
                destination_path.display()
            )
        })?;
        
        copied_size += file_size;
        if total_size > 0 {
            progress((copied_size as f32 / total_size as f32) * file_weight);
        }
    }

    if split_install_wim {
        fs::create_dir_all(&target_sources_dir).with_context(|| {
            format!(
                "Unable to create destination directory {}",
                target_sources_dir.display()
            )
        })?;
        
        // Progress for WIM starts at file_weight
        progress(file_weight);
        split_wim(
            &source_install_wim,
            &target_sources_dir.join("install.swm"),
            max_split_size_mb,
        )?;
        progress(1.0);
    }

    Ok(())
}

fn split_wim(source_wim: &Path, destination_swm: &Path, max_split_size_mb: u32) -> Result<()> {
    if !source_wim.exists() {
        return Ok(()); // Or handle as error if expected
    }

    let wimlib = WimLib::default();
    
    // Convert paths to nul-terminated UTF-8 for wimlib (on macOS)
    let source_cstr = CString::new(source_wim.to_str().context("Source path is not valid UTF-8")?)?;
    let dest_cstr = CString::new(destination_swm.to_str().context("Destination path is not valid UTF-8")?)?;
    
    let source_tstr = wimlib::string::TStr::from_impl(&source_cstr);
    let dest_tstr = wimlib::string::TStr::from_impl(&dest_cstr);

    let wim = wimlib.open_wim(source_tstr, OpenFlags::empty())
        .context("Failed to open source WIM file")?;
    
    let part_size_bytes = u64::from(max_split_size_mb) * 1024 * 1024;
    
    wim.split(dest_tstr, part_size_bytes, WriteFlags::empty())
        .context("Failed to split WIM file")?;

    Ok(())
}
