"""Build the complete public website, documents and individual media files.
uv run --no-project --with reportlab --with svglib --with qrcode --with beautifulsoup4 --with markdown-it-py --with fonttools --with brotli python website/build.py
"""
from pathlib import Path
import argparse,subprocess,sys,shutil
ROOT=Path(__file__).resolve().parent;REPO=ROOT.parent
parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--skip-media',action='store_true');args=parser.parse_args()
def run(*cmd):subprocess.run([str(x) for x in cmd],cwd=REPO,check=True)
run(sys.executable,ROOT/'artwork/build_quick_start.py')
run(sys.executable,ROOT/'artwork/build_quick_start_en.py')
for lang in ['zh','en']:shutil.copy2(ROOT/f'site/assets/guide/voloo-quick-start-{lang}.pdf',REPO/f'docs/assets/voloo-quick-start-{lang}.pdf')
run(sys.executable,ROOT/'build_guide.py',REPO/'docs/user-guide.md')
run('sh',ROOT/'build.sh')
run(sys.executable,ROOT/'build_english.py')
run(sys.executable,ROOT/'build_fonts.py')
if not args.skip_media:run('node',ROOT/'render_media.mjs')
run(sys.executable,ROOT/'build_downloads.py')
run(sys.executable,ROOT/'build_site_chrome.py')
run(sys.executable,ROOT/'build_public_site.py')
