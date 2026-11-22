#!/bin/bash
set -eu
ROOT=$1
WORK=$2
cd "$ROOT"
ICONSET="$WORK/ingetrazo.iconset"
ICNS="$ROOT/resources/icons/ingetrazo.icns"
rm -rf "$ICONSET"; mkdir -p "$ICONSET"
cp resources/icons/ingetrazo_16.png  "$ICONSET/icon_16x16.png"
cp resources/icons/ingetrazo_32.png  "$ICONSET/icon_16x16@2x.png"
cp resources/icons/ingetrazo_32.png  "$ICONSET/icon_32x32.png"
cp resources/icons/ingetrazo_64.png  "$ICONSET/icon_32x32@2x.png"
cp resources/icons/ingetrazo_128.png "$ICONSET/icon_128x128.png"
cp resources/icons/ingetrazo_256.png "$ICONSET/icon_128x128@2x.png"
cp resources/icons/ingetrazo_256.png "$ICONSET/icon_256x256.png"
cp resources/icons/ingetrazo_512.png "$ICONSET/icon_256x256@2x.png"
cp resources/icons/ingetrazo_512.png "$ICONSET/icon_512x512.png"
if [[ -f resources/icons/ingetrazo_master.png ]]; then
    sips -z 1024 1024 resources/icons/ingetrazo_master.png --out "$ICONSET/icon_512x512@2x.png" >/dev/null
else
    sips -z 1024 1024 resources/icons/ingetrazo_512.png --out "$ICONSET/icon_512x512@2x.png" >/dev/null
fi
iconutil -c icns "$ICONSET" -o "$ICNS"
rm -rf "$ICONSET"
