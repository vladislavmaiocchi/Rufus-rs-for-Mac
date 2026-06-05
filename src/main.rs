
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use rufus_rs::disk::{self, DiskInfo};
use rufus_rs::i18n::Language;
use rufus_rs::pipeline::{self, CreateUsbOptions, CreateUsbPlan, FileSystem};
use rufus_rs::{iso, validation};

#[derive(Parser, Debug)]
#[command(
    name = "rufus-rs",
    version,
    about = "Rufus-like CLI for creating Windows bootable USB drives on macOS"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    /// Language to use (en, it)
    #[arg(long, default_value = "en", global = true)]
    lang: String,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Show available whole disks and whether they are safe targets
    ListDisks,
    /// Create a Windows USB from an ISO
    CreateUsb(CreateUsbArgs),
}

#[derive(Args, Debug)]
struct CreateUsbArgs {
    /// Path to a Windows ISO image
    #[arg(long)]
    iso: PathBuf,
    /// Target whole disk identifier (example: disk4 or /dev/disk4)
    #[arg(long)]
    disk: String,
    /// Filesystem to use (fat32, exfat or ntfs)
    #[arg(long, default_value = "fat32")]
    fs: String,
    /// FAT32 volume label (max 11 chars, A-Z, 0-9 or underscore)
    #[arg(long, default_value = "WININSTALL")]
    label: String,
    /// Max split size (MB) when install.wim exceeds FAT32 limit
    #[arg(long, default_value_t = 3800)]
    max_split_size_mb: u32,
    /// Execute destructive operations (otherwise dry-run only)
    #[arg(long, default_value_t = false)]
    execute: bool,
    /// Required with --execute to acknowledge disk erase
    #[arg(long, default_value_t = false, requires = "execute")]
    yes_erase_disk: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let lang = match cli.lang.to_lowercase().as_str() {
        "it" => Language::Italian,
        _ => Language::English,
    };

    match cli.command {
        Commands::ListDisks => list_disks_command(lang),
        Commands::CreateUsb(args) => create_usb_command(args, lang),
    }
}

fn list_disks_command(lang: Language) -> Result<()> {
    let disks = disk::list_disks().context("Unable to query disk list with diskutil")?;
    print_disks(&disks, lang);
    Ok(())
}

fn print_disks(disks: &[DiskInfo], lang: Language) {
    let t = lang.translations();
    println!(
        "{:<14} {:<8} {:<9} {:<11} {}",
        t.cli_list_header_disk,
        t.cli_list_header_internal,
        t.cli_list_header_removable,
        t.cli_list_header_size,
        t.cli_list_header_model
    );
    for disk in disks {
        println!(
            "{:<14} {:<8} {:<9} {:<11} {}",
            format!("/dev/{}", disk.identifier),
            if disk.internal { "yes" } else { "no" },
            if disk.removable { "yes" } else { "no" },
            disk::format_bytes(disk.size_bytes),
            disk.model
        );
    }
    println!();
    println!("{}", t.cli_internal_warning);
}

fn create_usb_command(args: CreateUsbArgs, lang: Language) -> Result<()> {
    let t = lang.translations();
    let iso_path = args
        .iso
        .canonicalize()
        .with_context(|| format!("ISO not found: {}", args.iso.display()))?;
    let disk_identifier = disk::normalize_disk_identifier(&args.disk);
    let label = validation::normalize_volume_label(&args.label)?;
    
    let fs = match args.fs.to_lowercase().as_str() {
        "fat32" => FileSystem::Fat32,
        "exfat" => FileSystem::ExFat,
        "ntfs" => FileSystem::Ntfs,
        _ => bail!("Unsupported filesystem: {}. Use 'fat32', 'exfat' or 'ntfs'.", args.fs),
    };

    let target_disk = disk::find_disk(&disk_identifier)
        .with_context(|| format!("Target disk not found: {}", disk_identifier))?;
    if target_disk.internal {
        bail!(
            "Refusing to use /dev/{} because it is an internal disk",
            target_disk.identifier
        );
    }

    let options = CreateUsbOptions {
        iso_path,
        disk_identifier,
        label,
        fs,
        max_split_size_mb: args.max_split_size_mb,
        lang,
    };

    let inspection = iso::inspect_iso(&options.iso_path, options.max_split_size_mb)
        .context("Could not inspect ISO content")?;
    let plan = pipeline::build_plan(&options, &target_disk, &inspection);
    print_plan(&plan, lang);

    if !args.execute {
        println!("{}", t.cli_dry_run_only);
        println!("{}", t.cli_rerun_instruction);
        return Ok(());
    }
    if !args.yes_erase_disk {
        bail!("Missing --yes-erase-disk flag");
    }

    pipeline::execute_create_usb(&options, &inspection, |p| {
        print!("\r{}", t.cli_progress.replace("{:.1}%", &format!("{:.1}%", p * 100.0)));
        use std::io::{stdout, Write};
        let _ = stdout().flush();
    })?;
    println!();
    Ok(())
}

fn print_plan(plan: &CreateUsbPlan, lang: Language) {
    let t = lang.translations();
    println!("{}", t.cli_planned_actions);
    for (index, step) in plan.steps.iter().enumerate() {
        println!("  {}. {}", index + 1, step);
    }
    println!();
}
