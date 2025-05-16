use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use log::{debug, error, info, warn};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use thiserror::Error;
use walkdir::WalkDir;
use std::io;

pub mod constants;
pub mod api_key;

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

    #[error("No media files found in export directory. Make sure to export photos from Apple Photos first.")]
    NoPhotosFound,

    #[error("Export directory is empty: {0}")]
    ExportDirEmpty(String),

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
    let metadata = fs::metadata(path)
        .map_err(|_| BackupError::DirectoryNotAccessible(path.to_string_lossy().to_string()))?;

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
        debug!(
            "Path {} is a symlink pointing to {}",
            path_str,
            target.display()
        );

        // If the symlink target starts with /Volumes, it's likely on an external drive
        if target.to_string_lossy().starts_with("/Volumes") {
            // Check if the target exists
            if !target.exists() {
                return Err(BackupError::ExternalDriveNotConnected(format!(
                    "External drive for {} is not connected (symlink target: {})",
                    path_str,
                    target.display()
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

/// Initialize the required directories from environment variables
pub fn init_directories() -> Result<(), BackupError> {
    let vars = [
        constants::APPLE_PHOTOS_EXPORT_DIR,
        constants::RAW_PHOTOS_BACKUP_DIR,
        constants::IMMICH_LIB,
    ];

    for var in vars {
        let path = PathBuf::from(&var);

        // Check if directory already exists
        if path.exists() {
            info!("Directory for {} already exists at {}", var, path.display());
            continue;
        }

        // Check if it's on an external drive (might not be plugged in)
        if var.starts_with("/Volumes") {
            warn!("Path {} points to an external drive. Make sure the drive is connected before continuing.", path.display());
        }

        // Create the directory
        info!("Creating directory for {} at {}", var, path.display());
        match fs::create_dir_all(&path) {
            Ok(_) => info!("Successfully created directory {}", path.display()),
            Err(e) => {
                return Err(BackupError::IoError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to create directory {}: {}", path.display(), e),
                )));
            }
        }
    }

    info!("All required directories have been initialized");
    info!("You should now:");
    info!("1. Export photos from Apple Photos to the export directory");
    info!("2. Run 'backup-photos backup' to backup photos to your raw backup directory");
    info!("3. Run 'backup-photos import' to import photos to Immich");

    Ok(())
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

/// Fix XMP apple xmp files
/// Fixes XMP files exported by Apple Photos using exiftool.
/// This repairs GPS and EXIF tags in all .xmp files in the given directory.
/// Shows a progress bar for the number of XMP files processed.
pub fn fix_apple_xmp_files(dir: &Path) -> Result<(), BackupError> {
    // Count XMP files for progress bar
    let mut xmp_count = 0;
    for entry in WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            if let Some(ext) = entry.path().extension() {
                if ext.to_string_lossy().eq_ignore_ascii_case("xmp") {
                    xmp_count += 1;
                }
            }
        }
    }

    if xmp_count == 0 {
        info!("No XMP files found in {}", dir.display());
        return Ok(());
    }

    info!("Found {} XMP files to repair in {}", xmp_count, dir.display());


    // exiftool will process all .xmp files in the directory recursively
    let status = Command::new("exiftool")
        .args([
            "-P",
            "-overwrite_original",
            "-ext", "xmp",
            "-XMP-exif:All=",
            "-tagsFromFile", "@",
            "-XMP-exif:All",
            "-XMP-exif:GPSLongitude<${XMP-exif:GPSLongitude#}${XMP-exif:GPSLongitudeRef#}",
            "-XMP-exif:GPSLatitude<${XMP-exif:GPSLatitude#}${XMP-exif:GPSLatitudeRef#}",
            dir.to_string_lossy().as_ref(),
        ])
        .current_dir(dir)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .and_then(|mut child| {
            // Optionally, parse exiftool output for progress (not always possible)
            // Here, just wait for completion and update bar at the end
            let status = child.wait()?;
            Ok(status)
        })
        .map_err(|e| BackupError::CommandFailed(format!("Failed to run exiftool: {}", e)))?;

    if !status.success() {
        return Err(BackupError::CommandFailed(format!(
            "exiftool exited with status: {}",
            status
        )));
    }

    Ok(())
}

/// Backup photos and videos from export directory to backup directory
pub fn backup_photos_to_raw_dir() -> Result<(), BackupError> {
    let export_dir = PathBuf::from(constants::APPLE_PHOTOS_EXPORT_DIR);
    let backup_dir = PathBuf::from(constants::RAW_PHOTOS_BACKUP_DIR);

    let photo_extensions = [
        "jpg", "jpeg", "png", "heic", "dng", "raw", "arw", "cr2", "nef",
    ];
    let video_extensions = [
        "mp4", "mov", "avi", "m4v", "3gp", "mkv", "webm", "flv", "wmv", "mts", "m2ts",
    ];
    let metadata_extensions = ["xmp"];
    let all_extensions = [
        &photo_extensions[..],
        &video_extensions[..],
        &metadata_extensions[..],
    ]
    .concat();

    // Count files to process
    let file_count = count_files_with_extensions(&export_dir, &all_extensions)?;

    if file_count == 0 {
        return Err(BackupError::NoPhotosFound);
    }

    info!(
        "Found {} photos/videos and metadata files to backup",
        file_count
    );

    // Create progress bar
    let progress = ProgressBar::new(file_count as u64);
    match progress.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
            )
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("#>-"),
    ) {
        _ => {} // Ignore any styling errors
    }

    // Run rsync command for backup
    // First, check if the export directory exists and has files
    if !export_dir.exists() {
        return Err(BackupError::DirectoryNotFound(
            export_dir.to_string_lossy().to_string(),
        ));
    }

    // Check if there are any files in the export directory
    let has_files = fs::read_dir(&export_dir)?.next().is_some();
    if !has_files {
        return Err(BackupError::ExportDirEmpty(format!(
            "The export directory '{}' exists but is empty. Have you exported photos from Apple Photos?",
            export_dir.display()
        )));
    }

    debug!(
        "Running rsync from {} to {}",
        export_dir.display(),
        backup_dir.display()
    );

     let mut child = Command::new("rsync")
        .args([
            "-av", // archive mode, verbose
            "--progress", // show live progress
            "--ignore-existing", // skip files already in destination
            &format!("{}/", export_dir.display()), // source dir contents
            &format!("{}/", backup_dir.display()), // destination dir
        ])
        .stdout(Stdio::inherit()) // stream stdout to terminal
        .stderr(Stdio::inherit()) // stream stderr to terminal
        .spawn()
        .map_err(|e| BackupError::IoError(io::Error::new(io::ErrorKind::Other, format!("Failed to spawn rsync: {e}"))))?;

    let status = child
        .wait()
        .map_err(|e| BackupError::IoError(io::Error::new(io::ErrorKind::Other, format!("Failed to wait on rsync: {e}"))))?;

    progress.finish_with_message("Backup completed");

    if !status.success() {
        return Err(BackupError::CommandFailed(format!("rsync exited with status: {}", status)));
    }

    info!("Successfully backed up photos and videos to raw directory");

    Ok(())
}

/// Import photos and videos to Immich using the Immich CLI
pub fn import_to_immich() -> Result<(), BackupError> {
    let export_dir = PathBuf::from(constants::APPLE_PHOTOS_EXPORT_DIR);
    let immich_lib = PathBuf::from(constants::IMMICH_LIB);

    info!("Reparing XMP to import photos and videos to Immich");
    fix_apple_xmp_files(&export_dir)?;

    // Import photos and videos to Immich
    // You'll need to modify this section based on your specific Immich CLI commands
    info!(
        "Importing media to Immich from {} to {}",
        export_dir.display(),
        immich_lib.display()
    );

    // Count the media files to be imported
    let photo_extensions = [
        "jpg", "jpeg", "png", "heic", "dng", "raw", "arw", "cr2", "nef",
    ];
    let video_extensions = [
        "mp4", "mov", "avi", "m4v", "3gp", "mkv", "webm", "flv", "wmv", "mts", "m2ts",
    ];
    let all_media_extensions = [&photo_extensions[..], &video_extensions[..]].concat();

    let file_count = count_files_with_extensions(&export_dir, &all_media_extensions)?;

    if file_count == 0 {
        warn!("No photos or videos found in export directory for import to Immich");
        return Ok(());
    }

    info!("Found {} photos and videos to import to Immich", file_count);

    let output = Command::new("immich-go")
    .args([
            "-k", &api_key::API_KEY,
            "--server", &constants::IMMICH_SERVER,
            "upload",
            "from-folder",
            constants::APPLE_PHOTOS_EXPORT_DIR,
        ])
        .output()?;
    info!(
        "Immich CLI output: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BackupError::CommandFailed(stderr.to_string()));
    }

    Ok(())
}

/// Clear the export directory
pub fn clear_export_directory() -> Result<(), BackupError> {
    let export_dir = PathBuf::from(constants::APPLE_PHOTOS_EXPORT_DIR);

    let photo_extensions = [
        "jpg", "jpeg", "png", "heic", "dng", "raw", "arw", "cr2", "nef",
    ];
    let video_extensions = [
        "mp4", "mov", "avi", "m4v", "3gp", "mkv", "webm", "flv", "wmv", "mts", "m2ts",
    ];
    let metadata_extensions = ["xmp"];
    let all_extensions = [
        &photo_extensions[..],
        &video_extensions[..],
        &metadata_extensions[..],
    ]
    .concat();

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
    let export_dir = PathBuf::from(constants::APPLE_PHOTOS_EXPORT_DIR);

    let photo_extensions = [
        "jpg", "jpeg", "png", "heic", "dng", "raw", "arw", "cr2", "nef",
    ];
    let video_extensions = [
        "mp4", "mov", "avi", "m4v", "3gp", "mkv", "webm", "flv", "wmv", "mts", "m2ts",
    ];
    let metadata_extensions = ["xmp"];
    let all_extensions = [
        &photo_extensions[..],
        &video_extensions[..],
        &metadata_extensions[..],
    ]
    .concat();

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

/// Calculate SHA-256 hash of a file
fn calculate_file_hash(path: &Path) -> Result<String, BackupError> {
    let file = fs::File::open(path).map_err(|e| {
        BackupError::IoError(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to open file for hashing: {}", e),
        ))
    })?;

    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0; 1024 * 1024]; // 1MB buffer for reading

    loop {
        let bytes_read = reader.read(&mut buffer).map_err(|e| {
            BackupError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to read file for hashing: {}", e),
            ))
        })?;

        if bytes_read == 0 {
            break;
        }

        hasher.update(&buffer[..bytes_read]);
    }

    let hash = hasher.finalize();
    Ok(format!("{:x}", hash))
}

/// Find files in backup directory that are not in Immich library using content hashing
pub fn find_files_not_in_immich() -> Result<Vec<PathBuf>, BackupError> {
    let backup_dir = PathBuf::from(constants::RAW_PHOTOS_BACKUP_DIR);
    let immich_lib = PathBuf::from(constants::IMMICH_LIB);

    // Get all media files from backup directory (explicitly excluding XMP files)
    let mut backup_files = Vec::new();
    let photo_extensions = [
        "jpg", "jpeg", "png", "heic", "dng", "raw", "arw", "cr2", "nef",
    ];
    let video_extensions = [
        "mp4", "mov", "avi", "m4v", "3gp", "mkv", "webm", "flv", "wmv", "mts", "m2ts",
    ];
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

    info!(
        "Found {} media files in backup directory",
        backup_files.len()
    );

    // Find all media files in Immich library
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
    info!("Calculating hashes for Immich files (this may take a while)...");

    // Create a HashSet of Immich file hashes
    let mut immich_hashes = std::collections::HashSet::new();
    let immich_progress = ProgressBar::new(immich_files.len() as u64);
    match immich_progress.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
            )
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("#>-"),
    ) {
        _ => {} // Ignore any styling errors
    }

    for immich_file in &immich_files {
        match calculate_file_hash(&immich_file) {
            Ok(hash) => {
                immich_hashes.insert(hash);
            }
            Err(e) => {
                warn!("Failed to hash file {}: {}", immich_file.display(), e);
            }
        }
        immich_progress.inc(1);
    }

    immich_progress.finish_with_message("Immich file hashing completed");

    // Compare files by content hash
    info!("Comparing backup files with Immich library by content hash...");
    let mut files_not_in_immich = Vec::new();
    let progress = ProgressBar::new(backup_files.len() as u64);
    match progress.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
            )
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("#>-"),
    ) {
        _ => {} // Ignore any styling errors
    }

    for backup_file in &backup_files {
        match calculate_file_hash(&backup_file) {
            Ok(hash) => {
                if !immich_hashes.contains(&hash) {
                    files_not_in_immich.push(backup_file.clone());
                }
            }
            Err(e) => {
                warn!(
                    "Failed to hash backup file {}: {}",
                    backup_file.display(),
                    e
                );
                // Add file to not found list since we couldn't verify it
                files_not_in_immich.push(backup_file.clone());
            }
        }

        progress.inc(1);
    }

    progress.finish_with_message("Comparison completed");

    if files_not_in_immich.is_empty() {
        info!("All media files from backup are present in Immich library (based on content hash)");
    } else {
        warn!(
            "{} media files from backup are not in Immich library:",
            files_not_in_immich.len()
        );
        for file in files_not_in_immich.iter().take(10) {
            warn!("  - {}", file.display());
        }
        if files_not_in_immich.len() > 10 {
            warn!("  ... and {} more", files_not_in_immich.len() - 10);
        }
    }

    Ok(files_not_in_immich)
}

/// Compare files between backup directory and Immich library
pub fn compare_backup_to_immich() -> Result<(), BackupError> {
    find_files_not_in_immich()?;

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

/// Synchronize backup directory with Immich library
/// by interactively handling files that are in backup but not in Immich
pub fn sync_backup_with_immich() -> Result<(), BackupError> {
    use std::io::{self, BufRead, Write};

    // Get the list of files that are in the backup but not in Immich
    let mut files_not_in_immich = find_files_not_in_immich()?;

    if files_not_in_immich.is_empty() {
        info!("No discrepancies found. All media files from backup are present in Immich library.");
        return Ok(());
    }

    info!(
        "Found {} media files in backup that are not in Immich library.",
        files_not_in_immich.len()
    );

    // Offer option to filter by media type or pattern
    print!("Do you want to filter files by media type or pattern? [y/N]: ");
    io::stdout().flush()?;

    let stdin = io::stdin();
    let mut handle = stdin.lock();
    let mut input = String::new();
    handle.read_line(&mut input)?;

    if input.trim().eq_ignore_ascii_case("y") {
        print!("Filter by (1) Photos only, (2) Videos only, (3) Filename pattern: ");
        io::stdout().flush()?;
        input.clear();
        handle.read_line(&mut input)?;

        let choice = input.trim();
        match choice {
            "1" => {
                info!("Filtering by photos only");
                files_not_in_immich.retain(|path| {
                    if let Some(ext) = path.extension() {
                        let ext_str = ext.to_string_lossy().to_lowercase();
                        [
                            "jpg", "jpeg", "png", "heic", "dng", "raw", "arw", "cr2", "nef",
                        ]
                        .contains(&ext_str.as_ref())
                    } else {
                        false
                    }
                });
                info!("Found {} photo files to process", files_not_in_immich.len());
            }
            "2" => {
                info!("Filtering by videos only");
                files_not_in_immich.retain(|path| {
                    if let Some(ext) = path.extension() {
                        let ext_str = ext.to_string_lossy().to_lowercase();
                        [
                            "mp4", "mov", "avi", "m4v", "3gp", "mkv", "webm", "flv", "wmv", "mts",
                            "m2ts",
                        ]
                        .contains(&ext_str.as_ref())
                    } else {
                        false
                    }
                });
                info!("Found {} video files to process", files_not_in_immich.len());
            }
            "3" => {
                print!("Enter filename pattern to match: ");
                io::stdout().flush()?;
                input.clear();
                handle.read_line(&mut input)?;
                let pattern = input.trim().to_lowercase();
                info!("Filtering by pattern: '{}'", pattern);

                files_not_in_immich.retain(|path| {
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .map(|name| name.to_lowercase().contains(&pattern))
                        .unwrap_or(false)
                });
                info!("Found {} files matching pattern", files_not_in_immich.len());
            }
            _ => {
                info!("No filter applied");
            }
        }
    }

    if files_not_in_immich.is_empty() {
        info!("No files to process after filtering. Exiting.");
        return Ok(());
    }

    info!("Beginning interactive sync process...");
    info!("-------------------------------------------------");
    info!("Options for each file:");
    info!("[t] Move to trash (safer than permanent deletion)");
    info!("[k] Keep in backup (skip this file)");
    info!("[v] View file info and optionally open file");
    info!("[d] Open directory containing file");
    info!("[s] Select multiple files for batch processing");
    info!("[f] Apply filter to remaining files");
    info!("[q] Quit sync process");
    info!("[a] Process all remaining files with the same action");
    info!("-------------------------------------------------");

    // Prepare trash directory - on macOS, this is ~/.Trash
    let home_dir = dirs::home_dir().ok_or_else(|| {
        BackupError::DirectoryNotAccessible("Could not determine home directory".to_string())
    })?;
    let trash_dir = home_dir.join(".Trash");

    if !trash_dir.exists() {
        warn!(
            "Trash directory not found at expected location: {}",
            trash_dir.display()
        );
        warn!("Will attempt to use it anyway as macOS should create it if needed");
    }

    let stdin = io::stdin();
    let mut handle = stdin.lock();
    let mut input = String::new();
    let mut i = 0;
    let mut all_action: Option<char> = None;

    while i < files_not_in_immich.len() {
        let file = &files_not_in_immich[i];
        let file_name = file.file_name().unwrap_or_default().to_string_lossy();

        // If we have an "all" action set, use it without prompting
        if let Some(action) = all_action {
            match action {
                't' => {
                    // Move to trash
                    let mut destination = trash_dir.join(&*file_name);
                    let original_name = file_name.to_string();

                    // Handle name collisions by appending a timestamp
                    let mut counter = 1;
                    while destination.exists() {
                        let timestamp = chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
                        let new_name = format!("{}-{}-{}", original_name, timestamp, counter);
                        destination = trash_dir.join(new_name);
                        counter += 1;
                    }

                    info!("Moving to trash: {}", file.display());
                    match fs::copy(file, &destination) {
                        Ok(_) => match fs::remove_file(file) {
                            Ok(_) => info!("File successfully moved to trash"),
                            Err(e) => warn!(
                                "File was copied to trash but could not be deleted from backup: {}",
                                e
                            ),
                        },
                        Err(e) => error!("Failed to copy file to trash: {}", e),
                    }
                }
                'k' => {
                    // Keep in backup
                    info!("Keeping in backup: {}", file.display());
                }
                _ => {
                    warn!("Invalid action for all files. Exiting sync process.");
                    break;
                }
            }
            i += 1;
            continue;
        }

        info!(
            "File {}/{}: {}",
            i + 1,
            files_not_in_immich.len(),
            file.display()
        );
        print!("Action [t/k/v/q/a]: ");
        io::stdout().flush()?;

        input.clear();
        handle.read_line(&mut input)?;
        let action = input.trim().chars().next().unwrap_or('?');

        match action {
            't' => {
                // Move to trash
                let mut destination = trash_dir.join(&*file_name);
                let original_name = file_name.to_string();

                // Handle name collisions by appending a timestamp
                let mut counter = 1;
                while destination.exists() {
                    let timestamp = chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
                    let new_name = format!("{}-{}-{}", original_name, timestamp, counter);
                    destination = trash_dir.join(new_name);
                    counter += 1;
                }

                info!("Moving to trash: {}", file.display());
                match fs::copy(file, &destination) {
                    Ok(_) => {
                        match fs::remove_file(file) {
                            Ok(_) => info!("File successfully moved to trash"),
                            Err(e) => {
                                warn!("File was copied to trash but could not be deleted from backup: {}", e);
                                warn!("Manual deletion may be required");
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to copy file to trash: {}", e);
                        print!("Try again? [Y/n]: ");
                        io::stdout().flush()?;
                        input.clear();
                        handle.read_line(&mut input)?;
                        if !input.trim().eq_ignore_ascii_case("n") {
                            // Don't increment i so we try this file again
                            continue;
                        }
                    }
                }
                i += 1;
            }
            'k' => {
                // Keep in backup
                info!("Keeping in backup: {}", file.display());
                i += 1;
            }
            'v' => {
                // Show more file info
                let metadata = fs::metadata(file)?;
                let created = metadata.created().ok();
                let modified = metadata.modified().ok();

                info!("File info for {}", file.display());
                info!("Size: {} bytes", metadata.len());
                if let Some(time) = created {
                    info!("Created: {:?}", time);
                }
                if let Some(time) = modified {
                    info!("Modified: {:?}", time);
                }

                // Media type detection based on extension
                if let Some(ext) = file.extension() {
                    let ext_str = ext.to_string_lossy().to_lowercase();
                    let media_type = if [
                        "jpg", "jpeg", "png", "heic", "dng", "raw", "arw", "cr2", "nef",
                    ]
                    .contains(&ext_str.as_ref())
                    {
                        "Photo"
                    } else if [
                        "mp4", "mov", "avi", "m4v", "3gp", "mkv", "webm", "flv", "wmv", "mts",
                        "m2ts",
                    ]
                    .contains(&ext_str.as_ref())
                    {
                        "Video"
                    } else {
                        "Unknown"
                    };
                    info!("Media Type: {}", media_type);
                }

                // Optionally open the image for viewing (macOS only)
                print!("View this file? [y/N]: ");
                io::stdout().flush()?;

                input.clear();
                handle.read_line(&mut input)?;
                if input.trim().eq_ignore_ascii_case("y") {
                    info!("Opening file with default application...");
                    let _ = Command::new("open").arg(file).spawn()?;

                    // Give user a moment to view the file
                    print!("Press Enter to continue...");
                    io::stdout().flush()?;
                    input.clear();
                    handle.read_line(&mut input)?;
                }

                // Don't increment i so we process this file again
            }
            'd' => {
                // Open directory containing the file
                info!("Opening directory containing file...");
                if let Some(parent) = file.parent() {
                    let _ = Command::new("open").arg(parent).spawn()?;

                    // Give user a moment
                    print!("Press Enter to continue...");
                    io::stdout().flush()?;
                    input.clear();
                    handle.read_line(&mut input)?;
                } else {
                    warn!("Could not determine parent directory for file");
                }

                // Don't increment i so we process this file again
            }
            's' => {
                // Select multiple files for batch processing
                info!(
                    "Starting batch selection mode. You'll be shown each file to select or skip."
                );
                let mut selected_indices = Vec::new();
                let start_idx = i;
                let mut batch_idx = start_idx;

                // Loop through remaining files to select
                while batch_idx < files_not_in_immich.len() {
                    let batch_file = &files_not_in_immich[batch_idx];

                    info!(
                        "File {}/{}: {}",
                        batch_idx + 1,
                        files_not_in_immich.len(),
                        batch_file.display()
                    );
                    print!("Select this file? [y/n/v/d/q]: ");
                    io::stdout().flush()?;

                    input.clear();
                    handle.read_line(&mut input)?;
                    let select_action = input.trim().chars().next().unwrap_or('?');

                    match select_action {
                        'y' => {
                            info!("File selected");
                            selected_indices.push(batch_idx);
                            batch_idx += 1;
                        }
                        'n' => {
                            info!("File skipped");
                            batch_idx += 1;
                        }
                        'v' => {
                            // Show more file info and optionally open
                            let metadata = fs::metadata(batch_file)?;
                            info!("File info for {}", batch_file.display());
                            info!("Size: {} bytes", metadata.len());

                            print!("View this file? [y/N]: ");
                            io::stdout().flush()?;
                            input.clear();
                            handle.read_line(&mut input)?;

                            if input.trim().eq_ignore_ascii_case("y") {
                                info!("Opening file with default application...");
                                let _ = Command::new("open").arg(batch_file).spawn()?;

                                print!("Press Enter to continue...");
                                io::stdout().flush()?;
                                input.clear();
                                handle.read_line(&mut input)?;
                            }
                            // Don't increment batch_idx to see this file again
                        }
                        'd' => {
                            // Open directory
                            if let Some(parent) = batch_file.parent() {
                                info!("Opening directory containing file...");
                                let _ = Command::new("open").arg(parent).spawn()?;

                                print!("Press Enter to continue...");
                                io::stdout().flush()?;
                                input.clear();
                                handle.read_line(&mut input)?;
                            }
                            // Don't increment batch_idx to see this file again
                        }
                        'q' => {
                            info!("Exiting batch selection mode");
                            break;
                        }
                        _ => {
                            warn!(
                                "Invalid action '{}'. Please choose [y/n/v/d/q].",
                                select_action
                            );
                            // Don't increment batch_idx to try again
                        }
                    }
                }

                // If files were selected, ask for action to apply to all selected files
                if !selected_indices.is_empty() {
                    info!(
                        "Selected {} files. Choose action to apply to selected files:",
                        selected_indices.len()
                    );
                    info!("[t] Move all selected files to trash");
                    info!("[k] Keep all selected files in backup");
                    print!("Action for selected files [t/k]: ");
                    io::stdout().flush()?;

                    input.clear();
                    handle.read_line(&mut input)?;
                    let batch_action = input.trim().chars().next().unwrap_or('?');

                    match batch_action {
                        't' => {
                            info!("Moving {} selected files to trash", selected_indices.len());

                            // Process in reverse order to avoid index issues if we're removing from files_not_in_immich
                            for &idx in selected_indices.iter().rev() {
                                let batch_file = &files_not_in_immich[idx];
                                let file_name =
                                    batch_file.file_name().unwrap_or_default().to_string_lossy();

                                // Create unique name in trash to avoid collisions
                                let mut destination = trash_dir.join(&*file_name);
                                let original_name = file_name.to_string();

                                let mut counter = 1;
                                while destination.exists() {
                                    let timestamp =
                                        chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
                                    let new_name =
                                        format!("{}-{}-{}", original_name, timestamp, counter);
                                    destination = trash_dir.join(new_name);
                                    counter += 1;
                                }

                                info!("Moving to trash: {}", batch_file.display());
                                match fs::copy(batch_file, &destination) {
                                    Ok(_) => {
                                        match fs::remove_file(batch_file) {
                                            Ok(_) => info!("File successfully moved to trash"),
                                            Err(e) => warn!("File was copied to trash but could not be deleted from backup: {}", e)
                                        }
                                    },
                                    Err(e) => error!("Failed to copy file to trash: {}", e)
                                }
                            }

                            // Update our position in the list to avoid re-processing files
                            if selected_indices.contains(&start_idx) {
                                i += 1;
                            }
                        }
                        'k' => {
                            info!(
                                "Keeping {} selected files in backup",
                                selected_indices.len()
                            );
                            // Skip to the next file after the last one we just processed
                            i = start_idx + 1;
                        }
                        _ => {
                            warn!(
                                "Invalid action '{}'. No action taken on selected files.",
                                batch_action
                            );
                            // Don't increment i so we see the current file again
                        }
                    }
                } else {
                    info!("No files were selected. Continuing with regular processing.");
                    // Don't increment i since we didn't process any files
                }
            }
            'f' => {
                // Apply filter to remaining files
                print!("Filter by (1) Photos only, (2) Videos only, (3) Filename pattern: ");
                io::stdout().flush()?;
                input.clear();
                handle.read_line(&mut input)?;

                let choice = input.trim();
                let mut filtered_files = Vec::new();

                for idx in i..files_not_in_immich.len() {
                    let file = &files_not_in_immich[idx];
                    match choice {
                        "1" => {
                            if let Some(ext) = file.extension() {
                                let ext_str = ext.to_string_lossy().to_lowercase();
                                if [
                                    "jpg", "jpeg", "png", "heic", "dng", "raw", "arw", "cr2", "nef",
                                ]
                                .contains(&ext_str.as_ref())
                                {
                                    filtered_files.push(idx);
                                }
                            }
                        }
                        "2" => {
                            if let Some(ext) = file.extension() {
                                let ext_str = ext.to_string_lossy().to_lowercase();
                                if [
                                    "mp4", "mov", "avi", "m4v", "3gp", "mkv", "webm", "flv", "wmv",
                                    "mts", "m2ts",
                                ]
                                .contains(&ext_str.as_ref())
                                {
                                    filtered_files.push(idx);
                                }
                            }
                        }
                        "3" => {
                            print!("Enter filename pattern to match: ");
                            io::stdout().flush()?;

                            // Use a separate string for pattern to avoid borrow conflicts
                            let mut pattern_input = String::new();
                            handle.read_line(&mut pattern_input)?;
                            let pattern = pattern_input.trim().to_lowercase();

                            if file
                                .file_name()
                                .and_then(|n| n.to_str())
                                .map(|name| name.to_lowercase().contains(&pattern))
                                .unwrap_or(false)
                            {
                                filtered_files.push(idx);
                            }
                        }
                        _ => {
                            warn!("Invalid choice. Filter not applied.");
                        }
                    }
                }

                if !filtered_files.is_empty() {
                    info!(
                        "Found {} files matching filter criteria",
                        filtered_files.len()
                    );
                    print!("Apply action to all filtered files? [t/k/n]: ");
                    io::stdout().flush()?;

                    // Create a new String to avoid borrowing issues
                    let mut action_input = String::new();
                    handle.read_line(&mut action_input)?;
                    let filter_action = action_input.trim().chars().next().unwrap_or('?');

                    match filter_action {
                        't' => {
                            info!("Moving {} filtered files to trash", filtered_files.len());
                            for &idx in filtered_files.iter().rev() {
                                let file = &files_not_in_immich[idx];
                                let file_name =
                                    file.file_name().unwrap_or_default().to_string_lossy();
                                let mut destination = trash_dir.join(&*file_name);

                                let original_name = file_name.to_string();
                                let mut counter = 1;
                                while destination.exists() {
                                    let timestamp =
                                        chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
                                    let new_name =
                                        format!("{}-{}-{}", original_name, timestamp, counter);
                                    destination = trash_dir.join(new_name);
                                    counter += 1;
                                }

                                info!("Moving to trash: {}", file.display());
                                match fs::copy(file, &destination) {
                                    Ok(_) => {
                                        match fs::remove_file(file) {
                                            Ok(_) => info!("File successfully moved to trash"),
                                            Err(e) => warn!("File was copied to trash but could not be deleted from backup: {}", e)
                                        }
                                    },
                                    Err(e) => error!("Failed to copy file to trash: {}", e)
                                }
                            }

                            // Update our position in the list and skip processed files
                            if i == filtered_files[0] {
                                i += 1;
                            }
                        }
                        'k' => {
                            info!("Keeping {} filtered files in backup", filtered_files.len());
                            i = *filtered_files.iter().max().unwrap_or(&i) + 1;
                        }
                        _ => {
                            info!("No bulk action taken. Continuing with standard processing.");
                        }
                    }
                } else {
                    info!("No files matched the filter criteria");
                }
            }
            'q' => {
                // Quit sync process
                info!(
                    "Sync process cancelled. Processed {} of {} files.",
                    i,
                    files_not_in_immich.len()
                );
                return Ok(());
            }
            'a' => {
                // Apply an action to all remaining files
                print!("Apply which action to all remaining files? [t/k]: ");
                io::stdout().flush()?;

                input.clear();
                handle.read_line(&mut input)?;
                let all_char = input.trim().chars().next().unwrap_or('?');

                if all_char == 't' || all_char == 'k' {
                    all_action = Some(all_char);
                    info!("Applying '{}' to all remaining files.", all_char);
                } else {
                    warn!("Invalid action '{}'. Please choose again.", all_char);
                }

                // Don't increment i so we process this file with the new all_action
            }
            _ => {
                warn!(
                    "Invalid action '{}'. Please choose [t/k/v/d/s/f/q/a].",
                    action
                );
                // Don't increment i so we process this file again
            }
        }
    }

    // Count how many files were processed in different ways
    let mut trash_count = 0;
    let mut kept_count = 0;

    for original_file in files_not_in_immich {
        if !original_file.exists() {
            // File was moved to trash
            trash_count += 1;
        } else {
            // File was kept
            kept_count += 1;
        }
    }

    info!("Sync completed. Summary:");
    info!("  - {} files moved to trash", trash_count);
    info!("  - {} files kept in backup", kept_count);
    info!("  - {} total files processed", trash_count + kept_count);

    Ok(())
}
