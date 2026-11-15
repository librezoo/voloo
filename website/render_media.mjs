// Export each public poster as its own image. No archives.
// Requires Node.js, Python 3, npx and Google Chrome.
import {createServer} from 'node:http';
import {readFile,stat,mkdir} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {dirname,resolve,extname} from 'node:path';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
const root=dirname(fileURLToPath(import.meta.url));
const repo=dirname(root);process.chdir(repo);
const mime={'.html':'text/html; charset=utf-8','.svg':'image/svg+xml','.png':'image/png','.jpg':'image/jpeg','.pdf':'application/pdf','.css':'text/css','.woff2':'font/woff2','.txt':'text/plain'};
const server=createServer(async(req,res)=>{try{const path=resolve(root,'.'+decodeURIComponent(new URL(req.url,'http://localhost').pathname));if(!path.startsWith(root+'/')){res.writeHead(403).end();return;}const data=await readFile(path);res.writeHead(200,{'Content-Type':mime[extname(path)]||'application/octet-stream'}).end(data);}catch{res.writeHead(404).end();}});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const base=`http://127.0.0.1:${server.address().port}`;
const call=promisify(execFile);
async function cli(...args){const r=await call('npx',['--yes','--package','@playwright/cli','playwright-cli','-s=voloo-public-media',...args],{maxBuffer:8*1024*1024});process.stdout.write(r.stdout);}
await mkdir(resolve(root,'site/assets/downloads'),{recursive:true});
try{
 await cli('open',base+'/poster/poster.html','--browser=chrome');
 const code=`async(page)=>{const results=[];for(const lang of ['zh','en']){for(const kind of ['poster','friends-long']){const file=kind+(lang==='en'?'.en':'')+'.html';await page.setViewportSize({width:kind==='poster'?1200:1080,height:kind==='poster'?1800:1000});await page.goto('${base}/poster/'+file);await page.evaluate(async()=>{await document.fonts.ready;await Promise.all([...document.images].map(i=>i.decode()));});const image='website/site/assets/downloads/voloo-'+kind+'-'+lang+(kind==='poster'?'.png':'.jpg');await page.screenshot({path:image,fullPage:true,...(kind==='poster'?{}:{type:'jpeg',quality:92})});results.push({image,height:await page.evaluate(()=>document.documentElement.scrollHeight)});}}return results;}`;
 await cli('run-code',code);
}finally{try{await cli('close');}finally{server.close();}}
