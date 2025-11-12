/// Game-specific configuration and support
/// Currently supports Skyrim Special Edition
/// Fallout 4 support planned for future release

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Game {
    SkyrimSE,
    #[allow(dead_code)]  // Coming later
    Fallout4,
}

impl Game {
    /// Get the display name for the game
    pub fn display_name(&self) -> &'static str {
        match self {
            Game::SkyrimSE => "Skyrim SE",
            Game::Fallout4 => "Fallout 4",
        }
    }

    /// Get the exclusions filename for this game
    pub fn exclusions_file(&self) -> &'static str {
        match self {
            Game::SkyrimSE => "SkyrimSE_Exclusions.txt",
            Game::Fallout4 => "Fallout4_Exclusions.txt",
        }
    }

    /// Get the archive extension for this game
    pub fn archive_extension(&self) -> &'static str {
        match self {
            Game::SkyrimSE => "bsa",  // Skyrim uses BSA
            Game::Fallout4 => "ba2",  // Fallout 4 uses BA2
        }
    }

    /// Toggle to the other game
    pub fn toggle(&self) -> Self {
        match self {
            Game::SkyrimSE => Game::Fallout4,
            Game::Fallout4 => Game::SkyrimSE,
        }
    }
}

impl Default for Game {
    fn default() -> Self {
        Game::SkyrimSE
    }
}

impl std::fmt::Display for Game {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}
