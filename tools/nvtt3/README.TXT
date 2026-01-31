================================================================================
NVIDIA Texture Tools 3
================================================================================
NVIDIA Texture Tools is a set of command-line tools and an equivalent runtime
library for compressing textures into various GPU-friendly formats.


For more information of the use of command-line tools:

   * Open a "cmd" window and run each of the executables in ./bin without any argument.

For more information of the library APIs:
  
   * ./nvttlib/docs/index.html

================================================================================
New in 3.2.5
================================================================================
* Added `nvtt::MipmapFilter_Mitchell`, `nvtt::{MipmapFilter, ResizeFilter}_Min`,
and `nvtt::{MipmapFilter, ResizeFilter}_Max`.
* Attempting to create an `nvtt::Surface` with a width, height, or depth that
would overflow an underlying integer type now produces an error instead
of overflowing.
* nvcompress: Fixed a crash on Windows when attempting to compress more than
509 files at once, due to exhaustion of Windows C++ runtime file handles.
* nvcompress: Fixed a crash on Windows when attempting to compress a directory
whose files' names' included characters outside of the active OS code page.
* nvcompress: Fixed an off-by-one error when computing the number of mipmaps
that could be generated.

================================================================================
New in 3.2.4
================================================================================
* Fixed naming of NVTT_VERSION fields to make the mapping to semantic versioning
clearer. For instance, NVTT 3.2.4 is fork 3, major version 2 (incremented on
API breaks), minor version 4 (incremented on new features and bug fixes).
* Added a safer 4-argument overload of `TimingContext::GetRecord()`, which takes
the size of the buffer to write the description to, and deprecated the
3-argument version.

================================================================================
New in 3.2.3
================================================================================
* BC1-BC3: Fixes a bug in 3.2.2 where CUDA compression on Turing GPUs would
produce a `cudaErrorIllegalAddress` error.
* Fixes a bug where `nvtt::Surface::createSubImage` and `nvtt::Surface::diff()`
did not copy the texture type, alpha mode, wrap mode, or normal flag from their
input(s) to their output `nvtt::Surface`.
* Correct typo in readme file

================================================================================
New in 3.2.2
================================================================================
* BC1-BC3,BC1a: Improved compression quality at Quality::Production|Highest.
* BC7,ASTC: Fixes a bug where images marked opaque would use a transparent
compressor, and images marked transparent would use an opaque compressor.
* Improved compress' performance.
* Added --max-mip-count, --min-mip-size, and --no-mip-gamma-correct for
nvcompress.
* Renamed nvbatchcompress.exe to nvbatchcompress.bat, nvbatchcompress is 
merged to nvcompress.
* Retargeted CUDA dependency to version 11.8, it requires graphics driver 520.06
or above

================================================================================
New in 3.2.1
================================================================================
* Optimized CPU-to-GPU transfers, giving faster compression when using the
`nvtt::Context` API and `nvtt::Surface` data is not in VRAM.
* BC1-BC5: Optimized kernel parameters for faster compression.
* BC1/BC1a: Improved compression quality when the input block has close to a
flat color.
* CPU BC1: Fixes a bug where the compressor would sometimes encode transparent
single-color blocks with transparency.
* GPU BC1a: New fast-mode compressor when input data is on the GPU.
* GPU BC6: Improved compressor speed by merging partition kernels.
* Adds `nvtt::Surface::loadFromMemory()`,
`nvtt::SurfaceSet::loadDDSFromMemory()`, and
`nvtt::CubeSurface::loadFromMemory()`. These variants of
`nvtt::Surface::load()`, `nvtt::SurfaceSet::loadDDS()`, and
`nvtt::CubeSurface::load()` work on in-memory data. (Thank you to mijalko on the
NVIDIA Developer Forums.)
* Fixes a bug where the CPU BC1 compressor would sometimes encode transparent
single-color blocks with transparency. (Thank you to tgrimmer on the NVIDIA
Developer Forums.)
* Adds range checks on the parameters of `nvtt::Surface::toRGBE()`,
`nvtt::Surface::fromRGBE()`, and `nvtt::Surface()::quantize()`.
* Updated stb_image dependency.

================================================================================
New in 3.2.0
================================================================================
* Adds precompiled kernels for Hopper and Ada GPUs.
* Adds `nvtt::nvtt_encode()` and `nvtt::EncodeSettings` to the low-level API.
`nvtt_encode()` unifies all low-level compression functions under a single
interface, and `nvtt::EncodeSettings` can be extended to add new features
without breaking the API.
* Adds 16-bit half float (`nvtt::ValueType::FLOAT16`) as an input to the
low-level API.
* Adds `nvtt::SetMessageCallback()`. NVTT now reports errors, warnings, and
messages through each thread's message callback. In particular, this can be used
to detect errors even when a function does not return `bool`.
* Adds `nvtt::Surface::gpuDataMutable()`. This allows NVTT Surface data to be
used with your own custom CUDA kernels, without requiring a `const_cast`.
* Adds unclamped sRGB transfer functions. These allow HDR images to be converted
to sRGB and back without significant information loss:
  - `nvtt::Surface::toSrgbUnclamped()`
  - `nvtt::Surface::toLinearFromSrgbUnclamped()`
* Adds `nvtt::Surface::toLinearFromXenonSrgb()`, the inverse of
`nvtt::Surface::toXenonSrgb()`.
* Adds BC3n GPU compressor.
* Faster BC6H GPU compression.
* Fixes an out-of-bounds write within the BC2 GPU compressor.
* Fixes a bug where the C wrapper and C++ Error enumerations didn't match.
* Fixes a bug where the slow-mode BC1 CPU compressor would sometimes output
blocks with alpha, if the input had variegated blocks with semitransparent
pixels. (Thank you to tgrimmer on the NVIDIA Developer Forums.)
* Fixes a bug where `nvtt::Surface::toSrgb()` turned 1.0 into 0.999999940
instead of 1.
* Fixes a bug where `nvttContextQuantize()` was missing from the C wrapper
symbols. (Thank you to mijalko on the NVIDIA Developer Forums.)

================================================================================
New in 3.1.6
================================================================================
* Compatibility break from 3.1.5.
  A member name of struct RefImage has been renamed from num_chanels into num_channels.
* Retarget CUDA dependency to version 11.3.
* Retarget VisualStudio version to 2019.

* Support Ampere GPUs.
* Add new APIs.
  - nvtt::useCurrentDevice()
  - nvtt::Surface::clone()
* Add C wrapper.
* Fix memory leaks.
* Enrich API documentation.
* Fix out-of-bounds write at resizing 3D image.
* Fix out-of-bounds write at BC7.
* Fix various file reader issues.
* Clear uninitialized buffers.

================================================================================
New in 3.1.0
================================================================================
* Reworked ASTC/BC7 for better performance
* New APIs (nvtt_lowlevel.h) for more straightforward texture compression
* Better quality for BC6 - fast mode.

================================================================================
Features
================================================================================
* Compression/decompression of textures using command-line tools
* Compression/decompression of textures using library APIs
* Supported input format: PNG, TGA, DDS
* Output file format: DDS
* Supported texture formats: 
  - BC1(DXT1)
  - BC2(DXT3) 
  - BC3(DXT5)
  - BC4
  - BC5
  - BC6H
  - BC7
  - ASTC : 2D blocks, low dynamic range 
* CUDA acceleration:
  - Supported for ASTC compression in all quality levels
  - Supported for BCx compressions in default quality level    
  - Supported for basic image preprocessing 

================================================================================
Package Structure 
================================================================================ 
[PackageRoot]/
             /bin/                      ----- command-line tools
                 /nvcompress.exe        ----- compressing tool
                 /nvbatchcompress.exe   ----- batch compressing tool
                 /nvdecompress.exe      ----- decompressing tool
                 /nvddsinfo.exe         ----- tool for showing DDS file infomation
                 /nvimgdiff.exe         ----- tool for comparing 2 images
             /nvttlib                   ----- library
                 /docs                  ----- doxygen documentation
                 /external              ----- external dependencies
                 /include               ----- headers
                 /lib                   ----- static libraries 
                 /sample                ----- Sample code showing the use of library


================================================================================
Supported NVIDIA Display Drivers for CUDA acceleration
================================================================================
465.89 or newer (Windows support only)

================================================================================
Supported NVIDIA Hardware
================================================================================
NVTT supports the NVIDIA GPUs listed below:
* GeForce 30XX Series (Ampere Family)
* GeForce 20XX Series (Turing Family)
* GeForce 10XX Series (Pascal Family)
* GeForce 9XX Series (Maxwell Family)
* GeForce 8XX Series (Maxwell Family)

================================================================================
About DDS as a Container of Textures
================================================================================ 
Compressed texture data is stored into DDS files following the following specifications:
http://msdn.microsoft.com/en-us/library/bb943991.aspx
https://msdn.microsoft.com/en-us/library/bb173059.aspx

In addition, ASTC formats are defined as follows:

    	DXGI_FORMAT_ASTC_4X4_UNORM              = 134,
		DXGI_FORMAT_ASTC_4X4_UNORM_SRGB         = 135,
		DXGI_FORMAT_ASTC_5X4_TYPELESS           = 137,
		DXGI_FORMAT_ASTC_5X4_UNORM              = 138,
		DXGI_FORMAT_ASTC_5X4_UNORM_SRGB         = 139,
		DXGI_FORMAT_ASTC_5X5_TYPELESS           = 141,
		DXGI_FORMAT_ASTC_5X5_UNORM              = 142,
		DXGI_FORMAT_ASTC_5X5_UNORM_SRGB         = 143,
		DXGI_FORMAT_ASTC_6X5_TYPELESS           = 145,
		DXGI_FORMAT_ASTC_6X5_UNORM              = 146,
		DXGI_FORMAT_ASTC_6X5_UNORM_SRGB         = 147,
		DXGI_FORMAT_ASTC_6X6_TYPELESS           = 149,
		DXGI_FORMAT_ASTC_6X6_UNORM              = 150,
		DXGI_FORMAT_ASTC_6X6_UNORM_SRGB         = 151,
		DXGI_FORMAT_ASTC_8X5_TYPELESS           = 153,
		DXGI_FORMAT_ASTC_8X5_UNORM              = 154,
		DXGI_FORMAT_ASTC_8X5_UNORM_SRGB         = 155,
		DXGI_FORMAT_ASTC_8X6_TYPELESS           = 157,
		DXGI_FORMAT_ASTC_8X6_UNORM              = 158,
		DXGI_FORMAT_ASTC_8X6_UNORM_SRGB         = 159,
		DXGI_FORMAT_ASTC_8X8_TYPELESS           = 161,
		DXGI_FORMAT_ASTC_8X8_UNORM              = 162,
		DXGI_FORMAT_ASTC_8X8_UNORM_SRGB         = 163,
		DXGI_FORMAT_ASTC_10X5_TYPELESS          = 165,
		DXGI_FORMAT_ASTC_10X5_UNORM             = 166,
		DXGI_FORMAT_ASTC_10X5_UNORM_SRGB        = 167,
		DXGI_FORMAT_ASTC_10X6_TYPELESS          = 169,
		DXGI_FORMAT_ASTC_10X6_UNORM             = 170,
		DXGI_FORMAT_ASTC_10X6_UNORM_SRGB        = 171,
		DXGI_FORMAT_ASTC_10X8_TYPELESS          = 173,
		DXGI_FORMAT_ASTC_10X8_UNORM             = 174,
		DXGI_FORMAT_ASTC_10X8_UNORM_SRGB        = 175,
		DXGI_FORMAT_ASTC_10X10_TYPELESS         = 177,
		DXGI_FORMAT_ASTC_10X10_UNORM            = 178,
		DXGI_FORMAT_ASTC_10X10_UNORM_SRGB       = 179,
		DXGI_FORMAT_ASTC_12X10_TYPELESS         = 181,
		DXGI_FORMAT_ASTC_12X10_UNORM            = 182,
		DXGI_FORMAT_ASTC_12X10_UNORM_SRGB       = 183,
		DXGI_FORMAT_ASTC_12X12_TYPELESS         = 185,
		DXGI_FORMAT_ASTC_12X12_UNORM            = 186,
		DXGI_FORMAT_ASTC_12X12_UNORM_SRGB       = 187,

================================================================================
Known Issues
================================================================================
* ASTC HDR modes are not supported.

================================================================================
Revision History
================================================================================
2023/10/19  NVIDIA Texture Tools 3.2.4
2023/7/21   NVIDIA Texture Tools 3.2.3
2023/7/5    NVIDIA Texture Tools 3.2.2
2023/3/28   NVIDIA Texture Tools 3.2.1
2023/2/23   NVIDIA Texture Tools 3.2.0
2021/11/8   NVIDIA Texture Tools 3.1.6
2020/8/3    Nvidia Texture Tools 3.1.5
2020/6/13   Nvidia Texture Tools 3.1.4
2020/2/12   Nvidia Texture Tools 3.1.3
2020/2/4    Nvidia Texture Tools 3.1.2
2019/7/8    Nvidia Texture Tools 3.1.0
2019/2/13   Nvidia Texture Tools 3.0.2
2018/11/1   Nvidia Texture Tools 3.0.1
2018/9/20   Nvidia Texture Tools 3 Beta 9.7
2018/5/30   Nvidia Texture Tools 3 Beta 9.6
2018/5/15   Nvidia Texture Tools 3 Beta 9.5
2018/4/23   Nvidia Texture Tools 3 Beta 9.4
2018/1/19   Nvidia Texture Tools 3 Beta 9.3
2018/1/12   Nvidia Texture Tools 3 Beta 9.2
2017/12/19   Nvidia Texture Tools 3 Beta 9.1
2017/11/1   Nvidia Texture Tools 3 Beta 9
2017/4/6   Nvidia Texture Tools 3 Beta 8.4
2017/3/21   Nvidia Texture Tools 3 Beta 8.3
2017/3/15   Nvidia Texture Tools 3 Beta 8.2
2017/2/28   Nvidia Texture Tools 3 Beta 8.1
2017/2/15   Nvidia Texture Tools 3 Beta 8
2017/2/9   Nvidia Texture Tools 3 Beta 7.1
2017/2/8   Nvidia Texture Tools 3 Beta 7
2017/1/18   Nvidia Texture Tools 3 Beta 6
2016/11/22   Nvidia Texture Tools 3 Beta 5.2
2016/10/24   Nvidia Texture Tools 3 Beta 5.1
2016/10/19   Nvidia Texture Tools 3 Beta 5
2016/9/7   Nvidia Texture Tools 3 Beta 4
2016/7/8   Nvidia Texture Tools 3 Beta 3
2016/7/1   Nvidia Texture Tools 3 Beta 2
2016/3/23  Nvidia Texture Tools 3 Beta 1
2016/2/5   Nvidia Texture Tools 3 Alpha 3
2015/11/18 Nvidia Texture Tools 3 Alpha 2
2015/10/28 Nvidia Texture Tools 3 Alpha 1



