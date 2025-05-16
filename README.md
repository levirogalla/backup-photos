# Backup Photos

A CLI tool for backing up photos and videos from Apple Photos to external storage and managing them with Immich.

## Prerequisites

- Rust (1.53 or later)
- rsync
- immich
- immich-go
- exiftool

## Setup

1. Clone this repository
2. Create a `.env` file in the root directory with the following variables:
```
APPLE_PHOTOS_EXPORT_DIR='/path/to/your/photos/export'
RAW_PHOTOS_BACKUP_DIR='/path/to/your/backup/directory'
IMMICH_LIB='/path/to/your/immich/library'
```

3. Build the project:
```bash
cargo build --release
```

## Usage

The CLI provides several commands to manage your photo backup workflow.

### Initialize Directories

Create all required directories specified in the .env file:

```bash
backup-photos init
```

This will create the export, backup, and Immich library directories if they don't exist.

### Check Paths

Verify that all environment variable paths are correctly set and accessible:

```bash
backup-photos check-paths
```

### Backup Photos and Videos

Copy photos and videos from the export directory to the backup directory:

```bash
backup-photos backup
```

### Import Media to Immich

Import photos and videos from the export directory to Immich:

```bash
backup-photos import
```

### Compare Backup to Immich

Compare the files between the backup directory and the Immich library:

```bash
backup-photos compare
```

### Sync Backup with Immich

Interactively handle files that are in backup but missing from Immich:

```bash
backup-photos sync
```

This command allows you to:
- View file information and metadata
- View files with their default applications
- Open directories containing the files
- Move files to trash if they're no longer needed
- Keep files in backup if they should be preserved
- Select multiple files for batch processing
- Filter files by type (photos only, videos only)
- Filter files by filename pattern
- Apply actions to all remaining or filtered files

The sync command provides an interactive interface with these options:
- `[t]` Move to trash (safely copy to ~/.Trash and delete original)
- `[k]` Keep in backup (skip this file)
- `[v]` View file info and optionally open the file
- `[d]` Open directory containing the file
- `[s]` Select multiple files for batch processing
- `[f]` Apply filter to remaining files
- `[q]` Quit sync process
- `[a]` Process all remaining files with the same action

Advanced features:
- Batch selection mode allows you to quickly mark multiple files and process them together
- Filtering options let you narrow down the files by photos, videos, or custom patterns
- File collision detection ensures files with the same name don't overwrite each other in trash
- Full error handling for file operations ensures data safety

### Clear Export Directory

Clear the export directory (shows a summary without deleting):

```bash
backup-photos clear
```

Force delete all photos and videos in the export directory:

```bash
backup-photos clear --force
```

### Full Workflow

Run the entire backup workflow (backup → import → compare):

```bash
backup-photos full
```

## Safety Features

- Checks that all directories exist and are accessible before performing operations
- Verifies that external drives are connected if paths are symlinks to mounted volumes
- Requires explicit confirmation before deleting files
- Provides detailed logs of all operations

## Debug Mode

Run any command with the `--debug` flag to see more detailed logging:

```bash
backup-photos --debug backup
```

## Workflow

1. Export photos and videos from Apple Photos to the configured export directory
2. Run `backup-photos full` to:
   - Backup photos and videos to the backup directory
   - Import media to Immich
   - Compare files to ensure everything was properly imported
3. Run `backup-photos clear --force` to clear the export directory after verifying the backup

## Notes

- The Immich CLI import functionality needs to be customized based on your specific setup
- Be extra careful when using the `clear --force` command as it will delete files without further prompts