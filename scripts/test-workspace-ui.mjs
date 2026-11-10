// Runs the production bundle with a synthetic Tauri boundary. Never reads real profiles.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const root = fileURLToPath(new URL('../', import.meta.url));
const chrome = process.env.CHROME_BIN || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'switcher-workspace-'));
function fixture(mode = "normal", preview = false) {
  const callbacks = new Map(); const events = new Map(); let seq = 1;
  const calls = []; const errors = []; const checks = [];
  let visibility = { claude:true, codex:true, github:true, display:true, black:true, tfsd:false };
  let saved = false; let codexBlocked = false;
  const delay = () => new Promise(r => setTimeout(r, 30));
  const record = (id, passed) => checks.push({id, passed: Boolean(passed)});
  window.addEventListener('error', e => errors.push(e.message));
  window.addEventListener('unhandledrejection', e => errors.push(String(e.reason)));
  const emit = (event, payload) => { for(const id of events.get(event) ?? []) callbacks.get(id)?.({event, payload, id}); };
  window.__TAURI_INTERNALS__ = {
    metadata:{currentWindow:{label:'main'}, currentWebview:{label:'main'}},
    transformCallback(fn) { callbacks.set(seq,fn); return seq++; },
    unregisterCallback(id) { callbacks.delete(id); },
    async invoke(cmd, args={}) {
      calls.push({cmd, args: cmd === 'api_save' ? {name: args.name, key: '[REDACTED]'} : args});
      if(cmd === 'plugin:event|listen') { const ids = events.get(args.event) ?? []; ids.push(args.handler); events.set(args.event,ids); return seq++; }
      if(cmd === 'get_language') return 'ko';
      if(cmd === 'get_accent_theme') return 'deep-green';
      if(cmd === 'get_github_star_prompt_choice') return 'dismissed';
      if(cmd === 'initial_view_mode') return mode;
      if(cmd === 'get_visibility') return visibility;
      if(cmd === 'get_section_order') return ['claude','codex','github','display','system'];
      if(cmd === 'demo_mode' || cmd === 'pointer_button_down') return false;
      if(cmd === 'clamshell_mode' || cmd === 'clamshell_cycle') return 0;
      if(cmd === 'tfsd_toggle') return false;
      if(cmd === 'github_list') return {gh_found:true,accounts:[{login:'sample-dev',active:true}]};
      if(cmd === 'display_list') return [{id:0,name:'Built-in display',brightness:65}];
      if(cmd === 'stats_read') return {cpu:18,mem_used:6*1024**3,mem_total:16*1024**3,disk_read:2*1024**2,disk_write:1024**2,net_rx:600*1024,net_tx:200*1024};
      if(cmd === 'list_profiles' && args.provider==='codex' && codexBlocked) throw 'Codex가 파일 외 인증 저장소를 사용합니다';
      if(cmd === 'list_profiles') return { live:{id:args.provider+'-1',email:'personal@example.test'},live_saved:true,profiles:[
        {name:'personal', id:args.provider+'-1',email:'personal@example.test',plan:args.provider==='claude'?'Max':'Plus',active:true,saved_at:1,plan_tier:null},
        ...(args.provider==='claude' ? [{name:'work',id:'claude-2',email:'work@example.test',plan:'Pro',active:false,saved_at:1,plan_tier:null}] : [])] };
      if(cmd === 'fetch_usage') return { windows:[{key:'five_hour',label:'5 Hours',percent:32,resets_at:null},{key:'weekly',label:'Weekly',percent:54,resets_at:null}],stale:args.profile==='work',stale_age_secs:600,issue:args.profile==='work'?'토큰 갱신 권한이 거부됐습니다 — 계정·조직의 접근 정책을 확인하세요':null };
      if(cmd === 'api_list') return saved && args.provider==='claude' ? [{provider:'claude',name:'project'}] : [];
      if(cmd === 'api_save') { if(args.key !== 'sk-dummy-test-key-123456') throw 'test input mismatch'; saved=true; return; }
      if(cmd === 'api_check') return;
      if(cmd === 'api_delete') {saved=false;return;}
      if(cmd === 'api_launch_command') return "/bin/bash '/tmp/fake-api/run.sh'";
      if(cmd === 'plugin:window|current_monitor') return {name:'fixture',position:{x:0,y:0},size:{width:1400,height:1100},workArea:{position:{x:0,y:0},size:{width:1400,height:1100}},scaleFactor:1};
      if(cmd.endsWith('|outer_position') || cmd.endsWith('|inner_position')) return {x:20,y:20};
      if(cmd.endsWith('|outer_size') || cmd.endsWith('|inner_size')) return {width:420,height:1000};
      if(cmd === 'plugin:window|set_size') {
        const size = args.value?.size ?? args.value?.data ?? args.value;
        if (size?.width) {
          document.documentElement.style.width = size.width+'px';
          document.querySelector('.shell').style.width = size.width+'px';
          document.querySelector('.shell').style.height = size.height+'px';
        }
      }
      if(cmd.endsWith('|scale_factor')) return 1;
      return null;
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener(){} };
  const ready = async(selector) => { for(let i=0;i<100;i++) {const el=document.querySelector(selector);if(el)return el;await delay();} throw new Error('Missing '+selector); };
  window.addEventListener('load', async () => {
    try {
      await ready(mode === 'normal' ? '.workspace-heading' : mode === 'edge' ? '.edge-account' : '.compact-card');
      if(mode === 'edge') { emit('edge-hover',true); for(let i=0;i<45;i++) await delay(); }
      for(let i=0;i<5;i++) await delay();
      const tools = ['memobtn','tfsdbtn','privacybtn','clambtn','blackbtn'].map(id=>document.getElementById(id));
      const box = el=>el.getBoundingClientRect();
      record('all-five-tools-visible-'+mode, tools.every(el=>box(el).width>0 && box(el).height>=24 && Number(getComputedStyle(el).opacity)>=0.75));
      record('tools-do-not-overlap-'+mode, tools.every((el,i)=>tools.slice(i+1).every(other=>{
        const a=box(el),b=box(other); return a.right<=b.left+0.5 || b.right<=a.left+0.5 || a.bottom<=b.top+0.5 || b.bottom<=a.top+0.5;
      })));
      const shell=box(document.querySelector('.shell'));
      record('tools-inside-window-'+mode,tools.every(el=>box(el).left>=shell.left-1 && box(el).right<=shell.right+1 && box(el).bottom<=shell.bottom+1));
      if(mode!=='normal') {
        const regions=calls.filter(c=>c.cmd==='set_hit_regions').at(-1)?.args.regions ?? [];
        record('tools-have-native-click-regions-'+mode,tools.every(el=>{
          const r=box(el),x=r.left+r.width/2,y=r.top+r.height/2;
          return regions.some(({rect,action})=>action===null && x>=rect[0] && y>=rect[1] && x<=rect[0]+rect[2] && y<=rect[1]+rect[3]);
        }));
      }
      for(const id of ['memobtn','tfsdbtn','clambtn','blackbtn']) document.getElementById(id).click();
      await delay();
      record('tool-buttons-call-backend-'+mode,['memo_toggle','tfsd_toggle','clamshell_cycle','black_on'].every(cmd=>calls.some(c=>c.cmd===cmd)));
      const privacyBefore=document.body.classList.contains('privacy');document.getElementById('privacybtn').click();
      record('privacy-button-works-'+mode,document.body.classList.contains('privacy')!==privacyBefore);
      document.getElementById('privacybtn').click();
      if(preview) {
        record('no-browser-errors-'+mode,errors.length===0);
        return;
      }
      await ready('.api-panel');
      document.querySelector('.shell').style.width='420px';
      await delay();
      const cards=document.querySelectorAll('#app .card');
      record('production-account-cards', cards.length===3);
      const before=cards[1].getBoundingClientRect().top;
      cards[0].classList.add('hit-hover');
      record('hover-does-not-shift-next-card', Math.abs(cards[1].getBoundingClientRect().top-before)<1);
      for(const selector of ['.action-reconnect','.action-rename','.action-delete']) {
        const x=[...cards].map(card=>box(card.querySelector(selector)).left);
        record('same-column-'+selector,Math.max(...x)-Math.min(...x)<1);
      }
      const deleteButton=cards[1].querySelector('.action-delete');
      const deleteX=box(deleteButton).left;deleteButton.click();
      record('delete-confirm-keeps-column',Math.abs(box(deleteButton).left-deleteX)<1);
      cards[1].dispatchEvent(new PointerEvent('pointerleave',{pointerType:'mouse'}));
      const stale=cards[1].querySelector('.stale-overlay');
      record('auth-error-visible-with-stale-usage', stale?.textContent.includes('권한'));
      record('auth-error-does-not-overlap-bars', stale.getBoundingClientRect().top >= [...cards[1].querySelectorAll('.usage-row')].at(-1).getBoundingClientRect().bottom);
      let panel=document.querySelector('.api-panel'); panel.open=true;
      panel.querySelector('.api-add').click();
      let form=await ready('.api-form');
      const inputs=form.querySelectorAll('input'); inputs[0].value='project';inputs[1].value='sk-dummy-test-key-123456';
      record('secret-input-masked',inputs[1].type==='password');
      // Refresh while form is open must not erase input or switch the DOM out from under it.
      document.getElementById('refresh').click(); await delay();
      record('refresh-preserves-api-form', form.isConnected && inputs[1].value==='sk-dummy-test-key-123456');
      document.getElementById('pin').click(); await delay();
      record('view-change-protects-api-input', !document.body.classList.contains('locked') && form.isConnected);
      form.requestSubmit();
      await ready('.api-card');
      record('register-sent-once',calls.filter(c=>c.cmd==='api_save').length===1);
      record('secret-cleared-after-save',inputs[1].value==='');
      panel=document.querySelector('.api-panel');panel.open=true;
      const api=document.querySelector('.api-card');
      const check=[...api.querySelectorAll('button')].find(b=>b.textContent==='연결 확인');check.click();await delay();
      record('explicit-api-check',api.textContent.includes('모델 목록 조회 성공'));
      [...api.querySelectorAll('button')].find(b=>b.textContent==='실행 명령 복사').click(); await delay();
      record('launch-command-requested-without-key',calls.some(c=>c.cmd==='api_launch_command' && !('key' in c.args)));
      const remove=[...api.querySelectorAll('button')].find(b=>b.textContent==='삭제');remove.click();
      record('delete-needs-second-click',!calls.some(c=>c.cmd==='api_delete'));
      api.dispatchEvent(new Event('pointerleave'));remove.click();
      record('delete-disarms-on-leave',!calls.some(c=>c.cmd==='api_delete'));
      remove.click();await delay();record('delete-removes-api-row',!api.isConnected);
      record('api-never-switches-oauth',!calls.some(c=>c.cmd==='switch_profile'));
      // Show no AI sections when both are disabled, including their API managers.
      visibility={...visibility,claude:false,codex:false};emit('visibility-changed');await delay();await delay();
      record('hidden-ai-leaves-no-provider-panels',!document.querySelector('#app .profile-actions, #app .api-panel'));
      visibility={...visibility,claude:true,codex:true};emit('visibility-changed');await ready('.card');await delay();
      document.querySelector('.api-panel').open=true;
      record('no-horizontal-overflow',document.querySelector('#app').scrollWidth<=420);
      codexBlocked=true; emit('visibility-changed'); await delay(); await delay();
      record('api-available-with-blocked-subscription-store',document.querySelectorAll('.api-panel').length===2 && document.querySelector('#app').textContent.includes('파일 외'));
      codexBlocked=false; emit('visibility-changed'); await delay(); await delay();
      record('no-browser-errors',errors.length===0);
    } catch(e) { errors.push(String(e)); record('completed',false); } finally {
    const output=document.createElement('pre');output.id='ui-test-result';output.hidden=true;
    output.textContent=JSON.stringify({checks,errors,mode,width:document.querySelector('.shell').getBoundingClientRect().width,height:document.querySelector('.shell').getBoundingClientRect().height});document.body.append(output);
    }
  });
}
const allAt=process.argv.indexOf('--all-modes');
const allDir=allAt>=0 ? path.resolve(process.argv[allAt+1]) : null;
const outputAt=process.argv.indexOf('--output');
function run(mode, preview, screenshot) {
  const width={normal:420,compact:280,minimal:120,edge:80}[mode];
  let html=fs.readFileSync(path.join(root,'dist/index.html'),'utf8').replaceAll('"/assets/','"'+pathToFileURL(path.join(root,'dist/assets/')).href);
  html=html.replace('<head>','<head><script>('+fixture.toString()+')('+JSON.stringify(mode)+','+preview+');</script>');
  const entry=path.join(temp,mode+'.html');fs.writeFileSync(entry,html);
  const args=['--headless=new','--disable-gpu','--no-first-run','--no-default-browser-check',`--user-data-dir=${temp}/profile-${mode}-${preview}`,'--allow-file-access-from-files',`--window-size=${width},1100`,'--virtual-time-budget=7000','--dump-dom',...(screenshot?[`--screenshot=${screenshot}`]:[]),pathToFileURL(entry).href];
  const result=spawnSync(chrome,args,{encoding:'utf8',timeout:30000,maxBuffer:5e6});
  const encoded=result.stdout?.match(/<pre id="ui-test-result" hidden="">([\s\S]*?)<\/pre>/)?.[1];
  if(!encoded) {fs.writeFileSync('/tmp/switcher-ui-debug.html',result.stdout ?? '');throw new Error('No UI result: '+result.stderr?.slice(-500));}
  return JSON.parse(encoded.replaceAll('&quot;','"').replaceAll('&amp;','&').replaceAll('&lt;','<').replaceAll('&gt;','>'));
}
try {
  const screenshotAt=process.argv.indexOf('--screenshot');
  const reports=[run('normal',false,screenshotAt>=0?path.resolve(process.argv[screenshotAt+1]):null)];
  if(allDir) {
    fs.mkdirSync(allDir,{recursive:true});
    for(const mode of ['normal','compact','minimal','edge']) reports.push(run(mode,true,path.join(allDir,`mode-${mode}.png`)));
    const labels={normal:['01 일반','계정 · API 키 · 전체 도구'],compact:['02 컴팩트','계정과 사용량 요약'],minimal:['03 미니멀','사용량만 작게'],edge:['04 엣지','벽 붙임 · 펼친 상태']};
    const figures=reports.slice(1).map(r=>`<figure style="width:${r.width}px"><figcaption><b>${labels[r.mode][0]}</b><span>${labels[r.mode][1]}</span></figcaption><div style="height:${Math.ceil(r.height)}px"><img src="mode-${r.mode}.png" style="width:${r.width}px"></div></figure>`).join('');
    const board=`<!doctype html><html lang="ko"><meta charset="utf-8"><title>switcher · 네 가지 모드</title><style>*{box-sizing:border-box}body{margin:0;background:#e8ecec;color:#182024;font-family:-apple-system,BlinkMacSystemFont,sans-serif;padding:32px 38px}header{display:flex;align-items:baseline;gap:16px;margin-bottom:28px}h1{font-size:26px;letter-spacing:-1px;margin:0}header span{font-size:12px;color:#566568}.views{display:flex;align-items:flex-start;gap:30px}figure{margin:0;flex:none}figcaption{height:56px}b{font-size:15px;display:block}figcaption span{font-size:10px;color:#566568;display:block;margin-top:6px;white-space:nowrap}figure div{overflow:hidden;border-radius:12px;box-shadow:0 12px 32px #17212221}img{display:block}</style><header><h1>switcher</h1><span>실제 프론트 화면 · 예시 계정 · 4개 모드 유지</span></header><div class="views">${figures}</div></html>`;
    const boardPath=path.join(allDir,'four-modes.html');fs.writeFileSync(boardPath,board);
    const height=Math.ceil(Math.max(...reports.slice(1).map(r=>r.height))+170);
    const screen=spawnSync(chrome,['--headless=new','--disable-gpu','--no-first-run',`--user-data-dir=${temp}/board`,`--window-size=1100,${height}`,'--hide-scrollbars','--virtual-time-budget=1000',`--screenshot=${path.join(allDir,'four-modes.png')}`,pathToFileURL(boardPath).href],{encoding:'utf8',timeout:30000});
    if(screen.status!==0) throw new Error('Preview board failed');
  }
  const result={reports,passed:reports.reduce((n,r)=>n+r.checks.filter(c=>c.passed).length,0),failed:reports.reduce((n,r)=>n+r.checks.filter(c=>!c.passed).length,0)};
  if(outputAt>=0) fs.writeFileSync(process.argv[outputAt+1],JSON.stringify(result,null,2)+'\n');
  console.log(JSON.stringify(result,null,2));process.exitCode=result.failed?1:0;
} finally { fs.rmSync(temp,{recursive:true,force:true}); }
