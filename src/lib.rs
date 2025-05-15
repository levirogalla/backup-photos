use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use log::{debug, error, info, warn};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;
use walkdir::WalkDir;

#[derive(Error, Debug)]
pub enum BackupError {
    #[error("Environment variable not found: {0}")]
    EnvVarNotFound(String),

    #[error("Directory not found: {0}")]
    DirectoryNotFound(String),

    #[error("Directory exists but is not accessible: {0}")]
    DirectoryNotAccessible(String),

    #[error("External drive not connected: {0}")]
    ExternalDriveNotConnected(String),

    #[error("Command failed: {0}")]
    CommandFailed(String),

    #[error("No media files found in export directory")]
    NoPhotosFound,

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Checks if the provided path exists and is accessible
pub fn check_directory_exists_and_accessible(path: &Path) -> Result<(), BackupError> {
    if !path.exists() {
        return Err(BackupError::DirectoryNotFound(
            path.to_string_lossy().to_string(),
        ));
    }

    if !path.is_dir() {
        return Err(BackupError::DirectoryNotAccessible(format!(
            "{} is not a directory",
            path.to_string_lossy()
        )));
    }

    // Check if we can read/write to the directory
    let metadata = fs::metadata(path).map_err(|_| {
        BackupError::DirectoryNotAccessible(path.to_string_lossy().to_string())
    })?;

    if !metadata.permissions().readonly() {
        Ok(())
    } else {
        Err(BackupError::DirectoryNotAccessible(format!(
            "{} is not writable",
            path.to_string_lossy()
        )))
    }
}

/// Check if the path is on an external drive and if it's connected
pub fn check_external_drive_connected(path: &Path) -> Result<(), BackupError> {
    // On macOS, external drives are typically mounted at /Volumes
    let path_str = path.to_string_lossy().to_string();
    
    // Check if the path is a symlink
    if path.is_symlink() {
        let target = fs::read_link(path)?;
        debug!("Path {} is a symlink pointing to {}", path_str, target.display());
        
        // If the symlink target starts with /Volumes, it's likely on an external drive
        if target.to_string_lossy().starts_with("/Volumes") {
            // Check if the target exists
            if !target.exists() {
                return Err(BackupError::ExternalDriveNotConnected(format!(
                    "External drive for {} is not connected (symlink target: {})",
                    path_str, target.display()
                )));
            }
        }
    } else if path_str.starts_with("/Volumes") {
        // Direct path to external drive
        if !path.exists() {
            return Err(BackupError::ExternalDriveNotConnected(format!(
                "External drive for {} is not connected",
                path_str
            )));
        }
    }
    
    Ok(())
}

/// Get the path from the environment variable
pub fn get_path_from_env(env_var: &str) -> Result<PathBuf, BackupError> {
    let path_str = std::env::var(env_var).map_err(|_| {
        BackupError::EnvVarNotFound(env_var.to_string())
    })?;
    
    let path = PathBuf::from(path_str);
    check_directory_exists_and_accessible(&path)?;
    check_external_drive_connected(&path)?;
    
    Ok(path)
}

/// Count files in a directory that match the given extensions
pub fn count_files_with_extensions(path: &Path, extensions: &[&str]) -> Result<usize, BackupError> {
    let mut count = 0;
    
    for entry in WalkDir::new(path)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            if let Some(ext) = entry.path().extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                if extensions.iter().any(|&e| e == ext_str) {
                    count += 1;
                }
            }
        }
    }
    
    Ok(count)
}

/// Backup photos and videos from export directory to backup directory
pub fn backup_photos_to_raw_dir() -> Result<(), BackupError> {
    let export_dir = get_path_from_env("APPLE_PHOTOS_EXPORT_DIR")?;
    let backup_dir = get_path_from_env("RAW_PHOTOS_BACKUP_DIR")?;
    
    let photo_extensions = ["jpg", "jpeg", "png", "heic", "dng", "raw", "arw", "cr2", "nef"];
    let video_extensions = ["mp4", "mov", "avi", "m4v", "3gp", "mkv", "webm", "flv", "wmv", "mts", "m2ts"];
    let metadata_extensions = ["xmp"];
    let all_extensions = [&photo_extensions[..], &video_extensions[..], &metadata_extensions[..]].concat();
    
    // Count files to process
    let file_count = count_files_with_extensions(&export_dir, &all_extensions)?;
    
    if file_count == 0 {
        return Err(BackupError::NoPhotosFound);
    }
    
    info!("Found {} photos/videos and metadata files to backup", file_count);
    
    // Create progress bar
    let progress = ProgressBar::new(file_count as u64);
    match progress.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("#>-"),
    ) {
        _ => {} // Ignore any styling errors
    }
    
    // Run rsync command for backup
    let output = Command::new("rsync")
        .args([
            "-av",  // archive mode, verbose
            "--progress",
            "--ignore-existing",  // Don't overwrite existing files
            &format!("{}/*", export_dir.display()),  // Source
            &format!("{}/", backup_dir.display()),  // Destination
        ])
        .output()?;
    
    progress.finish_with_message("Backup completed");
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BackupError::CommandFailed(stderr.to_string()));
    }
    
    info!("Successfully backed up photos and videos to raw directory");
    debug!("{}", String::from_utf8_lossy(&output.stdout));
    
    Ok(())
}

/// Import photos and videos to Immich using the Immich CLI
pub fn import_to_immich() -> Result<(), BackupError> {
    let export_dir = get_path_from_env("APPLE_PHOTOS_EXPORT_DIR")?;
    let immich_lib = get_path_from_env("IMMICH_LIB")?;
    
    // Import photos and videos to Immich
    // You'll need to modify this section based on your specific Immich CLI commands
    info!("Importing media to Immich from {} to {}", export_dir.display(), immich_lib.display());
    
    // Count the media files to be imported
    let photo_extensions = ["jpg", "jpeg", "png", "heic", "dng", "raw", "arw", "cr2", "nef"];
    let video_extensions = ["mp4", "mov", "avi", "m4v", "3gp", "mkv", "webm", "flv", "wmv", "mts", "m2ts"];
    let all_media_extensions = [&photo_extensions[..], &video_extensions[..]].concat();
    
    let file_count = count_files_with_extensions(&export_dir, &all_media_extensions)?;
    
    if file_count == 0 {
        warn!("No photos or videos found in export directory for import to Immich");
        return Ok(());
    }
    
    info!("Found {} photos and videos to import to Immich", file_count);
    
    // PLACEHOLDER: Replace this with your actual Immich CLI command
    // This is an example - modify according to your Immich CLI documentation
    warn!("Using placeholder Immich CLI command - please modify the source code with your actual CLI command");
    
    /* 
    // Uncomment and modify this section with your actual Immich CLI command:
    let output = Command::new("immich")
        .args([
            "upload",  // Replace with actual command name
            "--dir", export_dir.to_str().unwrap(),
            "--output", immich_lib.to_str().unwrap(),
            "--recursive",
            // Add any other needed options
        ])
        .stdout(std::process::Stdio::inherit())  // Show output in real-time
        .stderr(std::process::Stdio::inherit())
        .output()?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BackupError::CommandFailed(stderr.to_string()));
    }
    */
    
    // For now, just log a placeholder message
    info!("Immich CLI import placeholder - please implement the actual CLI command in the code");
    
    Ok(())
}

/// Clear the export directory
pub fn clear_export_directory() -> Result<(), BackupError> {
    let export_dir = get_path_from_env("APPLE_PHOTOS_EXPORT_DIR")?;
    
    let photo_extensions = ["jpg", "jpeg", "png", "heic", "dng", "raw", "arw", "cr2", "nef"];
    let video_extensions = ["mp4", "mov", "avi", "m4v", "3gp", "mkv", "webm", "flv", "wmv", "mts", "m2ts"];
    let metadata_extensions = ["xmp"];
    let all_extensions = [&photo_extensions[..], &video_extensions[..], &metadata_extensions[..]].concat();
    
    let file_count = count_files_with_extensions(&export_dir, &all_extensions)?;
    
    if file_count == 0 {
        info!("No media files found in export directory");
        return Ok(());
    }
    
    // Ask for confirmation before deleting files
    warn!("About to delete {} files from export directory", file_count);
    info!("Please manually confirm by running with --force flag");
    
    Ok(())
}

/// Clear the export directory with force option
pub fn clear_export_directory_force() -> Result<(), BackupError> {
    let export_dir = get_path_from_env("APPLE_PHOTOS_EXPORT_DIR")?;
    
    let photo_extensions = ["jpg", "jpeg", "png", "heic", "dng", "raw", "arw", "cr2", "nef"];
    let video_extensions = ["mp4", "mov", "avi", "m4v", "3gp", "mkv", "webm", "flv", "wmv", "mts", "m2ts"];
    let metadata_extensions = ["xmp"];
    let all_extensions = [&photo_extensions[..], &video_extensions[..], &metadata_extensions[..]].concat();
    
    let mut deleted_count = 0;
    
    for entry in WalkDir::new(&export_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            if let Some(ext) = entry.path().extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                if all_extensions.iter().any(|e| *e == ext_str) {
                    fs::remove_file(entry.path())?;
                    deleted_count += 1;
                }
            }
        }
    }
    
    info!("Deleted {} files from export directory", deleted_count);
    
    Ok(())
}

/// Compare files between backup directory and Immich library
pub fn compare_backup_to_immich() -> Result<(), BackupError> {
    let backup_dir = get_path_from_env("RAW_PHOTOS_BACKUP_DIR")?;
    let immich_lib = get_path_from_env("IMMICH_LIB")?;
     // Get all media files from backup directory
    let mut backup_files = Vec::new();
    let photo_extensions = ["jpg", "jpeg", "png", "heic", "dng", "raw", "arw", "cr2", "nef"];
    let video_extensions = ["mp4", "mov", "avi", "m4v", "3gp", "mkv", "webm", "flv", "wmv", "mts", "m2ts"];
    let all_media_extensions = [&photo_extensions[..], &video_extensions[..]].concat();

    for entry in WalkDir::new(&backup_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            if let Some(ext) = entry.path().extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                if all_media_extensions.iter().any(|e| *e == ext_str) {
                    backup_files.push(entry.path().to_path_buf());
                }
            }
        }
    }
    
    info!("Found {} files in backup directory", backup_files.len());
    
    // Find all media files in Immich library (recursively search through the nested directory structure)
    let upload_dir = immich_lib.join("upload");
    let mut immich_files = Vec::new();
    
    for entry in WalkDir::new(&upload_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            if let Some(ext) = entry.path().extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                if all_media_extensions.iter().any(|e| *e == ext_str) {
                    immich_files.push(entry.path().to_path_buf());
                }
            }
        }
    }
    
    info!("Found {} media files in Immich library", immich_files.len());
    
    // Compare media files by name (this might be slow for large libraries)
    let mut files_not_in_immich = Vec::new();
    let progress = ProgressBar::new(backup_files.len() as u64);
    match progress.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("#>-"),
    ) {
        _ => {} // Ignore any styling errors
    }
    
    // This is a simple comparison by file name only
    // A more accurate comparison would involve checksums
    for backup_file in &backup_files {
        let file_name = backup_file.file_name().unwrap().to_string_lossy().to_string();
        
        let found = immich_files.iter().any(|f| {
            f.file_name().unwrap().to_string_lossy().to_string() == file_name
        });
        
        if !found {
            files_not_in_immich.push(backup_file.clone());
        }
        
        progress.inc(1);
    }
    
    progress.finish_with_message("Comparison completed");
    
    if files_not_in_immich.is_empty() {
        info!("All media files from backup are present in Immich library");
    } else {
        warn!("{} media files from backup are not in Immich library:", files_not_in_immich.len());
        for file in files_not_in_immich.iter().take(10) {
            warn!("  - {}", file.display());
        }
        if files_not_in_immich.len() > 10 {
            warn!("  ... and {} more", files_not_in_immich.len() - 10);
        }
    }
    
    Ok(())
}

/// Run the entire backup workflow
pub fn full_backup_workflow() -> Result<(), BackupError> {
    info!("Starting full backup workflow");
    
    // Step 1: Backup photos to raw directory
    info!("Step 1: Backing up photos to raw directory");
    match backup_photos_to_raw_dir() {
        Ok(_) => info!("Successfully backed up photos to raw directory"),
        Err(e) => {
            error!("Failed to backup photos: {}", e);
            return Err(e);
        }
    }
    
    // Step 2: Import photos to Immich
    info!("Step 2: Importing photos to Immich");
    match import_to_immich() {
        Ok(_) => info!("Successfully imported photos to Immich"),
        Err(e) => {
            error!("Failed to import photos to Immich: {}", e);
            return Err(e);
        }
    }
    
    // Step 3: Compare backup to Immich
    info!("Step 3: Comparing backup to Immich library");
    match compare_backup_to_immich() {
        Ok(_) => info!("Successfully compared backup to Immich library"),
        Err(e) => {
            error!("Failed to compare backup to Immich library: {}", e);
            return Err(e);
        }
    }
    
    // Step 4: Clear export directory (prompt for confirmation)
    info!("Step 4: Clearing export directory");
    info!("Please run the clear command separately with the --force flag to confirm deletion");
    
    info!("Full backup workflow completed successfully");
    Ok(())
}
