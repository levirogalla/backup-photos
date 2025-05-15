# Backup Photos

A CLI tool for backing up photos from Apple Photos to external storage and managing them with Immich.

## Prerequisites

- Rust (1.53 or later)
- rsync
- Immich CLI (for importing to Immich)

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

### Check Paths

Verify that all environment variable paths are correctly set and accessible:

```bash
backup-photos check-paths
```

### Backup Photos

Copy photos from the export directory to the backup directory:

```bash
backup-photos backup
```

### Import Photos to Immich

Import photos from the export directory to Immich:

```bash
backup-photos import
```

### Compare Backup to Immich

Compare the files between the backup directory and the Immich library:

```bash
backup-photos compare
```

### Clear Export Directory

Clear the export directory (shows a summary without deleting):

```bash
backup-photos clear
```

Force delete all photos in the export directory:

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

1. Export photos from Apple Photos to the configured export directory
2. Run `backup-photos full` to:
   - Backup photos to the backup directory
   - Import photos to Immich
   - Compare files to ensure everything was properly imported
3. Run `backup-photos clear --force` to clear the export directory after verifying the backup

## Notes

- The Immich CLI import functionality needs to be customized based on your specific setup
- Be extra careful when using the `clear --force` command as it will delete files without further prompts