#!/usr/bin/env bash
# Writes the AVD the Android runs use, by hand (avdmanager needs Java 17, which crashes on
# this machine): ~/.android/avd/moshpit-g3.{ini,avd/config.ini}, on the
# Android 15 Google Play arm64 image. A small phone (720x1600, xhdpi) keeps
# software rendering (swiftshader) cheap. Creating it boots nothing.
set -euo pipefail

: "${ANDROID_HOME:=$HOME/Library/Android/sdk}"
AVD_NAME=moshpit-g3
AVD_HOME="${ANDROID_AVD_HOME:-$HOME/.android/avd}"
IMAGE=system-images/android-35/google_apis_playstore/arm64-v8a
[ -e "$ANDROID_HOME/$IMAGE/system.img" ] || { echo "missing $ANDROID_HOME/$IMAGE" >&2; exit 1; }

mkdir -p "$AVD_HOME/$AVD_NAME.avd"
cat > "$AVD_HOME/$AVD_NAME.ini" <<EOF
avd.ini.encoding=UTF-8
path=$AVD_HOME/$AVD_NAME.avd
path.rel=avd/$AVD_NAME.avd
target=android-35
EOF
cat > "$AVD_HOME/$AVD_NAME.avd/config.ini" <<EOF
AvdId=$AVD_NAME
avd.ini.displayname=Moshpit (W12 runs; made for spike G3)
avd.ini.encoding=UTF-8
abi.type=arm64-v8a
hw.cpu.arch=arm64
hw.cpu.ncore=2
hw.ramSize=2048
vm.heapSize=256
disk.dataPartition.size=4G
image.sysdir.1=$IMAGE/
tag.id=google_apis_playstore
tag.display=Google Play
target=android-35
PlayStore.enabled=true
hw.lcd.width=720
hw.lcd.height=1600
hw.lcd.density=320
hw.initialOrientation=portrait
hw.keyboard=yes
hw.mainKeys=no
hw.dPad=no
hw.gpu.enabled=yes
hw.gpu.mode=swiftshader_indirect
hw.audioInput=no
hw.audioOutput=no
hw.camera.back=none
hw.camera.front=none
hw.gps=no
hw.sensors.proximity=no
showDeviceFrame=no
skin.dynamic=yes
skin.name=720x1600
fastboot.forceColdBoot=yes
EOF
echo "AVD $AVD_NAME at $AVD_HOME/$AVD_NAME.avd"
