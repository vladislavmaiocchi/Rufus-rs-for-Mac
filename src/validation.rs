use anyhow::{bail, Result};

pub fn normalize_volume_label(raw: &str) -> Result<String> {
    let label = raw.trim().to_ascii_uppercase();
    if label.is_empty() {
        bail!("Volume label cannot be empty");
    }
    if label.chars().count() > 11 {
        bail!("Volume label exceeds FAT32 11-character limit");
    }
    if !label
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        bail!("Volume label can only contain A-Z, 0-9 and underscore");
    }
    Ok(label)
}
