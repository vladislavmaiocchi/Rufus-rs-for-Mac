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

## 🛠 How to Compile (Building from Source)

If you want to build the project yourself, follow these steps:

### 1. Prerequisites
- **Rust & Cargo:** Install via [rustup.rs](https://rustup.rs/):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **Xcode Command Line Tools:**
  ```bash
  xcode-select --install
  ```

### 2. Compilation
To build the graphical version (GUI):
```bash
cargo build --release --bin rufus-rs-gui
```
The executable will be located at `target/release/rufus-rs-gui`.

### 3. Creating the macOS App (.app) and DMG
We use `cargo-bundle` to create the native macOS application:
1. Install the bundler: `cargo install cargo-bundle`
2. Run the bundle command:
   ```bash
   cargo bundle --release --bin rufus-rs-gui
   ```
3. Your **DMG** and **.app** will be in `target/release/bundle/`.

## 📦 Releases
You can find the pre-compiled **DMG** in the [Releases](https://github.com/vladislavmaiocchi/Rufus-rs-for-Mac/releases) section of this repository.

## 📜 License

This project is licensed under the MIT License.
