# Published X035 firmware

`manifest.json` is what `voloo firmware update` reads, from
`https://voloo.librezoo.com/firmware/x035/manifest.json` (this repository's `main`
copy is the fallback). Packages are the encrypted universal `.voloofw` files; the device
authenticates them, the manifest only says which one fits which hardware and host.

To publish a firmware: copy the package here, add an entry (`version`, `code` = major<<8|minor,
`sha256`, `size`, `hardware`, `min_host`, two `urls`: website first, GitHub release second),
build the site (`website/build.py`) and deploy it. To withdraw one, remove its entry and redeploy.
Never edit an entry's package in place; add a new version.
