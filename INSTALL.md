# Installing rod on a Raspberry Pi

rod runs on a Raspberry Pi (3, 4, or 5) and bridges your IAI
actuator to the web app over Bluetooth. There are two ways to install it — pick
the one that matches you.

> Replace `OWNER/rod` below with this project's GitHub repo.

---

## Option A — Flash a ready-made image (easiest, no terminal)

Best if you're starting with a blank SD card. You don't touch a command line at
all.

1. **Download the image** for your Pi from the project's
   [Releases](https://github.com/OWNER/rod/releases) page:
   - Pi 5 → `rod-rpi5_64.img.gz`
   - Pi 4 / 400 / CM4 → `rod-rpi4_64.img.gz`
   - Pi 3 → `rod-rpi3_64.img.gz`
2. **Install [Raspberry Pi Imager](https://www.raspberrypi.com/software/)** on your computer and open it.
3. Click **"Choose OS" → "Use custom"** and select the `.img.gz` you downloaded.
4. Click **"Choose Storage"** and pick your SD card.
5. Click the **gear / ⚙ (Edit settings)** before writing and set:
   - **Wi-Fi** name + password (so it can reach the internet),
   - **Hostname**: `rod`,
   - optionally enable **SSH** if you ever want to log in.
6. **Write** the card, put it in the Pi, plug the actuator into a USB port, and
   power it on. Give it ~30 seconds to boot.
7. Open the **Rod web app** in Chrome or Edge and **Connect via
   Bluetooth** — the device shows up as `Rod-…`.

That's it. It starts automatically on every boot.

---

## Option B — One-line installer (you already run Raspberry Pi OS 64-bit)

Best if your Pi is already set up. Open a terminal on the Pi (or SSH in) and run:

```bash
curl -sSL https://raw.githubusercontent.com/OWNER/rod/main/scripts/install.sh | sudo bash
```

It will:
- download the latest release binary for your Pi and verify its checksum,
- install it as a **systemd service** that starts on boot,
- add a default config at `/etc/rod/config.toml` (your existing one is kept),
- set the hostname to `rod` and install Avahi so it's reachable at
  `rod.local`.

**Update later:** just run the same command again — it replaces the binary and
leaves your config alone.

**Uninstall:**
```bash
curl -sSL https://raw.githubusercontent.com/OWNER/rod/main/scripts/uninstall.sh | sudo bash
# add --purge to also delete /etc/rod
```

---

## After install — useful commands

```bash
sudo systemctl status rod     # is it running?
journalctl -u rod -f          # live logs
sudo systemctl restart rod    # restart it
```

Config lives at **`/etc/rod/config.toml`** — edit it, then restart the
service.

---

## Troubleshooting

**The device doesn't appear in the web app's Bluetooth list.**
- Use **Chrome or Edge** (desktop or Android). Web Bluetooth is **not** available
  in Safari / on iPhone.
- Make sure the Pi is powered and has finished booting (~30 s).
- Bluetooth radio soft-blocked? `sudo rfkill unblock bluetooth` (the service
  also does this on start).
- Check the logs: `journalctl -u rod -e`.

**Connected, but the actuator doesn't move.**
- Confirm the actuator is plugged into USB and powered.
- The serial device defaults to `ttyUSB0`; if yours differs, set
  `serial_device` in `/etc/rod/config.toml` and restart.
- Watch `journalctl -u rod -f` while you send a command.

**I can't reach `rod.local`.**
- mDNS needs Avahi on the Pi (the installer adds it) and an mDNS-capable client
  (macOS/iOS built-in; Windows via Bonjour; Linux via `avahi`). You can always
  use the Pi's IP address instead.

---

## Building it yourself

- Cross-compile the binary: `scripts/build-pi.sh` (Docker) — see also the
  `build-pi` workflow.
- Build a flashable image: `scripts/setup-buildroot.sh` + the `buildroot`
  workflow (`buildroot-external/`).
