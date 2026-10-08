#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Build the audited media source versions for the native Linux/macOS host.
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_ROOT="${1:-$PROJECT_ROOT/work/media-unix}"
JOBS="${2:-4}"
case "$JOBS" in ''|*[!0-9]*) echo 'Jobs must be an integer between 1 and 32.' >&2; exit 1 ;; esac
if (( JOBS < 1 || JOBS > 32 )); then echo 'Jobs must be between 1 and 32.' >&2; exit 1; fi
PLATFORM="$(uname -s)"
ARCH="$(uname -m)"
case "$PLATFORM" in
  Linux) CXX_RUNTIME='-lstdc++' ;;
  Darwin) CXX_RUNTIME='-lc++' ;;
  *) echo 'This recipe supports native Linux and macOS. Use prepare-media.ps1 on Windows.' >&2; exit 1 ;;
esac
for tool in node curl tar unzip cmake ninja pkg-config make; do
  command -v "$tool" >/dev/null || { echo "Missing build tool: $tool" >&2; exit 1; }
done
mkdir -p "$BUILD_ROOT"
BUILD_ROOT="$(cd "$BUILD_ROOT" && pwd)"
PREFIX="$BUILD_ROOT/prefix"
RESOURCES="$PROJECT_ROOT/src-tauri/resources/media"
MANIFEST="$PROJECT_ROOT/scripts/media-sources.json"
PROVENANCE="$BUILD_ROOT/build-provenance.json"
mkdir -p "$RESOURCES"

# The Windows manifest also carries the exact platform-independent source inputs.
# Keep versions and checksums in one file rather than using moving system tools.
RECIPE_HASH="$(node - "$MANIFEST" "${BASH_SOURCE[0]}" "$PLATFORM" "$ARCH" <<'JS'
const fs = require('node:fs'), crypto = require('node:crypto');
const hash = crypto.createHash('sha256');
for (const file of process.argv.slice(2, 4)) hash.update(fs.readFileSync(file));
for (const value of process.argv.slice(4)) hash.update('\0' + value);
process.stdout.write(hash.digest('hex'));
JS
)"
if node - "$PROVENANCE" "$RECIPE_HASH" "$RESOURCES" <<'JS'
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');
try {
  const record = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
  if (record.recipe_sha256 !== process.argv[3]) process.exit(1);
  for (const name of ['ffmpeg', 'ffprobe', 'Inter.ttf']) {
    const bytes = fs.readFileSync(path.join(process.argv[4], name));
    if (crypto.createHash('sha256').update(bytes).digest('hex') !== record.files[name]) process.exit(1);
  }
} catch { process.exit(1); }
JS
then
  echo 'Unix media resources match this recipe and recorded SHA-256 checksums.'
  exit 0
fi

node - "$MANIFEST" > "$BUILD_ROOT/source-inputs.tsv" <<'JS'
const spec = require(process.argv[2]);
const versions = { FFmpeg: '8.1.1', x264: 'c24e06c2e184345ceb33eb20a15d1024d9fd3497', FreeType: '2.14.1', HarfBuzz: '14.6.0', zlib: '1.3.2', Inter: '4.1', 'Inter source': '4.1' };
if (spec.schemaVersion !== 1) throw new Error('Unsupported media manifest schema');
for (const [name, version] of Object.entries(versions)) {
  if (spec.sources.find(s => s.name === name)?.version !== version) throw new Error('Update this Unix recipe when changing ' + name);
}
for (const source of spec.sources) {
  if (!/^[A-Za-z0-9_.-]+$/.test(source.file) || !/^[0-9a-f]{64}$/.test(source.sha256) || !source.url.startsWith('https://')) throw new Error('Invalid media source entry');
  console.log([source.file, source.url, source.sha256].join('\t'));
}
JS

while IFS=$'\t' read -r file url expected; do
  archive="$BUILD_ROOT/$file"
  if [[ ! -f "$archive" ]]; then
    echo "Downloading verified source: $file"
    curl --fail --location --retry 3 --output "$archive.part" "$url"
    mv "$archive.part" "$archive"
  fi
  node - "$archive" "$expected" <<'JS'
const fs = require('node:fs'), crypto = require('node:crypto');
const actual = crypto.createHash('sha256').update(fs.readFileSync(process.argv[2])).digest('hex');
if (actual !== process.argv[3]) throw new Error('SHA-256 mismatch: ' + process.argv[2]);
JS
  case "$file" in
    *.tar.gz|*.tar.xz) tar -xf "$archive" -C "$BUILD_ROOT" ;;
    Inter-4.1.zip) unzip -oq "$archive" 'extras/ttf/Inter-Regular.ttf' -d "$BUILD_ROOT/inter" ;;
  esac
done < "$BUILD_ROOT/source-inputs.tsv"

export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig"
export CFLAGS="-O2 -ffile-prefix-map=$BUILD_ROOT=media-source"
export CXXFLAGS="$CFLAGS"

cmake -S "$BUILD_ROOT/zlib-1.3.2" -B "$BUILD_ROOT/zlib-build" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS="$CFLAGS" -DCMAKE_INSTALL_PREFIX="$PREFIX" \
  -DCMAKE_INSTALL_LIBDIR=lib -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
  -DZLIB_BUILD_SHARED=OFF -DZLIB_BUILD_STATIC=ON -DZLIB_BUILD_TESTING=OFF
cmake --build "$BUILD_ROOT/zlib-build" --parallel "$JOBS"
cmake --install "$BUILD_ROOT/zlib-build"
# FFmpeg checks the conventional -lz; zlib 1.3.2 calls the static archive zs.
cp "$PREFIX/lib/libzs.a" "$PREFIX/lib/libz.a"

cmake -S "$BUILD_ROOT/freetype-VER-2-14-1" -B "$BUILD_ROOT/ft-build" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS="$CFLAGS" -DCMAKE_INSTALL_PREFIX="$PREFIX" \
  -DCMAKE_INSTALL_LIBDIR=lib -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DBUILD_SHARED_LIBS=OFF \
  -DFT_DISABLE_ZLIB=ON -DFT_DISABLE_BZIP2=ON -DFT_DISABLE_PNG=ON \
  -DFT_DISABLE_HARFBUZZ=ON -DFT_DISABLE_BROTLI=ON
cmake --build "$BUILD_ROOT/ft-build" --parallel "$JOBS"
cmake --install "$BUILD_ROOT/ft-build"

cmake -S "$BUILD_ROOT/harfbuzz-14.6.0" -B "$BUILD_ROOT/hb-build" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS="$CFLAGS" -DCMAKE_CXX_FLAGS="$CXXFLAGS" \
  -DCMAKE_INSTALL_PREFIX="$PREFIX" -DCMAKE_PREFIX_PATH="$PREFIX" -DCMAKE_INSTALL_LIBDIR=lib \
  -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DBUILD_SHARED_LIBS=OFF \
  -DHB_HAVE_FREETYPE=ON -DHB_HAVE_CORETEXT=OFF -DHB_HAVE_GLIB=OFF -DHB_HAVE_GOBJECT=OFF -DHB_HAVE_ICU=OFF \
  -DHB_BUILD_UTILS=OFF -DHB_BUILD_SUBSET=OFF -DHB_BUILD_RASTER=OFF -DHB_BUILD_VECTOR=OFF -DHB_BUILD_GPU=OFF
cmake --build "$BUILD_ROOT/hb-build" --parallel "$JOBS"
cmake --install "$BUILD_ROOT/hb-build"

cd "$BUILD_ROOT/x264-c24e06c2e184345ceb33eb20a15d1024d9fd3497"
bash ./configure --prefix="$PREFIX" --enable-static --enable-pic --disable-cli --disable-opencl
make -j "$JOBS"
make install

cd "$BUILD_ROOT/ffmpeg-8.1.1"
sh ./configure --prefix=/mono-cut-media --enable-gpl --enable-version3 --enable-static --disable-shared \
  --disable-autodetect --disable-network --disable-hwaccels --disable-doc --disable-debug \
  --disable-ffplay --enable-libx264 --enable-libfreetype --enable-libharfbuzz --enable-zlib \
  --pkg-config-flags=--static --extra-cflags="$CFLAGS" --extra-libs="$CXX_RUNTIME"
# Replace only the descriptive configure string's local directory, as in the
# Windows recipe. The original audited upstream sources remain unchanged.
sed "s|$BUILD_ROOT|/build/mono-cut-media|g" config.h > config.h.clean
mv config.h.clean config.h
make -j "$JOBS" ffmpeg ffprobe
cp ffmpeg ffprobe "$RESOURCES/"
cp "$BUILD_ROOT/inter/extras/ttf/Inter-Regular.ttf" "$RESOURCES/Inter.ttf"

node - "$PROVENANCE" "$RECIPE_HASH" "$RESOURCES" "$PLATFORM" "$ARCH" <<'JS'
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');
const dir = process.argv[4], files = {};
for (const name of ['ffmpeg', 'ffprobe', 'Inter.ttf']) {
  files[name] = crypto.createHash('sha256').update(fs.readFileSync(path.join(dir, name))).digest('hex');
}
const version = execFileSync(path.join(dir, 'ffmpeg'), ['-version'], { encoding: 'utf8' });
if (!version.startsWith('ffmpeg version 8.1.1 ')) throw new Error('Unexpected compiled FFmpeg version');
const record = {
  schemaVersion: 1, recipe_sha256: process.argv[3], platform: process.argv[5], architecture: process.argv[6],
  ffmpeg_version: version.split('\n')[0],
  configuration: execFileSync(path.join(dir, 'ffmpeg'), ['-buildconf'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }),
  files,
};
fs.writeFileSync(process.argv[2], JSON.stringify(record, null, 2) + '\n');
JS
echo 'Pinned native FFmpeg 8.1.1/FFprobe and Inter title font compiled and verified.'
