"""Copy only the public HTML pages and their local dependencies into a static-site directory; never archive."""
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit
import json
import shutil
import argparse
import re

HERE = Path(__file__).resolve().parent
SOURCE = HERE / 'site'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, default=HERE.parent/'output/website')
PUBLIC = parser.parse_args().output.resolve()

class References(HTMLParser):
    def __init__(self):
        super().__init__()
        self.urls = []

    def handle_starttag(self, tag, attrs):
        for key, value in attrs:
            if key in ('src', 'href') and value:
                self.urls.append(value)
            elif key == 'srcset' and value:
                self.urls.extend(part.strip().split()[0] for part in value.split(',') if part.strip())

pending = [Path('index.html'), Path('index.en.html'), Path('guide.html'), Path('guide.en.html')]
pending.extend(p.relative_to(SOURCE) for p in (SOURCE/'assets/fonts').glob('*-OFL.txt'))
selected = set()
while pending:
    relative = pending.pop()
    source = (SOURCE / relative).resolve()
    if not source.is_relative_to(SOURCE.resolve()) or not source.is_file():
        raise ValueError(f'Missing or non-public dependency: {relative}')
    relative = source.relative_to(SOURCE.resolve())
    if relative in selected:
        continue
    selected.add(relative)
    if source.suffix in ('.html', '.svg'):
        parser = References()
        parser.feed(source.read_text())
        for url in parser.urls:
            parsed = urlsplit(url)
            if parsed.scheme or parsed.netloc or not parsed.path:
                continue
            pending.append(relative.parent / unquote(parsed.path))
    elif source.suffix == '.css':
        for url in re.findall(r'url\([\s\"\x27]*([^\)\"\x27]+)', source.read_text()):
            parsed = urlsplit(url.strip())
            if not parsed.scheme and not parsed.netloc and parsed.path:
                pending.append(relative.parent / unquote(parsed.path))

FIRMWARE = HERE.parent / 'firmware/x035'
firmware_files = {Path('firmware/x035') / p.name for p in FIRMWARE.iterdir() if p.suffix in ('.json', '.voloofw')} if FIRMWARE.is_dir() else set()
selected |= firmware_files
existing = {p.relative_to(PUBLIC) for p in PUBLIC.rglob('*') if p.is_file()} if PUBLIC.exists() else set()
if existing - selected:
    raise ValueError('Unexpected files already exist in public output; refusing to publish them.')
for relative in selected:
    destination = PUBLIC / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2((HERE.parent if relative in firmware_files else SOURCE) / relative, destination)
print(json.dumps({'directory': str(PUBLIC), 'files': [str(p) for p in sorted(selected)]}, ensure_ascii=False))
