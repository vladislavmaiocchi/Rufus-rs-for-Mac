# Rufus-rs for Mac

A Rufus-inspired utility for creating bootable Windows USB drives on macOS. This tool is designed to simplify the process of making Windows installers directly from a Mac, with support for both FAT32 and NTFS filesystems.

## 🚀 Features

- **GUI & CLI:** Easy-to-use graphical interface built with `egui` and a powerful command-line interface.
- **WIM Splitting:** Automatically splits `install.wim` files larger than 4GB when using FAT32, ensuring compatibility with UEFI systems.
- **NTFS Support:** Includes built-in `mkntfs` to format drives as NTFS (requires Full Disk Access).
- **Disk Safety:** Filters out internal disks to prevent accidental data loss.
- **Dry-run Mode:** Preview the actions before they are executed.

## ⚠️ Current Status & Known Issues

### NTFS Formatting
- **Issue:** macOS aggressively locks raw disks, which can cause `mkntfs` to fail with a "Read-only file system" error.
- **Solution:** The app includes a button to open **System Settings > Privacy & Security > Full Disk Access**. You must grant this permission to the app and restart it to enable NTFS formatting.
- **Recommendation:** Use **FAT32** for maximum reliability. The tool's WIM-splitting feature makes FAT32 perfectly capable of handling large Windows ISOs.

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
