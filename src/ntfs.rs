use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use tempfile::TempDir;

#[derive(Debug)]
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

    pub fn format(&self, disk_node: &str, partition_node: &str, label: &str) -> Result<()> {
        let binary_path = self.mkntfs_path.to_string_lossy();
        // Combine unmount and format into one atomic sudo call to prevent macOS from remounting the disk
        let shell_cmd = format!("diskutil unmountDisk force '{}'; {} -Q -L '{}' -F '{}'", disk_node, binary_path, label, partition_node);
        
        let mut cmd = Command::new("sudo");
        cmd.arg("-S");
        cmd.arg("-p");
        cmd.arg(""); // Empty prompt
        cmd.arg("sh");
        cmd.arg("-c");
        cmd.arg(&shell_cmd);
        cmd.stdin(Stdio::piped());

        let mut child = cmd.spawn().context("Failed to spawn sudo process for mkntfs")?;
        
        // Provide the password provided by the user
        let password = "Vladik12"; 
        let mut stdin = child.stdin.take().context("Failed to open stdin for sudo")?;
        std::io::Write::write_all(&mut stdin, format!("{}\n", password).as_bytes())?;
        
        let output = child.wait_with_output().context("Failed to wait for mkntfs output")?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("mkntfs failed: {}", stderr);
        }
        
        Ok(())
    }
}
