/// Texture extraction from BSA archives and loose files
/// Extracts textures to a working directory for optimization

use anyhow::Result;
use log::{debug, info, warn};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::database::TextureRecord;

/// Extract texture from BSA or copy from loose file
pub fn extract_texture(record: &TextureRecord, output_dir: &Path) -> Result<PathBuf> {
    // Create output path preserving internal structure (keep textures/ prefix)
    let internal_path = &record.internal_path;

    // DEBUG: Log the paths
    debug!("extract_texture: output_dir = {:?}", output_dir);
    debug!("extract_texture: internal_path = {}", internal_path);

    // Keep the full path including "textures/" prefix so files go into textures/ subdirectory
    let output_path = output_dir.join(internal_path);
    debug!("extract_texture: output_path = {:?}", output_path);

    // Create parent directories
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Extract based on source type
    if record.source == "loose" {
        // Copy loose file
        fs::copy(&record.actual_path, &output_path)?;
        debug!("Copied loose file: {} -> {:?}", record.internal_path, output_path);
    } else {
        // Extract from BSA (source contains BSA filename)
        extract_from_bsa(record, &output_path)?;
        debug!("Extracted from BSA {}: {} -> {:?}", record.source, record.internal_path, output_path);
    }

    Ok(output_path)
}

/// Extract a single file from BSA archive
fn extract_from_bsa(record: &TextureRecord, output_path: &Path) -> Result<()> {
    use ba2::Reader;

    // Open BSA archive
    let archive_path = &record.actual_path;
    let (archive, _options) = ba2::tes4::Archive::read(archive_path.as_path())?;

    // Parse internal path to directory and filename
    let internal_clean = record.internal_path.replace('/', "\\");
    let parts: Vec<&str> = internal_clean.rsplitn(2, '\\').collect();

    let (file_name, dir_name) = if parts.len() == 2 {
        (parts[0], parts[1])
    } else {
        (parts[0], "")
    };

    // Find the file in the archive
    let mut found = false;
    let dir_name_lower = dir_name.to_lowercase();
    let file_name_lower = file_name.to_lowercase();

    for (dir_key, directory) in archive.iter() {
        let dir_name_actual = dir_key.name().to_string().to_lowercase();

        // Case-insensitive comparison
        if dir_name_actual == dir_name_lower {
            for (file_key, file) in directory.iter() {
                let file_name_actual = file_key.name().to_string().to_lowercase();

                if file_name_actual == file_name_lower {
                    // Found the file - extract it
                    let data = if file.is_decompressed() {
                        file.as_bytes().to_vec()
                    } else {
                        // Try standard decompression first
                        match file.decompress(&Default::default()) {
                            Ok(decompressed) => decompressed.as_bytes().to_vec(),
                            Err(_) => {
                                // Try LZ4 fallback
                                try_lz4_decompress(file.as_bytes())?
                            }
                        }
                    };

                    // Write to output file
                    let mut output_file = File::create(output_path)?;
                    output_file.write_all(&data)?;

                    found = true;
                    break;
                }
            }
        }

        if found {
            break;
        }
    }

    if !found {
        anyhow::bail!("File not found in BSA: {}", record.internal_path);
    }

    Ok(())
}

/// Try LZ4 decompression as fallback
fn try_lz4_decompress(compressed_data: &[u8]) -> Result<Vec<u8>> {
    use std::io::Read;

    let mut decoder = lz4::Decoder::new(compressed_data)?;
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)?;
    Ok(decompressed)
}

/// Extract all textures that need optimization
pub fn extract_all_textures(
    textures: &[(String, TextureRecord, u32, u32)],
    output_dir: &Path,
) -> Result<Vec<(String, TextureRecord, u32, u32, PathBuf)>> {
    info!("Extracting {} textures to {:?}...", textures.len(), output_dir);

    // Create output directory
    fs::create_dir_all(output_dir)?;

    let start_time = std::time::Instant::now();
    let mut extracted = Vec::new();
    let mut failed = 0;

    for (internal_path, record, target_width, target_height) in textures {
        match extract_texture(record, output_dir) {
            Ok(extracted_path) => {
                extracted.push((
                    internal_path.clone(),
                    record.clone(),
                    *target_width,
                    *target_height,
                    extracted_path,
                ));
            }
            Err(e) => {
                warn!("Failed to extract {}: {}", internal_path, e);
                failed += 1;
            }
        }
    }

    let elapsed = start_time.elapsed();
    info!(
        "Extracted {} textures ({} failed) in {:.2?}",
        extracted.len(),
        failed,
        elapsed
    );

    Ok(extracted)
}
