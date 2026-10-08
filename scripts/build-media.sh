#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

# Invoked by prepare-media.ps1 in a w64devkit shell with pinned inputs.
MEDIA_BUILD_ROOT="${MEDIA_BUILD_ROOT:?Set MEDIA_BUILD_ROOT}"
PREFIX="$MEDIA_BUILD_ROOT/prefix"
JOBS="${MEDIA_BUILD_JOBS:-6}"
export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig"
export CFLAGS="-O2 -ffile-prefix-map=$MEDIA_BUILD_ROOT=media-source"
export CXXFLAGS="$CFLAGS"

cmake -S "$MEDIA_BUILD_ROOT/zlib-1.3.2" -B "$MEDIA_BUILD_ROOT/zlib-build" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS="$CFLAGS" -DCMAKE_INSTALL_PREFIX="$PREFIX" \
  -DZLIB_BUILD_SHARED=OFF -DZLIB_BUILD_STATIC=ON -DZLIB_BUILD_TESTING=OFF
cmake --build "$MEDIA_BUILD_ROOT/zlib-build" --parallel "$JOBS"
cmake --install "$MEDIA_BUILD_ROOT/zlib-build"
# zlib 1.3.2 names its static archive zs; FFmpeg checks the conventional -lz.
cp "$PREFIX/lib/libzs.a" "$PREFIX/lib/libz.a"

cmake -S "$MEDIA_BUILD_ROOT/freetype-VER-2-14-1" -B "$MEDIA_BUILD_ROOT/ft-build" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS="$CFLAGS" -DCMAKE_INSTALL_PREFIX="$PREFIX" -DBUILD_SHARED_LIBS=OFF \
  -DFT_DISABLE_ZLIB=ON -DFT_DISABLE_BZIP2=ON -DFT_DISABLE_PNG=ON \
  -DFT_DISABLE_HARFBUZZ=ON -DFT_DISABLE_BROTLI=ON
cmake --build "$MEDIA_BUILD_ROOT/ft-build" --parallel "$JOBS"
cmake --install "$MEDIA_BUILD_ROOT/ft-build"

cmake -S "$MEDIA_BUILD_ROOT/harfbuzz-14.6.0" -B "$MEDIA_BUILD_ROOT/hb-build" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS="$CFLAGS" -DCMAKE_CXX_FLAGS="$CXXFLAGS" \
  -DCMAKE_INSTALL_PREFIX="$PREFIX" -DCMAKE_PREFIX_PATH="$PREFIX" \
  -DBUILD_SHARED_LIBS=OFF -DHB_HAVE_FREETYPE=ON -DHB_BUILD_UTILS=OFF \
  -DHB_BUILD_SUBSET=OFF -DHB_BUILD_RASTER=OFF -DHB_BUILD_VECTOR=OFF -DHB_BUILD_GPU=OFF
cmake --build "$MEDIA_BUILD_ROOT/hb-build" --parallel "$JOBS"
cmake --install "$MEDIA_BUILD_ROOT/hb-build"

cd "$MEDIA_BUILD_ROOT/x264-c24e06c2e184345ceb33eb20a15d1024d9fd3497"
bash ./configure --prefix="$PREFIX" --host=x86_64-w64-mingw32 --enable-static --disable-cli --disable-opencl
# FFmpeg's multiline quiet recipes are incompatible with Windows GNU make's
# leading-command handling. Use verbose recipes and the selected POSIX shell.
make -j "$JOBS" V=1 SHELL="${MEDIA_BUILD_SHELL:?Set MEDIA_BUILD_SHELL}"
make V=1 SHELL="$MEDIA_BUILD_SHELL" install

cd "$MEDIA_BUILD_ROOT/ffmpeg-8.1.1"
sh ./configure --prefix=/mono-cut-media --target-os=mingw32 --arch=x86_64 \
  --enable-gpl --enable-version3 --enable-static --disable-shared \
  --disable-autodetect --disable-network --disable-hwaccels --disable-doc --disable-debug \
  --disable-ffplay --enable-libx264 --enable-libfreetype --enable-libharfbuzz \
  --enable-zlib \
  --pkg-config-flags=--static --extra-cflags="-O2 -ffile-prefix-map=$MEDIA_BUILD_ROOT=media-source" \
  --extra-ldflags="-static -static-libgcc -static-libstdc++"
# FFmpeg embeds its invocation in FFMPEG_CONFIGURATION. Preserve the flags
# while replacing this machine's build location in that descriptive string.
sed "s|$MEDIA_BUILD_ROOT|/build/mono-cut-media|g" config.h > config.h.clean
mv config.h.clean config.h
make -j "$JOBS" V=1 SHELL="$MEDIA_BUILD_SHELL" ffmpeg.exe ffprobe.exe
mkdir -p "$PREFIX/bin"
cp ffmpeg.exe ffprobe.exe "$PREFIX/bin/"
