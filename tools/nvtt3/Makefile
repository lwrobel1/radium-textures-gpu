# Makefile for NVTT3 tools

CXX = g++
CXXFLAGS = -O2 -Wall -I.
LDFLAGS = -L. -Wl,-rpath,'$$ORIGIN'
LIBS = -lnvtt

# The shared library has version suffix, create symlink
NVTT_LIB = libnvtt.so.30205
NVTT_LINK = libnvtt.so

TARGETS = nvtt_resize_compress nvtt_batch_compress

all: $(NVTT_LINK) $(TARGETS)

$(NVTT_LINK): $(NVTT_LIB)
	ln -sf $(NVTT_LIB) $(NVTT_LINK)

nvtt_resize_compress: nvtt_resize_compress.cpp $(NVTT_LINK)
	$(CXX) $(CXXFLAGS) -o $@ $< $(LDFLAGS) $(LIBS)

nvtt_batch_compress: nvtt_batch_compress.cpp $(NVTT_LINK)
	$(CXX) $(CXXFLAGS) -o $@ $< $(LDFLAGS) $(LIBS)

clean:
	rm -f $(TARGETS) $(NVTT_LINK)

test: nvtt_resize_compress
	@echo "Running test..."
	./nvtt_resize_compress || true

.PHONY: all clean test
