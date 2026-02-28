# Linux Setup Guide

This guide covers the additional setup required to run Core Deck on Linux.

## System Dependencies

Install the required development libraries:

### Debian/Ubuntu
```bash
sudo apt install libudev-dev libhidapi-dev
```

### Fedora/RHEL
```bash
sudo dnf install systemd-devel hidapi-devel
```

### Arch Linux
```bash
sudo pacman -S hidapi
```

## HID Device Permissions

By default, Linux requires root access to communicate with HID devices. To allow non-root access to the Core Deck:

### 1. Create udev rules file

Create `/etc/udev/rules.d/99-coredeck.rules`:

```bash
sudo tee /etc/udev/rules.d/99-coredeck.rules << 'EOF'
# Core Deck QMK Raw HID device
# VID: 0xFEED, PID: 0x0803 (default QMK VID/PID)
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="feed", ATTRS{idProduct}=="0803", MODE="0666"
SUBSYSTEM=="usb", ATTRS{idVendor}=="feed", ATTRS{idProduct}=="0803", MODE="0666"
EOF
```

If your device uses different VID/PID values (check your QMK config), update the rules accordingly.

### 2. Reload udev rules

```bash
sudo udevadm control --reload-rules
sudo udevadm trigger
```

### 3. Reconnect the device

Unplug and replug the Core Deck for the new rules to take effect.

## Verification

After setup, verify the device is accessible:

```bash
# List HID devices (should show Core Deck without sudo)
ls -la /dev/hidraw*

# Check device permissions
stat /dev/hidraw* | grep -E "(File|Access)"
```

## Troubleshooting

### Device not found
- Check that the device is connected: `lsusb | grep -i feed`
- Verify VID/PID matches your udev rules
- Ensure udev rules were reloaded after creation

### Permission denied
- Verify the udev rule file syntax
- Check file is in correct location: `ls -la /etc/udev/rules.d/99-coredeck.rules`
- Try logging out and back in, or reboot

### hidapi initialization fails
- Ensure libhidapi is installed
- Check if you're running on Wayland (some older hidapi versions have issues)
