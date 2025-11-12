/// Exclusions system for texture optimization
/// Supports wildcard patterns and path matching
/// Game-specific exclusion files are located in ./exclusions/

use anyhow::Result;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub struct ExclusionList {
    /// Wildcard filename patterns (e.g., "*_color.dds", "*DrJ*.dds")
    filename_patterns: Vec<String>,

    /// Path/directory exclusions (e.g., "\terrain", "\actors\dragon")
    path_exclusions: Vec<String>,

    /// Exact filename matches (e.g., "dummy.dds")
    exact_matches: Vec<String>,
}

impl ExclusionList {
    /// Load exclusions from file
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let file = File::open(path.as_ref())?;
        let reader = BufReader::new(file);

        let mut filename_patterns = Vec::new();
        let mut path_exclusions = Vec::new();
        let mut exact_matches = Vec::new();

        for line in reader.lines() {
            let line = line?;
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
                continue;
            }

            // Path exclusions start with backslash
            if line.starts_with('\\') {
                // Normalize to forward slashes and lowercase for case-insensitive matching
                let normalized = line[1..].replace('\\', "/").to_lowercase();
                path_exclusions.push(normalized);
            }
            // Wildcard patterns contain asterisks
            else if line.contains('*') {
                // Normalize to lowercase for case-insensitive matching
                filename_patterns.push(line.to_lowercase());
            }
            // Exact matches
            else {
                exact_matches.push(line.to_lowercase());
            }
        }

        Ok(Self {
            filename_patterns,
            path_exclusions,
            exact_matches,
        })
    }

    /// Check if a texture path should be excluded
    /// Path should be normalized (lowercase, forward slashes)
    pub fn should_exclude(&self, texture_path: &str) -> bool {
        let lower_path = texture_path.to_lowercase();

        // Check path exclusions first (most common)
        for path_pattern in &self.path_exclusions {
            if lower_path.contains(path_pattern) {
                return true;
            }
        }

        // Extract filename from path
        let filename = lower_path
            .rsplit('/')
            .next()
            .unwrap_or(&lower_path);

        // Check exact matches
        if self.exact_matches.contains(&filename.to_string()) {
            return true;
        }

        // Check wildcard patterns
        for pattern in &self.filename_patterns {
            if Self::matches_wildcard(filename, pattern) {
                return true;
            }
        }

        false
    }

    /// Simple wildcard matching (* matches any characters)
    fn matches_wildcard(text: &str, pattern: &str) -> bool {
        // Split pattern by asterisks
        let parts: Vec<&str> = pattern.split('*').collect();

        if parts.is_empty() {
            return text.is_empty();
        }

        let mut text_pos = 0;

        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }

            // First part must match at start
            if i == 0 {
                if !text.starts_with(part) {
                    return false;
                }
                text_pos = part.len();
                continue;
            }

            // Last part must match at end
            if i == parts.len() - 1 && !pattern.ends_with('*') {
                return text.ends_with(part);
            }

            // Middle parts must exist in order
            if let Some(pos) = text[text_pos..].find(part) {
                text_pos += pos + part.len();
            } else {
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wildcard_matching() {
        assert!(ExclusionList::matches_wildcard("test_color.dds", "*_color.dds"));
        assert!(ExclusionList::matches_wildcard("DrJ_grass.dds", "*DrJ*.dds"));
        assert!(ExclusionList::matches_wildcard("icewall01.dds", "icewall*.dds"));
        assert!(ExclusionList::matches_wildcard("mydummy.dds", "*dummy.dds"));

        assert!(!ExclusionList::matches_wildcard("test_normal.dds", "*_color.dds"));
        assert!(!ExclusionList::matches_wildcard("grass.dds", "*DrJ*.dds"));
    }

    #[test]
    fn test_path_exclusions() {
        let mut list = ExclusionList {
            filename_patterns: vec![],
            path_exclusions: vec!["terrain".to_string(), "actors/dragon".to_string()],
            exact_matches: vec![],
        };

        assert!(list.should_exclude("textures/terrain/rocks/rock01.dds"));
        assert!(list.should_exclude("textures/actors/dragon/dragonscale.dds"));
        assert!(!list.should_exclude("textures/actors/character/player.dds"));
    }

    #[test]
    fn test_filename_patterns() {
        let list = ExclusionList {
            filename_patterns: vec!["*_color.dds".to_string(), "*lod*_p.dds".to_string()],
            path_exclusions: vec![],
            exact_matches: vec!["dummy.dds".to_string()],
        };

        assert!(list.should_exclude("textures/test_color.dds"));
        assert!(list.should_exclude("textures/mountainlod01_p.dds"));
        assert!(list.should_exclude("textures/dummy.dds"));
        assert!(!list.should_exclude("textures/normal.dds"));
    }
}
