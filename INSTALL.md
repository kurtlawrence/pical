# Alpine Linux Installation

- https://wiki.alpinelinux.org/wiki/Raspberry_Pi
- https://wiki.alpinelinux.org/wiki/Create_a_Bootable_Device#Manually_copying_Alpine_files
- https://github.com/macmpi/alpine-linux-headless-bootstrap
- https://github.com/macmpi/alpine-linux-headless-bootstrap/blob/main/sample_wpa_supplicant.conf

1. Download Alpine for Raspberry Pi tarball
2. Create bootable FAT32 partition on SD card
- Ensure partition type is `W95 FAT32 (LBA)`
3. Extract tarball to root of SD card

```sh
# Naviate to SD card mount point
cd /media/<user>/<sd-card>
# Extract Alpine linux
tar -xvzf path/to/alpine-rpi-<version>.tar.gz
```

4. Download `headless.apkovl.tar.gz` from https://github.com/macmpi/alpine-linux-headless-bootstrap
5. Copy the `headless.apkovl.tar.gz` onto SD card (as is, no extraction)
6. Copy the `wpa_supplicant.conf` onto SD card

```sh
wget https://github.com/macmpi/alpine-linux-headless-bootstrap/raw/main/sample_wpa_supplicant.conf -O wpa_supplicant.conf
cp wpa_supplicant.conf /media/<user>/<sd-card>/ -v
```

7. Edit `wpa_supplicant.conf` with WiFi SSID and passphrase
8. Boot Raspberry Pi with SD card
9. Find IP address of Raspberry Pi, `ssh` into it, setup Alpine

```sh
# Find the device, the MAC address should start with B8:27:EB
# You might get a line item: Nmap scan report for alpine-headless (10.0.0.###)
nmap -sP 192.168.0.0/24 # or 10.0.0.0/24
# ssh into raspberry
ssh root@<IP>
# Recommended to rename headless bootstrap
mount -o remount,rw /dev/mmcblk0p1 # NOTE: device name might be different
mv /media/mmcblk0p1/headless.apkovl.tar.gz /media/mmcblk0p1/headless.apkovl.tar.gz.old
# Setup Alpine
setup-alpine
# Update and commit
apk update
apk upgrade
lbu commit -d
```

10. Reboot and check it all works!

# pical Installation

1. Build for arm/musl target
- RasberryPi Zero with Alpine Linux uses `arm-unknown-linux-musleabihf`

```sh
cargo build --release --target arm-unknown-linux-musleabihf
```

2. Copy binary to Raspberry Pi

```sh
rsync {TARGET_DIR}/arm-unknown-linux-musleabihf/release/pical {RaspberryPi}:~/pical -vzh
```

3. Commit the changes

```sh
# on raspberry pi
lbu commit -d
```

# Running pical

1. Run `pical`

```sh
./pical # maybe run on another screen: screen ; ./pical
```

2. Make any configuration changes in `config.pical.toml`

```toml
width = 800             # Width of image (in pixels)
height = 600            # Height of image (in pixels)
zoom = 1                # The amount to increase sizing of text
scaling = 1             # The 'upscaling' factor, can make images more smooth
display_refresh = "30s" # How often to redraw the image
timezone = "+10:00:00"  # Timezone UTC offset
calendars = [[          # list of calendar tuples
    "Name",
    # example - "https://calendar.google.com/calendar/ical/..."
    "URL for iCal data",
]]
coords = [-27.467900,153.032500] # [latitude, longitude]
# API key to stormglass.io
stormglassio_apikey = "KEY"
```
