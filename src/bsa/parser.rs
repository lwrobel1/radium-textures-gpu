use super::types::*;
use anyhow::{Result, Context};
use ba2::Reader; // Import the Reader trait
use log::debug;
use std::path::Path;

/// BSA Archive parser using ba2 crate
pub struct BsaArchive {
    pub header: BsaHeader,
    pub files: Vec<BsaFile>,
    archive_path: std::path::PathBuf,
    // Keep the ba2 archive alive for streaming later
    _archive: ba2::tes4::Archive<'static>,
}

impl BsaArchive {
    /// Open and parse a BSA archive
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        debug!("Opening BSA: {:?}", path);

        // Use ba2 crate to parse the archive with the Reader trait
        let (archive, _options) = ba2::tes4::Archive::read(path)
            .with_context(|| format!("Failed to parse BSA: {:?}", path))?;

        debug!("BSA parsed successfully: {} folders", archive.len());

        // Count total files across all directories
        let total_files: usize = archive.iter()
            .map(|(_, dir)| dir.len())
            .sum();

        debug!("Total files in BSA: {}", total_files);

        // Extract header information
        let header = BsaHeader {
            magic: 0x00415342, // "BSA\0"
            version: 105,      // SSE version
            offset: 36,
            archive_flags: 0,  // ba2 crate abstracts this away
            folder_count: archive.len() as u32,
            file_count: total_files as u32,
            total_folder_name_length: 0,
            total_file_name_length: 0,
            file_flags: 0,
        };

        // Convert ba2 files to our BsaFile format
        let mut files = Vec::new();
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

                // Normalize path separators to forward slashes
                let normalized_path = full_path.replace('\\', "/");

                files.push(BsaFile {
                    path: normalized_path,
                    offset: 0,  // ba2 abstracts this away
                    size: file.len() as u32,
                    compression: CompressionType::None, // ba2 handles decompression automatically
                    compressed: false, // ba2 returns decompressed data
                });
            }
        }

        debug!("Parsed {} files from BSA", files.len());

        Ok(Self {
            header,
            files,
            archive_path: path.to_path_buf(),
            _archive: archive,
        })
    }

    /// Get all DDS texture files from this archive
    pub fn get_textures(&self) -> impl Iterator<Item = &BsaFile> {
        self.files.iter().filter(|f| {
            f.path.to_lowercase().ends_with(".dds")
                && f.path.to_lowercase().starts_with("textures")
        })
    }

    /// Get the archive path
    pub fn path(&self) -> &Path {
        &self.archive_path
    }
}
