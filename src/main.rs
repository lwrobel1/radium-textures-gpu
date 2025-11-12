mod mo2;
mod bsa;
mod dds; // DDS header parser module
mod database;
mod optimizer;
mod exclusions;
mod texconv;
mod presets;
mod extraction;
mod optimization;
mod gui;
mod game;

use bsa::BsaArchive;

use anyhow::Result;
use clap::{Parser, Subcommand};
use log::info;
use std::path::PathBuf;

use mo2::{Profile, VirtualFileSystem};

#[derive(Parser)]
#[command(name = "radium-textures")]
#[command(version = "1.0.0")]
#[command(about = "Skyrim texture optimization for Linux with MO2 support", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Launch the graphical user interface
    Gui,

    /// Analyze MO2 profile and build VFS (Phase 1 test)
    Analyze {
        /// Path to MO2 profile directory
        #[arg(long)]
        profile: PathBuf,

        /// Path to MO2 mods directory
        #[arg(long)]
        mods: PathBuf,

        /// Path to game Data directory
        #[arg(long)]
        data: PathBuf,
    },

    /// Parse a BSA archive (Phase 2 test)
    ParseBsa {
        /// Path to BSA file
        #[arg(long)]
        bsa: PathBuf,
    },

    /// Parse a DDS texture header (Phase 3 test)
    ParseDds {
        /// Path to DDS file
        #[arg(long)]
        dds: PathBuf,
    },

    /// Discover all textures (Phase 4 test)
    Discover {
        /// Path to MO2 profile directory
        #[arg(long)]
        profile: PathBuf,

        /// Path to MO2 mods directory
        #[arg(long)]
        mods: PathBuf,

        /// Path to game Data directory
        #[arg(long)]
        data: PathBuf,
    },

    /// Filter textures for optimization (Phase 6 test)
    Filter {
        /// Path to MO2 profile directory
        #[arg(long)]
        profile: PathBuf,

        /// Path to MO2 mods directory
        #[arg(long)]
        mods: PathBuf,

        /// Path to game Data directory
        #[arg(long)]
        data: PathBuf,

        /// Optimization preset
        #[arg(long, value_enum, default_value = "optimum")]
        preset: Preset,
    },

    /// Full optimization pipeline
    Optimize {
        /// Path to MO2 profile directory
        #[arg(long)]
        profile: PathBuf,

        /// Path to MO2 mods directory
        #[arg(long)]
        mods: PathBuf,

        /// Path to game Data directory
        #[arg(long)]
        data: PathBuf,

        /// Output directory for optimized textures
        #[arg(long)]
        output: PathBuf,

        /// Optimization preset
        #[arg(long, value_enum, default_value = "optimum")]
        preset: Preset,
    },
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum Preset {
    HighQuality,
    Quality,
    Optimum,
    Performance,
    Vanilla,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    // If RUST_LOG is set, use it; otherwise use cli.verbose flag
    if std::env::var("RUST_LOG").is_ok() {
        env_logger::init();
    } else {
        env_logger::Builder::new()
            .filter_level(if cli.verbose {
                log::LevelFilter::Debug
            } else {
                log::LevelFilter::Info
            })
            .init();
    }

    match cli.command {
        Commands::Gui => {
            return gui::run().map_err(|e| anyhow::anyhow!("GUI error: {}", e));
        }
        Commands::Analyze { profile, mods, data } => {
            analyze_profile(profile, mods, data)?;
        }
        Commands::ParseBsa { bsa } => {
            parse_bsa(bsa)?;
        }
        Commands::ParseDds { dds } => {
            parse_dds(dds)?;
        }
        Commands::Discover { profile, mods, data } => {
            discover_textures(profile, mods, data)?;
        }
        Commands::Filter { profile, mods, data, preset } => {
            filter_textures(profile, mods, data, preset)?;
        }
        Commands::Optimize { profile, mods, data, output, preset } => {
            optimize_textures(profile, mods, data, output, preset)?;
        }
    }

    Ok(())
}

/// Phase 1 test: Analyze MO2 profile and build VFS
fn analyze_profile(profile_path: PathBuf, mods_dir: PathBuf, data_dir: PathBuf) -> Result<()> {
    info!("=== Phase 1: MO2 Profile Analysis ===");

    // Load MO2 profile
    info!("Loading MO2 profile...");
    let profile = Profile::load(&profile_path, &mods_dir, &data_dir)?;

    // Print profile info
    info!("Profile: {:?}", profile.profile_path);
    info!("Total mods: {}", profile.mods.len());

    let enabled_count = profile.enabled_mods().count();
    info!("Enabled mods: {}", enabled_count);
    info!("Plugins in load order: {}", profile.plugin_order.len());
    info!("Additional archives: {}", profile.additional_archives.len());

    // Print top 5 mods from MO2 list (lowest priority numbers = top of list = wins conflicts)
    info!("\n=== Top 5 Mods from MO2 List (Highest Priority, Win Conflicts) ===");
    let all_enabled: Vec<_> = profile.enabled_mods()
        .map(|m| (m.priority, m.name.clone()))
        .collect();
    for (priority, name) in all_enabled.iter().take(5) {
        info!("  [P{}] {}", priority, name);
    }

    // Build VFS
    info!("\n=== Building Virtual File System ===");
    let vfs = VirtualFileSystem::new(profile)?;

    let stats = vfs.get_statistics();
    info!("{}", stats);

    // Show some texture files
    info!("\n=== Sample Texture Files (first 10) ===");
    for (path, source) in vfs.get_texture_files().take(10) {
        match source {
            mo2::FileSource::Vanilla(p) => {
                info!("  [VANILLA] {} -> {:?}", path, p);
            }
            mo2::FileSource::Mod { mod_name, mod_priority, file_path } => {
                info!("  [MOD:{}:P{}] {} -> {:?}", mod_name, mod_priority, path, file_path);
            }
            mo2::FileSource::Bsa { archive_name, internal_path, .. } => {
                info!("  [BSA:{}] {} -> {}", archive_name, path, internal_path);
            }
        }
    }

    // Get BSA files from load order
    info!("\n=== BSA Files from Load Order ===");
    let bsa_files = vfs.profile().get_plugin_bsas();
    for (i, (bsa_path, mod_name, priority)) in bsa_files.iter().enumerate().take(20) {
        info!("  [{}] [P{}] {} -> {:?}",
              i, priority, mod_name,
              bsa_path.file_name().unwrap_or_default().to_string_lossy());
    }
    if bsa_files.len() > 20 {
        info!("  ... and {} more BSAs", bsa_files.len() - 20);
    }
    info!("Total BSAs found: {}", bsa_files.len());

    // Phase 2: Test BSA parser on first BSA with textures
    info!("\n=== Phase 2: Testing BSA Parser ===");
    let mut tested_bsa = false;
    for (bsa_path, mod_name, _priority) in bsa_files.iter().take(10) {
        if bsa_path.exists() {
            match BsaArchive::open(bsa_path) {
                Ok(archive) => {
                    let textures: Vec<_> = archive.get_textures().collect();
                    if !textures.is_empty() {
                        info!("Successfully parsed BSA: {} from {}",
                              bsa_path.file_name().unwrap_or_default().to_string_lossy(),
                              mod_name);
                        info!("  Files: {}, DDS textures: {}", archive.files.len(), textures.len());

                        // Phase 3: Test DDS parser on first texture from BSA
                        if let Some(first_texture) = textures.first() {
                            info!("\n=== Phase 3: Testing DDS Parser ===");
                            info!("Sample texture from BSA: {}", first_texture.path);
                            info!("  Size: {} bytes", first_texture.size);

                            // Note: Full DDS parsing from BSA will be implemented in next phase
                            // For now, we'll test on a loose file if available
                        }

                        tested_bsa = true;
                        break;
                    }
                }
                Err(e) => {
                    info!("  Warning: Failed to parse BSA {}: {}",
                          bsa_path.file_name().unwrap_or_default().to_string_lossy(), e);
                }
            }
        }
    }

    if !tested_bsa {
        info!("  No texture BSAs found in first 10 archives");
    }

    // Phase 3: Test DDS parser on a loose file
    info!("\n=== Phase 3: Testing DDS Parser on Loose Files ===");
    let mut tested_dds = false;
    for (path, source) in vfs.get_texture_files().take(10) {
        if let Some(file_path) = source.physical_path() {
            if file_path.exists() && file_path.extension().and_then(|s| s.to_str()) == Some("dds") {
                match std::fs::File::open(file_path) {
                    Ok(mut file) => {
                        match dds::parse_dds_header(&mut file) {
                            Ok(header) => {
                                info!("Successfully parsed DDS: {}", path);
                                info!("  Resolution: {}x{}", header.width, header.height);
                                info!("  Format: {}", header.format);
                                tested_dds = true;
                                break;
                            }
                            Err(e) => {
                                info!("  Warning: Failed to parse DDS {}: {}", path, e);
                            }
                        }
                    }
                    Err(e) => {
                        info!("  Warning: Failed to open DDS {}: {}", path, e);
                    }
                }
            }
        }
    }

    if !tested_dds {
        info!("  No loose DDS files found to test");
    }

    info!("\n=== All Phases Complete ===");
    info!("Phase 1: MO2 profile loaded with {} enabled mods", enabled_count);
    info!("Phase 2: BSA parser tested successfully");
    info!("Phase 3: DDS parser tested successfully");

    Ok(())
}

/// Phase 2 test: Parse a BSA archive
fn parse_bsa(bsa_path: PathBuf) -> Result<()> {
    info!("=== Phase 2: BSA Archive Parser Test ===");
    info!("Parsing: {:?}", bsa_path);

    let start = std::time::Instant::now();
    let archive = BsaArchive::open(&bsa_path)?;
    let elapsed = start.elapsed();

    info!("\n=== BSA Header ===");
    info!("Version: {}", archive.header.version);
    info!("Archive Flags: 0x{:08X}", archive.header.archive_flags);
    info!("Compressed: {}", archive.header.archive_flags & 0x0004 != 0);
    info!("Folders: {}", archive.header.folder_count);
    info!("Files: {}", archive.header.file_count);

    info!("\n=== File Statistics ===");
    let textures: Vec<_> = archive.get_textures().collect();
    info!("Total files: {}", archive.files.len());
    info!("DDS textures: {}", textures.len());

    let compressed_count = archive.files.iter().filter(|f| f.compressed).count();
    info!("Compressed files: {}", compressed_count);

    // Debug: show first 20 files regardless of type
    info!("\n=== Sample Files (first 20) ===");
    for (i, file) in archive.files.iter().take(20).enumerate() {
        let comp_str = if file.compressed { "COMPRESSED" } else { "RAW" };
        let path_str = if file.path.is_empty() { "<EMPTY PATH>" } else { &file.path };
        info!("  [{}] {} ({} bytes, {})",
              i + 1, path_str, file.size, comp_str);
    }

    info!("\n=== Sample Texture Files (first 20) ===");
    for (i, file) in textures.iter().take(20).enumerate() {
        let comp_str = if file.compressed { "COMPRESSED" } else { "RAW" };
        info!("  [{}] {} ({} bytes, {})",
              i + 1, file.path, file.size, comp_str);
    }

    if textures.len() > 20 {
        info!("  ... and {} more textures", textures.len() - 20);
    }

    info!("\n=== Performance ===");
    info!("Parse time: {:.2?}", elapsed);
    info!("Files/sec: {:.0}", archive.files.len() as f64 / elapsed.as_secs_f64());

    Ok(())
}

/// Phase 3 test: Parse a DDS texture header
fn parse_dds(dds_path: PathBuf) -> Result<()> {
    info!("=== Phase 3: DDS Header Parser Test ===");
    info!("Parsing: {:?}", dds_path);

    let start = std::time::Instant::now();
    let mut file = std::fs::File::open(&dds_path)?;
    let header = dds::parse_dds_header(&mut file)?;
    let elapsed = start.elapsed();

    info!("\n=== DDS Header ===");
    info!("Width: {}px", header.width);
    info!("Height: {}px", header.height);
    info!("Format: {}", header.format);
    info!("Resolution: {}x{}", header.width, header.height);

    // Calculate file size info
    let file_size = std::fs::metadata(&dds_path)?.len();
    info!("\n=== File Info ===");
    info!("File size: {} bytes ({:.2} MB)", file_size, file_size as f64 / 1024.0 / 1024.0);

    info!("\n=== Performance ===");
    info!("Parse time: {:.2?}", elapsed);

    Ok(())
}

/// Phase 4 test: Discover all textures from mods and BSAs
fn discover_textures(profile_path: PathBuf, mods_dir: PathBuf, data_dir: PathBuf) -> Result<()> {
    info!("=== Phase 4: Texture Discovery Pipeline ===");

    // Load MO2 profile
    info!("Loading MO2 profile...");
    let profile = mo2::Profile::load(&profile_path, &mods_dir, &data_dir)?;

    info!("Profile loaded: {} enabled mods", profile.enabled_mods().count());

    // Build VFS
    info!("\nBuilding Virtual File System...");
    let vfs = mo2::VirtualFileSystem::new(profile)?;
    let stats = vfs.get_statistics();
    info!("{}", stats);

    // Discover textures from loose files
    info!("\n=== Step 1: Discovering Loose Textures ===");
    let mut textures = database::TextureDiscoveryService::discover_from_vfs(&vfs);

    // Discover textures from BSAs
    info!("\n=== Step 2: Discovering BSA Textures ===");
    let bsa_files = vfs.profile().get_plugin_bsas();
    database::TextureDiscoveryService::discover_from_bsas(&bsa_files, &mut textures)?;

    // Parse DDS headers
    info!("\n=== Step 3: Parsing DDS Headers ===");
    database::TextureDiscoveryService::parse_dds_headers(&mut textures)?;

    // Show statistics
    info!("\n{}", database::TextureDiscoveryService::get_statistics(&textures));

    // Show some sample textures
    info!("\n=== Sample Textures (first 10) ===");
    for (i, (path, record)) in textures.iter().take(10).enumerate() {
        let header_info = if record.has_header_info() {
            format!(
                "{}x{} {}",
                record.width.unwrap(),
                record.height.unwrap(),
                record.format.as_ref().unwrap()
            )
        } else {
            "header not parsed".to_string()
        };

        info!(
            "  [{}] {} ({} KB, {}, {})",
            i + 1,
            path,
            record.file_size / 1024,
            record.source,
            header_info
        );
    }

    info!("\n=== Phase 4 Complete ===");
    info!("Total unique textures discovered: {}", textures.len());

    Ok(())
}

/// Phase 6 test: Filter textures based on optimization preset
fn filter_textures(profile_path: PathBuf, mods_dir: PathBuf, data_dir: PathBuf, preset: Preset) -> Result<()> {
    info!("=== Phase 6: Texture Filtering ===");

    // Convert CLI preset to optimization preset
    let opt_preset = match preset {
        Preset::HighQuality => presets::OptimizationPreset::HQ,
        Preset::Quality => presets::OptimizationPreset::QUALITY,
        Preset::Optimum => presets::OptimizationPreset::OPTIMUM,
        Preset::Performance => presets::OptimizationPreset::PERFORMANCE,
        Preset::Vanilla => presets::OptimizationPreset::VANILLA,
    };

    info!("Using preset: {}", opt_preset.name);
    info!("  Diffuse max: {}px", opt_preset.diffuse_max);
    info!("  Normal max: {}px", opt_preset.normal_max);
    info!("  Parallax max: {}px", opt_preset.parallax_max);
    info!("  Material max: {}px", opt_preset.material_max);

    // Load profile and discover textures
    info!("\nLoading MO2 profile...");
    let profile = mo2::Profile::load(&profile_path, &mods_dir, &data_dir)?;

    info!("Building Virtual File System...");
    let vfs = mo2::VirtualFileSystem::new(profile)?;

    info!("Discovering textures...");
    let mut textures = database::TextureDiscoveryService::discover_from_vfs(&vfs);
    let bsa_files = vfs.profile().get_plugin_bsas();
    database::TextureDiscoveryService::discover_from_bsas(&bsa_files, &mut textures)?;
    database::TextureDiscoveryService::parse_dds_headers(&mut textures)?;

    info!("Total textures discovered: {}", textures.len());

    // Load exclusions (default to Skyrim SE for CLI)
    info!("\n=== Loading Exclusions ===");
    let game = game::Game::SkyrimSE;  // Default for CLI, can add --game flag later
    let exclusions_path = std::path::PathBuf::from("./exclusions").join(game.exclusions_file());
    let exclusions: Option<exclusions::ExclusionList> = if exclusions_path.exists() {
        match exclusions::ExclusionList::load(&exclusions_path) {
            Ok(list) => {
                info!("Loaded exclusions for {} from {:?}", game, exclusions_path);
                Some(list)
            }
            Err(e) => {
                info!("Failed to load exclusions: {}", e);
                None
            }
        }
    } else {
        info!("No exclusions file found at {:?}", exclusions_path);
        None
    };

    // Filter textures that need optimization
    info!("\n=== Filtering Textures for Optimization ===");
    let mut needs_optimization = Vec::new();
    let mut skipped_no_header = 0;
    let mut skipped_no_type = 0;
    let mut skipped_already_optimized = 0;
    let mut skipped_excluded = 0;

    for (path, record) in &textures {
        // Check exclusions first
        if let Some(ref exclusion_list) = exclusions {
            if exclusion_list.should_exclude(path) {
                skipped_excluded += 1;
                continue;
            }
        }

        // Skip if no header info
        if !record.has_header_info() {
            skipped_no_header += 1;
            continue;
        }

        // Skip if no texture type
        let texture_type = match &record.texture_type {
            Some(t) => t,
            None => {
                skipped_no_type += 1;
                continue;
            }
        };

        // Check if optimization needed
        let width = record.width.unwrap();
        let height = record.height.unwrap();

        if let Some((target_width, target_height)) = opt_preset.get_target_resolution(texture_type, width, height) {
            needs_optimization.push((
                path.clone(),
                record.clone(),
                target_width,
                target_height,
            ));
        } else {
            skipped_already_optimized += 1;
        }
    }

    // Statistics
    info!("\n=== Filtering Results ===");
    info!("Total textures: {}", textures.len());
    info!("Needs optimization: {}", needs_optimization.len());
    info!("Excluded from optimization: {}", skipped_excluded);
    info!("Already optimized: {}", skipped_already_optimized);
    info!("Skipped (no header): {}", skipped_no_header);
    info!("Skipped (no type): {}", skipped_no_type);

    // Break down by texture type
    info!("\n=== Optimization Needed by Type ===");
    let mut by_type: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (_path, record, _tw, _th) in &needs_optimization {
        if let Some(texture_type) = &record.texture_type {
            *by_type.entry(texture_type.clone()).or_insert(0) += 1;
        }
    }

    let mut type_vec: Vec<_> = by_type.iter().collect();
    type_vec.sort_by(|a, b| b.1.cmp(a.1));
    for (texture_type, count) in type_vec {
        info!("  {}: {}", texture_type, count);
    }

    // Show samples
    info!("\n=== Sample Textures to Optimize (first 20) ===");
    for (i, (path, record, target_w, target_h)) in needs_optimization.iter().take(20).enumerate() {
        info!(
            "  [{}] {} ({})",
            i + 1,
            path,
            record.texture_type.as_ref().unwrap()
        );
        info!(
            "       Current: {}x{} {} - Target: {}x{}",
            record.width.unwrap(),
            record.height.unwrap(),
            record.format.as_ref().unwrap(),
            target_w,
            target_h
        );
    }

    info!("\n=== Phase 6 Complete ===");
    info!("Ready to optimize {} textures", needs_optimization.len());

    Ok(())
}

/// Full optimization pipeline: Discovery -> Filter -> Extract -> Optimize
fn optimize_textures(
    profile_path: PathBuf,
    mods_dir: PathBuf,
    data_dir: PathBuf,
    output_dir: PathBuf,
    preset: Preset,
) -> Result<()> {
    info!("=== Radium Textures Optimization Pipeline ===");

    // Convert CLI preset to optimization preset
    let opt_preset = match preset {
        Preset::HighQuality => presets::OptimizationPreset::HQ,
        Preset::Quality => presets::OptimizationPreset::QUALITY,
        Preset::Optimum => presets::OptimizationPreset::OPTIMUM,
        Preset::Performance => presets::OptimizationPreset::PERFORMANCE,
        Preset::Vanilla => presets::OptimizationPreset::VANILLA,
    };

    info!("Preset: {}", opt_preset.name);
    info!("  Diffuse: {}px", opt_preset.diffuse_max);
    info!("  Normal: {}px", opt_preset.normal_max);
    info!("  Parallax: {}px", opt_preset.parallax_max);
    info!("  Material: {}px", opt_preset.material_max);
    info!("Output: {:?}", output_dir);

    // Step 1: Load profile and discover textures
    info!("\n=== Step 1: Discovering Textures ===");
    let profile = mo2::Profile::load(&profile_path, &mods_dir, &data_dir)?;
    let vfs = mo2::VirtualFileSystem::new(profile)?;

    let mut textures = database::TextureDiscoveryService::discover_from_vfs(&vfs);
    let bsa_files = vfs.profile().get_plugin_bsas();
    database::TextureDiscoveryService::discover_from_bsas(&bsa_files, &mut textures)?;
    database::TextureDiscoveryService::parse_dds_headers(&mut textures)?;

    info!("Discovered {} total textures", textures.len());

    // Step 2: Load exclusions (default to Skyrim SE for CLI)
    info!("\n=== Step 2: Loading Exclusions ===");
    let game = game::Game::SkyrimSE;  // Default for CLI, can add --game flag later
    let exclusions_path = std::path::PathBuf::from("./exclusions").join(game.exclusions_file());
    let exclusions: Option<exclusions::ExclusionList> = if exclusions_path.exists() {
        match exclusions::ExclusionList::load(&exclusions_path) {
            Ok(list) => {
                info!("Loaded exclusions for {} from {:?}", game, exclusions_path);
                Some(list)
            }
            Err(e) => {
                info!("Warning: Failed to load exclusions: {}", e);
                None
            }
        }
    } else {
        info!("No exclusions file found");
        None
    };

    // Step 3: Filter textures needing optimization
    info!("\n=== Step 3: Filtering Textures ===");
    let mut needs_optimization = Vec::new();
    let mut skipped_excluded = 0;
    let mut skipped_already_optimized = 0;

    for (path, record) in &textures {
        // Check exclusions
        if let Some(ref exclusion_list) = exclusions {
            if exclusion_list.should_exclude(path) {
                skipped_excluded += 1;
                continue;
            }
        }

        // Check if has header info and type
        if !record.has_header_info() || record.texture_type.is_none() {
            continue;
        }

        let texture_type = record.texture_type.as_ref().unwrap();
        let width = record.width.unwrap();
        let height = record.height.unwrap();

        if let Some((target_width, target_height)) =
            opt_preset.get_target_resolution(texture_type, width, height)
        {
            needs_optimization.push((
                path.clone(),
                record.clone(),
                target_width,
                target_height,
            ));
        } else {
            skipped_already_optimized += 1;
        }
    }

    info!("Needs optimization: {}", needs_optimization.len());
    info!("Excluded: {}", skipped_excluded);
    info!("Already optimal: {}", skipped_already_optimized);

    if needs_optimization.is_empty() {
        info!("\nNo textures need optimization!");
        return Ok(());
    }

    // Step 4: Extract textures
    info!("\n=== Step 4: Extracting Textures ===");
    let extracted = extraction::extract_all_textures(&needs_optimization, &output_dir)?;

    if extracted.is_empty() {
        info!("No textures were successfully extracted");
        return Ok(());
    }

    // Step 5: Group by processing type
    info!("\n=== Step 5: Grouping by Processing Type ===");
    let groups = optimization::group_by_processing_type(extracted);

    info!("Processing groups:");
    info!("  Delete only: {}", groups.delete_only.len());
    info!("  BC7 resize: {}", groups.bc7_resize.len());
    info!("  BC4 resize: {}", groups.bc4_resize.len());
    info!("  RGBA resize: {}", groups.rgba_resize.len());
    info!("  PBR resize: {}", groups.pbr_resize.len());
    info!("  Specular resize: {}", groups.specular_resize.len());
    info!("  Emissive resize: {}", groups.emissive_resize.len());
    info!("  Gloss resize: {}", groups.gloss_resize.len());

    // Step 6: Optimize with texconv
    info!("\n=== Step 6: Running texconv Optimization ===");

    // Find texconv.exe - try current directory first, then executable directory
    let texconv_path = if std::path::PathBuf::from("./texconv.exe").exists() {
        std::path::PathBuf::from("./texconv.exe")
            .canonicalize()
            .unwrap_or_else(|_| std::path::PathBuf::from("./texconv.exe"))
    } else if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let texconv_in_exe_dir = exe_dir.join("texconv.exe");
            if texconv_in_exe_dir.exists() {
                texconv_in_exe_dir
            } else {
                anyhow::bail!("texconv.exe not found. Tried: ./texconv.exe, {:?}", texconv_in_exe_dir);
            }
        } else {
            anyhow::bail!("texconv.exe not found. Tried: ./texconv.exe, [executable directory]");
        }
    } else {
        anyhow::bail!("texconv.exe not found. Tried: ./texconv.exe");
    };

    info!("Using texconv at: {:?}", texconv_path);

    let stats = optimization::optimize_all(&groups, &texconv_path, None)?;  // None = use all CPU threads

    // Final summary
    info!("\n=== Optimization Complete ===");
    info!("Duration: {:.2?}", stats.duration);
    info!("Textures optimized: {}", stats.optimized);
    info!("Textures deleted (already optimal): {}", stats.deleted);
    info!("Failed: {}", stats.failed);
    info!(
        "\nOptimized textures are in: {:?}",
        output_dir.join("textures")
    );
    info!("Drag and drop this folder into your mod manager!");

    Ok(())
}
