#!/bin/sh
# Install the editor's shared FFmpeg and private CLI without replacing a system ffmpeg.
set -eu
source_prefix=$1
bin_directory=$2
lib_directory=$3
shift 3
install -d "$lib_directory"
install -m644 external/ffmpeg/COPYING* external/ffmpeg/LICENSE.md "$lib_directory/"
for name in avcodec avdevice avfilter avformat avutil swresample swscale; do
    cp -a "$source_prefix/lib/lib$name.so"* "$lib_directory/"
done
for library in "$lib_directory"/lib*.so*; do
    if [ ! -L "$library" ]; then
        patchelf --set-rpath '$ORIGIN' "$library"
    fi
done
install -m755 "$source_prefix/bin/ffmpeg" "$bin_directory/shrimply-ffmpeg"
relative_lib=$(realpath --relative-to="$bin_directory" "$lib_directory")
for executable in shrimply-ffmpeg "$@"; do
    patchelf --add-rpath "\$ORIGIN/$relative_lib" "$bin_directory/$executable"
done
