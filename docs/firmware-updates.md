# USB firmware updates and A/B rollback

Version 0.7.0-rc.4 supports automatic encrypted USB updates for factory-initialized
CH32X035 USB boards running secure firmware 3.0. The firmware source and factory
keys are private. npm contains the host program, not plaintext firmware or keys.

## Update or roll back

Replace `DEV` with the complete UID or a unique nickname. Writes always require an
explicit device.

```sh
voloo firmware status DEV
voloo firmware update DEV                  # download the release paired with this CLI
voloo firmware update DEV /path/to/firmware.voloofw
voloo firmware rollback DEV
voloo firmware inspect /path/to/firmware.voloofw  # offline structural inspection
```

In `voloo ui`, open the device details under **USB firmware** and hold **Update
firmware**. No file selection is needed for online updating. An optional local
file can be selected for offline updating. **Roll back firmware** starts the
previous confirmed image when available. The page shows download, upload,
reconnection, verification, confirmation and rollback results in English,
Simplified Chinese or Traditional Chinese.

The device must be idle OFF, with valid VIN/VOUT and VOUT <= 1 V. Updating never
switches off, discharges or restores an earlier ON for you. Configuration/runtime
locks still refuse firmware changes.

**Do not stop the daemon.** Its device worker owns maintenance, releases the
exclusive serial handle when needed, and reacquires the same UID after USB
re-enumeration. Polling and new jobs for that device are suspended; other devices
and HTTP/SSE remain available. Firmware commands use managed mode, not
`--direct/--port`. A third-party serial application must release its own handle.

The daemon finishes an accepted operation even if the CLI or browser disconnects.
It does not repeat a modifying command after a lost acknowledgement. It reads the
transfer offset or boot state to establish the result, or reports an unknown
outcome. Check fresh state before another action after an unknown result.

## Download and cleanup

The host downloads only this public release's paired encrypted asset:
`https://github.com/librezoo/voloo/releases/download/v<CLI-version>/voloo-switch-x035.voloofw`.
Updating the npm app selects its paired firmware; this command does not search
arbitrary newer releases. HTTPS, a 30-second request timeout and a 512 KiB limit
apply. A plaintext download is refused.

Only ciphertext is staged in a unique directory under the operating system's
temporary directory (`TMPDIR` on macOS, normally `/tmp` on Linux, the user temp
directory on Windows). On Unix the directory is owner-only. It is kept until
verification or rollback finishes, then the file and directory are removed on
success or handled failure, including interrupted/truncated downloads. There is
no persistent firmware cache. Forced process termination or OS power loss can
leave ciphertext in the temporary directory; file deletion is not secure erasure.

Online updates first require device-reported read protection plus boot/key write
protection (`ab.encrypted: true`, `ab.protection_flags: 14`). They refuse devices
whose factory security initialization is incomplete before downloading anything.
The encrypted asset must contain a ticket for the device UID.

## Device authentication and confidentiality

Secure packages use standard XChaCha20-Poly1305 (Monocypher 4.0.3 on the MCU,
interoperability tested against libsodium). Each device has an independent random
256-bit root key provisioned by the trusted factory. Each package uses a fresh
random content key, wrapped separately for each enrolled device. The host never
receives a plaintext key or application.

The device authenticates the UID-bound ticket before invalidating the inactive
slot. Each encrypted chunk binds the package header, slot, offset and length;
it is authenticated before decryption/programming. The complete image is read
back and checked with the existing CRC16. A root-key authentication tag also binds
the stored application, slot, version and length, and is checked by the boot
program before execution. CRC16 alone is not source authentication. Offline
`firmware inspect` checks structure and rejects private/plaintext fields; only an
enrolled device can authenticate the ciphertext.

Chip read protection blocks normal external Flash readout. Write protection
covers the boot program and device key at offsets `0x0000..0x1FFF`; normal updates
write the inactive application slot and boot metadata. Calibration and nickname
pages remain writable. Protection is a separate trusted factory operation, not
an npm command. Removing read protection later mass-erases user Flash, including
keys and board configuration; it is not a reversible debug toggle.

This raises the cost of firmware extraction; it cannot guarantee that hardware
fault injection, invasive attacks or a future application vulnerability will
never expose code or a key. The implementation has not had an independent
security audit. Deleting a temporary download does not itself prevent reversing.
A/B rollback deliberately permits an authenticated previous version; this is not
an anti-downgrade policy.

## Trial boot and recovery

Normal updates write only the inactive slot. The confirmed application stays
intact. The new image boots provisionally; the host verifies the same UID,
hardware, package version, nickname, idle OFF state and valid measurements before
sending one confirmation. Only committed confirmation makes it current.

A hanging trial resets through the independent watchdog. A running trial without
host confirmation resets after 15 seconds. The boot program then selects the
previous confirmed application. The host verifies that image and OFF, reports
`rolled_back`, and exits 2 because the requested upgrade failed. Explicit rollback
also trial-boots and verifies the previous image before confirmation.

A new upload overwrites the previous-image slot. Interrupted upload preserves
the current confirmed application but can leave no separate rollback image until
a later successful upgrade. Corrupt boot/key storage or damage during initial
factory provisioning is outside application A/B recovery.

## Legacy devices and factory initialization

Legacy 2.8 and plaintext A/B 2.9 local files remain supported by the host, with
existing safety checks. 2.8 ROM ISP updating needs `wchisp`; plaintext A/B uses
native CDC. They cannot install the sealed public package by themselves. No
plaintext first-install image, production key or factory vault is publicly
provided. Transitioning an existing uninitialized board to secure 3.0 requires
one trusted provisioning operation with private factory tools and preserved
per-device configuration. Subsequent ordinary updates use USB without pad shorts.

The initial boot-layout migration has no old-layout fallback. Power loss during
that first migration can require physical recovery. Once read protection is
active, recovery that removes it erases configuration and device credentials and
requires trusted reprovisioning. Do not use generic erase/unprotect commands as
an upgrade procedure.

## Local API

Existing loopback/Host/origin checks, authentication, locks and operation logging
apply to firmware operations.

| Endpoint | Meaning |
| --- | --- |
| `GET /v1/devices/{id}/firmware` | Fresh USB support and boot/protection state |
| `POST /v1/devices/{id}/firmware` with `{}` | Download paired encrypted firmware, update and remove temporary file |
| Same POST with `{"data":[byte,...]}` | Update from a local file, up to 512 KiB |
| `POST /v1/devices/{id}/firmware/rollback` | Verify and confirm the previous image |
| Device snapshots / SSE `firmware.progress` | Maintenance progress |

Other jobs for the target return Busy during maintenance. They are not queued
across a reset. Results and failures are fsynced into the existing journal.

## Evidence and limits

Host tests cover simulated device/PTY maintenance, concurrent-job refusal, other
devices remaining available, lost acknowledgements without modifying retries,
failed transfer cleanup, failed-trial rollback and valid-measurement settling.
Verification retries transient read-only failures for up to two seconds; persistent
loss still refuses confirmation and rolls back. Modifying requests are never
repeated. Deterministic tests exercise both one lost trial-status reply and
persistent loss.
Encrypted download tests cover success/failure cleanup, truncation, read errors,
oversized/plaintext downloads, and pairing with the CLI version. UI tests cover
protection initialization and OFF/lock/voltage requirements.

Independent libsodium/MCU C tests cover encrypted ticket/chunk interoperability,
wrong UID, tampering before erase/staging, stored-image authentication, trial
rollback and preservation of unrelated protection options.

One real USB board (`CD-AB-75-F3-69-BE-AB-5D`) has exercised encrypted 3.0 uploads,
explicit rollback, bad-ticket/bad-chunk refusal and an authenticated but
non-booting candidate returning to the confirmed image through the watchdog.
After explicit approval, read/boot/key protection was enabled and read back as
flags 14; encrypted updating and explicit rollback passed again with protection
active, followed by a USB-only cold boot. A read-only WCH-Link check did not recover
the provisioned boot/key bytes; attaching the debugger interrupted execution and
required disconnecting it and power-cycling USB. Leave the debugger disconnected
for normal use. Online release downloading is a separate remaining check.
Earlier plaintext A/B tests also covered 2.8 migration and the real dashboard.

Evidence is kept outside the repository in workspace `output/` directories;
private key-containing backups stay in the private factory directory. Simulated
metadata power cuts are not physical power-disconnection qualification. These
update tests do not qualify Flash endurance, loaded switching, physical 30.5 V
trip accuracy/latency or other board revisions.
