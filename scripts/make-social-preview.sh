#!/usr/bin/env bash
# Build the 1280x640 GitHub social preview image from the editor screenshot.
#
#   scripts/make-social-preview.sh [OUTPUT.png]
#
# Upload the result under the repository's Settings > Social preview; GitHub
# has no API for it.
set -euo pipefail

project_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
output=$(realpath -m -- "${1:-$project_dir/docs/images/social-preview.png}")
screenshot="$project_dir/docs/images/wayexpand-editor-try-it-live.png"
icon="$project_dir/assets/icon/wayexpand-512.png"
bold=/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf
regular=/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf
mono=/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf

command -v convert >/dev/null || {
    printf '%s\n' 'error: ImageMagick (convert) is required' >&2
    exit 1
}
for font in "$bold" "$regular" "$mono"; do
    [ -f "$font" ] || {
        printf 'error: font %s is missing (fonts-noto-core, fonts-dejavu-core)\n' "$font" >&2
        exit 1
    }
done

work=$(mktemp -d "${TMPDIR:-/tmp}/wayexpand-social.XXXXXXXX")
trap 'rm -rf "$work"' EXIT

# The screenshot with rounded corners and a soft drop shadow.
convert "$screenshot" -resize 700x \
    \( +clone -alpha extract \
    -draw 'fill black polygon 0,0 0,14 14,0 fill white circle 14,14 14,0' \
    \( +clone -flip \) -compose Multiply -composite \
    \( +clone -flop \) -compose Multiply -composite \) \
    -alpha off -compose CopyOpacity -composite "$work/rounded.png"
convert "$work/rounded.png" \( +clone -background black -shadow 70x18+0+12 \) \
    +swap -background none -layers merge +repage "$work/shot.png"

convert -size 1280x640 gradient:'#121528'-'#24123a' \
    \( "$icon" -resize 88x88 \) -geometry +64+150 -composite \
    -font "$bold" -pointsize 64 -fill white -annotate +64+320 'WayExpand' \
    -font "$regular" -pointsize 28 -fill '#d9d4f2' -annotate +66+372 'Text expansion for Wayland' \
    -pointsize 22 -fill '#a59fc6' \
    -annotate +66+420 'Snippets, picker, and config-file templates' \
    -annotate +66+452 'Local · offline · no telemetry' \
    -font "$mono" -pointsize 20 -fill '#7ee0c3' -annotate +66+520 '$ wayexpand new playbook' \
    "$work/shot.png" -geometry +520+76 -composite \
    -strip -depth 8 "$output"
printf 'Wrote %s\n' "$output"
