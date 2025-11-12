use super::TextureRecord;
use crate::bsa::BsaArchive;
use crate::dds;
use crate::mo2::VirtualFileSystem;
use anyhow::Result;
use ba2::Reader; // For BSA file extraction
use log::{debug, info, warn};
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;

/// Service for discovering all textures from mods (loose files and BSAs)
/// Uses in-memory HashMap for fast lookups (like sky-tex-opti)
pub struct TextureDiscoveryService;

impl TextureDiscoveryService {
    /// Try LZ4 decompression as fallback when deflate fails
    /// BSA files may use LZ4 compression that ba2's deflate decoder can't handle
    fn try_lz4_decompress(compressed_data: &[u8]) -> Result<Vec<u8>> {
        use std::io::Read;

        // Try LZ4 decompression
        let mut decoder = lz4::Decoder::new(compressed_data)?;
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)?;
        Ok(decompressed)
    }

    /// Classify texture type based on filename suffix
    /// Skyrim uses specific naming conventions for different texture types
    fn classify_texture(path: &str) -> Option<String> {
        let lower = path.to_lowercase();

        // Remove .dds extension
        let without_ext = lower.strip_suffix(".dds")?;

        // Check for common Skyrim suffixes
        if without_ext.ends_with("_n") {
            Some("Normal".to_string())
        } else if without_ext.ends_with("_s") {
            Some("Specular".to_string())
        } else if without_ext.ends_with("_m") {
            Some("Parallax".to_string())
        } else if without_ext.ends_with("_sk") {
            Some("Subsurface".to_string())
        } else if without_ext.ends_with("_msn") {
            Some("Multi-layer".to_string())
        } else if without_ext.ends_with("_e") || without_ext.ends_with("_g") {
            Some("Emissive".to_string())
        } else if without_ext.ends_with("_em") {
            Some("Emissive Mask".to_string())
        } else if without_ext.ends_with("_envmap") || without_ext.ends_with("_env") {
            Some("Environment".to_string())
        } else {
            // No recognized suffix = Diffuse/Albedo
            Some("Diffuse".to_string())
        }
    }

    /// Discover all textures from the VFS
    /// Returns HashMap with lowercase internal paths as keys (case-insensitive)
    /// First write wins (highest priority mod)
    pub fn discover_from_vfs(vfs: &VirtualFileSystem) -> HashMap<String, TextureRecord> {
        let mut textures = HashMap::new();

        info!("Discovering textures from VFS...");
        let start_time = std::time::Instant::now();

        // Scan all texture files from VFS (loose files only for now)
        for (internal_path, source) in vfs.get_texture_files() {
            if let Some(file_path) = source.physical_path() {
                if file_path.exists() {
                    // Get file size
                    let file_size = match std::fs::metadata(file_path) {
                        Ok(meta) => meta.len(),
                        Err(_) => continue,
                    };

                    // Create record
                    let mut record = TextureRecord::from_loose_file(
                        internal_path.clone(),
                        file_path.to_path_buf(),
                        file_size,
                    );

                    // Classify texture type
                    record.texture_type = Self::classify_texture(&internal_path);

                    // Insert with case-insensitive key
                    textures.insert(internal_path.clone(), record);
                }
            }
        }

        let elapsed = start_time.elapsed();
        info!(
            "Discovered {} loose textures in {:.2?}",
            textures.len(),
            elapsed
        );

        textures
    }

    /// Discover textures from BSA archives
    /// Integrates with existing texture map (loose files take priority)
    /// Now parses DDS headers during discovery!
    pub fn discover_from_bsas(
        bsa_paths: &[(std::path::PathBuf, String, usize)],
        textures: &mut HashMap<String, TextureRecord>,
    ) -> Result<()> {
        info!("Discovering textures from {} BSA archives...", bsa_paths.len());
        let start_time = std::time::Instant::now();

        let mut bsa_count = 0;
        let mut texture_count = 0;
        let mut headers_parsed = 0;
        let mut parse_failed = 0;
        let mut too_small = 0;
        let mut decompress_failed = 0;
        let mut sample_errors: Vec<(String, String)> = Vec::new();

        // Process BSAs in reverse priority order (lowest priority first)
        // So higher priority BSAs overwrite lower ones
        for (bsa_path, mod_name, _priority) in bsa_paths.iter().rev() {
            if !bsa_path.exists() {
                continue;
            }

            // Re-open BSA with ba2 for streaming
            match ba2::tes4::Archive::read(bsa_path.as_path()) {
                Ok((archive, _options)) => {
                    bsa_count += 1;
                    let bsa_name = bsa_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown.bsa")
                        .to_string();

                    // Iterate through all directories and files
                    for (dir_key, directory) in archive.iter() {
                        let dir_name = dir_key.name();

                        for (file_key, file) in directory.iter() {
                            let file_name = file_key.name();

                            // Build full path
                            let full_path = if dir_name.is_empty() {
                                file_name.to_string()
                            } else {
                                format!("{}/{}", dir_name, file_name)
                            };

                            // Normalize and filter for textures
                            let normalized_path = full_path.replace('\\', "/").to_lowercase();
                            if !normalized_path.ends_with(".dds") || !normalized_path.starts_with("textures") {
                                continue;
                            }

                            // Only add if not already present (loose files win)
                            if !textures.contains_key(&normalized_path) {
                                texture_count += 1;

                                let mut record = TextureRecord::from_bsa_file(
                                    normalized_path.clone(),
                                    bsa_path.clone(),
                                    bsa_name.clone(),
                                    file.len() as u64,
                                );

                                // Classify texture type
                                record.texture_type = Self::classify_texture(&normalized_path);

                                // Extract and parse DDS header
                                // Files in SSE BSAs are typically already decompressed
                                let mut parse_failed_reason = None;

                                if file.is_decompressed() {
                                    let bytes = file.as_bytes();
                                    if bytes.len() < 128 {
                                        too_small += 1;
                                        parse_failed_reason = Some(format!("too small: {} bytes", bytes.len()));
                                    } else {
                                        let mut cursor = std::io::Cursor::new(bytes);
                                        match dds::parse_dds_header(&mut cursor) {
                                            Ok(header) => {
                                                record.width = Some(header.width);
                                                record.height = Some(header.height);
                                                record.format = Some(header.format);
                                                headers_parsed += 1;
                                            }
                                            Err(e) => {
                                                parse_failed += 1;
                                                parse_failed_reason = Some(format!("DDS parse error: {}", e));
                                            }
                                        }
                                    }
                                } else {
                                    // Try decompressing if it's compressed
                                    match file.decompress(&Default::default()) {
                                        Ok(decompressed) => {
                                            let bytes = decompressed.as_bytes();
                                            if bytes.len() < 128 {
                                                too_small += 1;
                                                parse_failed_reason = Some(format!("decompressed too small: {} bytes", bytes.len()));
                                            } else {
                                                let mut cursor = std::io::Cursor::new(bytes);
                                                match dds::parse_dds_header(&mut cursor) {
                                                    Ok(header) => {
                                                        record.width = Some(header.width);
                                                        record.height = Some(header.height);
                                                        record.format = Some(header.format);
                                                        headers_parsed += 1;
                                                    }
                                                    Err(e) => {
                                                        parse_failed += 1;
                                                        parse_failed_reason = Some(format!("DDS parse error after decompress: {}", e));
                                                    }
                                                }
                                            }
                                        }
                                        Err(_e) => {
                                            // Deflate decompression failed - try LZ4
                                            // Many Skyrim BSAs use LZ4 compression
                                            let compressed_bytes = file.as_bytes();
                                            match Self::try_lz4_decompress(compressed_bytes) {
                                                Ok(lz4_data) => {
                                                    if lz4_data.len() < 128 {
                                                        too_small += 1;
                                                        parse_failed_reason = Some(format!("LZ4 decompressed file too small"));
                                                    } else {
                                                        let mut cursor = std::io::Cursor::new(&lz4_data);
                                                        match dds::parse_dds_header(&mut cursor) {
                                                            Ok(header) => {
                                                                record.width = Some(header.width);
                                                                record.height = Some(header.height);
                                                                record.format = Some(header.format);
                                                                headers_parsed += 1;
                                                                // Successfully parsed via LZ4 decompression
                                                            }
                                                            Err(e) => {
                                                                parse_failed += 1;
                                                                parse_failed_reason = Some(format!("LZ4 decompressed but DDS parse failed: {}", e));
                                                            }
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    decompress_failed += 1;
                                                    parse_failed_reason = Some(format!("both deflate and LZ4 decompression failed: {}", e));
                                                }
                                            }
                                        }
                                    }
                                }

                                if let Some(reason) = parse_failed_reason {
                                    debug!("Failed to parse BSA texture {} from {}: {}", normalized_path, bsa_name, reason);
                                    if sample_errors.len() < 10 {
                                        sample_errors.push((format!("{} ({})", normalized_path, bsa_name), reason));
                                    }
                                }

                                textures.insert(normalized_path, record);
                            } else {
                                // Increment conflict count
                                if let Some(existing) = textures.get_mut(&normalized_path) {
                                    existing.conflict_count += 1;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to parse BSA {}: {}", mod_name, e);
                }
            }
        }

        let elapsed = start_time.elapsed();
        info!(
            "Discovered {} textures from {} BSAs in {:.2?}",
            texture_count, bsa_count, elapsed
        );
        info!("  Parsed {} DDS headers from BSA files", headers_parsed);

        let failed_total = too_small + parse_failed + decompress_failed;
        if failed_total > 0 {
            info!("  Failed to parse {} BSA textures:", failed_total);
            if too_small > 0 {
                info!("    - {} files too small (< 128 bytes)", too_small);
            }
            if parse_failed > 0 {
                info!("    - {} DDS header parse errors", parse_failed);
            }
            if decompress_failed > 0 {
                info!("    - {} decompression failures", decompress_failed);
            }
            if !sample_errors.is_empty() {
                info!("  Sample errors:");
                for (path, reason) in sample_errors.iter().take(5) {
                    info!("    {} - {}", path, reason);
                }
            }
        }

        Ok(())
    }

    /// Parse DDS headers for all textures
    /// Updates width, height, and format fields
    pub fn parse_dds_headers(textures: &mut HashMap<String, TextureRecord>) -> Result<()> {
        info!("Parsing DDS headers for {} textures...", textures.len());
        let start_time = std::time::Instant::now();

        let mut parsed = 0;
        let mut failed = 0;

        for record in textures.values_mut() {
            // Only parse loose files for now (BSA streaming comes later)
            if record.source == "loose" {
                if let Ok(mut file) = File::open(&record.actual_path) {
                    match dds::parse_dds_header(&mut file) {
                        Ok(header) => {
                            record.width = Some(header.width);
                            record.height = Some(header.height);
                            record.format = Some(header.format);
                            parsed += 1;
                        }
                        Err(e) => {
                            debug!(
                                "Failed to parse DDS header for {}: {}",
                                record.internal_path, e
                            );
                            failed += 1;
                        }
                    }
                }
            }
        }

        let elapsed = start_time.elapsed();
        info!(
            "Parsed {} DDS headers ({} failed) in {:.2?}",
            parsed, failed, elapsed
        );

        Ok(())
    }

    /// Get statistics about discovered textures
    pub fn get_statistics(textures: &HashMap<String, TextureRecord>) -> TextureStats {
        let mut stats = TextureStats::default();

        stats.total = textures.len();

        for record in textures.values() {
            if record.source == "loose" {
                stats.loose += 1;
            } else {
                stats.bsa += 1;
            }

            if record.has_header_info() {
                stats.parsed += 1;

                // Count by format
                if let Some(format) = &record.format {
                    *stats.by_format.entry(format.clone()).or_insert(0) += 1;
                }

                // Count by resolution
                if let Some(res) = record.resolution_string() {
                    *stats.by_resolution.entry(res).or_insert(0) += 1;
                }
            }

            // Count by texture type
            if let Some(texture_type) = &record.texture_type {
                *stats.by_type.entry(texture_type.clone()).or_insert(0) += 1;
            }
        }

        stats
    }
}

#[derive(Debug, Default)]
pub struct TextureStats {
    pub total: usize,
    pub loose: usize,
    pub bsa: usize,
    pub parsed: usize,
    pub by_format: HashMap<String, usize>,
    pub by_resolution: HashMap<String, usize>,
    pub by_type: HashMap<String, usize>,
}

impl std::fmt::Display for TextureStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "=== Texture Discovery Statistics ===")?;
        writeln!(f, "Total textures: {}", self.total)?;
        writeln!(f, "  Loose files: {}", self.loose)?;
        writeln!(f, "  From BSAs: {}", self.bsa)?;
        writeln!(f, "  Parsed headers: {}", self.parsed)?;

        if !self.by_type.is_empty() {
            writeln!(f, "\nBy texture type:")?;
            let mut types: Vec<_> = self.by_type.iter().collect();
            types.sort_by(|a, b| b.1.cmp(a.1));
            for (texture_type, count) in types {
                writeln!(f, "  {}: {}", texture_type, count)?;
            }
        }

        if !self.by_format.is_empty() {
            writeln!(f, "\nBy format:")?;
            let mut formats: Vec<_> = self.by_format.iter().collect();
            formats.sort_by(|a, b| b.1.cmp(a.1));
            for (format, count) in formats.iter().take(10) {
                writeln!(f, "  {}: {}", format, count)?;
            }
        }

        if !self.by_resolution.is_empty() {
            writeln!(f, "\nTop 10 resolutions:")?;
            let mut resolutions: Vec<_> = self.by_resolution.iter().collect();
            resolutions.sort_by(|a, b| b.1.cmp(a.1));
            for (res, count) in resolutions.iter().take(10) {
                writeln!(f, "  {}: {}", res, count)?;
            }
        }

        Ok(())
    }
}
