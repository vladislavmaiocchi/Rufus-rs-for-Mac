use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
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
    detached: bool,
}

impl MountedIso {
    pub fn attach(iso_path: &Path) -> Result<Self> {
        if !iso_path.exists() {
            bail!("ISO file not found: {}", iso_path.display());
        }

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

        if !output.status.success() {
            bail!(
                "hdiutil attach failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(Self {
            mount_dir,
            detached: false,
        })
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
            .arg(self.mount_dir.path())
            .output()
            .context("Failed to execute hdiutil detach")?;

        if !output.status.success() {
            bail!(
                "hdiutil detach failed: {}",
                String::from_utf8_lossy(&output.stderr)
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
            .arg(self.mount_dir.path())
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

    mounted.detach()?;

    Ok(IsoInspection {
        iso_path: iso_path.to_path_buf(),
        iso_size_bytes: metadata.len(),
        install_wim_size,
        needs_wim_split,
    })
}
