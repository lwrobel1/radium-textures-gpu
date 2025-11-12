/// Worker thread for running optimization in background
use super::{AppSettings, ControlMessage, WorkerMessage};
use crate::{database, exclusions, extraction, mo2, optimization, presets};
use crossbeam_channel::{Receiver, Sender};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

pub struct OptimizationWorker;

impl OptimizationWorker {
    pub fn run(
        settings: AppSettings,
        tx: Sender<WorkerMessage>,
        rx: Receiver<ControlMessage>,
    ) {
        // Send initial log message
        let _ = tx.send(WorkerMessage::Log("Starting Radium Textures optimization...".to_string()));

        // Check for cancel
        let should_cancel = Arc::new(AtomicBool::new(false));
        let should_cancel_clone = should_cancel.clone();

        // Spawn cancel listener thread
        std::thread::spawn(move || {
            while let Ok(msg) = rx.recv() {
                match msg {
                    ControlMessage::Cancel => {
                        should_cancel_clone.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            }
        });

        // Run the optimization pipeline
        match Self::run_optimization(&settings, &tx, &should_cancel) {
            Ok(_) => {
                let _ = tx.send(WorkerMessage::Complete {
                    success: true,
                    message: "Optimization completed successfully!".to_string(),
                });
            }
            Err(e) => {
                let _ = tx.send(WorkerMessage::Log(format!("Error: {}", e)));
                let _ = tx.send(WorkerMessage::Complete {
                    success: false,
                    message: format!("Optimization failed: {}", e),
                });
            }
        }
    }

    fn run_optimization(
        settings: &AppSettings,
        tx: &Sender<WorkerMessage>,
        should_cancel: &Arc<AtomicBool>,
    ) -> anyhow::Result<()> {
        // Helper function to expand and canonicalize paths
        let expand_path = |path_str: &str| -> anyhow::Result<PathBuf> {
            use log::debug;
            debug!("expand_path: INPUT = {:?}", path_str);

            let path = if path_str.starts_with('~') {
                // Expand ~ to home directory
                if let Some(home) = dirs::home_dir() {
                    home.join(path_str.strip_prefix("~/").unwrap_or(&path_str[1..]))
                } else {
                    PathBuf::from(path_str)
                }
            } else {
                PathBuf::from(path_str)
            };

            debug!("expand_path: INTERMEDIATE = {:?}, is_absolute = {}", path, path.is_absolute());

            // Convert to absolute path
            let result = if path.is_absolute() {
                path
            } else {
                // Make relative path absolute by joining with current directory
                std::env::current_dir()?.join(path)
            };

            debug!("expand_path: OUTPUT = {:?}", result);
            Ok(result)
        };

        // Parse preset
        let opt_preset = match settings.preset.as_str() {
            "HighQuality" => presets::OptimizationPreset::HQ,
            "Quality" => presets::OptimizationPreset::QUALITY,
            "Optimum" => presets::OptimizationPreset::OPTIMUM,
            "Performance" => presets::OptimizationPreset::PERFORMANCE,
            "Vanilla" => presets::OptimizationPreset::VANILLA,
            "Custom" => presets::OptimizationPreset {
                name: "Custom",
                diffuse_max: settings.custom_diffuse,
                normal_max: settings.custom_normal,
                parallax_max: settings.custom_parallax,
                material_max: settings.custom_material,
            },
            _ => presets::OptimizationPreset::OPTIMUM,
        };

        let _ = tx.send(WorkerMessage::Log(format!(
            "Using preset: {} (Diffuse: {}px, Normal: {}px, Parallax: {}px, Material: {}px)",
            opt_preset.name,
            opt_preset.diffuse_max,
            opt_preset.normal_max,
            opt_preset.parallax_max,
            opt_preset.material_max
        )));

        // Step 1: Load profile
        if should_cancel.load(Ordering::Relaxed) {
            return Ok(());
        }

        let _ = tx.send(WorkerMessage::Progress {
            current: 0,
            total: 6,
            message: "Loading MO2 profile...".to_string(),
        });
        let _ = tx.send(WorkerMessage::Log("Step 1/6: Loading MO2 profile...".to_string()));

        // Expand all paths to absolute paths
        let profile_path = expand_path(&settings.profile_path)?;
        let mods_path = expand_path(&settings.mods_path)?;
        let data_path = expand_path(&settings.data_path)?;
        let output_dir = expand_path(&settings.output_path)?;

        let _ = tx.send(WorkerMessage::Log(format!("Profile: {:?}", profile_path)));
        let _ = tx.send(WorkerMessage::Log(format!("Mods: {:?}", mods_path)));
        let _ = tx.send(WorkerMessage::Log(format!("Data: {:?}", data_path)));
        let _ = tx.send(WorkerMessage::Log(format!("Output: {:?}", output_dir)));

        let profile = mo2::Profile::load(
            &profile_path,
            &mods_path,
            &data_path,
        )?;

        let _ = tx.send(WorkerMessage::Log(format!(
            "Profile loaded: {} enabled mods",
            profile.enabled_mods().count()
        )));

        // Step 2: Build VFS
        if should_cancel.load(Ordering::Relaxed) {
            return Ok(());
        }

        let _ = tx.send(WorkerMessage::Progress {
            current: 1,
            total: 6,
            message: "Building virtual file system...".to_string(),
        });
        let _ = tx.send(WorkerMessage::Log("Step 2/6: Building virtual file system...".to_string()));

        let vfs = mo2::VirtualFileSystem::new(profile)?;
        let stats = vfs.get_statistics();
        let _ = tx.send(WorkerMessage::Log(format!("VFS built: {}", stats)));

        // Step 3: Discover textures
        if should_cancel.load(Ordering::Relaxed) {
            return Ok(());
        }

        let _ = tx.send(WorkerMessage::Progress {
            current: 2,
            total: 6,
            message: "Discovering textures...".to_string(),
        });
        let _ = tx.send(WorkerMessage::Log("Step 3/6: Discovering textures...".to_string()));

        let mut textures = database::TextureDiscoveryService::discover_from_vfs(&vfs);
        let bsa_files = vfs.profile().get_plugin_bsas();

        let _ = tx.send(WorkerMessage::Log(format!("Scanning {} BSA archives...", bsa_files.len())));
        database::TextureDiscoveryService::discover_from_bsas(&bsa_files, &mut textures)?;

        let _ = tx.send(WorkerMessage::Log("Parsing DDS headers...".to_string()));
        database::TextureDiscoveryService::parse_dds_headers(&mut textures)?;

        let _ = tx.send(WorkerMessage::Log(format!(
            "Discovered {} total textures",
            textures.len()
        )));

        // Step 4: Load exclusions
        if should_cancel.load(Ordering::Relaxed) {
            return Ok(());
        }

        let _ = tx.send(WorkerMessage::Progress {
            current: 3,
            total: 6,
            message: "Loading exclusions...".to_string(),
        });
        let _ = tx.send(WorkerMessage::Log("Step 4/6: Loading exclusions...".to_string()));

        // Load game-specific exclusions
        let exclusions_filename = settings.game.exclusions_file();
        let exclusions_path = PathBuf::from("./exclusions").join(exclusions_filename);
        let _ = tx.send(WorkerMessage::Log(format!("Loading exclusions from: {:?}", exclusions_path)));

        let exclusions: Option<exclusions::ExclusionList> = if exclusions_path.exists() {
            match exclusions::ExclusionList::load(&exclusions_path) {
                Ok(list) => {
                    let _ = tx.send(WorkerMessage::Log(format!("Loaded exclusions for {}", settings.game)));
                    Some(list)
                }
                Err(e) => {
                    let _ = tx.send(WorkerMessage::Log(format!("Warning: Failed to load exclusions: {}", e)));
                    None
                }
            }
        } else {
            let _ = tx.send(WorkerMessage::Log(format!("No exclusions file found at {:?}", exclusions_path)));
            None
        };

        // Step 5: Filter textures
        if should_cancel.load(Ordering::Relaxed) {
            return Ok(());
        }

        let _ = tx.send(WorkerMessage::Progress {
            current: 4,
            total: 6,
            message: "Filtering textures...".to_string(),
        });
        let _ = tx.send(WorkerMessage::Log("Step 5/6: Filtering textures for optimization...".to_string()));

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

        let _ = tx.send(WorkerMessage::Log(format!(
            "Needs optimization: {}, Excluded: {}, Already optimal: {}",
            needs_optimization.len(),
            skipped_excluded,
            skipped_already_optimized
        )));

        if needs_optimization.is_empty() {
            let _ = tx.send(WorkerMessage::Log("No textures need optimization!".to_string()));
            return Ok(());
        }

        // Step 6: Extract and optimize
        if should_cancel.load(Ordering::Relaxed) {
            return Ok(());
        }

        let _ = tx.send(WorkerMessage::Progress {
            current: 5,
            total: 6,
            message: "Extracting textures...".to_string(),
        });
        let _ = tx.send(WorkerMessage::Log("Step 6/6: Extracting and optimizing textures...".to_string()));
        let _ = tx.send(WorkerMessage::Log(format!("Extracting {} textures...", needs_optimization.len())));

        let extracted = extraction::extract_all_textures(&needs_optimization, &output_dir)?;

        if extracted.is_empty() {
            let _ = tx.send(WorkerMessage::Log("No textures were successfully extracted".to_string()));
            return Ok(());
        }

        let _ = tx.send(WorkerMessage::Log(format!("Extracted {} textures", extracted.len())));

        // Group by processing type
        let groups = optimization::group_by_processing_type(extracted);

        let _ = tx.send(WorkerMessage::Log(format!(
            "Processing groups: Delete={}, BC7={}, BC4={}, RGBA={}, PBR={}, Specular={}, Emissive={}, Gloss={}",
            groups.delete_only.len(),
            groups.bc7_resize.len(),
            groups.bc4_resize.len(),
            groups.rgba_resize.len(),
            groups.pbr_resize.len(),
            groups.specular_resize.len(),
            groups.emissive_resize.len(),
            groups.gloss_resize.len()
        )));

        // Optimize - find texconv.exe (MUST be absolute path for Wine)
        // Try multiple locations and convert to absolute path
        let texconv_path = if PathBuf::from("./texconv.exe").exists() {
            // Convert relative to absolute - look in current working directory
            std::env::current_dir()?.join("texconv.exe")
        } else if PathBuf::from("texconv.exe").exists() {
            std::env::current_dir()?.join("texconv.exe")
        } else {
            // Look in the executable's directory
            if let Ok(exe_path) = std::env::current_exe() {
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
                anyhow::bail!("texconv.exe not found. Tried: ./texconv.exe, texconv.exe");
            }
        };

        let _ = tx.send(WorkerMessage::Log(format!("Using texconv at: {:?}", texconv_path)));
        let _ = tx.send(WorkerMessage::Log(format!("Using {} threads for texconv processing", settings.thread_count)));

        let _ = tx.send(WorkerMessage::Progress {
            current: 6,
            total: 6,
            message: "Optimizing textures...".to_string(),
        });
        let _ = tx.send(WorkerMessage::Log("Running texconv optimization...".to_string()));

        let stats = optimization::optimize_all(&groups, &texconv_path, Some(settings.thread_count))?;

        let _ = tx.send(WorkerMessage::Log(format!(
            "Optimization complete in {:.2?}",
            stats.duration
        )));
        let _ = tx.send(WorkerMessage::Log(format!(
            "Optimized: {}, Deleted: {}, Failed: {}",
            stats.optimized, stats.deleted, stats.failed
        )));
        let _ = tx.send(WorkerMessage::Log(format!(
            "Output directory: {:?}",
            output_dir.join("textures")
        )));

        Ok(())
    }
}
