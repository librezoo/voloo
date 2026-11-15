"""Build the portable illustrated manual from the repository's Markdown.

uv run --no-project --with markdown-it-py python build_guide.py /path/to/docs/user-guide.md
"""
from pathlib import Path
import argparse
import html
import re
import shutil
import unicodedata

from markdown_it import MarkdownIt

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('source', type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parent
source = args.source.resolve()
markdown = source.read_text()
assets = root / 'site/assets/guide'
assets.mkdir(parents=True, exist_ok=True)
for name in re.findall(r'!\[[^\]]*\]\(assets/([^\)]+)\)', markdown):
    shutil.copy2(source.parent / 'assets' / name, assets / name)

md = MarkdownIt('commonmark', {'html': False}).enable('table')
tokens = md.parse(markdown)
for i, token in enumerate(tokens):
    if token.type == 'heading_open':
        text = tokens[i + 1].content.lower()
        text = ''.join(c for c in text if c.isspace() or c in '-_' or unicodedata.category(c)[0] in 'LN')
        token.attrSet('id', re.sub(r'\s+', '-', text.strip()))
body = md.renderer.render(tokens, md.options, {})
body = body.replace('src="assets/', 'src="assets/guide/')
body = body.replace('href="assets/', 'href="assets/guide/')
body = body.replace('href="quick-start.md', 'href="quick-start.html')
body = body.replace('href="quick-start.en.md', 'href="quick-start.en.html')
body = body.replace('href="user-guide.en.md"', 'href="guide.en.html"')
body = body.replace('href="user-guide.en.md#', 'href="guide.en.html#')
body = body.replace('href="firmware-updates.md"', 'href="https://github.com/librezoo/voloo/blob/main/docs/firmware-updates.md"')
body = re.sub(r'(<img src="([^"]+)"[^>]*>)', lambda m: '<a class="figure-link" href="' + html.escape(m[2], quote=True) + '" target="_blank" rel="noopener">' + m[1] + '</a>', body)
css = '''
:root{color-scheme:light}*{box-sizing:border-box}html{scroll-behavior:smooth;scroll-padding-top:24px}
body{max-width:900px;margin:30px auto;padding:0 22px 80px;font:17px/1.85 -apple-system,BlinkMacSystemFont,"PingFang SC","Microsoft YaHei",sans-serif;color:#2d1a2b;background:#f5f1e9;overflow-wrap:anywhere}
.page-nav{display:flex;flex-wrap:wrap;justify-content:space-between;gap:12px;border-bottom:2px solid #2d1a2b;padding-bottom:16px;margin-bottom:28px;font-size:15px}
h1{font-size:38px;line-height:1.35;letter-spacing:-.03em;margin:0 0 24px}h2{margin-top:50px;font-size:27px;line-height:1.5;border-bottom:2px solid #b49cf0;padding-bottom:8px}h3{margin-top:34px;font-size:21px;line-height:1.6}p,ul,ol{margin:18px 0}li+li{margin-top:8px}a{color:#5e43b7;text-underline-offset:3px}strong{font-weight:750}
pre{background:#241622;color:#fffdf8;padding:20px;border-radius:14px;white-space:pre-wrap;overflow-wrap:anywhere;line-height:1.7}code{font-family:ui-monospace,Menlo,Consolas,monospace;font-size:.87em}p code,li code,td code{background:rgba(94,67,183,.07);padding:2px 4px;border-radius:4px}pre code{font-size:14px;background:transparent;padding:0}
table{border-collapse:collapse;width:100%;font-size:15px;line-height:1.75;margin:22px 0}th,td{border:1px solid #c7bcc5;text-align:left;padding:10px 12px;vertical-align:top}th{background:#fec600}td{background:#fffdf8}blockquote{margin:22px 0;padding:2px 22px;border-left:5px solid #5fc49a;background:#fffdf8;border-radius:0 10px 10px 0}
img{display:block;max-width:100%;height:auto;margin:24px auto 0;border-radius:14px;border:1px solid #c7bcc5}.figure-link{display:block}.figure-link:focus-visible{outline:3px solid #3fb2ff;outline-offset:4px}p:has(>em:only-child){font-size:14px;line-height:1.75;color:#65545f;margin-top:12px}em{font-style:normal}
@media(max-width:600px){body{margin-top:18px;padding:0 18px 50px;font-size:16px}h1{font-size:30px}h2{font-size:24px;margin-top:38px}th,td{padding:7px;font-size:14px}table{display:block;overflow-x:auto}pre{padding:16px}img{border-radius:10px}}
@media print{body{max-width:none;padding:0;font-size:11pt;background:white}.page-nav{display:none}h2,h3{break-after:avoid}pre,blockquote,img{break-inside:avoid}img{max-height:230mm;object-fit:contain}a{color:inherit}}
'''
page = '<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="icon" href="data:,"><title>voloo 中文图文使用说明</title><style>' + css + '</style></head><body><nav class="page-nav"><a href="index.html">← 返回 voloo 介绍</a><span>中文图文使用说明 · 图片可点开查看大图</span></nav><main>' + body + '</main></body></html>'
(root / 'site/guide.html').write_text(page)
print('Built illustrated Chinese guide with', len(re.findall(r'<img ', body)), 'images.')

# Keep both public languages on the same reviewed documentation revision.
english = (source.parent / 'user-guide.en.md').read_text()
etokens = md.parse(english)
for i, token in enumerate(etokens):
    if token.type == 'heading_open':
        text = etokens[i + 1].content.lower()
        text = ''.join(c for c in text if c.isspace() or c in '-_' or unicodedata.category(c)[0] in 'LN')
        token.attrSet('id', re.sub(r'\s+', '-', text.strip()))
ebody = md.renderer.render(etokens, md.options, {})
ebody = ebody.replace('href="user-guide.md', 'href="guide.html')
ebody = ebody.replace('href="firmware-updates.md', 'href="https://github.com/librezoo/voloo/blob/main/docs/firmware-updates.md')
ebody = ebody.replace('src="assets/', 'src="assets/guide/')
ebody = ebody.replace('href="assets/', 'href="assets/guide/')
ebody = ebody.replace('href="quick-start.md', 'href="quick-start.html')
ebody = ebody.replace('href="quick-start.en.md', 'href="quick-start.en.html')
epage = '<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="icon" href="data:,"><title>voloo User Guide</title><style>' + css + '</style></head><body><nav class="page-nav"><a href="index.en.html">← voloo</a></nav><main>' + ebody + '</main></body></html>'
(root / 'site/guide.en.html').write_text(epage)
print('Built matching English guide.')
