use std::path::PathBuf;

/// Represents a single texture in the game
#[derive(Debug, Clone)]
pub struct TextureRecord {
    /// Internal path (e.g., "textures/actors/character/female/femalebody_1.dds")
    pub internal_path: String,

    /// Actual file path (for loose files) or BSA path (for archived files)
    pub actual_path: PathBuf,

    /// Source: either "loose", BSA name, or "Vanilla"
    pub source: String,

    /// File size in bytes
    pub file_size: u64,

    /// Width in pixels (parsed from DDS header)
    pub width: Option<u32>,

    /// Height in pixels (parsed from DDS header)
    pub height: Option<u32>,

    /// Format (BC1, BC3, BC7, etc.)
    pub format: Option<String>,

    /// Texture type (will be classified later: diffuse, normal, etc.)
    pub texture_type: Option<String>,

    /// Number of lower-priority mods that also provide this texture
    pub conflict_count: usize,
}

impl TextureRecord {
    /// Create a new texture record for a loose file
    pub fn from_loose_file(internal_path: String, file_path: PathBuf, file_size: u64) -> Self {
        Self {
            internal_path,
            actual_path: file_path,
            source: "loose".to_string(),
            file_size,
            width: None,
            height: None,
            format: None,
            texture_type: None,
            conflict_count: 0,
        }
    }

    /// Create a new texture record for a BSA file
    pub fn from_bsa_file(
        internal_path: String,
        bsa_path: PathBuf,
        bsa_name: String,
        file_size: u64,
    ) -> Self {
        Self {
            internal_path,
            actual_path: bsa_path,
            source: bsa_name,
            file_size,
            width: None,
            height: None,
            format: None,
            texture_type: None,
            conflict_count: 0,
        }
    }

    /// Check if DDS header has been parsed
    pub fn has_header_info(&self) -> bool {
        self.width.is_some() && self.height.is_some() && self.format.is_some()
    }

    /// Get resolution as string (e.g., "2048x2048")
    pub fn resolution_string(&self) -> Option<String> {
        match (self.width, self.height) {
            (Some(w), Some(h)) => Some(format!("{}x{}", w, h)),
            _ => None,
        }
    }
}
