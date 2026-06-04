# Changelog

All notable changes to this project will be documented in this file.

## [0.1.0] - 2026-06-04

### Added
- **Initial GUI Implementation**: Created a user-friendly interface using `egui` and `eframe`.
- **WIM Splitting**: Added support for splitting large `install.wim` files (>4GB) into `.swm` chunks for FAT32 compatibility.
- **NTFS Support**: Integrated `mkntfs` to allow creating NTFS-formatted bootable drives on macOS.
- **Full Disk Access Guidance**: Added a detection system that prompts users to grant "Full Disk Access" in macOS System Settings when raw disk access is blocked.
- **Direct System Settings Button**: Added a button in the GUI to open the Privacy & Security panel directly.
- **Improved Logging**: Made the log area interactive (selectable/copyable) for easier troubleshooting.
- **Robust Disk Management**: Implemented aggressive unmounting and partition header clearing (using `dd`) to combat macOS auto-mounting issues.
- **GitHub Repository**: Initialized the project on GitHub with comprehensive documentation.

### Improved (Work in Progress)
- **NTFS Formatting Robustness**: Implemented multiple strategies to fix "Read-only file system" errors, including whole-disk unmounting, partition header clearing with `dd`, and use of raw device paths. However, **NTFS formatting may still fail** on some systems due to macOS SIP/Kernel locks.
- **Partition Selection**: Improved logic to identify the correct data partition and avoid system EFI partitions.

### Known Issues
- **NTFS "Read-only file system"**: Despite aggressive unmounting attempts, macOS may still lock the raw disk device, preventing `mkntfs` from working. This is a known limitation of direct disk access on modern macOS.
