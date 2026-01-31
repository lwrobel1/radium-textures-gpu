/// Texture optimization using texconv.exe via Wine or NVTT3 (native CUDA)
/// Smart batching and parallel processing for optimal performance

use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use log::{debug, error, info, warn};
use rayon::prelude::*;
use rayon::ThreadPoolBuilder;
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::database::TextureRecord;

/// DDS file validation result
#[derive(Debug)]
pub struct DdsValidationResult {
    pub valid: bool,
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub error: Option<String>,
}

/// Validate a DDS file to ensure it's not corrupted
/// Checks: magic number, header structure, dimensions, format
pub fn validate_dds_file(path: &Path) -> DdsValidationResult {
    let mut result = DdsValidationResult {
        valid: false,
        width: 0,
        height: 0,
        format: String::new(),
        error: None,
    };

    // Check file exists and has minimum size (DDS header = 128 bytes minimum)
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            result.error = Some(format!("Cannot read file: {}", e));
            return result;
        }
    };

    if metadata.len() < 128 {
        result.error = Some(format!("File too small: {} bytes (min 128)", metadata.len()));
        return result;
    }

    // Read header
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            result.error = Some(format!("Cannot open file: {}", e));
            return result;
        }
    };

    let mut header = [0u8; 148]; // DDS header + DX10 extension
    let bytes_read = match file.read(&mut header) {
        Ok(n) => n,
        Err(e) => {
            result.error = Some(format!("Cannot read header: {}", e));
            return result;
        }
    };

    if bytes_read < 128 {
        result.error = Some(format!("Header too short: {} bytes", bytes_read));
        return result;
    }

    // Check DDS magic number: "DDS " = 0x20534444
    let magic = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    if magic != 0x20534444 {
        result.error = Some(format!("Invalid DDS magic: 0x{:08X} (expected 0x20534444)", magic));
        return result;
    }

    // Check header size (should be 124)
    let header_size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if header_size != 124 {
        result.error = Some(format!("Invalid header size: {} (expected 124)", header_size));
        return result;
    }

    // Extract dimensions
    result.height = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
    result.width = u32::from_le_bytes([header[16], header[17], header[18], header[19]]);

    // Validate dimensions
    if result.width == 0 || result.height == 0 {
        result.error = Some(format!("Invalid dimensions: {}x{}", result.width, result.height));
        return result;
    }

    if result.width > 16384 || result.height > 16384 {
        result.error = Some(format!("Dimensions too large: {}x{}", result.width, result.height));
        return result;
    }

    // Check pixel format flags at offset 76
    let pf_size = u32::from_le_bytes([header[76], header[77], header[78], header[79]]);
    if pf_size != 32 {
        result.error = Some(format!("Invalid pixel format size: {} (expected 32)", pf_size));
        return result;
    }

    // Check FourCC at offset 84
    let fourcc = u32::from_le_bytes([header[84], header[85], header[86], header[87]]);

    // Determine format from FourCC
    result.format = match fourcc {
        0x31545844 => "DXT1/BC1".to_string(),
        0x33545844 => "DXT3/BC2".to_string(),
        0x35545844 => "DXT5/BC3".to_string(),
        0x55344342 => "BC4U".to_string(),
        0x53344342 => "BC4S".to_string(),
        0x32495441 => "ATI2/BC5".to_string(),
        0x30315844 => { // "DX10" - check DX10 header
            if bytes_read >= 148 {
                let dxgi_format = u32::from_le_bytes([header[128], header[129], header[130], header[131]]);
                match dxgi_format {
                    98 => "BC7_UNORM".to_string(),
                    99 => "BC7_UNORM_SRGB".to_string(),
                    80 => "BC4_UNORM".to_string(),
                    81 => "BC4_SNORM".to_string(),
                    83 => "BC5_UNORM".to_string(),
                    84 => "BC5_SNORM".to_string(),
                    71 => "BC1_UNORM".to_string(),
                    72 => "BC1_UNORM_SRGB".to_string(),
                    74 => "BC2_UNORM".to_string(),
                    77 => "BC3_UNORM".to_string(),
                    _ => format!("DX10(DXGI={})", dxgi_format),
                }
            } else {
                result.error = Some("DX10 header missing".to_string());
                return result;
            }
        }
        0 => "Uncompressed".to_string(),
        _ => format!("Unknown(0x{:08X})", fourcc),
    };

    // Calculate expected minimum file size based on format and dimensions
    let block_size = match fourcc {
        0x31545844 | 0x55344342 | 0x53344342 => 8,  // BC1, BC4
        0x33545844 | 0x35545844 | 0x32495441 | 0x30315844 => 16, // BC2, BC3, BC5, DX10
        _ => 4,
    };

    let blocks_wide = (result.width + 3) / 4;
    let blocks_high = (result.height + 3) / 4;
    let min_data_size = (blocks_wide * blocks_high * block_size) as u64;
    let header_overhead = if fourcc == 0x30315844 { 148 } else { 128 };

    if metadata.len() < header_overhead + min_data_size / 2 {
        // Allow some tolerance (mipmap chain can reduce total size)
        result.error = Some(format!(
            "File size {} too small for {}x{} {} (expected ~{}+ bytes)",
            metadata.len(), result.width, result.height, result.format,
            header_overhead as u64 + min_data_size
        ));
        return result;
    }

    result.valid = true;
    result
}

/// Compression backend selection
#[derive(Debug, Clone, Copy, PartialEq, Default, clap::ValueEnum)]
pub enum CompressionBackend {
    /// texconv.exe via Wine (original method)
    #[default]
    Texconv,
    /// NVIDIA Texture Tools 3 with CUDA (native Linux, much faster)
    Nvtt3,
}

impl CompressionBackend {
    pub fn name(&self) -> &'static str {
        match self {
            CompressionBackend::Texconv => "texconv (Wine)",
            CompressionBackend::Nvtt3 => "NVTT3 (CUDA)",
        }
    }
}

/// Paths for compression tools
#[derive(Debug, Clone)]
pub struct CompressionTools {
    pub texconv_path: Option<PathBuf>,
    pub nvtt3_path: Option<PathBuf>,
    pub nvtt3_batch_path: Option<PathBuf>,
    pub nvtt3_lib_path: Option<PathBuf>,
}

impl CompressionTools {
    /// Find compression tools in standard locations
    pub fn find() -> Self {
        let exe_dir = env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));

        // Find texconv.exe
        let texconv_path = Self::find_texconv(&exe_dir);

        // Find NVTT3 tools (single file + batch)
        let (nvtt3_path, nvtt3_batch_path, nvtt3_lib_path) = Self::find_nvtt3(&exe_dir);

        Self {
            texconv_path,
            nvtt3_path,
            nvtt3_batch_path,
            nvtt3_lib_path,
        }
    }

    fn find_texconv(exe_dir: &Option<PathBuf>) -> Option<PathBuf> {
        // Check current directory
        let cwd = PathBuf::from("texconv.exe");
        if cwd.exists() {
            return Some(cwd.canonicalize().unwrap_or(cwd));
        }

        // Check executable directory
        if let Some(dir) = exe_dir {
            let path = dir.join("texconv.exe");
            if path.exists() {
                return Some(path);
            }
        }

        None
    }

    fn find_nvtt3(exe_dir: &Option<PathBuf>) -> (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>) {
        // Check tools/nvtt3 in current directory - look for nvtt_resize_compress and nvtt_batch_compress
        let nvtt3_dir = PathBuf::from("tools/nvtt3");
        if nvtt3_dir.exists() {
            let nvtt_tool = nvtt3_dir.join("nvtt_resize_compress");
            let nvtt_batch = nvtt3_dir.join("nvtt_batch_compress");
            if nvtt_tool.exists() {
                let single_path = Some(nvtt_tool.canonicalize().unwrap_or(nvtt_tool));
                let batch_path = if nvtt_batch.exists() {
                    Some(nvtt_batch.canonicalize().unwrap_or(nvtt_batch))
                } else {
                    None
                };
                let lib_path = Some(nvtt3_dir.canonicalize().unwrap_or(nvtt3_dir));
                return (single_path, batch_path, lib_path);
            }
        }

        // Check executable directory
        if let Some(dir) = exe_dir {
            let nvtt3_dir = dir.join("tools/nvtt3");
            if nvtt3_dir.exists() {
                let nvtt_tool = nvtt3_dir.join("nvtt_resize_compress");
                let nvtt_batch = nvtt3_dir.join("nvtt_batch_compress");
                if nvtt_tool.exists() {
                    let single_path = Some(nvtt_tool);
                    let batch_path = if nvtt_batch.exists() {
                        Some(nvtt_batch)
                    } else {
                        None
                    };
                    return (single_path, batch_path, Some(nvtt3_dir));
                }
            }
        }

        // Check if nvtt_resize_compress is in PATH (system install)
        if let Ok(output) = Command::new("which").arg("nvtt_resize_compress").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return (Some(PathBuf::from(path)), None, None);
                }
            }
        }

        (None, None, None)
    }

    /// Check if a backend is available
    pub fn is_available(&self, backend: CompressionBackend) -> bool {
        match backend {
            CompressionBackend::Texconv => self.texconv_path.is_some(),
            CompressionBackend::Nvtt3 => self.nvtt3_path.is_some(),
        }
    }

    /// Get the best available backend (prefers NVTT3 for speed)
    pub fn best_available(&self) -> Option<CompressionBackend> {
        if self.nvtt3_path.is_some() {
            Some(CompressionBackend::Nvtt3)
        } else if self.texconv_path.is_some() {
            Some(CompressionBackend::Texconv)
        } else {
            None
        }
    }
}

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

/// Minimum texture dimension - textures smaller than this are skipped (VRAMr Rule 1)
const MIN_TEXTURE_DIMENSION: u32 = 512;

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
    pub skipped_small: usize,  // Textures skipped due to being < 512x512
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

    pub fn total_with_skipped(&self) -> usize {
        self.total() + self.skipped_small
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

        // VRAMr Rule 1: Skip textures smaller than 512 in any dimension
        // These are too small to benefit from optimization
        if width < MIN_TEXTURE_DIMENSION || height < MIN_TEXTURE_DIMENSION {
            debug!(
                "Skipping small texture: {} ({}x{} < {})",
                internal_path, width, height, MIN_TEXTURE_DIMENSION
            );
            groups.skipped_small += 1;
            continue;
        }

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

    // Validate the output DDS file
    let validation = validate_dds_file(&record.extracted_path);
    if !validation.valid {
        let error_msg = validation.error.unwrap_or_else(|| "Unknown validation error".to_string());
        error!(
            "DDS validation failed for {}: {}",
            record.internal_path, error_msg
        );
        // Don't copy corrupted output, but leave original file intact
        anyhow::bail!("Output DDS validation failed: {}", error_msg);
    }

    debug!(
        "Texconv validated: {} -> {}x{} [{}]",
        record.internal_path, validation.width, validation.height, validation.format
    );

    Ok(())
}

/// Process a single texture with NVTT3 nvtt_resize_compress tool
/// Handles resize + compression + mipmap generation in one CUDA-accelerated step
fn process_single_texture_nvtt3(
    record: &ProcessingRecord,
    format: Option<&str>,
    nvtt_tool_path: &Path,
    lib_path: Option<&Path>,
) -> Result<()> {
    let full_path = &record.extracted_path;

    debug!("process_single_texture_nvtt3: extracted_path = {:?}", full_path);

    // Create temp file for output (we'll move it back after success)
    let temp_output = tempfile::Builder::new()
        .suffix(".dds")
        .tempfile()?;
    let temp_path = temp_output.path();

    // Determine target size (max dimension)
    let max_extent = record.target_width.max(record.target_height);

    // Determine format argument
    let format_arg = match format {
        Some(fmt) => match fmt.to_uppercase().as_str() {
            "BC7" | "BC7_UNORM" => "bc7",
            "BC4" | "BC4_UNORM" => "bc4",
            "BC3" | "BC3_UNORM" => "bc3",
            "BC1" | "BC1_UNORM" => "bc1",
            "BC5" | "BC5_UNORM" => "bc5",
            _ => "bc7",
        },
        None => "bc7", // Default to BC7
    };

    // Build nvtt_resize_compress command
    let mut cmd = Command::new(nvtt_tool_path);

    // Set LD_LIBRARY_PATH for the NVTT shared library
    if let Some(lib_dir) = lib_path {
        cmd.env("LD_LIBRARY_PATH", lib_dir);
    }

    cmd.arg(full_path);           // Input DDS
    cmd.arg(temp_path);           // Output DDS (temp)
    cmd.arg(max_extent.to_string()); // Max dimension
    cmd.arg(format_arg);          // Format

    debug!("Running nvtt_resize_compress: {:?}", cmd);

    let output = cmd.output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        error!(
            "nvtt_resize_compress failed for {}: stderr={}, stdout={}",
            record.internal_path, stderr, stdout
        );
        anyhow::bail!("nvtt_resize_compress failed with exit code {:?}", output.status.code());
    }

    // Validate the output DDS file before copying
    let validation = validate_dds_file(temp_path);
    if !validation.valid {
        let error_msg = validation.error.unwrap_or_else(|| "Unknown validation error".to_string());
        error!(
            "DDS validation failed for {}: {}",
            record.internal_path, error_msg
        );
        // Don't copy corrupted output, but leave original file intact
        // (user can manually review failed textures)
        anyhow::bail!("Output DDS validation failed: {}", error_msg);
    }

    // Verify dimensions match expected target
    let expected_max = record.target_width.max(record.target_height);
    let actual_max = validation.width.max(validation.height);
    if actual_max > expected_max {
        warn!(
            "Output dimensions {}x{} exceed target {} for {}",
            validation.width, validation.height, expected_max, record.internal_path
        );
    }

    // Move temp file to original location (overwrite)
    fs::copy(temp_path, full_path)?;

    debug!(
        "NVTT3 processed: {} ({}x{} -> {}x{}) [{}] validated OK",
        record.internal_path,
        record.current_width, record.current_height,
        validation.width, validation.height,
        format_arg
    );
    Ok(())
}

/// Process a batch of textures with NVTT3 - uses batch tool for better GPU utilization
/// Falls back to per-file processing if batch tool is not available
/// If texconv_fallback is provided, retries failed files with texconv
/// Returns (success_count, failed_count)
pub fn process_batch_nvtt3(
    batch: &[ProcessingRecord],
    format: Option<&str>,
    nvtt_tool_path: &Path,
    lib_path: Option<&Path>,
    texconv_fallback: Option<&Path>,
) -> Result<(usize, usize)> {
    // Check if batch tool exists alongside the single-file tool
    let batch_tool_path = nvtt_tool_path
        .parent()
        .map(|p| p.join("nvtt_batch_compress"));

    if let Some(ref batch_path) = batch_tool_path {
        if batch_path.exists() {
            return process_batch_nvtt3_batched(batch, format, batch_path, lib_path, texconv_fallback);
        }
    }

    // Fallback to per-file processing
    process_batch_nvtt3_perfile(batch, format, nvtt_tool_path, lib_path)
}

/// Batch size for NVTT3 processing - balance between CUDA init overhead and parallelism
/// Smaller batches = more parallelism, larger batches = less CUDA init overhead
const NVTT3_BATCH_SIZE: usize = 100;

/// Process textures using nvtt_batch_compress with parallel batch execution
/// Multiple batch processes run in parallel for better GPU utilization
/// If texconv_fallback is provided, retries failed files with texconv
fn process_batch_nvtt3_batched(
    batch: &[ProcessingRecord],
    format: Option<&str>,
    batch_tool_path: &Path,
    lib_path: Option<&Path>,
    texconv_fallback: Option<&Path>,
) -> Result<(usize, usize)> {
    use std::sync::Mutex;

    if batch.is_empty() {
        return Ok((0, 0));
    }

    let format_name = format.unwrap_or("BC7");
    let format_arg = match format {
        Some(fmt) => match fmt.to_uppercase().as_str() {
            "BC7" | "BC7_UNORM" => "bc7",
            "BC4" | "BC4_UNORM" => "bc4",
            "BC3" | "BC3_UNORM" => "bc3",
            "BC1" | "BC1_UNORM" => "bc1",
            "BC5" | "BC5_UNORM" => "bc5",
            _ => "bc7",
        },
        None => "bc7",
    };

    // Use thread pool configured by user settings
    let parallel_jobs = rayon::current_num_threads();

    info!(
        "NVTT3 Batch: Processing {} {} textures ({} parallel jobs, {} per batch)",
        batch.len(),
        format_name,
        parallel_jobs,
        NVTT3_BATCH_SIZE
    );

    // Create progress bar for all textures
    let pb = ProgressBar::new(batch.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.green/black} {pos}/{len} NVTT3 {msg} (ETA: {eta})")
            .unwrap()
            .progress_chars("=>-"),
    );
    pb.set_message(format!("[{}]", format_name));

    let total_success = AtomicUsize::new(0);
    let total_failed = AtomicUsize::new(0);

    // Collect failed records for texconv fallback
    let failed_records: Mutex<Vec<&ProcessingRecord>> = Mutex::new(Vec::new());

    // Create a map from path to record for fallback lookup
    let record_map: std::collections::HashMap<String, &ProcessingRecord> = batch
        .iter()
        .map(|r| (r.extracted_path.to_string_lossy().to_string(), r))
        .collect();

    // Split into chunks and process in parallel
    let chunks: Vec<_> = batch.chunks(NVTT3_BATCH_SIZE).collect();

    chunks.par_iter().enumerate().for_each(|(batch_idx, chunk)| {
        let batch_num = batch_idx + 1;

        debug!(
            "Processing batch {}/{} ({} textures)",
            batch_num,
            chunks.len(),
            chunk.len()
        );

        // Create batch file
        let batch_file = match tempfile::Builder::new()
            .prefix("nvtt_batch_")
            .suffix(".txt")
            .tempfile() {
                Ok(f) => f,
                Err(e) => {
                    error!("Failed to create batch file: {}", e);
                    total_failed.fetch_add(chunk.len(), Ordering::Relaxed);
                    pb.inc(chunk.len() as u64);
                    return;
                }
            };

        // Write batch entries: input|output|max_extent|format
        {
            let mut writer = std::io::BufWriter::new(&batch_file);
            for record in *chunk {
                let max_extent = record.target_width.max(record.target_height);
                if let Err(e) = writeln!(
                    writer,
                    "{}|{}|{}|{}",
                    record.extracted_path.display(),
                    record.extracted_path.display(),
                    max_extent,
                    format_arg
                ) {
                    error!("Failed to write batch entry: {}", e);
                }
            }
            if let Err(e) = writer.flush() {
                error!("Failed to flush batch file: {}", e);
            }
        }

        // Run batch compress with streaming output
        let mut cmd = Command::new(batch_tool_path);

        if let Some(lib_dir) = lib_path {
            cmd.env("LD_LIBRARY_PATH", lib_dir);
        }

        cmd.arg(batch_file.path());
        cmd.stderr(Stdio::piped());
        cmd.stdout(Stdio::null());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to spawn batch process: {}", e);
                total_failed.fetch_add(chunk.len(), Ordering::Relaxed);
                pb.inc(chunk.len() as u64);
                return;
            }
        };

        // Read stderr for progress updates
        if let Some(stderr) = child.stderr.take() {
            let reader = BufReader::new(stderr);

            for line in reader.lines().flatten() {
                if line.starts_with("OK:") {
                    total_success.fetch_add(1, Ordering::Relaxed);
                    pb.inc(1);
                } else if line.starts_with("FAIL:") {
                    // Extract failed file path for fallback
                    let parts: Vec<&str> = line.splitn(4, ':').collect();
                    if parts.len() >= 3 {
                        let failed_path = parts[2].to_string();
                        if let Some(record) = record_map.get(&failed_path) {
                            if texconv_fallback.is_some() {
                                // Don't count as failed yet - will retry with texconv
                                if let Ok(mut failed) = failed_records.lock() {
                                    failed.push(*record);
                                }
                            } else {
                                total_failed.fetch_add(1, Ordering::Relaxed);
                                error!("NVTT3 batch failed: {} - {}", parts[2], parts.get(3).unwrap_or(&"unknown error"));
                            }
                        } else {
                            total_failed.fetch_add(1, Ordering::Relaxed);
                            error!("NVTT3 batch failed: {} - {}", parts[2], parts.get(3).unwrap_or(&"unknown error"));
                        }
                    } else {
                        total_failed.fetch_add(1, Ordering::Relaxed);
                    }
                    pb.inc(1);
                } else if line.starts_with("CUDA:") {
                    debug!("NVTT3 batch {}: {}", batch_num, line);
                }
            }
        }

        // Wait for process to finish
        if let Err(e) = child.wait() {
            error!("Failed to wait for batch process: {}", e);
        }
    });

    let mut success = total_success.into_inner();
    let mut failed = total_failed.into_inner();

    pb.finish_with_message(format!(
        "NVTT3: {} success, {} failed",
        success, failed
    ));

    // Retry failed files with texconv if available
    if let Some(texconv_path) = texconv_fallback {
        let failed_list = failed_records.into_inner().unwrap_or_default();
        if !failed_list.is_empty() {
            info!(
                "Retrying {} NVTT3 failures with texconv fallback...",
                failed_list.len()
            );

            let fallback_pb = ProgressBar::new(failed_list.len() as u64);
            fallback_pb.set_style(
                ProgressStyle::default_bar()
                    .template("[{elapsed_precise}] {bar:40.yellow/black} {pos}/{len} texconv fallback")
                    .unwrap()
                    .progress_chars("=>-"),
            );

            let fallback_success = AtomicUsize::new(0);
            let fallback_failed = AtomicUsize::new(0);

            failed_list.par_iter().for_each(|record| {
                match process_single_texture(record, format, texconv_path) {
                    Ok(_) => {
                        fallback_success.fetch_add(1, Ordering::Relaxed);
                        debug!("Texconv fallback succeeded: {}", record.internal_path);
                    }
                    Err(e) => {
                        fallback_failed.fetch_add(1, Ordering::Relaxed);
                        error!("Texconv fallback also failed for {}: {}", record.internal_path, e);
                    }
                }
                fallback_pb.inc(1);
            });

            let fb_success = fallback_success.into_inner();
            let fb_failed = fallback_failed.into_inner();

            fallback_pb.finish_with_message(format!(
                "Fallback: {} success, {} failed",
                fb_success, fb_failed
            ));

            success += fb_success;
            failed += fb_failed;

            info!(
                "Texconv fallback: {} succeeded, {} failed",
                fb_success, fb_failed
            );
        }
    }

    info!(
        "NVTT3 batch complete: {} succeeded, {} failed total",
        success, failed
    );

    Ok((success, failed))
}

/// Process a batch of textures with NVTT3 - per-file processing (fallback)
/// Each file is processed independently with nvtt_resize_compress
/// Returns (success_count, failed_count)
pub fn process_batch_nvtt3_perfile(
    batch: &[ProcessingRecord],
    format: Option<&str>,
    nvtt_tool_path: &Path,
    lib_path: Option<&Path>,
) -> Result<(usize, usize)> {
    if batch.is_empty() {
        return Ok((0, 0));
    }

    let format_name = format.unwrap_or("BC7");
    let parallel_jobs = rayon::current_num_threads();

    info!(
        "NVTT3 (per-file): Processing {} {} textures ({} parallel jobs)",
        batch.len(),
        format_name,
        parallel_jobs
    );

    let total_success = AtomicUsize::new(0);
    let total_failed = AtomicUsize::new(0);

    // Create progress bar
    let pb = ProgressBar::new(batch.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.green/black} {pos}/{len} NVTT3 {msg} (ETA: {eta})")
            .unwrap()
            .progress_chars("=>-"),
    );
    pb.set_message(format!("[{}]", format_name));

    // Process in parallel using the thread pool configured by optimize_all
    batch.par_iter().for_each(|record| {
        match process_single_texture_nvtt3(record, format, nvtt_tool_path, lib_path) {
            Ok(_) => {
                total_success.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                error!("NVTT3 failed for {}: {}", record.internal_path, e);
                total_failed.fetch_add(1, Ordering::Relaxed);
            }
        }
        pb.inc(1);
    });

    let success = total_success.into_inner();
    let failed = total_failed.into_inner();

    pb.finish_with_message(format!("Complete: {} success, {} failed", success, failed));

    info!(
        "NVTT3 complete: {} succeeded, {} failed",
        success, failed
    );

    Ok((success, failed))
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

/// Optimize all texture groups using the specified backend
pub fn optimize_all(
    groups: &ProcessingGroups,
    tools: &CompressionTools,
    backend: CompressionBackend,
    thread_count: Option<usize>,
) -> Result<OptimizationStats> {
    let num_threads = thread_count.unwrap_or_else(num_cpus::get);

    info!(
        "Starting optimization of {} textures with {} threads using {}...",
        groups.total(),
        num_threads,
        backend.name()
    );
    info!(
        "Groups: Delete={}, BC7={}, BC4={}, RGBA={}, PBR={}, Specular={}, Emissive={}, Gloss={}, Skipped(<512)={}",
        groups.delete_only.len(),
        groups.bc7_resize.len(),
        groups.bc4_resize.len(),
        groups.rgba_resize.len(),
        groups.pbr_resize.len(),
        groups.specular_resize.len(),
        groups.emissive_resize.len(),
        groups.gloss_resize.len(),
        groups.skipped_small
    );

    // Verify backend is available
    match backend {
        CompressionBackend::Texconv => {
            if tools.texconv_path.is_none() {
                anyhow::bail!("texconv.exe not found");
            }
        }
        CompressionBackend::Nvtt3 => {
            if tools.nvtt3_path.is_none() {
                anyhow::bail!("nvtt_resize_compress (NVTT3) not found in tools/nvtt3/");
            }
        }
    }

    // Create thread pool
    let pool = ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()?;

    let start_time = std::time::Instant::now();
    let mut stats = OptimizationStats::default();
    stats.skipped_small = groups.skipped_small;

    // Process delete-only (already optimal)
    if !groups.delete_only.is_empty() {
        info!("Processing {} delete-only textures...", groups.delete_only.len());
        let (success, failed) = process_delete_batch(&groups.delete_only)?;
        stats.deleted += success;
        stats.failed += failed;
    }

    // Helper macro to process a batch with the selected backend
    macro_rules! process_group {
        ($group:expr, $format:expr, $name:expr) => {
            if !$group.is_empty() {
                info!("Processing {} {} textures...", $group.len(), $name);
                let (success, failed) = match backend {
                    CompressionBackend::Texconv => {
                        let texconv_path = tools.texconv_path.as_ref().unwrap();
                        pool.install(|| process_batch_texconv(&$group, $format, texconv_path))?
                    }
                    CompressionBackend::Nvtt3 => {
                        // nvtt_resize_compress handles resize + compress in one CUDA step
                        // Falls back to texconv for files NVTT3 can't load (malformed DDS headers)
                        let nvtt3_path = tools.nvtt3_path.as_ref().unwrap();
                        let lib_path = tools.nvtt3_lib_path.as_deref();
                        let texconv_fallback = tools.texconv_path.as_deref();
                        pool.install(|| process_batch_nvtt3(&$group, $format, nvtt3_path, lib_path, texconv_fallback))?
                    }
                };
                stats.optimized += success;
                stats.failed += failed;
            }
        };
    }

    // Process all texture groups
    process_group!(groups.bc7_resize, Some("BC7"), "BC7");
    process_group!(groups.bc4_resize, Some("BC4"), "BC4");
    process_group!(groups.rgba_resize, Some("RGBA"), "RGBA");
    process_group!(groups.pbr_resize, None, "PBR");
    process_group!(groups.specular_resize, Some("BC4"), "Specular");  // BC4 for grayscale specular (0.5 bytes/pixel)
    process_group!(groups.emissive_resize, Some("BC1"), "Emissive");  // BC1 for emissive (has color)
    process_group!(groups.gloss_resize, Some("BC4"), "Gloss");

    let elapsed = start_time.elapsed();
    stats.duration = elapsed;

    info!(
        "Optimization complete in {:.2?}: {} optimized, {} deleted, {} failed, {} skipped (<512)",
        elapsed, stats.optimized, stats.deleted, stats.failed, stats.skipped_small
    );

    Ok(stats)
}

/// Legacy function for backwards compatibility - uses texconv
pub fn optimize_all_legacy(
    groups: &ProcessingGroups,
    texconv_path: &Path,
    thread_count: Option<usize>,
) -> Result<OptimizationStats> {
    let tools = CompressionTools {
        texconv_path: Some(texconv_path.to_path_buf()),
        nvtt3_path: None,
        nvtt3_batch_path: None,
        nvtt3_lib_path: None,
    };
    optimize_all(groups, &tools, CompressionBackend::Texconv, thread_count)
}

#[derive(Debug, Default)]
pub struct OptimizationStats {
    pub optimized: usize,
    pub deleted: usize,
    pub failed: usize,
    pub skipped_small: usize,  // Textures skipped due to being < 512x512
    pub duration: std::time::Duration,
}
