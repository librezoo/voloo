# USB firmware updates and A/B rollback

These are local, unreleased CLI/dashboard changes. The connected X035 USB test
board is running A/B firmware 2.9; the npm release has not been republished.

## Update or roll back

Use a trusted local firmware file. Replace `DEV` with the complete UID or a unique
nickname; writes always require an explicit device.

```sh
voloo firmware status DEV
voloo firmware update DEV /path/to/firmware.voloofw
voloo firmware rollback DEV
```

In `voloo ui`, open the device details, select a file under **USB firmware**, and
hold **Update firmware** to confirm. **Roll back firmware** starts the previous
confirmed image when one is available. The page shows upload, reconnection,
verification, confirmation and rollback results. English, Simplified Chinese and
Traditional Chinese are supported.

The device must be idle OFF, with valid VIN/VOUT and VOUT <= 1 V. Updating never
switches off, discharges or restores an earlier ON for you. If needed, explicitly
turn off/discharge first. Configuration/runtime locks still refuse firmware changes.

**Do not stop the daemon.** Its device worker owns the whole maintenance operation,
including releasing the exclusive serial handle when needed and reacquiring the
same UID after USB re-enumeration. Scanning/polling and new jobs for this device
are suspended; other devices and the local HTTP/SSE service remain available.
The CLI's firmware commands therefore use managed mode, not `--direct/--port`.
A third-party serial application still needs to release its own handle.

The daemon finishes an accepted operation even if the CLI or browser disconnects.
It never repeats a modifying command because its acknowledgement was lost. It
reads the transfer offset or boot state to establish the result, or reports an
unknown outcome. Check fresh state before another action after an unknown result.

## What is verified

A `.voloofw` package identifies the X035 hardware, version and separately linked
A/B images. The host rejects a wrong target, slot, size or inconsistent factory
image before starting maintenance. Ordinary A/B updates write only the inactive
slot. The device verifies each programmed block and the existing CRC16 before
staging it; the current confirmed image stays intact.

The new image boots provisionally. The host reads back the same UID, hardware,
expected package version, nickname and idle OFF/measurement state, then sends one
confirmation. Only a committed confirmation makes it current. Flash erase and USB
startup can briefly invalidate measurements; the host waits read-only for a fresh
valid group rather than bypassing the device's checks.

A hanging trial is reset by the independent watchdog. A running trial without host
confirmation resets after 15 seconds. The boot program then selects the previously
confirmed image. The host reconnects and verifies that image and OFF state, reports
`rolled_back`, and exits 2 because the requested upgrade did not succeed. An
explicit rollback is also trial-booted and verified before it is confirmed.

The previous image occupies the inactive slot until the next upload. Starting a
new upload overwrites that previous image; an interrupted upload preserves the
current confirmed application but can leave no separate rollback image until a
later successful upgrade.

## Legacy firmware and first installation

Firmware 2.8 uses software entry into WCH USB ROM ISP and requires `wchisp` on the
host PATH (or `VOLOO_WCHISP`). The worker releases the CDC handle, verifies that
exactly one matching chip/UID is in ROM ISP, programs with the configuration-safe
erase bound, verifies, reconnects and checks the application. No manual pad short
or daemon stop is needed when the running device supports this entry.

A/B packages include the first-install image, so the same command can migrate 2.8
to the new boot program and dual-slot layout. This initial layout migration cannot
keep the old address-0 2.8 application as an A/B fallback. **Power loss that damages
the boot program during this first migration can still require ROM pads or
WCH-Link recovery.** Normal subsequent A/B updates do not write the boot program.

A/B firmware uses native CDC updating and does not need `wchisp`. It rejects raw
single-image BIN files. Legacy USB-updatable `.bin` files remain supported on 2.8,
but have no A/B protection and encode no expected version: their result reports the
actual read-back version, with `expected_version_verified: false`.

Package target identifiers and CRC16 are not firmware-source authentication. Use
trusted local files. Firmware/host publishing and other-platform qualification are
outside this local change.

## Local API

The existing loopback/Host/origin checks, bearer/browser-session authentication,
locks and operation logging apply unchanged.

| Endpoint | Meaning |
| --- | --- |
| `GET /v1/devices/{id}/firmware` | Fresh USB support and A/B boot state |
| `POST /v1/devices/{id}/firmware` with `{"data":[byte,...]}` | Update from local file bytes, up to 512 KiB |
| `POST /v1/devices/{id}/firmware/rollback` | Verify and confirm the previous image |
| `GET /v1/devices` / SSE `firmware.progress` | Maintenance progress without opening another serial transaction |

Other device jobs return Busy during maintenance; they are not queued across a
reset. Completed operations and failures are fsynced into the existing journal.

## Evidence and limits

The host tests run only against simulations/PTYs: same-process maintenance,
concurrent-job refusal, another device remaining available, lost BEGIN/WRITE/END/
confirmation acknowledgements without repeats, failed upload cleanup, failed trial
rollback, valid-measurement settling, firmware type/size checks and UI guards.

One real X035 USB board (`CD-AB-75-F3-69-BE-AB-5D`) has exercised migration from
2.8, inactive-slot updating, explicit rollback and a deliberately non-booting trial
that the watchdog rolled back to the confirmed 2.9. The daemon PID stayed unchanged
across device updates. The real dashboard upgraded 2.9 to 2.10 and rolled back
to 2.9, and showed a healthy but version-mismatched trial waiting for automatic
rollback without enabling power controls. A prototype watchdog initialization error was found on the
first migration, recovered from the exact full-flash backup, corrected from WCH's
X035 example and regression-tested before the successful retest.

Raw backups and evidence are under the workspace's outer
`output/voloo-integrated-updates-20261112/`; they are not committed. Firmware C tests
simulate power cuts at metadata erase/program word boundaries; this is not a
physical power-disconnection qualification. Load protection, physical 30.5 V trip
accuracy/latency, flash endurance and other hardware revisions are not qualified by
these update tests.
