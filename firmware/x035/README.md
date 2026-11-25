# X035 firmware distribution

Publish encrypted `.voloofw` packages as assets on [Voloo GitHub Releases](https://github.com/librezoo/voloo/releases).
Keep firmware images outside the Git checkout, npm packages and website build.

For a host release, attach `voloo-switch-x035.voloofw` to the matching draft
release before pushing the host tag. The release workflow inspects the asset and
checks the public download before publishing the npm packages.

To announce new firmware, add an entry to `manifest.json` once its versioned
Release download is public. Record the firmware version, `code` (major shifted
left eight bits plus minor), SHA-256, byte size, compatible hardware, minimum
host version and the GitHub Release asset URL in `urls`. Publish a new version
for changed bytes so existing clients can verify the download.

Deploy the updated manifest to the product website. The CLI reads it there and
uses the copy on this repository's `main` branch as a fallback. To withdraw a
version, remove its manifest entry and redeploy. When a new host version is
published on npm, update `host.latest` and deploy the manifest for older clients.
