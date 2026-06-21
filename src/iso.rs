use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use plist::Value;
use tempfile::TempDir;

#[derive(Debug, Clone)]
pub struct IsoInspection {
    pub iso_path: PathBuf,
    pub iso_size_bytes: u64,
    pub install_wim_size: Option<u64>,
    pub needs_wim_split: bool,
}

pub struct MountedIso {
    mount_dir: TempDir,
    device_node: String,
    detached: bool,
}

impl MountedIso {
    pub fn attach(iso_path: &Path) -> Result<Self> {
        if !iso_path.exists() {
            bail!("ISO file not found: {}", iso_path.display());
        }

        let mut attempts = 0;
        let max_attempts = 5;

        loop {
            attempts += 1;
            
            // 1. Clean up existing mounts
            Self::force_detach_iso(iso_path);
            thread::sleep(Duration::from_millis(1000 * attempts));

            let mount_dir = tempfile::Builder::new()
                .prefix("rufus-rs-iso-")
                .tempdir()
                .context("Unable to create temporary mount directory")?;

            let output = Command::new("hdiutil")
                .arg("attach")
                .arg("-readonly")
                .arg("-nobrowse")
                .arg("-mountpoint")
                .arg(mount_dir.path())
                .arg(iso_path)
                .output()
                .with_context(|| format!("Failed to execute hdiutil attach for {}", iso_path.display()))?;

            if output.status.success() {
                // Parse device node from output
                let stdout = String::from_utf8_lossy(&output.stdout);
                let device_node = stdout
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().next())
                    .map(|s| s.to_string())
                    .ok_or_else(|| anyhow::anyhow!("Could not determine device node from hdiutil attach output"))?;

                let base_node = if device_node.contains('s') {
                    let parts: Vec<&str> = device_node.split('s').collect();
                    parts[0].to_string()
                } else {
                    device_node
                };

                return Ok(Self {
                    mount_dir,
                    device_node: base_node,
                    detached: false,
                });
            }

            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("Risorsa occupata") || stderr.contains("Resource busy") {
                if attempts >= max_attempts {
                    bail!("hdiutil attach failed after {} attempts: {}", max_attempts, stderr);
                }
                // Continue loop to retry
                continue;
            } else {
                bail!("hdiutil attach failed: {}", stderr);
            }
        }
    }

    fn force_detach_iso(iso_path: &Path) {
        // 1. Try path-based detach for the target ISO
        let _ = Command::new("hdiutil").arg("detach").arg("-force").arg(iso_path).output();

        // 2. Extreme measure: Detach ALL currently mounted disk images
        // This clears any ghost locks or stuck mounts that could cause 'Resource busy'
        if let Ok(out) = Command::new("hdiutil").arg("info").output() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                // Look for lines starting with /dev/diskX
                if line.starts_with("/dev/disk") {
                    if let Some(disk_node) = line.split_whitespace().next() {
                        let _ = Command::new("hdiutil")
                            .arg("detach")
                            .arg("-force")
                            .arg(disk_node)
                            .output();
                    }
                }
            }
        }
    }

    pub fn mount_point(&self) -> &Path {
        self.mount_dir.path()
    }

    pub fn detach(&mut self) -> Result<()> {
        if self.detached {
            return Ok(());
        }

        let output = Command::new("hdiutil")
            .arg("detach")
            .arg("-force")
            .arg(&self.device_node)
            .output()
            .context("Failed to execute hdiutil detach")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Ignore errors that indicate the image is already detached
            if stderr.contains("not found") || stderr.contains("does not exist") || stderr.contains("inesistente") {
                self.detached = true;
                return Ok(());
            }
            bail!(
                "hdiutil detach failed: {}",
                stderr
            );
        }

        self.detached = true;
        Ok(())
    }
}

impl Drop for MountedIso {
    fn drop(&mut self) {
        if self.detached {
            return;
        }

        let _ = Command::new("hdiutil")
            .arg("detach")
            .arg("-force")
            .arg(&self.device_node)
            .output();
    }
}

pub fn inspect_iso(iso_path: &Path, max_split_size_mb: u32) -> Result<IsoInspection> {
    let metadata = fs::metadata(iso_path)
        .with_context(|| format!("Unable to read metadata for {}", iso_path.display()))?;
    let mut mounted = MountedIso::attach(iso_path)?;

    let install_wim_path = mounted.mount_point().join("sources").join("install.wim");
    let install_wim_size = if install_wim_path.exists() {
        Some(
            fs::metadata(&install_wim_path)
                .with_context(|| format!("Unable to read metadata for {}", install_wim_path.display()))?
                .len(),
        )
    } else {
        None
    };

    let max_split_size_bytes = u64::from(max_split_size_mb) * 1024 * 1024;
    let needs_wim_split = install_wim_size
        .map(|size| size > max_split_size_bytes)
        .unwrap_or(false);

    let _ = mounted.detach();

    Ok(IsoInspection {
        iso_path: iso_path.to_path_buf(),
        iso_size_bytes: metadata.len(),
        install_wim_size,
        needs_wim_split,
    })
}
