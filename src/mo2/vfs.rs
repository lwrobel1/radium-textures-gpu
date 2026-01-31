use super::profile::{Mod, Profile};
use anyhow::Result;
use log::{debug, info};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;
use walkdir::WalkDir;

/// Source of a file in the virtual file system
#[derive(Debug, Clone)]
pub enum FileSource {
    /// File from game Data directory (vanilla)
    Vanilla(PathBuf),
    /// File from a mod
    Mod {
        mod_name: String,
        mod_priority: usize,
        file_path: PathBuf,
    },
    /// File from a BSA archive
    Bsa {
        archive_name: String,
        internal_path: String,
        archive_path: PathBuf,
    },
}

impl FileSource {
    /// Get the priority of this source (higher = wins conflicts)
    pub fn priority(&self) -> usize {
        match self {
            FileSource::Vanilla(_) => 0,
            FileSource::Mod { mod_priority, .. } => *mod_priority,
            FileSource::Bsa { .. } => 0, // BSAs handled separately by load order
        }
    }

    /// Get the physical path (if not in BSA)
    pub fn physical_path(&self) -> Option<&Path> {
        match self {
            FileSource::Vanilla(p) | FileSource::Mod { file_path: p, .. } => Some(p),
            FileSource::Bsa { .. } => None,
        }
    }
}

/// Virtual File System that simulates MO2's file layering
///
/// In MO2:
/// - Game Data files are the base layer (lowest priority)
/// - Mods override in priority order (higher priority = top of mod list)
/// - BSAs are loaded based on plugin order
/// - Loose files always beat BSA files at the same priority level
pub struct VirtualFileSystem {
    /// Map of normalized virtual path -> winning file source
    /// Virtual path is relative to Data/ (e.g., "textures/foo/bar.dds")
    file_map: HashMap<String, FileSource>,
    profile: Profile,
}

impl VirtualFileSystem {
    /// Build the VFS from an MO2 profile
    pub fn new(profile: Profile) -> Result<Self> {
        let mut vfs = Self {
            file_map: HashMap::new(),
            profile,
        };

        vfs.build_file_map()?;
        Ok(vfs)
    }

    /// Build the complete file map by layering all sources
    fn build_file_map(&mut self) -> Result<()> {
        info!("Building VFS file map...");

        // Layer 1: Vanilla game files (lowest priority)
        self.index_vanilla_files()?;

        // Layer 2: Enabled mods (top of modlist = highest priority)
        // Higher priority mods are processed first; lower priority mods
        // only insert if no higher-priority file exists for that path
        // Collect first to avoid borrow checker issues
        let enabled_mods: Vec<_> = self.profile.enabled_mods().cloned().collect();
        for mod_entry in &enabled_mods {
            self.index_mod_files(mod_entry)?;
        }

        info!(
            "VFS built with {} unique file entries",
            self.file_map.len()
        );
        Ok(())
    }

    /// Index vanilla game Data directory
    fn index_vanilla_files(&mut self) -> Result<()> {
        let data_dir = &self.profile.game_data_dir;
        debug!("Checking Game Data directory: {:?}", data_dir);
        if !data_dir.exists() {
            info!("Game Data directory not found: {:?}", data_dir);
            return Ok(());
        }
        debug!("Game Data directory found, indexing vanilla files...");

        let mut count = 0;
        for entry in WalkDir::new(data_dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            // Get relative path from Data/
            if let Ok(rel_path) = path.strip_prefix(data_dir) {
                let virtual_path = self.normalize_path(rel_path);

                self.file_map.insert(
                    virtual_path,
                    FileSource::Vanilla(path.to_path_buf()),
                );
                count += 1;
            }
        }

        debug!("Indexed {} vanilla files", count);
        Ok(())
    }

    /// Index files from a single mod
    fn index_mod_files(&mut self, mod_entry: &Mod) -> Result<()> {
        if !mod_entry.path.exists() {
            debug!("Mod path doesn't exist, skipping: {:?}", mod_entry.path);
            return Ok(());
        }

        let mut count = 0;
        for entry in WalkDir::new(&mod_entry.path)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            // Get relative path from mod root
            if let Ok(rel_path) = path.strip_prefix(&mod_entry.path) {
                let virtual_path = self.normalize_path(rel_path);

                // Insert or overwrite - higher priority wins
                self.file_map
                    .entry(virtual_path)
                    .and_modify(|existing| {
                        if mod_entry.priority > existing.priority() {
                            *existing = FileSource::Mod {
                                mod_name: mod_entry.name.clone(),
                                mod_priority: mod_entry.priority,
                                file_path: path.to_path_buf(),
                            };
                        }
                    })
                    .or_insert_with(|| FileSource::Mod {
                        mod_name: mod_entry.name.clone(),
                        mod_priority: mod_entry.priority,
                        file_path: path.to_path_buf(),
                    });

                count += 1;
            }
        }

        debug!("Indexed {} files from mod: {}", count, mod_entry.name);
        Ok(())
    }

    /// Normalize a path for case-insensitive comparison
    /// Windows file systems are case-insensitive, so we normalize to lowercase
    /// Also normalize Unicode to NFC form
    fn normalize_path(&self, path: &Path) -> String {
        path.to_string_lossy()
            .to_lowercase()
            .nfc()
            .collect::<String>()
            .replace('\\', "/")
    }

    /// Get the winning file source for a virtual path
    pub fn get_file(&self, virtual_path: &str) -> Option<&FileSource> {
        let normalized = virtual_path.to_lowercase().nfc().collect::<String>();
        self.file_map.get(&normalized)
    }

    /// Get all sources for a virtual path (shows conflict resolution)
    /// Returns Vec of (mod_name, priority, path) sorted by priority (lowest to highest)
    /// This is case-insensitive to handle Linux filesystems
    pub fn get_file_layers(&self, virtual_path: &str) -> Vec<(String, usize, PathBuf)> {
        let normalized = self.normalize_path(Path::new(virtual_path));
        let mut layers = Vec::new();

        // Search through all indexed files for ones matching this normalized path
        // This handles case-insensitivity since we normalized everything during indexing
        for (indexed_path, source) in &self.file_map {
            if indexed_path == &normalized {
                match source {
                    FileSource::Vanilla(path) => {
                        layers.push(("Vanilla".to_string(), 0, path.clone()));
                    }
                    FileSource::Mod { mod_name, mod_priority, file_path } => {
                        layers.push((mod_name.clone(), *mod_priority, file_path.clone()));
                    }
                    FileSource::Bsa { .. } => {
                        // BSAs not implemented yet
                    }
                }
            }
        }

        // Sort by priority (lowest first, highest last = winner)
        layers.sort_by_key(|(_, priority, _)| *priority);
        layers
    }

    /// Get all texture files (DDS) from the VFS
    pub fn get_texture_files(&self) -> impl Iterator<Item = (&String, &FileSource)> {
        self.file_map.iter().filter(|(path, _)| {
            path.ends_with(".dds")
                && path.starts_with("textures/")
        })
    }

    /// Get count of files by source type
    pub fn get_statistics(&self) -> VfsStatistics {
        let mut stats = VfsStatistics::default();

        for source in self.file_map.values() {
            match source {
                FileSource::Vanilla(_) => stats.vanilla_files += 1,
                FileSource::Mod { .. } => stats.mod_files += 1,
                FileSource::Bsa { .. } => stats.bsa_files += 1,
            }
        }

        stats.total_files = self.file_map.len();
        stats
    }

    /// Get reference to the profile
    pub fn profile(&self) -> &Profile {
        &self.profile
    }
}

#[derive(Debug, Default)]
pub struct VfsStatistics {
    pub total_files: usize,
    pub vanilla_files: usize,
    pub mod_files: usize,
    pub bsa_files: usize,
}

impl std::fmt::Display for VfsStatistics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "VFS Stats: {} total ({} vanilla, {} from mods, {} from BSAs)",
            self.total_files, self.vanilla_files, self.mod_files, self.bsa_files
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_normalization() {
        // Test case insensitivity and path separator normalization
    }

    #[test]
    fn test_priority_resolution() {
        // Test that higher priority mods win conflicts
    }
}
