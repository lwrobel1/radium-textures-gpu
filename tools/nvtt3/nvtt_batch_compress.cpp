/*
 * nvtt_batch_compress - NVTT3 SDK batch processor for multiple textures
 *
 * Usage: nvtt_batch_compress <batch_file>
 *
 * Batch file format (one entry per line):
 *   input.dds|output.dds|max_extent|format
 *
 * Features:
 * - Single CUDA context initialization for entire batch
 * - BatchList API for efficient mipmap compression
 * - Streaming progress output for GUI feedback
 */

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>
#include <fstream>
#include <sstream>
#include "include/nvtt/nvtt.h"

using namespace nvtt;

struct TextureJob {
    std::string inputPath;
    std::string outputPath;
    int maxExtent;
    std::string format;
};

Format parseFormat(const std::string& fmt) {
    if (fmt.empty() || fmt == "bc7") return Format_BC7;
    if (fmt == "bc4") return Format_BC4;
    if (fmt == "bc3") return Format_BC3;
    if (fmt == "bc1") return Format_BC1;
    if (fmt == "bc5") return Format_BC5;
    if (fmt == "bc6") return Format_BC6U;
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
// Fixes: missing DDSD_LINEARSIZE flag, zero pitchOrLinearSize,
// NVTT watermark in dwReserved1, and miscFlags2 alpha mode.
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

std::vector<TextureJob> parseBatchFile(const char* batchFile) {
    std::vector<TextureJob> jobs;
    std::ifstream file(batchFile);

    if (!file.is_open()) {
        fprintf(stderr, "ERROR:Failed to open batch file: %s\n", batchFile);
        return jobs;
    }

    std::string line;
    while (std::getline(file, line)) {
        if (line.empty() || line[0] == '#') continue;

        std::istringstream iss(line);
        TextureJob job;

        std::getline(iss, job.inputPath, '|');
        std::getline(iss, job.outputPath, '|');

        std::string maxExtentStr, formatStr;
        std::getline(iss, maxExtentStr, '|');
        std::getline(iss, formatStr, '|');

        job.maxExtent = std::atoi(maxExtentStr.c_str());
        job.format = formatStr;

        if (!job.inputPath.empty() && !job.outputPath.empty() && job.maxExtent > 0) {
            jobs.push_back(job);
        }
    }

    return jobs;
}

bool processTexture(const TextureJob& job, Context& context, int index, int total) {
    // Diagnostic: dump input file info before processing
    {
        FILE* diag = fopen(job.inputPath.c_str(), "rb");
        if (diag) {
            fseek(diag, 0, SEEK_END);
            long fsize = ftell(diag);
            fseek(diag, 0, SEEK_SET);
            unsigned char hdr[32] = {0};
            fread(hdr, 1, 32, diag);
            fclose(diag);
            fprintf(stderr, "DIAG:%d/%d:%s:size=%ld:hdr=",
                    index + 1, total, job.inputPath.c_str(), fsize);
            for (int i = 0; i < 32; i++) fprintf(stderr, "%02x", hdr[i]);
            fprintf(stderr, "\n");
        }
    }

    // Load input DDS
    Surface surface;
    if (!surface.load(job.inputPath.c_str())) {
        fprintf(stderr, "FAIL:%d/%d:%s:Failed to load DDS file\n",
                index + 1, total, job.inputPath.c_str());
        return false;
    }

    int origW = surface.width();
    int origH = surface.height();

    // Force alpha mode to None so DX10 header writes miscFlags2=0
    // Skyrim expects DDS_ALPHA_MODE_UNKNOWN (0), not DDS_ALPHA_MODE_STRAIGHT (1)
    surface.setAlphaMode(AlphaMode_None);

    // Move surface to GPU for CUDA-accelerated operations
    surface.ToGPU();

    // Resize if needed
    int maxDim = (origW > origH) ? origW : origH;
    if (maxDim > job.maxExtent) {
        surface.resize(job.maxExtent, RoundMode_None, ResizeFilter_Kaiser);
    }

    int newW = surface.width();
    int newH = surface.height();
    int numMipmaps = calcMipCount(newW, newH);

    Format format = parseFormat(job.format);

    // Set up compression options
    CompressionOptions compressionOptions;
    compressionOptions.setFormat(format);
    compressionOptions.setQuality(Quality_Normal);

    // Set up output options
    OutputOptions outputOptions;
    outputOptions.setFileName(job.outputPath.c_str());
    outputOptions.setContainer(Container_DDS10);

    // Write header
    if (!context.outputHeader(surface, numMipmaps, compressionOptions, outputOptions)) {
        fprintf(stderr, "FAIL:%d/%d:%s:Failed to write DDS header\n",
                index + 1, total, job.inputPath.c_str());
        return false;
    }

    // Use BatchList for all mipmaps of this texture
    BatchList batch;
    std::vector<Surface> mipSurfaces;
    mipSurfaces.reserve(numMipmaps);

    // Generate all mip levels first
    Surface mipSurface = surface;
    for (int mip = 0; mip < numMipmaps; mip++) {
        mipSurfaces.push_back(mipSurface);
        if (mip < numMipmaps - 1) {
            mipSurface.buildNextMipmap(MipmapFilter_Kaiser);
        }
    }

    // Add all mips to batch
    for (int mip = 0; mip < numMipmaps; mip++) {
        batch.Append(&mipSurfaces[mip], 0, mip, &outputOptions);
    }

    // Compress all mips in one GPU call
    if (!context.compress(batch, compressionOptions)) {
        fprintf(stderr, "FAIL:%d/%d:%s:Compression failed\n",
                index + 1, total, job.inputPath.c_str());
        return false;
    }

    // Patch legacy DDS header to match texconv output
    patchDdsHeader(job.outputPath.c_str(), newW, newH, format);

    // Report success with details
    fprintf(stderr, "OK:%d/%d:%s:%dx%d->%dx%d:%s:%d\n",
            index + 1, total, job.inputPath.c_str(),
            origW, origH, newW, newH, formatName(format), numMipmaps);

    return true;
}

int main(int argc, char* argv[]) {
    if (argc < 2) {
        fprintf(stderr, "NVTT3 Batch Compress Tool\n");
        fprintf(stderr, "Usage: %s <batch_file>\n", argv[0]);
        fprintf(stderr, "\n");
        fprintf(stderr, "Batch file format (one per line):\n");
        fprintf(stderr, "  input.dds|output.dds|max_extent|format\n");
        fprintf(stderr, "\n");
        fprintf(stderr, "Formats: bc7 (default), bc4, bc3, bc1, bc5, bc6\n");
        return 1;
    }

    const char* batchFile = argv[1];

    // Parse batch file
    std::vector<TextureJob> jobs = parseBatchFile(batchFile);

    if (jobs.empty()) {
        fprintf(stderr, "ERROR:No valid jobs found in batch file\n");
        return 1;
    }

    // Report batch start
    fprintf(stderr, "BATCH_START:%zu\n", jobs.size());

    // Create compression context ONCE for entire batch (CUDA init here)
    Context context(true);

    if (context.isCudaAccelerationEnabled()) {
        fprintf(stderr, "CUDA:enabled\n");
    } else {
        fprintf(stderr, "CUDA:disabled\n");
    }

    // Process all textures
    int succeeded = 0;
    int failed = 0;

    for (size_t i = 0; i < jobs.size(); i++) {
        if (processTexture(jobs[i], context, (int)i, (int)jobs.size())) {
            succeeded++;
        } else {
            failed++;
        }
    }

    // Report batch complete
    fprintf(stderr, "BATCH_END:%d:%d\n", succeeded, failed);

    return (failed > 0) ? 1 : 0;
}
