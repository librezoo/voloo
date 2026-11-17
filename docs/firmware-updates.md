# USB firmware updates and rollback

Use the CLI or dashboard to update a compatible USB-C Voloo Switch. If the host
reports an unsupported device or an incompatible package, stop and contact support.
Do not erase the device or change its protection settings to bypass an update error.

## Before updating

Install the host version recommended by the release and connect the target by USB.
Run `voloo ls` and copy its complete UID. Replace `DEV` below with that UID or a
unique nickname. The device must already be idle OFF, with valid VIN/VOUT readings
and VOUT <= 1 V. OFF alone does not mean zero voltage. Updating does not switch
off, discharge, or restore an earlier ON state for you. Device locks still apply.

```sh
voloo firmware status DEV
voloo status DEV
voloo firmware update DEV
```

With several Voloo connected, `voloo firmware update` (no device) first lists each device
that will be updated and each one skipped with the reason (already up to date, locked, not
idle OFF with VOUT <= 1 V, needs a newer voloo), asks once in a terminal, and then updates one
device at a time; a skipped or failed device never stops the others and the exit code is 2
when any update failed. Scripts pass `--all` to skip the question; `--all` cannot be combined
with a device or a local file. With one Voloo connected the command behaves as before.

The online command reads the published firmware manifest, chooses the newest encrypted
firmware that supports the device's hardware identity and this CLI version, checks its
SHA-256 and never installs older firmware than the device runs. If a newer firmware needs
a newer host it says so; it does not install anything it cannot verify. For an offline file from
the matching official release:

```sh
voloo firmware inspect /path/to/firmware.voloofw
voloo firmware update DEV /path/to/firmware.voloofw
```

`inspect` checks file structure without touching a device; it does not replace
device-side authentication. Use only the package recommended for your device.

In `voloo ui`, open device details under **USB firmware**. Hold **Update firmware**
for an online update, or select the matching local file. The dashboard displays
progress and the result in English, Simplified Chinese or Traditional Chinese.

## Update notices

Once a day at most, the background process reads the public firmware manifest (one small
GET; nothing about you or your devices is sent) and notes which connected devices could be
updated. `voloo ls`, `voloo status` and `voloo info` then print one line in a terminal, once
per device and version; JSON, MCP and piped output stay unchanged apart from a read-only
`firmware_update_available` field, and the dashboard shows the version in the USB firmware
panel. The same manifest names the newest voloo (npm) version; when yours is older you get
one line saying so (merged with the firmware line when a firmware needs a newer voloo). Nothing is ever installed automatically. Turn it off with `check_updates = false` in
`config.toml` or `VOLOO_NO_UPDATE_CHECK=1`.

## During and after updating

Keep USB power stable. **Do not stop the daemon during a firmware operation.**
It manages serial ownership and reconnection to the same UID. Close third-party
serial applications that own the port. Other devices remain independently available.

The host checks identity, version, nickname, state and valid measurements before
confirming the new image. A failed trial returns to the previous confirmed image
when recovery is available. After an unknown result, read fresh state before any
further action; do not blindly repeat a write.

```sh
voloo firmware status DEV
voloo info DEV
voloo status DEV
```

The output remains OFF. The host removes its temporary encrypted download after
success or handled failure. Forced process termination or OS power loss may leave
a temporary file; it is not a persistent firmware cache.

## Rollback and errors

```sh
voloo firmware rollback DEV
```

Rollback requires an available previous image and the same state/measurement
checks. A new upload can replace the previous-image slot. If the host reports no
usable previous image, persistent communication failure, or incompatible firmware,
retain the report and contact support. Do not use generic erase commands as recovery.
Interrupted initial installation or physical hardware damage is outside ordinary
application rollback.

## Upgrading the host program

Installing a new npm package does not replace an already running older daemon.
Wait for every device operation to finish, run `voloo daemon stop` once, then use
the new `voloo ls` and check `voloo daemon status`. This host replacement is separate
from ordinary firmware updating, during which the daemon stays running.

## Scope of verification

Automated tests cover managed transfers, identity checks, interrupted communication,
failed trials, rollback and download cleanup. Published release notes identify
tested platforms and physical devices. Simulated failures are not physical power-cut,
Flash-endurance, loaded-switching or whole-board qualification.
