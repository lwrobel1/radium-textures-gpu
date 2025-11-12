use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use log::{info, warn, debug};

/// Represents the state of a mod in MO2
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModState {
    Enabled,
    Disabled,
}

/// Represents a mod with its metadata
#[derive(Debug, Clone)]
pub struct Mod {
    pub name: String,
    pub path: PathBuf,
    pub priority: usize,  // Higher number = higher priority (wins conflicts)
    pub state: ModState,
}

/// MO2 Profile parser - handles modlist.txt, loadorder.txt, and archives.txt
#[derive(Debug)]
pub struct Profile {
    pub profile_path: PathBuf,
    pub mods_dir: PathBuf,
    pub game_data_dir: PathBuf,
    pub mods: Vec<Mod>,
    pub plugin_order: Vec<String>,
    pub additional_archives: Vec<String>,
}

impl Profile {
    /// Load an MO2 profile from the given path
    ///
    /// # Arguments
    /// * `profile_path` - Path to MO2 profile directory (contains modlist.txt)
    /// * `mods_dir` - Path to MO2 mods directory
    /// * `game_data_dir` - Path to game Data directory
    pub fn load(
        profile_path: impl AsRef<Path>,
        mods_dir: impl AsRef<Path>,
        game_data_dir: impl AsRef<Path>,
    ) -> Result<Self> {
        let profile_path = profile_path.as_ref().to_path_buf();
        let mods_dir = mods_dir.as_ref().to_path_buf();
        let game_data_dir = game_data_dir.as_ref().to_path_buf();

        info!("Loading MO2 profile from: {:?}", profile_path);

        let modlist_path = profile_path.join("modlist.txt");
        let loadorder_path = profile_path.join("loadorder.txt");
        let archives_path = profile_path.join("archives.txt");

        // Parse modlist.txt - this defines mod priority
        let mods = Self::parse_modlist(&modlist_path, &mods_dir)?;
        info!("Loaded {} mods ({} enabled)",
            mods.len(),
            mods.iter().filter(|m| m.state == ModState::Enabled).count()
        );

        // Parse loadorder.txt - plugin load order
        let plugin_order = if loadorder_path.exists() {
            Self::parse_loadorder(&loadorder_path)?
        } else {
            warn!("loadorder.txt not found, skipping plugin parsing");
            Vec::new()
        };

        // Parse archives.txt - additional BSAs to load
        let additional_archives = if archives_path.exists() {
            Self::parse_archives(&archives_path)?
        } else {
            warn!("archives.txt not found, skipping additional archives");
            Vec::new()
        };

        Ok(Self {
            profile_path,
            mods_dir,
            game_data_dir,
            mods,
            plugin_order,
            additional_archives,
        })
    }

    /// Parse modlist.txt - format is:
    /// +ModName (enabled)
    /// -ModName (disabled)
    /// *ModName (separator, treated as disabled)
    ///
    /// CRITICAL: Mods at the TOP of the file have LOWEST priority
    ///           Mods at the BOTTOM have HIGHEST priority (win conflicts)
    fn parse_modlist(path: &Path, mods_dir: &Path) -> Result<Vec<Mod>> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read modlist.txt from {:?}", path))?;

        let mut mods = Vec::new();
        let mut priority = 0;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let (state, name) = match line.chars().next() {
                Some('+') => (ModState::Enabled, &line[1..]),
                Some('-') | Some('*') => (ModState::Disabled, &line[1..]),
                _ => {
                    warn!("Invalid modlist.txt line format: {}", line);
                    continue;
                }
            };

            let mod_path = mods_dir.join(name);

            mods.push(Mod {
                name: name.to_string(),
                path: mod_path,
                priority,
                state,
            });

            priority += 1;
        }

        debug!("Parsed {} mods from modlist.txt", mods.len());
        Ok(mods)
    }

    /// Parse loadorder.txt - list of plugin files (.esp, .esm, .esl)
    fn parse_loadorder(path: &Path) -> Result<Vec<String>> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read loadorder.txt from {:?}", path))?;

        let plugins: Vec<String> = content
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                // Handle both "Plugin.esp" and "*Plugin.esp" format
                if line.starts_with('*') {
                    line[1..].to_string()
                } else {
                    line.to_string()
                }
            })
            .collect();

        debug!("Parsed {} plugins from loadorder.txt", plugins.len());
        Ok(plugins)
    }

    /// Parse archives.txt - list of additional BSA files to load
    fn parse_archives(path: &Path) -> Result<Vec<String>> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read archives.txt from {:?}", path))?;

        let archives: Vec<String> = content
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .map(String::from)
            .collect();

        debug!("Parsed {} additional archives from archives.txt", archives.len());
        Ok(archives)
    }

    /// Get all enabled mods in priority order (lowest to highest)
    pub fn enabled_mods(&self) -> impl Iterator<Item = &Mod> {
        self.mods.iter().filter(|m| m.state == ModState::Enabled)
    }

    /// Get all enabled mods in reverse priority order (highest to lowest)
    /// This is useful when building a "winning file" map
    pub fn enabled_mods_reverse(&self) -> impl Iterator<Item = &Mod> {
        self.mods
            .iter()
            .rev()
            .filter(|m| m.state == ModState::Enabled)
    }

    /// Get BSA files from plugin load order, searching in mod folders
    /// Returns BSAs with their owning mod priority for proper VFS layering
    pub fn get_plugin_bsas(&self) -> Vec<(PathBuf, String, usize)> {
        let mut bsas = Vec::new();

        // First, check vanilla game Data directory
        for plugin in &self.plugin_order {
            let base_name = plugin
                .strip_suffix(".esp")
                .or_else(|| plugin.strip_suffix(".esm"))
                .or_else(|| plugin.strip_suffix(".esl"))
                .unwrap_or(plugin);

            // Vanilla BSAs in game Data directory
            let vanilla_bsa1 = self.game_data_dir.join(format!("{}.bsa", base_name));
            if vanilla_bsa1.exists() {
                debug!("Found vanilla BSA: {:?}", vanilla_bsa1);
                bsas.push((vanilla_bsa1, "Vanilla".to_string(), 0));
            }

            let vanilla_bsa2 = self.game_data_dir.join(format!("{} - Textures.bsa", base_name));
            if vanilla_bsa2.exists() {
                debug!("Found vanilla BSA: {:?}", vanilla_bsa2);
                bsas.push((vanilla_bsa2, "Vanilla".to_string(), 0));
            }
        }

        // Now search for BSAs in mod folders based on plugin ownership
        for plugin in &self.plugin_order {
            let base_name = plugin
                .strip_suffix(".esp")
                .or_else(|| plugin.strip_suffix(".esm"))
                .or_else(|| plugin.strip_suffix(".esl"))
                .unwrap_or(plugin);

            // Find which mod provides this plugin
            for mod_entry in self.enabled_mods() {
                // Check if this mod contains the plugin
                let plugin_path = mod_entry.path.join(plugin);
                if plugin_path.exists() {
                    // Look for associated BSAs in this mod's folder
                    let bsa1 = mod_entry.path.join(format!("{}.bsa", base_name));
                    if bsa1.exists() {
                        debug!("Found BSA in mod {}: {:?}", mod_entry.name, bsa1);
                        bsas.push((bsa1, mod_entry.name.clone(), mod_entry.priority));
                    }

                    let bsa2 = mod_entry.path.join(format!("{} - Textures.bsa", base_name));
                    if bsa2.exists() {
                        debug!("Found BSA in mod {}: {:?}", mod_entry.name, bsa2);
                        bsas.push((bsa2, mod_entry.name.clone(), mod_entry.priority));
                    }

                    break; // Found the mod, stop searching
                }
            }
        }

        // Also check archives.txt entries in mod folders
        for archive_name in &self.additional_archives {
            // Search in enabled mods
            for mod_entry in self.enabled_mods() {
                let archive_path = mod_entry.path.join(archive_name);
                if archive_path.exists() {
                    debug!("Found archive from archives.txt in mod {}: {:?}",
                           mod_entry.name, archive_path);
                    bsas.push((archive_path, mod_entry.name.clone(), mod_entry.priority));
                    break;
                }
            }
        }

        info!("Found {} BSA files from load order and mods", bsas.len());
        bsas
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_modlist_parsing() {
        // Test that priority increases from top to bottom
        let content = "+Mod1\n+Mod2\n-Mod3\n+Mod4\n";
        // Mod4 should have highest priority
    }
}
