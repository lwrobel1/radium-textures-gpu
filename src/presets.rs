/// Optimization presets for texture downscaling
/// Based on texture type, determines maximum resolution

#[derive(Debug, Clone, Copy)]
pub struct OptimizationPreset {
    pub name: &'static str,
    pub diffuse_max: u32,
    pub normal_max: u32,
    pub parallax_max: u32,
    pub material_max: u32,
}

impl OptimizationPreset {
    /// High Quality preset: 4K Heavy Modlist Downscaled to 2K
    /// Diffuse=2K, Normal=2K, Parallax=1K, Material=1K
    pub const HQ: Self = Self {
        name: "High Quality",
        diffuse_max: 2048,
        normal_max: 2048,
        parallax_max: 1024,
        material_max: 1024,
    };

    /// Quality preset: Balance of Quality / VRAM Savings
    /// Diffuse=2K, Normal=1K, Parallax=1K, Material=1K
    pub const QUALITY: Self = Self {
        name: "Quality",
        diffuse_max: 2048,
        normal_max: 1024,
        parallax_max: 1024,
        material_max: 1024,
    };

    /// Optimum preset: Good Starting Preset if Unsure
    /// Diffuse=2K, Normal=1K, Parallax=512, Material=512
    pub const OPTIMUM: Self = Self {
        name: "Optimum",
        diffuse_max: 2048,
        normal_max: 1024,
        parallax_max: 512,
        material_max: 512,
    };

    /// Performance preset: Big Gains Lower Close Up Quality
    /// Diffuse=2K, Normal=512, Parallax=512, Material=512
    pub const PERFORMANCE: Self = Self {
        name: "Performance",
        diffuse_max: 2048,
        normal_max: 512,
        parallax_max: 512,
        material_max: 512,
    };

    /// Vanilla preset: I Just Want my PC to Play Skyrim
    /// Diffuse=512, Normal=512, Parallax=512, Material=512
    pub const VANILLA: Self = Self {
        name: "Vanilla",
        diffuse_max: 512,
        normal_max: 512,
        parallax_max: 512,
        material_max: 512,
    };

    /// Get target resolution for a texture based on its type
    /// Returns None if texture should not be optimized
    pub fn get_target_resolution(&self, texture_type: &str, current_width: u32, current_height: u32) -> Option<(u32, u32)> {
        // Determine max dimension based on texture type
        let max_res = match texture_type {
            "Diffuse" => self.diffuse_max,
            "Normal" => self.normal_max,
            "Parallax" => self.parallax_max,
            "Specular" | "Emissive" | "Emissive Mask" | "Subsurface" | "Environment" | "Multi-layer" => {
                self.material_max
            }
            _ => return None, // Unknown type, skip
        };

        // Use the larger dimension to determine if downscaling needed
        let current_max = current_width.max(current_height);

        // Only downscale if current resolution exceeds target
        if current_max > max_res {
            // Calculate aspect ratio preserving dimensions
            if current_width > current_height {
                // Landscape
                let ratio = current_height as f32 / current_width as f32;
                let new_height = (max_res as f32 * ratio).round() as u32;
                // Round to nearest power of 2
                let new_height = Self::round_to_power_of_2(new_height);
                Some((max_res, new_height))
            } else if current_height > current_width {
                // Portrait
                let ratio = current_width as f32 / current_height as f32;
                let new_width = (max_res as f32 * ratio).round() as u32;
                // Round to nearest power of 2
                let new_width = Self::round_to_power_of_2(new_width);
                Some((new_width, max_res))
            } else {
                // Square
                Some((max_res, max_res))
            }
        } else {
            // Already at or below target, no optimization needed
            None
        }
    }

    /// Round to nearest power of 2 (for DDS texture dimensions)
    fn round_to_power_of_2(n: u32) -> u32 {
        if n == 0 {
            return 1;
        }
        let mut power = 1;
        while power < n {
            power *= 2;
        }
        // Choose closest power of 2
        if power - n < n - (power / 2) {
            power
        } else {
            power / 2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hq_preset() {
        let preset = OptimizationPreset::HQ;
        assert_eq!(preset.diffuse_max, 2048);
        assert_eq!(preset.normal_max, 2048);
        assert_eq!(preset.parallax_max, 1024);
        assert_eq!(preset.material_max, 1024);
    }

    #[test]
    fn test_target_resolution_downscale() {
        let preset = OptimizationPreset::OPTIMUM;

        // 4K diffuse -> 2K
        let target = preset.get_target_resolution("Diffuse", 4096, 4096);
        assert_eq!(target, Some((2048, 2048)));

        // 4K normal -> 1K
        let target = preset.get_target_resolution("Normal", 4096, 4096);
        assert_eq!(target, Some((1024, 1024)));

        // 2K parallax -> 512
        let target = preset.get_target_resolution("Parallax", 2048, 2048);
        assert_eq!(target, Some((512, 512)));
    }

    #[test]
    fn test_target_resolution_no_downscale() {
        let preset = OptimizationPreset::OPTIMUM;

        // 1K diffuse -> no change (already below 2K target)
        let target = preset.get_target_resolution("Diffuse", 1024, 1024);
        assert_eq!(target, None);

        // 512 normal -> no change
        let target = preset.get_target_resolution("Normal", 512, 512);
        assert_eq!(target, None);
    }

    #[test]
    fn test_aspect_ratio_preservation() {
        let preset = OptimizationPreset::OPTIMUM;

        // 4096x2048 diffuse -> should preserve aspect ratio
        let target = preset.get_target_resolution("Diffuse", 4096, 2048);
        assert_eq!(target, Some((2048, 1024)));

        // 2048x4096 normal -> should preserve aspect ratio
        let target = preset.get_target_resolution("Normal", 2048, 4096);
        assert_eq!(target, Some((512, 1024)));
    }
}
