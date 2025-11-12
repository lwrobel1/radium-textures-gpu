/// Texture optimization using texconv.exe via Wine
/// Smart batching and parallel processing for optimal performance

use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use log::{debug, error, info};
use rayon::prelude::*;
use rayon::ThreadPoolBuilder;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::database::TextureRecord;

/// Processing group for batch optimization
#[derive(Debug, Clone)]
pub struct ProcessingRecord {
    pub internal_path: String,
    pub record: TextureRecord,
    pub extracted_path: PathBuf,
    pub target_width: u32,
    pub target_height: u32,
    pub texture_type: String,
    pub current_width: u32,
    pub current_height: u32,
    pub oversized: bool,
}

/// Groups for different processing types
#[derive(Debug, Default)]
pub struct ProcessingGroups {
    pub delete_only: Vec<ProcessingRecord>,
    pub bc7_resize: Vec<ProcessingRecord>,
    pub bc4_resize: Vec<ProcessingRecord>,
    pub rgba_resize: Vec<ProcessingRecord>,
    pub pbr_resize: Vec<ProcessingRecord>,
    pub specular_resize: Vec<ProcessingRecord>,
    pub emissive_resize: Vec<ProcessingRecord>,
    pub gloss_resize: Vec<ProcessingRecord>,
}

impl ProcessingGroups {
    pub fn total(&self) -> usize {
        self.delete_only.len()
            + self.bc7_resize.len()
            + self.bc4_resize.len()
            + self.rgba_resize.len()
            + self.pbr_resize.len()
            + self.specular_resize.len()
            + self.emissive_resize.len()
            + self.gloss_resize.len()
    }
}

/// Group textures by processing type (following Optimise.py logic)
pub fn group_by_processing_type(
    textures: Vec<(String, TextureRecord, u32, u32, PathBuf)>,
) -> ProcessingGroups {
    let mut groups = ProcessingGroups::default();

    for (internal_path, record, target_width, target_height, extracted_path) in textures {
        // Get texture properties
        let Some(texture_type) = &record.texture_type else {
            continue;
        };
        let Some(width) = record.width else { continue };
        let Some(height) = record.height else { continue };
        let format = record.format.as_ref().map(|s| s.as_str()).unwrap_or("");

        // Determine if texture is oversized
        let current_max = width.max(height);
        let target_max = target_width.max(target_height);
        let oversized = current_max > target_max;

        // Check for RGB/PBR formats
        let is_rgb = format.to_uppercase() == "ARGB_8888";
        let is_pbr = internal_path.to_lowercase().contains("/pbr/");

        let proc_record = ProcessingRecord {
            internal_path: internal_path.clone(),
            record: record.clone(),
            extracted_path,
            target_width,
            target_height,
            texture_type: texture_type.clone(),
            current_width: width,
            current_height: height,
            oversized,
        };

        // Group by processing type (exact Optimise.py logic)
        if !oversized {
            // Already at target size - delete only
            groups.delete_only.push(proc_record);
        } else {
            match texture_type.as_str() {
                "Specular" => {
                    // Specular: resize to 1024, no format conversion
                    groups.specular_resize.push(proc_record);
                }
                "Emissive" | "Emissive Mask" => {
                    // Emissive: resize to 1024, no format conversion
                    groups.emissive_resize.push(proc_record);
                }
                "Subsurface" => {
                    // Gloss/Subsurface: BC4, halve resolution
                    groups.gloss_resize.push(proc_record);
                }
                "Normal" => {
                    if is_rgb || is_pbr {
                        groups.pbr_resize.push(proc_record);
                    } else {
                        groups.bc7_resize.push(proc_record);
                    }
                }
                "Parallax" => {
                    if is_rgb || is_pbr {
                        groups.pbr_resize.push(proc_record);
                    } else {
                        groups.bc4_resize.push(proc_record);
                    }
                }
                "Diffuse" => {
                    if is_rgb {
                        groups.rgba_resize.push(proc_record);
                    } else if is_pbr {
                        groups.pbr_resize.push(proc_record);
                    } else {
                        groups.bc7_resize.push(proc_record);
                    }
                }
                _ => {
                    // Material/Multi-layer/Environment: BC7
                    if is_rgb || is_pbr {
                        groups.pbr_resize.push(proc_record);
                    } else {
                        groups.bc7_resize.push(proc_record);
                    }
                }
            }
        }
    }

    groups
}

/// Process a batch of textures with texconv (parallel with progress bar)
/// Returns (success_count, failed_count)
pub fn process_batch_texconv(
    batch: &[ProcessingRecord],
    format: Option<&str>,
    texconv_path: &Path,
) -> Result<(usize, usize)> {
    if batch.is_empty() {
        return Ok((0, 0));
    }

    let total_success = AtomicUsize::new(0);
    let total_failed = AtomicUsize::new(0);

    // Create progress bar with better info
    let pb = ProgressBar::new(batch.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg} (ETA: {eta})")
            .unwrap()
            .progress_chars("=>-"),
    );

    let format_name = format.unwrap_or("resize");
    pb.set_message(format!("Processing {} textures...", format_name));

    // Process textures in parallel (16 processes, each single-threaded)
    batch.par_iter().for_each(|record| {
            match process_single_texture(record, format, texconv_path) {
                Ok(_) => {
                    total_success.fetch_add(1, Ordering::Relaxed);
                    pb.inc(1);
                }
                Err(e) => {
                    error!("Failed to process {}: {}", record.internal_path, e);
                    total_failed.fetch_add(1, Ordering::Relaxed);
                    pb.inc(1);
                }
            }
        });

    let success = total_success.into_inner();
    let failed = total_failed.into_inner();

    pb.finish_with_message(format!("Complete: {} success, {} failed", success, failed));

    info!("Batch complete: {} succeeded, {} failed", success, failed);

    Ok((success, failed))
}

/// Process a single texture with texconv
fn process_single_texture(
    record: &ProcessingRecord,
    format: Option<&str>,
    texconv_path: &Path,
) -> Result<()> {
    // Use FULL absolute path to the extracted file
    let full_path = &record.extracted_path;

    // DEBUG: Log the path we're about to process
    debug!("process_single_texture: extracted_path = {:?}", full_path);
    debug!("process_single_texture: is_absolute = {}", full_path.is_absolute());

    // Get output directory (parent of the file)
    let output_dir = full_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("No parent directory for: {:?}", full_path))?;

    debug!("process_single_texture: output_dir = {:?}", output_dir);
    debug!("process_single_texture: texconv_path = {:?}", texconv_path);

    // Convert Linux paths to Wine Z: drive format to handle spaces correctly
    // Z: maps to the Linux root /
    let wine_texconv = format!("Z:{}", texconv_path.display());
    let wine_output_dir = format!("Z:{}", output_dir.display());
    let wine_input_path = format!("Z:{}", full_path.display());

    debug!("Wine paths: texconv={}, output={}, input={}", wine_texconv, wine_output_dir, wine_input_path);

    // Build texconv command
    let mut cmd = Command::new("wine");
    cmd.arg(&wine_texconv);
    cmd.arg("-gpu").arg("0");  // Enable GPU
    cmd.arg("-sepalpha");
    cmd.arg("-nologo");
    cmd.arg("-y");  // Overwrite original file directly
    cmd.arg("-o").arg(&wine_output_dir);  // Output to the same directory as input
    cmd.arg("--single-proc");  // Single-threaded, let Rayon handle parallelization

    // Add format conversion if specified
    if let Some(fmt) = format {
        let texconv_format = match fmt.to_uppercase().as_str() {
            "BC7" => "BC7_UNORM",
            "BC4" => "BC4_UNORM",
            "BC3" => "BC3_UNORM",
            "RGBA" => "RGBA",
            "ARGB_8888" => "RGBA",
            _ => fmt,
        };
        cmd.arg("-f").arg(texconv_format);
    }

    // Add resize dimensions if oversized
    if record.oversized {
        cmd.arg("-w").arg(record.target_width.to_string());
        cmd.arg("-h").arg(record.target_height.to_string());
    }

    // Add input file (use Wine Z: drive format)
    cmd.arg(&wine_input_path);

    // Execute texconv
    debug!("Running texconv: {:?}", cmd);
    debug!("Command args breakdown:");
    debug!("  - wine");
    debug!("  - texconv_path: {:?}", texconv_path);
    debug!("  - output_dir: {:?}", output_dir);
    debug!("  - full_path: {:?}", full_path);
    debug!("  - full_path.display(): {}", full_path.display());
    debug!("  - full_path.is_absolute(): {}", full_path.is_absolute());

    let output = cmd.output()?;

    // Get stdout and stderr
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Filter Wine warnings from stderr to show real errors
    let filter_wine_warnings = |text: &str| -> Vec<String> {
        text.lines()
            .filter(|line| {
                !line.contains("fixme:") &&
                !line.contains("MANGOHUD") &&
                !line.contains("WARNING: radv") &&
                !line.contains("err:environ:init_peb") &&
                !line.contains("process") &&
                !line.starts_with("00") &&
                !line.trim().is_empty()
            })
            .map(|s| s.to_string())
            .collect()
    };

    let filtered_stderr = filter_wine_warnings(&stderr);
    let filtered_stdout = filter_wine_warnings(&stdout);

    // Check exit status and provide detailed error
    if !output.status.success() {
        let exit_code = output.status.code().unwrap_or(-1);

        // Build detailed error message
        let mut error_details = format!("texconv failed with exit code {}", exit_code);

        if !filtered_stderr.is_empty() {
            error_details.push_str(&format!("\nStderr: {}", filtered_stderr.join("\n")));
        }
        if !filtered_stdout.is_empty() {
            error_details.push_str(&format!("\nStdout: {}", filtered_stdout.join("\n")));
        }

        error!("Failed to process {}: {}", record.internal_path, error_details);
        anyhow::bail!("{}", error_details);
    }

    // Log success
    if stdout.contains("writing") || stdout.contains("converted") {
        debug!("Texconv successfully processed: {}", record.internal_path);
    }

    // Verify the file was actually modified
    if !record.extracted_path.exists() {
        let error_msg = if filtered_stderr.is_empty() {
            "Output file doesn't exist after texconv (no error details)".to_string()
        } else {
            format!("Output file doesn't exist: {}", filtered_stderr.join("\n"))
        };
        anyhow::bail!("{}", error_msg);
    }

    Ok(())
}

/// Process delete-only batch (textures already at target size) - parallel
pub fn process_delete_batch(batch: &[ProcessingRecord]) -> Result<(usize, usize)> {
    let success = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);

    batch.par_iter().for_each(|record| {
            match fs::remove_file(&record.extracted_path) {
                Ok(_) => {
                    debug!(
                        "Deleted (already optimal): {} ({}x{})",
                        record.internal_path, record.current_width, record.current_height
                    );
                    success.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    error!("Failed to delete {}: {}", record.internal_path, e);
                    failed.fetch_add(1, Ordering::Relaxed);
                }
            }
        });

    Ok((success.into_inner(), failed.into_inner()))
}

/// Optimize all texture groups
pub fn optimize_all(groups: &ProcessingGroups, texconv_path: &Path, thread_count: Option<usize>) -> Result<OptimizationStats> {
    let num_threads = thread_count.unwrap_or_else(|| num_cpus::get());

    info!("Starting optimization of {} textures with {} threads...", groups.total(), num_threads);
    info!(
        "Groups: Delete={}, BC7={}, BC4={}, RGBA={}, PBR={}, Specular={}, Emissive={}, Gloss={}",
        groups.delete_only.len(),
        groups.bc7_resize.len(),
        groups.bc4_resize.len(),
        groups.rgba_resize.len(),
        groups.pbr_resize.len(),
        groups.specular_resize.len(),
        groups.emissive_resize.len(),
        groups.gloss_resize.len()
    );

    // Create thread pool with specified thread count for texconv operations
    let pool = ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()?;

    let start_time = std::time::Instant::now();
    let mut stats = OptimizationStats::default();

    // Process delete-only (already optimal)
    if !groups.delete_only.is_empty() {
        info!("Processing {} delete-only textures...", groups.delete_only.len());
        let (success, failed) = process_delete_batch(&groups.delete_only)?;
        stats.deleted += success;
        stats.failed += failed;
    }

    // Process BC7 conversions (diffuse, normal, material) - using custom thread pool
    if !groups.bc7_resize.is_empty() {
        info!("Processing {} BC7 textures...", groups.bc7_resize.len());
        let (success, failed) = pool.install(|| process_batch_texconv(&groups.bc7_resize, Some("BC7"), texconv_path))?;
        stats.optimized += success;
        stats.failed += failed;
    }

    // Process BC4 conversions (parallax)
    if !groups.bc4_resize.is_empty() {
        info!("Processing {} BC4 textures...", groups.bc4_resize.len());
        let (success, failed) = pool.install(|| process_batch_texconv(&groups.bc4_resize, Some("BC4"), texconv_path))?;
        stats.optimized += success;
        stats.failed += failed;
    }

    // Process RGBA (RGB textures)
    if !groups.rgba_resize.is_empty() {
        info!("Processing {} RGBA textures...", groups.rgba_resize.len());
        let (success, failed) = pool.install(|| process_batch_texconv(&groups.rgba_resize, Some("RGBA"), texconv_path))?;
        stats.optimized += success;
        stats.failed += failed;
    }

    // Process PBR (no format conversion)
    if !groups.pbr_resize.is_empty() {
        info!("Processing {} PBR textures...", groups.pbr_resize.len());
        let (success, failed) = pool.install(|| process_batch_texconv(&groups.pbr_resize, None, texconv_path))?;
        stats.optimized += success;
        stats.failed += failed;
    }

    // Process Specular (no format conversion)
    if !groups.specular_resize.is_empty() {
        info!("Processing {} Specular textures...", groups.specular_resize.len());
        let (success, failed) = pool.install(|| process_batch_texconv(&groups.specular_resize, None, texconv_path))?;
        stats.optimized += success;
        stats.failed += failed;
    }

    // Process Emissive (no format conversion)
    if !groups.emissive_resize.is_empty() {
        info!("Processing {} Emissive textures...", groups.emissive_resize.len());
        let (success, failed) = pool.install(|| process_batch_texconv(&groups.emissive_resize, None, texconv_path))?;
        stats.optimized += success;
        stats.failed += failed;
    }

    // Process Gloss (BC4, halve resolution)
    if !groups.gloss_resize.is_empty() {
        info!("Processing {} Gloss textures...", groups.gloss_resize.len());
        let (success, failed) = pool.install(|| process_batch_texconv(&groups.gloss_resize, Some("BC4"), texconv_path))?;
        stats.optimized += success;
        stats.failed += failed;
    }

    let elapsed = start_time.elapsed();
    stats.duration = elapsed;

    info!(
        "Optimization complete in {:.2?}: {} optimized, {} deleted, {} failed",
        elapsed, stats.optimized, stats.deleted, stats.failed
    );

    Ok(stats)
}

#[derive(Debug, Default)]
pub struct OptimizationStats {
    pub optimized: usize,
    pub deleted: usize,
    pub failed: usize,
    pub duration: std::time::Duration,
}
