use std::path::PathBuf;

/// BSA archive header
#[derive(Debug, Clone)]
pub struct BsaHeader {
    pub magic: u32,           // "BSA\0" = 0x00415342
    pub version: u32,         // 105 for SSE
    pub offset: u32,          // Usually 36
    pub archive_flags: u32,   // Archive-level flags
    pub folder_count: u32,
    pub file_count: u32,
    pub total_folder_name_length: u32,
    pub total_file_name_length: u32,
    pub file_flags: u32,
}

/// Compression type for BSA files
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionType {
    None,
    Zlib,
    Lz4,
}

/// A file entry in a BSA archive
#[derive(Debug, Clone)]
pub struct BsaFile {
    /// Full path within the archive (e.g., "textures/foo/bar.dds")
    pub path: String,
    /// Offset in the BSA file
    pub offset: u64,
    /// Size of the file data
    pub size: u32,
    /// Compression type
    pub compression: CompressionType,
    /// Whether this file is compressed (overrides archive default)
    pub compressed: bool,
}

/// BSA archive flags
pub const ARCHIVE_COMPRESSED: u32 = 0x0004;
pub const ARCHIVE_INCLUDE_DIR_NAMES: u32 = 0x0001;
pub const ARCHIVE_INCLUDE_FILE_NAMES: u32 = 0x0002;
pub const ARCHIVE_PREFIX_FULLFILENAMES: u32 = 0x0100;

/// File flags
pub const FILE_COMPRESSED: u32 = 0x40000000;

/// BSA magic number "BSA\0"
pub const BSA_MAGIC: u32 = 0x00415342;
pub const SSE_VERSION: u32 = 105;
