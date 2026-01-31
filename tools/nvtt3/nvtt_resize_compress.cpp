/*
 * nvtt_resize_compress - NVTT3 SDK wrapper for resize + BC7 compression
 *
 * Usage: nvtt_resize_compress <input.dds> <output.dds> <max_extent> [format]
 *
 * Formats: bc7 (default), bc4, bc3, bc1
 */

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>
#include "include/nvtt/nvtt.h"

using namespace nvtt;

void printUsage(const char* prog) {
    fprintf(stderr, "NVTT3 Resize + Compress Tool\n");
    fprintf(stderr, "Usage: %s <input.dds> <output.dds> <max_extent> [format]\n", prog);
    fprintf(stderr, "\n");
    fprintf(stderr, "Arguments:\n");
    fprintf(stderr, "  input.dds   - Input DDS file (supports BC7/DX10, BC1-BC5, etc.)\n");
    fprintf(stderr, "  output.dds  - Output DDS file\n");
    fprintf(stderr, "  max_extent  - Maximum dimension (e.g., 1024, 2048)\n");
    fprintf(stderr, "  format      - Output format: bc7 (default), bc4, bc3, bc1\n");
    fprintf(stderr, "\n");
    fprintf(stderr, "Example: %s diffuse.dds diffuse_1k.dds 1024 bc7\n", prog);
}

Format parseFormat(const char* fmt) {
    if (!fmt || strcmp(fmt, "bc7") == 0) return Format_BC7;
    if (strcmp(fmt, "bc4") == 0) return Format_BC4;
    if (strcmp(fmt, "bc3") == 0) return Format_BC3;
    if (strcmp(fmt, "bc1") == 0) return Format_BC1;
    if (strcmp(fmt, "bc5") == 0) return Format_BC5;
    if (strcmp(fmt, "bc6") == 0) return Format_BC6U;
    fprintf(stderr, "Warning: Unknown format '%s', using BC7\n", fmt);
    return Format_BC7;
}

const char* formatName(Format fmt) {
    switch (fmt) {
        case Format_BC7: return "BC7";
        case Format_BC4: return "BC4";
        case Format_BC3: return "BC3";
        case Format_BC1: return "BC1";
        case Format_BC5: return "BC5";
        case Format_BC6U: return "BC6";
        default: return "Unknown";
    }
}

int calcMipCount(int w, int h) {
    int count = 1;
    while (w > 1 || h > 1) {
        w = (w > 1) ? w / 2 : 1;
        h = (h > 1) ? h / 2 : 1;
        count++;
    }
    return count;
}

int blockSizeForFormat(Format fmt) {
    switch (fmt) {
        case Format_BC1: return 8;
        case Format_BC4: return 8;
        default: return 16; // BC3, BC5, BC6, BC7
    }
}

// Patch NVTT3's DDS legacy header to match texconv output.
// NVTT3 omits DDSD_LINEARSIZE flag, writes zero pitchOrLinearSize,
// and leaves a watermark in dwReserved1. Skyrim's DDS loader may
// use these legacy fields even for DX10-format files.
void patchDdsHeader(const char* path, int width, int height, Format format) {
    FILE* f = fopen(path, "r+b");
    if (!f) return;

    // 1. Add DDSD_LINEARSIZE (0x80000) to dwFlags at offset 8
    uint32_t flags = 0;
    fseek(f, 8, SEEK_SET);
    fread(&flags, sizeof(uint32_t), 1, f);
    flags |= 0x80000; // DDSD_LINEARSIZE
    fseek(f, 8, SEEK_SET);
    fwrite(&flags, sizeof(uint32_t), 1, f);

    // 2. Write correct pitchOrLinearSize at offset 20
    //    For block-compressed: total bytes of top-level mip surface
    //    = max(1, (width+3)/4) * max(1, (height+3)/4) * blockSize
    //    (matches DirectXTex/texconv behavior)
    int bsize = blockSizeForFormat(format);
    int wBlocks = (width + 3) / 4;
    if (wBlocks < 1) wBlocks = 1;
    int hBlocks = (height + 3) / 4;
    if (hBlocks < 1) hBlocks = 1;
    uint32_t linearSize = (uint32_t)(wBlocks * hBlocks * bsize);
    fseek(f, 20, SEEK_SET);
    fwrite(&linearSize, sizeof(uint32_t), 1, f);

    // 3. Set dwDepth to 1 at offset 24 (texconv writes 1 for 2D textures)
    uint32_t one = 1;
    fseek(f, 24, SEEK_SET);
    fwrite(&one, sizeof(uint32_t), 1, f);

    // 4. Zero out dwReserved1[11] at offsets 32-75 (remove NVTT watermark)
    uint32_t zeros[11] = {0};
    fseek(f, 32, SEEK_SET);
    fwrite(zeros, sizeof(uint32_t), 11, f);

    // 5. Patch DX10 miscFlags2 to DDS_ALPHA_MODE_UNKNOWN (0) at offset 144
    uint32_t zero = 0;
    fseek(f, 144, SEEK_SET);
    fwrite(&zero, sizeof(uint32_t), 1, f);

    fclose(f);
}

int main(int argc, char* argv[]) {
    if (argc < 4) {
        printUsage(argv[0]);
        return 1;
    }

    const char* inputPath = argv[1];
    const char* outputPath = argv[2];
    int maxExtent = atoi(argv[3]);
    Format format = parseFormat(argc > 4 ? argv[4] : nullptr);

    if (maxExtent <= 0 || maxExtent > 16384) {
        fprintf(stderr, "Error: Invalid max_extent %d (must be 1-16384)\n", maxExtent);
        return 1;
    }

    // Load input DDS
    Surface surface;
    if (!surface.load(inputPath)) {
        fprintf(stderr, "Error: Failed to load DDS file: %s\n", inputPath);
        return 1;
    }

    int origW = surface.width();
    int origH = surface.height();

    // Force alpha mode to None so DX10 header writes miscFlags2=0
    // Skyrim expects DDS_ALPHA_MODE_UNKNOWN (0), not DDS_ALPHA_MODE_STRAIGHT (1)
    // NVTT3 auto-detects alpha and sets Transparency mode, which breaks Skyrim rendering
    surface.setAlphaMode(AlphaMode_None);

    // Move surface to GPU for CUDA-accelerated resize and mipmap operations
    // This makes resize() and buildNextMipmap() run on GPU instead of CPU
    if (!getenv("NVTT_CPU_ONLY") || strcmp(getenv("NVTT_CPU_ONLY"), "1") != 0) {
        surface.ToGPU();
    }

    // Check if resize is needed
    int maxDim = (origW > origH) ? origW : origH;
    if (maxDim > maxExtent) {
        // Resize to fit within maxExtent while preserving aspect ratio (GPU-accelerated)
        surface.resize(maxExtent, RoundMode_None, ResizeFilter_Kaiser);
    }

    int newW = surface.width();
    int newH = surface.height();
    int numMipmaps = calcMipCount(newW, newH);

    // Create compression context (uses CUDA if available)
    // Set NVTT_CPU_ONLY=1 to force CPU-only mode for diagnostics
    bool useCuda = true;
    const char* cpuOnly = getenv("NVTT_CPU_ONLY");
    if (cpuOnly && strcmp(cpuOnly, "1") == 0) {
        useCuda = false;
        fprintf(stderr, "CPU-only mode forced via NVTT_CPU_ONLY=1\n");
    }
    Context context(useCuda);

    // Check if CUDA is available
    if (context.isCudaAccelerationEnabled()) {
        fprintf(stderr, "CUDA acceleration: enabled (compression + resize + mipmaps)\n");
    } else {
        fprintf(stderr, "CUDA acceleration: disabled (using CPU)\n");
    }

    // Set up compression options
    CompressionOptions compressionOptions;
    compressionOptions.setFormat(format);
    // Set NVTT_QUALITY=production|highest to override (default: normal)
    const char* qualEnv = getenv("NVTT_QUALITY");
    Quality quality = Quality_Normal;
    if (qualEnv) {
        if (strcmp(qualEnv, "production") == 0) { quality = Quality_Production; fprintf(stderr, "Quality: Production\n"); }
        else if (strcmp(qualEnv, "highest") == 0) { quality = Quality_Highest; fprintf(stderr, "Quality: Highest\n"); }
        else if (strcmp(qualEnv, "fastest") == 0) { quality = Quality_Fastest; fprintf(stderr, "Quality: Fastest\n"); }
        else { fprintf(stderr, "Quality: Normal (default)\n"); }
    }
    compressionOptions.setQuality(quality);

    // Set up output options - use built-in file output
    OutputOptions outputOptions;
    outputOptions.setFileName(outputPath);
    outputOptions.setContainer(Container_DDS10); // Use DX10 container for BC7

    // Write header with mipmap count
    if (!context.outputHeader(surface, numMipmaps, compressionOptions, outputOptions)) {
        fprintf(stderr, "Error: Failed to write DDS header\n");
        return 1;
    }

    // Compress base level and mipmaps
    Surface mipSurface = surface;
    for (int mip = 0; mip < numMipmaps; mip++) {
        if (!context.compress(mipSurface, 0, mip, compressionOptions, outputOptions)) {
            fprintf(stderr, "Error: Compression failed at mip level %d\n", mip);
            return 1;
        }

        // Generate next mipmap level
        if (mip < numMipmaps - 1) {
            mipSurface.buildNextMipmap(MipmapFilter_Kaiser);
        }
    }

    // Patch legacy DDS header to match texconv output
    patchDdsHeader(outputPath, newW, newH, format);

    fprintf(stderr, "OK: %dx%d -> %dx%d [%s] (%d mips)\n",
            origW, origH, newW, newH, formatName(format), numMipmaps);

    return 0;
}
