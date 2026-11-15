# voloo public website and media

The public product website is [voloo.librezoo.com](https://voloo.librezoo.com/). Chinese is the default; [English](https://voloo.librezoo.com/index.en.html) has its own URL. Landing pages, quick starts, full manuals and downloads link to their matching language. No language redirect hides the installation command.

## Content and source

- `site/page.html`: Chinese landing-page source, including the clearly labeled browser-only simulator. `build.sh` generates `site/index.html`.
- `i18n/en.json`: reviewed English text; `build_english.py` generates English landing, quick-start and poster pages. Client configuration examples and commands keep their original syntax.
- `../docs/user-guide.md`, `../docs/user-guide.en.md`: canonical full manuals. `build_guide.py` renders both; do not edit the generated guide pages independently.
- `artwork/build_quick_start.py`, `artwork/build_quick_start_en.py`: illustrated quick-start HTML, terminal SVGs and two-page A4 PDFs. The PDF generator uses the manufacturer-specified **11 mm** strip length, [KEFA KF142V-5.08 / LCSC C475134](https://www.lcsc.com/datasheet/C475134.pdf).
- `poster/`: editable Chinese and English poster / long-image pages and shared website QR source. `render_media.mjs` exports individual PNG / JPG files.
- `site/assets/`: approved public product renderings, brand marks, illustrations and downloadable documents / media. Product renderings are AI-assisted, based on the actual yellow / purple enclosure, green terminals, orange actuators and red positive / black negative leads. The original reference photograph is private and is not stored here.
- `build_site_chrome.py` and `site/assets/site-chrome.css`: shared brand navigation, localized footer, and the same favicon / touch icon as librezoo.com. LibreZoo links to the parent website; the header voloo wordmark returns to the top of the current page.
- `build_downloads.py`: Chinese / English downloads pages. Every poster and PDF QR code points to **https://voloo.librezoo.com/**.

These are static product pages and simulated interactions, not a public remote-control endpoint for physical hardware. No daemon, local token, private firmware, provisioning material, factory backup or maintenance log is deployed.

## Build and preview

Use Python 3.10+, `uv`, Node.js / npm and an installed Google Chrome for media export:

```sh
uv run --no-project --with reportlab --with svglib --with qrcode --with beautifulsoup4 --with markdown-it-py --with fonttools --with brotli python website/build.py
python3 -m http.server 8000 --bind 127.0.0.1 --directory output/website
```

Open `http://127.0.0.1:8000/`. Build output is `output/website/` and `output/pdf/`, both ignored by Git. `site/` retains reviewed publication files for inspection and direct maintenance. PDFs and download images are published documentation assets; screenshots, temporary exports and logs belong in ignored `output/`.

The Chinese PDF requires an embeddable CJK TrueType font. On macOS the default is Arial Unicode; on other systems set `VOLOO_DOC_FONT` to an appropriate installed font. The PDF source font is not copied into the repository. Locally hosted webfont subsets (Noto Sans SC, Unbounded and DM Mono) retain their upstream OFL licenses in `site/assets/fonts/`; `build_fonts.py` rebuilds them from the public Google Fonts sources. Use `--skip-media` when the reviewed poster images do not need re-exporting.

**Do not create ZIP, tar or other delivery archives. Deliver individual files and directories.**

## Deployment

`deploy/nginx.conf` is the vhost for the existing server: `/var/www/voloo/current` points at a release directory, HTTP redirects to HTTPS, and the ACME challenge path stays available. The certificate and private key stay on the server; this repository contains their paths only.

After building, deploy to the explicitly selected SSH host:

```sh
website/deploy.sh user@server
```

The script uploads individual files into a new timestamped directory, then atomically switches the `current` symlink. It does not change DNS, issue certificates, alter host authentication or publish packages. Keep earlier release directories to allow rollback by restoring the prior symlink.

For this domain, Certbot uses webroot `/var/www/letsencrypt`. The server's `certbot.timer` handles renewal; the renewal hook is `nginx -t && systemctl reload nginx`. To check it:

```sh
systemctl is-enabled certbot.timer
certbot renew --cert-name voloo.librezoo.com --dry-run --run-deploy-hooks --no-random-sleep-on-renew
```

## Review before publication

Keep **3.3–28 V DC nominal**, **3 A maximum recommended** (higher current can damage hardware), reversed-polarity damage, independent supplies, OUT-only device power, and **OFF ≠ 0 V** in both languages. At valid VIN <30.0 V the overvoltage latch clears automatically, but a new explicit ON is still needed. Do not turn the 28 V × 3 A operating point into a voltage-independent 84 W rating. Never use a simulator result as hardware test evidence.
