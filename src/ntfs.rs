use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use tempfile::TempDir;

pub struct NtfsTools {
    _temp_dir: TempDir,
    mkntfs_path: PathBuf,
}

impl NtfsTools {
    pub fn new() -> Result<Self> {
        let temp_dir = TempDir::new().context("Failed to create temporary directory for NTFS tools")?;
        
        let mkntfs_bytes = include_bytes!("../assets/bin/mkntfs");
        let mkntfs_path = temp_dir.path().join("mkntfs");
        
        fs::write(&mkntfs_path, mkntfs_bytes).context("Failed to write mkntfs to temporary directory")?;
        fs::set_permissions(&mkntfs_path, Permissions::from_mode(0o755))
            .context("Failed to set execution permissions on mkntfs")?;
            
        Ok(Self {
            _temp_dir: temp_dir,
            mkntfs_path,
        })
    }

    pub fn format(&self, device: &str, label: &str) -> Result<()> {
        let mut cmd = Command::new(&self.mkntfs_path);
        cmd.arg("-Q") // Quick format
           .arg("-L")
           .arg(label)
           .arg("-F") // Force (needed for some devices)
           .arg(device);
           
        let output = cmd.output().context("Failed to execute mkntfs")?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("mkntfs failed: {}", stderr);
        }
        
        Ok(())
    }
}
