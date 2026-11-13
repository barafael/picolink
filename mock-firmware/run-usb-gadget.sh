#!/bin/sh
# Runs the mock firmware as a USB device on this machine, through the kernel's dummy_hcd. Linux only.
# The server then finds it like the real firmware, no PICOLINK_MOCK_FIRMWARE needed. Stop with Ctrl-C.
set -eu
cd "$(dirname "$0")/.."

cargo build -p picolink-mock-firmware --features usb-gadget
sudo modprobe dummy_hcd
if [ ! -e /etc/udev/rules.d/70-picolink.rules ]; then
    echo "Note: udev/70-picolink.rules is not installed, so only root can open the device." >&2
fi
exec sudo target/debug/picolink-mock-firmware
