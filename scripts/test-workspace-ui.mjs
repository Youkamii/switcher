// Runs the production bundle with a synthetic Tauri boundary. Never reads real profiles.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const root = fileURLToPath(new URL('../', import.meta.url));
const chrome = process.env.CHROME_BIN || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'switcher-workspace-'));
function fixture() {
  const callbacks = new Map(); const events = new Map(); let seq = 1;
  const calls = []; const errors = []; const checks = [];
  let visibility = { claude:true, codex:true, github:false, display:false, system:false, tfsd:false };
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
      if(cmd === 'initial_view_mode') return 'normal';
      if(cmd === 'get_visibility') return visibility;
      if(cmd === 'get_section_order') return ['claude','codex','github','display','system'];
      if(cmd === 'demo_mode' || cmd === 'pointer_button_down') return false;
      if(cmd === 'clamshell_mode') return -1;
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
      if(cmd === 'plugin:window|current_monitor') return {name:'fixture',position:{x:0,y:0},size:{width:1400,height:1200},workArea:{position:{x:0,y:0},size:{width:1400,height:1200}},scaleFactor:1};
      if(cmd.endsWith('|outer_position') || cmd.endsWith('|inner_position')) return {x:20,y:20};
      if(cmd.endsWith('|outer_size') || cmd.endsWith('|inner_size')) return {width:420,height:1000};
      if(cmd.endsWith('|scale_factor')) return 1;
      return null;
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener(){} };
  const ready = async(selector) => { for(let i=0;i<100;i++) {const el=document.querySelector(selector);if(el)return el;await delay();} throw new Error('Missing '+selector); };
  window.addEventListener('load', async () => {
    try {
      await ready('.workspace-heading'); await ready('.api-panel');
      document.querySelector('.shell').style.width='420px';
      await delay();
      const cards=document.querySelectorAll('#app .card');
      record('production-account-cards', cards.length===3);
      const before=cards[1].getBoundingClientRect().top;
      const menu=cards[0].querySelector('.account-menu'); menu.open=true;
      record('menu-does-not-shift-next-card', Math.abs(cards[1].getBoundingClientRect().top-before)<1);
      menu.open=false;
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
      record('hidden-ai-leaves-no-provider-panels',!document.querySelector('#app .card, #app .api-panel'));
      visibility={...visibility,claude:true,codex:true};emit('visibility-changed');await ready('.card');await delay();
      document.querySelector('.api-panel').open=true;
      record('no-horizontal-overflow',document.querySelector('#app').scrollWidth<=420);
      codexBlocked=true; emit('visibility-changed'); await delay(); await delay();
      record('api-available-with-blocked-subscription-store',document.querySelectorAll('.api-panel').length===2 && document.querySelector('#app').textContent.includes('파일 외'));
      codexBlocked=false; emit('visibility-changed'); await delay(); await delay();
      record('no-browser-errors',errors.length===0);
    } catch(e) { errors.push(String(e)); record('completed',false); }
    const output=document.createElement('pre');output.id='ui-test-result';output.hidden=true;
    output.textContent=JSON.stringify({checks,errors});document.body.append(output);
  });
}
try {
  let html=fs.readFileSync(path.join(root,'dist/index.html'),'utf8').replaceAll('"/assets/','"'+pathToFileURL(path.join(root,'dist/assets/')).href);
  html=html.replace('<head>','<head><script>('+fixture.toString()+')();</script>');
  const entry=path.join(temp,'index.html');fs.writeFileSync(entry,html);
  const screenshotAt=process.argv.indexOf('--screenshot');
  const args=['--headless=new','--disable-gpu','--no-first-run','--no-default-browser-check',`--user-data-dir=${temp}/profile`,'--allow-file-access-from-files','--window-size=420,1050','--virtual-time-budget=7000','--dump-dom',...(screenshotAt>=0?[`--screenshot=${path.resolve(process.argv[screenshotAt+1])}`]:[]),pathToFileURL(entry).href];
  const result=spawnSync(chrome,args,{encoding:'utf8',timeout:30000,maxBuffer:5e6});
  const encoded=result.stdout?.match(/<pre id="ui-test-result" hidden="">([\s\S]*?)<\/pre>/)?.[1];
  if(!encoded) {fs.writeFileSync('/tmp/switcher-ui-debug.html',result.stdout ?? '');throw new Error('No UI result: '+result.stderr?.slice(-500));}
  const report=JSON.parse(encoded.replaceAll('&quot;','"').replaceAll('&amp;','&').replaceAll('&lt;','<').replaceAll('&gt;','>'));
  console.log(JSON.stringify(report,null,2));process.exitCode=report.checks.every(c=>c.passed)?0:1;
} finally { fs.rmSync(temp,{recursive:true,force:true}); }
