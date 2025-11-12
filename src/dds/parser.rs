use anyhow::{Result, bail};
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Read, Seek, SeekFrom};

/// DDS file format constants
const DDS_MAGIC: u32 = 0x20534444; // "DDS " in ASCII
const DDS_HEADER_SIZE: usize = 124;
const DX10_HEADER_SIZE: usize = 20;

// DDS_PIXELFORMAT flags
const DDPF_FOURCC: u32 = 0x0000_0004;

// FourCC codes
const FOURCC_DXT1: u32 = 0x3154_5844; // "DXT1"
const FOURCC_DXT2: u32 = 0x3254_5844; // "DXT2"
const FOURCC_DXT3: u32 = 0x3354_5844; // "DXT3"
const FOURCC_DXT4: u32 = 0x3454_5844; // "DXT4"
const FOURCC_DXT5: u32 = 0x3554_5844; // "DXT5"
const FOURCC_DX10: u32 = 0x3031_5844; // "DX10"
const FOURCC_BC4U: u32 = 0x5534_4342; // "BC4U"
const FOURCC_BC4S: u32 = 0x5334_4342; // "BC4S"
const FOURCC_BC5U: u32 = 0x5535_4342; // "BC5U"
const FOURCC_BC5S: u32 = 0x5335_4342; // "BC5S"
const FOURCC_ATI1: u32 = 0x3149_5441; // "ATI1"
const FOURCC_ATI2: u32 = 0x3249_5441; // "ATI2"

// DXGI_FORMAT values for DX10 header
const DXGI_FORMAT_BC1_UNORM: u32 = 71;
const DXGI_FORMAT_BC1_UNORM_SRGB: u32 = 72;
const DXGI_FORMAT_BC2_UNORM: u32 = 74;
const DXGI_FORMAT_BC2_UNORM_SRGB: u32 = 75;
const DXGI_FORMAT_BC3_UNORM: u32 = 77;
const DXGI_FORMAT_BC3_UNORM_SRGB: u32 = 78;
const DXGI_FORMAT_BC4_UNORM: u32 = 80;
const DXGI_FORMAT_BC4_SNORM: u32 = 81;
const DXGI_FORMAT_BC5_UNORM: u32 = 83;
const DXGI_FORMAT_BC5_SNORM: u32 = 84;
const DXGI_FORMAT_BC6H_UF16: u32 = 95;
const DXGI_FORMAT_BC6H_SF16: u32 = 96;
const DXGI_FORMAT_BC7_UNORM: u32 = 98;
const DXGI_FORMAT_BC7_UNORM_SRGB: u32 = 99;

/// DDS texture header information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DDSHeader {
    pub width: u32,
    pub height: u32,
    pub format: String,
}

/// Parse DDS header from a stream (file or BSA file entry)
/// Only reads the first 148 bytes maximum (header only, no texture data!)
/// Based on Microsoft DDS format specification
pub fn parse_dds_header<R: Read + Seek>(stream: &mut R) -> Result<DDSHeader> {
    // Read header bytes (128 bytes standard + 20 bytes DX10 if present)
    let mut header_buffer = vec![0u8; 148];
    stream.seek(SeekFrom::Start(0))?;
    let bytes_read = stream.read(&mut header_buffer)?;

    if bytes_read < 128 {
        bail!("File too small to be valid DDS (need at least 128 bytes, got {})", bytes_read);
    }

    // Check magic number "DDS "
    let mut cursor = &header_buffer[..];
    let magic = cursor.read_u32::<LittleEndian>()?;
    if magic != DDS_MAGIC {
        bail!("Invalid DDS magic number: 0x{:08X} (expected 0x{:08X})", magic, DDS_MAGIC);
    }

    // Skip dwSize (4 bytes)
    cursor.read_u32::<LittleEndian>()?;

    // Skip dwFlags (4 bytes)
    cursor.read_u32::<LittleEndian>()?;

    // Read dwHeight (offset 12)
    let height = cursor.read_u32::<LittleEndian>()?;

    // Read dwWidth (offset 16)
    let width = cursor.read_u32::<LittleEndian>()?;

    // Skip to DDS_PIXELFORMAT at offset 76
    // We've read 20 bytes so far (magic + size + flags + height + width)
    // Need to skip 56 more bytes to get to offset 76
    let mut skip_buffer = vec![0u8; 56];
    cursor.read_exact(&mut skip_buffer)?;

    // Read DDS_PIXELFORMAT (32 bytes starting at offset 76)
    // Skip dwSize (4 bytes)
    cursor.read_u32::<LittleEndian>()?;

    // Read dwFlags (offset 80)
    let pixel_format_flags = cursor.read_u32::<LittleEndian>()?;

    // Read dwFourCC (offset 84)
    let fourcc = cursor.read_u32::<LittleEndian>()?;

    // Determine format based on FourCC
    let format = if (pixel_format_flags & DDPF_FOURCC) != 0 {
        match fourcc {
            FOURCC_DXT1 => "BC1".to_string(),
            FOURCC_DXT2 | FOURCC_DXT3 => "BC2".to_string(),
            FOURCC_DXT4 | FOURCC_DXT5 => "BC3".to_string(),
            FOURCC_BC4U | FOURCC_BC4S | FOURCC_ATI1 => "BC4".to_string(),
            FOURCC_BC5U | FOURCC_BC5S | FOURCC_ATI2 => "BC5".to_string(),
            FOURCC_DX10 => {
                // Parse DX10 extended header
                parse_dx10_format(&header_buffer[128..])?
            }
            _ => format!("FourCC_{}", fourcc_to_string(fourcc)),
        }
    } else {
        "UNCOMPRESSED".to_string()
    };

    Ok(DDSHeader {
        width,
        height,
        format,
    })
}

/// Parse DX10 extended header for BC6H, BC7, etc.
fn parse_dx10_format(dx10_header: &[u8]) -> Result<String> {
    if dx10_header.len() < 4 {
        return Ok("UNKNOWN".to_string());
    }

    let mut cursor = dx10_header;
    let dxgi_format = cursor.read_u32::<LittleEndian>()?;

    let format = match dxgi_format {
        DXGI_FORMAT_BC1_UNORM | DXGI_FORMAT_BC1_UNORM_SRGB => "BC1",
        DXGI_FORMAT_BC2_UNORM | DXGI_FORMAT_BC2_UNORM_SRGB => "BC2",
        DXGI_FORMAT_BC3_UNORM | DXGI_FORMAT_BC3_UNORM_SRGB => "BC3",
        DXGI_FORMAT_BC4_UNORM | DXGI_FORMAT_BC4_SNORM => "BC4",
        DXGI_FORMAT_BC5_UNORM | DXGI_FORMAT_BC5_SNORM => "BC5",
        DXGI_FORMAT_BC6H_UF16 | DXGI_FORMAT_BC6H_SF16 => "BC6H",
        DXGI_FORMAT_BC7_UNORM | DXGI_FORMAT_BC7_UNORM_SRGB => "BC7",
        _ => return Ok(format!("DXGI_{}", dxgi_format)),
    };

    Ok(format.to_string())
}

/// Convert FourCC uint to string
fn fourcc_to_string(fourcc: u32) -> String {
    let bytes = fourcc.to_le_bytes();
    String::from_utf8_lossy(&bytes).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_fourcc_conversion() {
        assert_eq!(fourcc_to_string(FOURCC_DXT1), "DXT1");
        assert_eq!(fourcc_to_string(FOURCC_DXT5), "DXT5");
    }

    #[test]
    fn test_invalid_magic() {
        let bad_data = vec![0u8; 128];
        let mut cursor = Cursor::new(bad_data);
        let result = parse_dds_header(&mut cursor);
        assert!(result.is_err());
    }

    #[test]
    fn test_too_small() {
        let bad_data = vec![0u8; 64];
        let mut cursor = Cursor::new(bad_data);
        let result = parse_dds_header(&mut cursor);
        assert!(result.is_err());
    }
}
