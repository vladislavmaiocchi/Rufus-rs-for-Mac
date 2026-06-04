# Rufus-rs for Mac

A Rufus-inspired utility for creating bootable Windows USB drives on macOS. This tool is designed to simplify the process of making Windows installers directly from a Mac, with support for both FAT32 and NTFS filesystems.

## 🚀 Features

- **GUI & CLI:** Easy-to-use graphical interface built with `egui` and a powerful command-line interface.
- **WIM Splitting:** Automatically splits `install.wim` files larger than 4GB when using FAT32, ensuring compatibility with UEFI systems.
- **NTFS Support:** Includes built-in `mkntfs` to format drives as NTFS (requires Full Disk Access).
- **Disk Safety:** Filters out internal disks to prevent accidental data loss.
- **Dry-run Mode:** Preview the actions before they are executed.

## ⚠️ CRITICAL: NTFS Status & Recommended Workaround

**NTFS formatting is currently UNSTABLE on macOS.**

Due to macOS System Integrity Protection (SIP) and aggressive disk locking by the kernel, `mkntfs` often fails with a **"Read-only file system"** error, even when the app has Full Disk Access. We have implemented several workarounds (header clearing, forced unmounts), but success is not guaranteed.

### ✅ The Recommended Way: Use FAT32
Do not worry about the 4GB file limit of FAT32. This tool features **Automatic WIM Splitting**:
1. Select **FAT32 (Standard)** in the app.
2. The tool will automatically detect if `install.wim` is too large.
3. It will split it into `.swm` chunks that fit on FAT32.
4. Windows Installer recognizes these chunks perfectly.
5. This method is **100% reliable** on macOS and results in a more compatible UEFI bootable drive.

### Permissions
- Direct disk access requires specific macOS permissions. Always follow the prompts within the app log.

## 🛠 Installation (Development)

1.  Ensure you have Rust installed: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
2.  Clone the repository:
    ```bash
    git clone git@github.com:vladislavmaiocchi/Rufus-rs-for-Mac.git
    cd Rufus-rs-for-Mac
    ```
3.  Build the project:
    ```bash
    cargo build --release
    ```

## 📦 Creating a Bundle (DMG)

To create a macOS `.app` bundle and a `.dmg` installer:
```bash
cargo bundle --release --bin rufus-rs-gui
```
The output will be in `target/release/bundle/dmg/`.

## 📜 License

This project is licensed under the MIT License.
