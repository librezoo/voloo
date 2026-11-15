"""Generate public download pages and their navigation. Individual files only."""
from pathlib import Path
from bs4 import BeautifulSoup
ROOT=Path(__file__).resolve().parent
CSS='''*{box-sizing:border-box}body{margin:0;background:#f5f1e9;color:#2d1a2b;font:17px/1.7 system-ui,sans-serif}main{max-width:1080px;margin:auto;padding:24px 22px 64px}header{display:flex;align-items:center;flex-wrap:wrap;gap:14px;justify-content:space-between;border-bottom:1px solid #d7ccd4;padding-bottom:20px}header img{width:230px;max-width:65vw}nav{display:flex;gap:16px}nav a{display:inline-flex;align-items:center;min-width:44px;min-height:44px}a{color:#5e43b7;text-underline-offset:4px}h1{font-size:clamp(30px,5vw,44px);line-height:1.25;margin:36px 0 14px}h2{font-size:22px;margin:0 0 10px}.intro{color:#65545f;max-width:68ch}.cards{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:20px;margin-top:30px}.card{padding:24px;border:2px solid #2d1a2b;border-radius:18px;background:#fffdf8;box-shadow:5px 5px 0 #2d1a2b;min-width:0}.card img{display:block;width:100%;height:280px;object-fit:contain;margin-bottom:20px}.links{display:flex;flex-wrap:wrap;gap:14px}.links a{display:inline-block;padding:8px 13px;border-radius:9px;border:1px solid #2d1a2b;text-decoration:none;font-weight:650}.links a:first-child{background:#fec600;color:#2d1a2b}footer{margin-top:36px;font-size:14px;color:#65545f}code{font-family:ui-monospace,monospace;overflow-wrap:anywhere}@media(max-width:650px){.cards{grid-template-columns:1fr}.card{padding:20px}nav{font-size:14px}}
'''
for english in [False,True]:
 suffix='.en' if english else '';other='downloads.html' if english else 'downloads.en.html';home='index.en.html' if english else 'index.html'
 title='Downloads' if english else '下载与分享'
 intro='Download posters, a shareable product overview and quick-start guides in English or Chinese. Every QR code opens the voloo website, where you can choose your language and read the latest instructions.' if english else '选择中文或英文，直接下载单独文件。所有二维码均打开 voloo 官网，可切换语言并查看最新使用说明。'
 cards=[]
 for kind,cn,en in [('poster','海报','Poster'),('friends-long','介绍长图','Shareable product overview'),('quick-start','快速说明书','Quick-start guide')]:
  label=en if english else cn
  links=[]
  for lang,name in [('zh','中文'),('en','English')]:
   path=f'assets/guide/voloo-quick-start-{lang}.pdf' if kind=='quick-start' else f'assets/downloads/voloo-{kind}-{lang}.'+('png' if kind=='poster' else 'jpg')
   links.append(f'<a href="{path}" download>{name} · {"PDF" if kind=="quick-start" else "PNG" if kind=="poster" else "JPG"}</a>')
  preview=f'<img src="assets/downloads/voloo-poster-{"en" if english else "zh"}.png" alt="{label}" loading="lazy">' if kind=='poster' else ''
  detail=('Two A4 pages, ready to print on both sides of one sheet.' if english else '两页 A4，可双面打印。') if kind=='quick-start' else ('Save the image and share it with friends or colleagues.' if english else '可直接发给朋友或同事。')
  cards.append(f'<article class="card">{preview}<h2>{label}</h2><p>{detail}</p><div class="links">{"".join(links)}</div></article>')
 cards.append(f'<article class="card"><h2>{"Read online" if english else "在线阅读"}</h2><div class="links"><a href="quick-start{suffix}.html">{"Quick start" if english else "快速上手"}</a><a href="guide{suffix}.html">{"Full user guide" if english else "完整使用说明"}</a></div><p><code>npm install -g @librezoo/voloo</code></p></article>')
 page=f'''<!doctype html><html lang="{'en' if english else 'zh-CN'}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="icon" href="data:,"><link rel="canonical" href="https://voloo.librezoo.com/downloads{suffix}.html"><link rel="alternate" hreflang="zh-CN" href="downloads.html"><link rel="alternate" hreflang="en" href="downloads.en.html"><title>voloo · {title}</title><style>{CSS}</style></head><body><main><header><a href="{home}"><img src="assets/brand/librezoo-voloo-compact.svg" alt="librezoo · voloo"></a><nav><a href="{home}">{'Product website' if english else '产品官网'}</a><a href="{other}" lang="{'zh-CN' if english else 'en'}">{'中文' if english else 'EN'}</a></nav></header><h1>{title}</h1><p class="intro">{intro}</p><section class="cards">{''.join(cards)}</section><footer>{'AI-assisted product rendering. Always follow physical markings and the safety instructions.' if english else '产品图为 AI 辅助效果渲染，实际接线请核对实物标记并遵守安全说明。'}</footer></main></body></html>'''
 (ROOT/f'site/downloads{suffix}.html').write_text(page)
 for name in [f'index{suffix}.html',f'quick-start{suffix}.html',f'guide{suffix}.html']:
  p=ROOT/'site'/name;soup=BeautifulSoup(p.read_text(),'html.parser')
  for old in soup.select('#downloads-link'):old.decompose()
  target=soup.select_one('footer .wrap') or soup.select_one('footer .links') or soup.select_one('.page-nav')
  if target:
   a=soup.new_tag('a',href=f'downloads{suffix}.html',id='downloads-link');a.string='Downloads' if english else '海报与说明书下载';target.append(a)
  p.write_text(str(soup).rstrip()+'\n')
print('Built bilingual downloads pages and navigation.')
