use anyhow::Result;
use backup_photos::*;
use clap::{Parser, Subcommand};
use dotenv::dotenv;
use env_logger::Env;
use log::{error, info};
use std::io::{self, Write};
use std::path::PathBuf;
use constants;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Turn on debug logging
    #[arg(short, long)]
    debug: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize required directories based on environment variables
    /// Creates export, backup, and Immich directories if they don't exist
    Init,
    
    /// Backup photos and videos from Apple Photos export directory to backup directory
    /// using rsync to avoid duplicates and preserve metadata
    Backup,
    
    /// Import photos and videos from export directory to Immich
    /// using the Immich CLI
    Import,
    
    /// Clear the export directory after successful backup
    /// Without --force, this only shows what would be deleted
    Clear {
        /// Force deletion without additional prompts
        #[arg(short, long)]
        force: bool,
    },
    
    /// Compare media files between backup directory and Immich library
    /// Reports any discrepancies found
    Compare,
    
    /// Sync backup with Immich by interactively handling discrepancies
    /// Provides options to view, filter, batch select, and process files
    /// that are in backup but missing from Immich
    Sync,
    
    /// Run the full backup workflow (backup -> import -> compare)
    /// in a single command
    Full,
    
    /// Check environment variable paths for existence and accessibility
    /// Verifies that external drives are connected if paths point to them
    CheckPaths,

    /// Repair Apple XMP export files in export directory
    RepairXMP,

    /// Start the docker server for immich
    StartServer
    
}

fn main() -> Result<()> {
    // Load environment variables from .env file
    dotenv().ok();
    
    // Parse command line arguments
    let cli = Cli::parse();
    
    // Setup logging
    let env = if cli.debug {
        Env::default().default_filter_or("debug")
    } else {
        Env::default().default_filter_or("info")
    };
    
    env_logger::Builder::from_env(env)
        .format(|buf, record| {
            writeln!(
                buf,
                "[{}] {} - {}",
                record.level(),
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.args()
            )
        })
        .init();
    
    // Execute the appropriate command
    match &cli.command {
        Commands::Init => {
            info!("Initializing required directories");
            match init_directories() {
                Ok(_) => info!("Directories initialized successfully"),
                Err(e) => {
                    error!("Failed to initialize directories: {}", e);
                    return Err(e.into());
                }
            }
        },
        
        Commands::Backup => {
            info!("Running backup command");
            match backup_photos_to_raw_dir() {
                Ok(_) => info!("Backup completed successfully"),
                Err(e) => {
                    error!("Backup failed: {}", e);
                    return Err(e.into());
                }
            }
        }
        
        Commands::Import => {
            info!("Running import command");
            match import_to_immich() {
                Ok(_) => info!("Import completed successfully"),
                Err(e) => {
                    error!("Import failed: {}", e);
                    return Err(e.into());
                }
            }
        }
        
        Commands::Clear { force } => {
            info!("Running clear command");
            if *force {
                match clear_export_directory_force() {
                    Ok(_) => info!("Export directory cleared successfully"),
                    Err(e) => {
                        error!("Failed to clear export directory: {}", e);
                        return Err(e.into());
                    }
                }
            } else {
                match clear_export_directory() {
                    Ok(_) => info!("Please run with --force to confirm deletion"),
                    Err(e) => {
                        error!("Failed to analyze export directory: {}", e);
                        return Err(e.into());
                    }
                }
            }
        }
        
        Commands::Compare => {
            info!("Running compare command");
            match compare_backup_to_immich() {
                Ok(_) => info!("Comparison completed successfully"),
                Err(e) => {
                    error!("Comparison failed: {}", e);
                    return Err(e.into());
                }
            }
        }
        
        Commands::Sync => {
            info!("Running sync command");
            match sync_backup_with_immich() {
                Ok(_) => info!("Sync completed successfully"),
                Err(e) => {
                    error!("Sync failed: {}", e);
                    return Err(e.into());
                }
            }
        }
        
        Commands::Full => {
            info!("Running full backup workflow");
            match full_backup_workflow() {
                Ok(_) => info!("Full backup workflow completed successfully"),
                Err(e) => {
                    error!("Full backup workflow failed: {}", e);
                    return Err(e.into());
                }
            }
        }
        
        Commands::CheckPaths => {
            info!("Checking environment variable paths");
            let paths = [
                (constants::APPLE_PHOTOS_EXPORT_DIR, "Photos export directory"),
                (constants::RAW_PHOTOS_BACKUP_DIR, "Raw photos backup directory"),
                (constants::IMMICH_LIB, "Immich library directory"),
            ];
            
            for (var, desc) in paths.iter() {
                let path = var;
                        let path_buf = PathBuf::from(path);
                        print!("{}: {} - ", desc, path_buf.display());
                        io::stdout().flush()?;
                        
                        match check_directory_exists_and_accessible(&path_buf) {
                            Ok(_) => {
                                print!("✓ exists and is accessible");
                                io::stdout().flush()?;
                                
                                match check_external_drive_connected(&path_buf) {
                                    Ok(_) => println!(" - ✓ drive connected"),
                                    Err(e) => println!(" - ❌ drive not connected: {}", e),
                                }
                            }
                            Err(e) => println!("❌ {}", e),
                        }
                    }
        }
    

        Commands::RepairXMP => {
            info!("Running repair command");
            match fix_apple_xmp_files(&PathBuf::from(constants::APPLE_PHOTOS_EXPORT_DIR)) {
                Ok(_) => info!("Repair completed successfully"),
                Err(e) => {
                    error!("Repair failed: {}", e);
                    return Err(e.into());
                }
            }
        }

        Commands::StartServer => {
            info!("Starting Immich server");
            match start_immich_server() {
                Ok(_) => info!("Immich server started successfully"),
                Err(e) => {
                    error!("Failed to start Immich server: {}", e);
                    return Err(e.into());
                }
            }
        }
    }
    
    Ok(())
}
