// Runs the production bundle with a synthetic Tauri boundary. Never reads real profiles.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL, fileURLToPath } from 'node:url';
import { spawnSync, spawn } from 'node:child_process';
const root = fileURLToPath(new URL('../', import.meta.url));
const chrome = process.env.CHROME_BIN || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const useWebKit = process.argv.includes('--webkit');
const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'switcher-workspace-'));
function fixture(mode = "normal", preview = false) {
  const callbacks = new Map(); const events = new Map(); let seq = 1;
  const calls = []; const errors = []; const checks = [];
  let visibility = { claude:preview!=='no-ai', codex:preview!=='no-ai', github:true, display:true, black:true, tfsd:false };
  const activeApi = new Map(preview===true ? [['claude','project']] : []); let activationFailure = false;
  let codexBlocked = false; let checkFailure = false; let releaseCheck = null; let holdCheck = false;
  let costFailure = false;
  let sessionPhase = null;
  const costs = new Map();
  const usageFor = (provider, name) => costs.get(provider+'/'+name) ?? {month:'2026-11',budget_usd:195,spent_usd:null,remaining_usd:null,percent:null,source:'manual',api_key_id:'',admin_key_stored:false,checked_at:null,provider_updated_at:null,error:null};
  const updateCost = (provider,name,value) => { const data={...usageFor(provider,name),...value}; data.remaining_usd=data.spent_usd===null?null:data.budget_usd-data.spent_usd;data.percent=data.spent_usd===null?null:data.spent_usd/data.budget_usd*100;costs.set(provider+'/'+name,data);return data; };
  if(preview) updateCost('claude','project',{spent_usd:78.45,checked_at:1794300000});
  const connections = new Map(preview ? [['claude/project',{provider:'claude',name:'project',check_state:'ready',checked_at:1794300000}]] : []);
  // A native host drives test interactions even when WebKit throttles page timers.
  let tickId=0; const ticks=new Map();
  window.__uiTick=id=>{ticks.get(id)?.();ticks.delete(id);};
  const delay = () => new Promise(r => {
    if(window.webkit?.messageHandlers?.tick){const id=++tickId;ticks.set(id,r);window.webkit.messageHandlers.tick.postMessage(id);}
    else setTimeout(r,30);
  });
  window.__uiDebug={checks,errors};
  const record = (id, passed) => checks.push({id, passed: Boolean(passed)});
  window.addEventListener('error', e => errors.push(e.message));
  window.addEventListener('unhandledrejection', e => errors.push(String(e.reason)));
  const emit = (event, payload) => { for(const id of events.get(event) ?? []) callbacks.get(id)?.({event, payload, id}); };
  window.__TAURI_INTERNALS__ = {
    metadata:{currentWindow:{label:'main'}, currentWebview:{label:'main'}},
    transformCallback(fn) { callbacks.set(seq,fn); return seq++; },
    unregisterCallback(id) { callbacks.delete(id); },
    async invoke(cmd, args={}) {
      calls.push({cmd, args: cmd === 'api_save' ? {provider:args.provider,name: args.name, key: '[REDACTED]',replace:args.replace} : cmd==='api_usage_save' ? {...args,settings:{...args.settings,admin_key:args.settings.admin_key?'[REDACTED]':null}} : args});
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
      if(cmd === 'list_profiles') return { api_active:activeApi.get(args.provider)??null, live:activeApi.has(args.provider)?null:{id:args.provider+'-1',email:'personal@example.test'},live_saved:!activeApi.has(args.provider),profiles:[
        {name:'personal', id:args.provider+'-1',email:'personal@example.test',plan:args.provider==='claude'?'Max':'Plus',active:!activeApi.has(args.provider),saved_at:1,plan_tier:null},
        ...(args.provider==='claude' ? [{name:'work',id:'claude-2',email:'work@example.test',plan:'Pro',active:false,saved_at:1,plan_tier:null}] : [])] };
      if(cmd === 'codex_session_status') return sessionPhase ? {phase:sessionPhase,reason:null} : null;
      if(cmd === 'fetch_usage') return { windows:[{key:'five_hour',label:'5 Hours',percent:32,resets_at:null},{key:'weekly',label:'Weekly',percent:54,resets_at:null}],stale:args.profile==='work',stale_age_secs:600,issue:args.profile==='work'?'토큰 갱신 권한이 거부됐습니다 — 계정·조직의 접근 정책을 확인하세요':null };
      if(cmd === 'api_list') return [...connections.values()].filter(c=>c.provider===args.provider).map(c=>({...c,active:activeApi.get(c.provider)===c.name}));
      if(cmd === 'start_login') throw 'fixture login stopped';
      if(cmd === 'api_usage_read') return usageFor(args.provider,args.name);
      if(cmd === 'api_usage_save') {
        const old=usageFor(args.provider,args.name), settings=args.settings;
        return updateCost(args.provider,args.name,{...settings,api_key_id:settings.source==='automatic'?settings.api_key_id:'',spent_usd:settings.source==='automatic'?null:settings.spent_usd??(old.source==='manual'?old.spent_usd:null),admin_key:undefined,admin_key_stored:settings.source==='automatic',error:null,checked_at:1794300000});
      }
      if(cmd === 'api_usage_refresh') return updateCost(args.provider,args.name,costFailure?{error:'비용 조회 권한이 없습니다'}:{spent_usd:42.5,error:null,checked_at:1794300000});
      if(cmd === 'api_save') {
        if(args.key !== 'sk-dummy-test-key-123456') throw 'test input mismatch';
        const id=args.provider+'/'+args.name;
        if(connections.has(id) && !args.replace) throw '같은 이름의 API 연결이 있습니다';
        connections.set(id,{provider:args.provider,name:args.name,check_state:'unchecked'}); return;
      }
      if(cmd === 'api_activate') {
        if(activationFailure) throw 'API 설정 적용 실패';
        activeApi.set(args.provider,args.name); return {...connections.get(args.provider+'/'+args.name),active:true};
      }
      if(cmd === 'api_deactivate') {activeApi.delete(args.provider);return;}
      if(cmd === 'api_check') {
        if(holdCheck) await new Promise(resolve=>{releaseCheck=resolve;});
        const id=args.provider+'/'+args.name;
        const entry={provider:args.provider,name:args.name,check_state:checkFailure?'invalid_key':'ready',checked_at:1794300000};
        connections.set(id,entry); if(checkFailure) throw 'API 키가 거부됐습니다 — 키를 교체하세요'; return entry;
      }
      if(cmd === 'api_delete') {connections.delete(args.provider+'/'+args.name);if(activeApi.get(args.provider)===args.name)activeApi.delete(args.provider);return;}
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
  const openApi = async(provider='claude') => {
    document.querySelector('.add-account[data-provider="'+provider+'"]').click();
    await ready('.connection-methods');
    document.querySelector('[data-method="api"]').click();
    return ready('.api-form');
  };
  window.addEventListener('load', async () => {
    try {
      await ready(preview==='no-ai' ? '.mon-section' : mode === 'normal' ? '.workspace-heading' : mode === 'edge' ? '.edge-account' : '.compact-card');
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
        if(preview!=='no-ai') {
          const codexHint=mode==='normal'?document.querySelector('.codex-session-note')?.textContent:document.querySelector(mode==='edge'?'.edge-account.active[data-provider="codex"]':'.compact-card.active.prov-codex')?.title;
          record('codex-next-launch-scope-'+mode,codexHint?.includes('다음 Codex 실행'));
        }
        if(preview===true) {
          const selected=document.querySelector(mode==='normal'?'.api-active .api-badge':'.compact-api-active');
          record('selected-api-visible-'+mode, selected && box(selected).width>0 && selected.textContent.includes('API'));
          record('selected-api-inside-window-'+mode, selected && box(selected).left>=0 && box(selected).right<=innerWidth);
        }
        if(preview==='registration' || preview==='result') {
          await openApi();
          const form=await ready('.api-form');
          if(preview==='result') {
            form.elements.namedItem('connection-name').value='demo'; form.elements.namedItem('api-key').value='sk-dummy-test-key-123456';
            form.requestSubmit(); await ready('.api-applied'); await delay();
          }
        }
        if(preview==='registration' || preview==='result') { const r=box(document.querySelector('.api-dialog')); record('api-'+preview+'-inside-window',r.left>=0 && r.right<=innerWidth && r.top>=0 && r.bottom<=innerHeight); }
        if(preview==='no-ai') record('unused-ai-leaves-no-account-space',!document.querySelector('.compact-card') && document.querySelector('.mon-section').offsetHeight>0);
        record('no-browser-errors-'+mode,errors.length===0);
        return;
      }
      record('codex-selection-is-not-labelled-as-live-session',document.querySelector('.codex-session-note')?.textContent.includes('다음 Codex 실행')&&[...document.querySelectorAll('.connection-active')].some(el=>el.textContent==='선택됨'));
      for (const [phase, text] of [['waiting','현재 작업이 끝나면'],['reloading','갱신하고 있습니다'],['reloaded','공유 서버에 새 계정을 반영'],['failed','갱신하지 못했습니다']]) {
        sessionPhase=phase;document.getElementById('refresh').click();
        await ready('.codex-session-note[data-phase="'+phase+'"]');
        record('codex-session-status-'+phase,document.querySelector('.codex-session-note')?.textContent.includes(text));
      }
      record('codex-refresh-button-retries-session-reload',calls.some(c=>c.cmd==='retry_codex_session_refresh'));
      sessionPhase=null;document.getElementById('refresh').click();await delay();await delay();
      document.querySelector('.add-account[data-provider="claude"]').click();await ready('.connection-methods');
      record('account-add-offers-both-authentication-methods',document.querySelectorAll('.connection-methods button').length===2&&!document.querySelector('.connection-subscription').hidden);
      record('account-add-does-not-start-login-before-choice',!calls.some(c=>c.cmd==='start_login'));
      record('account-add-label-exposes-api',[...document.querySelectorAll('.add-account')].every(button=>button.textContent.includes('API')));
      record('api-registration-choice-visible',document.querySelector('[data-method="api"]').textContent==='API 키 등록'&&box(document.querySelector('[data-method="api"]')).width>0);
      document.querySelector('.connection-login').click();await delay();await delay();await delay();
      record('subscription-choice-uses-existing-login-flow',calls.some(c=>c.cmd==='start_login'&&c.args.provider==='claude')&&!document.querySelector('.api-dialog'));
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
      record('one-add-entry-for-subscription-and-api',!document.querySelector('.api-register') && [...document.querySelectorAll('.add-account[data-provider]')].length===2);
      record('no-empty-api-space',[...document.querySelectorAll('.api-panel')].every(el=>box(el).height===0));
      await openApi();
      let form=await ready('.api-form');
      let inputs=form.querySelectorAll('input'); inputs[0].value='project';inputs[1].value='sk-dummy-test-key-123456';
      record('secret-input-masked',inputs[1].type==='password');
      const dragCount=()=>calls.filter(c=>c.cmd==='plugin:window|start_dragging').length;
      const dragBefore=dragCount();
      const mouseDown=(element,button=0,position={})=>element.dispatchEvent(new MouseEvent('mousedown',{bubbles:true,cancelable:true,button,...position}));
      mouseDown(document.querySelector('.api-dialog-head h2')); await delay();
      record('registration-title-starts-native-window-drag',dragCount()===dragBefore+1);
      const bar=box(document.querySelector('.titlebar'));
      mouseDown(document.querySelector('.api-dialog'),0,{clientX:bar.left+3,clientY:bar.top+3}); await delay();
      record('original-titlebar-drags-through-modal-backdrop',dragCount()===dragBefore+2);
      mouseDown(document.querySelector('.api-close'));mouseDown(inputs[0]);mouseDown(inputs[1]);mouseDown(document.querySelector('.api-dialog-head'),2);await delay();
      record('form-controls-and-right-click-do-not-drag',dragCount()===dragBefore+2);
      emit('tauri://move',{x:50,y:70});await delay();
      record('window-move-preserves-registration-inputs',form.isConnected&&inputs[0].value==='project'&&inputs[1].value==='sk-dummy-test-key-123456');
      const dialogBox=box(document.querySelector('.api-dialog'));
      record('api-dialog-stays-inside-window',dialogBox.left>=0 && dialogBox.right<=innerWidth && dialogBox.top>=0 && dialogBox.bottom<=innerHeight);
      document.querySelector('[data-method="subscription"]').click();
      record('changing-login-method-clears-secret',inputs[1].value==='' && form.hidden && !document.querySelector('.connection-subscription').hidden);
      document.querySelector('[data-method="api"]').click(); inputs[1].value='sk-dummy-test-key-123456';
      record('provider-is-inherited-from-account-add',form.querySelector('.api-providers').hidden);
      form.querySelector('.api-console').click(); await delay();
      record('key-issuance-page-uses-selected-provider',calls.some(c=>c.cmd==='api_open_console' && c.args.provider==='claude'));
      document.getElementById('refresh').click(); await delay();
      record('refresh-preserves-api-form', form.isConnected && inputs[1].value==='sk-dummy-test-key-123456');
      document.getElementById('pin').click(); await delay();
      record('view-change-protects-api-input', !document.body.classList.contains('locked') && form.isConnected);
      form.requestSubmit(); form.requestSubmit();
      await ready('.api-applied'); await delay();
      record('register-sent-once',calls.filter(c=>c.cmd==='api_save').length===1);
      record('secret-cleared-after-save',inputs[1].value==='');
      record('registration-automatically-checks-connection',document.querySelector('.api-check-result').dataset.state==='ready');
      record('registration-applies-api-without-command-copy',activeApi.get('claude')==='project'&&calls.filter(c=>c.cmd==='api_activate').length===1&&!document.querySelector('.api-launch'));
      record('api-activation-never-sends-key-back-through-ui',calls.filter(c=>c.cmd==='api_activate').every(c=>!('key' in c.args))&&!calls.some(c=>c.cmd==='api_launch_command'));
      const afterSaveDrag=dragCount();mouseDown(document.querySelector('.api-dialog-head h2'));await delay();
      record('saved-result-remains-draggable',dragCount()===afterSaveDrag+1);
      document.querySelector('.api-close').click();
      let api=await ready('.api-card');
      record('registered-api-immediately-visible',box(api).height>0 && !api.closest('.api-panel').hidden);
      record('verification-persists-after-render',api.querySelector('.api-status').dataset.state==='ready');
      record('new-api-default-monthly-budget-is-195',api.querySelector('.api-cost-amount').textContent.includes('$195.00') && api.querySelector('.api-cost-amount').textContent.includes('—'));
      api.querySelector('.api-usage-settings').click();let usageForm=await ready('.api-usage-form');
      record('usage-settings-start-with-195',usageForm.elements.namedItem('budget').value==='195');
      usageForm.elements.namedItem('spent').value='78.45';usageForm.requestSubmit();await delay();await delay();api=document.querySelector('.api-card');
      record('manual-api-cost-and-remaining-budget-visible',api.textContent.includes('$78.45')&&api.textContent.includes('$116.55')&&api.querySelector('[role="progressbar"]'));
      api.querySelector('.api-usage-settings').click();usageForm=await ready('.api-usage-form');usageForm.elements.namedItem('spent').value='210';usageForm.requestSubmit();await delay();await delay();api=document.querySelector('.api-card');
      record('over-budget-amount-is-visible-and-bar-is-capped',api.textContent.includes('$15.00 초과')&&api.querySelector('.cost-exceeded')&&api.querySelector('[role="progressbar"]').getAttribute('aria-valuenow')==='100');
      api.querySelector('.api-usage-settings').click();usageForm=await ready('.api-usage-form');usageForm.elements.namedItem('source').value='automatic';usageForm.elements.namedItem('source').dispatchEvent(new Event('change'));usageForm.elements.namedItem('key-id').value='apikey_fixture';const adminField=usageForm.elements.namedItem('admin-key');adminField.value='sk-dummy-admin-123456';usageForm.requestSubmit();await delay();await delay();api=document.querySelector('.api-card');
      record('automatic-api-costs-use-separate-secret-and-key-id',adminField.value===''&&api.textContent.includes('$42.50')&&api.textContent.includes('자동 조회')&&calls.some(c=>c.cmd==='api_usage_save'&&c.args.settings.api_key_id==='apikey_fixture'&&c.args.settings.admin_key==='[REDACTED]'));
      costFailure=true;api.querySelector('.api-usage-refresh').click();await delay();
      record('cost-query-error-keeps-last-known-amount',api.textContent.includes('$42.50')&&api.textContent.includes('조회 권한'));
      costFailure=false;api.querySelector('.api-usage-settings').click();usageForm=await ready('.api-usage-form');usageForm.elements.namedItem('source').value='manual';usageForm.elements.namedItem('source').dispatchEvent(new Event('change'));usageForm.elements.namedItem('spent').value='78.45';usageForm.requestSubmit();await delay();await delay();api=document.querySelector('.api-card');
      record('manual-mode-removes-reporting-credential',!usageFor('claude','project').admin_key_stored);

      api.querySelector('.api-check').click();await delay();
      record('explicit-api-check',api.textContent.includes('연결 확인됨'));
      record('selected-api-shows-active-state',api.classList.contains('api-active')&&api.querySelector('.api-use').textContent==='해제');
      api.querySelector('.api-use').click();await delay();await delay();api=document.querySelector('.api-card');
      record('api-can-restore-previous-cli-settings',!activeApi.has('claude')&&!api.classList.contains('api-active'));
      api.querySelector('.api-use').click();await delay();await delay();api=document.querySelector('.api-card');
      record('saved-api-uses-one-click-without-terminal-command',activeApi.get('claude')==='project'&&api.classList.contains('api-active'));
      api=document.querySelector('.api-card'); api.querySelector('.api-replace').click(); form=await ready('.api-form');inputs=form.querySelectorAll('input');
      record('replacement-never-reveals-stored-key',inputs[0].readOnly && inputs[0].value==='project' && inputs[1].value==='');
      inputs[1].value='sk-dummy-test-key-123456';
      const replaceDrag=dragCount();mouseDown(document.querySelector('.api-dialog-head'));await delay();
      record('key-replacement-can-drag-with-input-preserved',dragCount()===replaceDrag+1&&inputs[1].value==='sk-dummy-test-key-123456');
      form.requestSubmit(); await ready('.api-applied'); await delay();
      record('replacement-is-explicit',calls.filter(c=>c.cmd==='api_save').at(-1).args.replace===true);
      document.querySelector('.api-close').click(); await delay();
      await openApi();form=await ready('.api-form');inputs=form.querySelectorAll('input');inputs[0].value='project';inputs[1].value='sk-dummy-test-key-123456';
      form.requestSubmit(); await delay();
      record('duplicate-does-not-silently-replace-key',form.isConnected && form.querySelector('.api-error').textContent.includes('같은 이름'));
      record('secret-cleared-on-save-error',inputs[1].value==='');
      document.querySelector('.api-close').click(); await delay();
      activationFailure=true;
      await openApi();form=await ready('.api-form');inputs=form.querySelectorAll('input');inputs[0].value='apply-retry';inputs[1].value='sk-dummy-test-key-123456';form.requestSubmit();await ready('.api-apply');
      record('activation-failure-keeps-saved-key-and-offers-retry',connections.has('claude/apply-retry')&&inputs[1].value===''&&!document.querySelector('.api-applied'));
      activationFailure=false;document.querySelector('.api-apply').click();await ready('.api-applied');
      record('activation-retry-uses-saved-key',activeApi.get('claude')==='apply-retry'&&!document.querySelector('.api-apply'));
      document.querySelector('.api-close').click();await delay();
      // A network check may finish after the user closes registration, without freezing the app.
      checkFailure=true;holdCheck=true;
      await openApi('codex');form=await ready('.api-form');inputs=form.querySelectorAll('input');inputs[0].value='secondary';inputs[1].value='sk-dummy-test-key-123456';form.requestSubmit();
      await ready('.api-applied');await delay();document.querySelector('.api-close').click();await delay();
      record('slow-network-check-does-not-trap-dialog',!document.querySelector('.api-dialog'));
      releaseCheck();holdCheck=false;await delay();await delay();
      record('failed-check-persists-after-dialog-close',document.querySelector('.api-status[data-state="invalid_key"]')?.textContent.includes('키 교체'));
      checkFailure=false;
      api=[...document.querySelectorAll('.api-card')].find(el=>el.textContent.includes('project'));
      const remove=api.querySelector('.api-delete');remove.click();
      record('delete-needs-second-click',!calls.some(c=>c.cmd==='api_delete'));
      window.dispatchEvent(new Event('blur'));remove.click();
      record('delete-disarms-on-window-blur',!calls.some(c=>c.cmd==='api_delete'));
      api.dispatchEvent(new Event('pointerleave'));remove.click();
      record('delete-disarms-on-leave',!calls.some(c=>c.cmd==='api_delete'));
      const realNow=Date.now;Date.now=()=>realNow()+4000;remove.click();
      record('delete-expiry-works-without-timer',!calls.some(c=>c.cmd==='api_delete'));
      remove.click();await delay();Date.now=realNow;
      record('delete-removes-api-row',!api.isConnected);
      record('api-never-switches-oauth',!calls.some(c=>c.cmd==='switch_profile'));
      // Show no AI sections when both are disabled, including their API managers.
      visibility={...visibility,claude:false,codex:false};emit('visibility-changed');await delay();await delay();
      record('hidden-ai-leaves-no-provider-panels',!document.querySelector('#app .profile-actions, #app .api-panel'));
      visibility={...visibility,claude:true,codex:true};emit('visibility-changed');await ready('.card');await delay();
      record('no-horizontal-overflow',document.querySelector('#app').scrollWidth<=420);
      codexBlocked=true; emit('visibility-changed'); await delay(); await delay();
      record('api-available-with-blocked-subscription-store',document.querySelectorAll('.api-panel').length===2 && document.querySelector('#app').textContent.includes('파일 외'));
      codexBlocked=false; emit('visibility-changed'); await delay(); await delay();
      record('api-selection-removes-subscription-session-note',!document.querySelector('.codex-session-note'));
      record('no-browser-errors',errors.length===0);
    } catch(e) { errors.push(String(e)); record('completed',false); } finally {
    const output=document.createElement('pre');output.id='ui-test-result';output.hidden=true;
    output.textContent=JSON.stringify({checks,errors,mode,width:document.querySelector('.shell').getBoundingClientRect().width,height:document.querySelector('.shell').getBoundingClientRect().height});document.body.append(output);
    window.webkit?.messageHandlers?.report?.postMessage(output.textContent);
    }
  });
}
const allAt=process.argv.indexOf('--all-modes');
const allDir=allAt>=0 ? path.resolve(process.argv[allAt+1]) : null;
const outputAt=process.argv.indexOf('--output');
async function chromium(entry, width, screenshot, profile) {
  const child=spawn(chrome,['--headless=new','--disable-gpu','--no-first-run','--no-default-browser-check',`--user-data-dir=${profile}`,'--allow-file-access-from-files','--remote-debugging-port=0','about:blank'],{stdio:'ignore'});
  let socket;
  const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
  try {
    const portFile=path.join(profile,'DevToolsActivePort');
    // Fresh hosted Windows runners can take longer than five seconds to launch
    // Chromium. Wait for its actual readiness signal before opening the port file.
    for(let i=0;i<400 && !fs.existsSync(portFile);i++) {
      if(child.exitCode !== null) throw new Error('Chromium exited before readiness: '+child.exitCode);
      await sleep(50);
    }
    if(!fs.existsSync(portFile)) throw new Error('Chromium did not become ready within 20 seconds');
    const port=Number(fs.readFileSync(portFile,'utf8').split('\n')[0]);
    const pages=await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    socket=new WebSocket(pages.find(page=>page.type==='page').webSocketDebuggerUrl);
    await new Promise((resolve,reject)=>{socket.onopen=resolve;socket.onerror=reject;});
    let seq=0;const pending=new Map();
    socket.onmessage=event=>{const result=JSON.parse(event.data);if(result.id){const job=pending.get(result.id);if(job){pending.delete(result.id);clearTimeout(job.timer);result.error?job.reject(new Error(JSON.stringify(result.error))):job.resolve(result.result);}}};
    const send=(method,params={})=>new Promise((resolve,reject)=>{const id=++seq;const timer=setTimeout(()=>{pending.delete(id);reject(new Error('CDP timeout: '+method));},15000);pending.set(id,{resolve,reject,timer});socket.send(JSON.stringify({id,method,params}));});
    await send('Emulation.setDeviceMetricsOverride',{width,height:1100,deviceScaleFactor:1,mobile:false});
    await send('Page.navigate',{url:pathToFileURL(entry).href});
    let report;
    for(let i=0;i<200;i++) {
      const value=await send('Runtime.evaluate',{expression:"document.querySelector('#ui-test-result')?.textContent",returnByValue:true});
      if(value.result.value){report=JSON.parse(value.result.value);break;}await sleep(50);
    }
    if(!report) throw new Error('No UI result');
    if(screenshot){
      for(let i=0;i<40;i++) {
        const stats=await send('Runtime.evaluate',{expression:"[...document.querySelectorAll('.mon-val')].every(el=>el.textContent!=='--')",returnByValue:true});
        if(stats.result.value)break;await sleep(50);
      }
      await send('Emulation.setDeviceMetricsOverride',{width,height:Math.ceil(report.height),deviceScaleFactor:1,mobile:false});await sleep(150);
      const rect=await send('Runtime.evaluate',{expression:"(()=>{const r=document.querySelector('.api-dialog')?.getBoundingClientRect();return r?{x:r.x,y:r.y,width:r.width,height:r.height}:null})()",returnByValue:true});
      report.dialog=rect.result.value;
      const shot=await send('Page.captureScreenshot',{format:'png'});fs.writeFileSync(screenshot,Buffer.from(shot.data,'base64'));
    }
    return report;
  } finally {
    socket?.close();
    const exited=new Promise(resolve=>{if(child.exitCode!==null||child.signalCode!==null)resolve();else child.once('exit',resolve);});
    if(process.platform==='win32' && child.pid) spawnSync('taskkill',['/PID',String(child.pid),'/T','/F'],{stdio:'ignore',windowsHide:true});
    else child.kill();
    await exited;
  }
}
async function run(mode, preview, screenshot) {
  const width={normal:420,compact:280,minimal:120,edge:80}[mode];
  let html=fs.readFileSync(path.join(root,'dist/index.html'),'utf8').replaceAll('"/assets/','"'+(useWebKit?'./assets/':pathToFileURL(path.join(root,'dist/assets/')).href));
  html=html.replace('<head>','<head><script>('+fixture.toString()+')('+JSON.stringify(mode)+','+JSON.stringify(preview)+');</script>');
  const entry=path.join(temp,mode+'.html');fs.writeFileSync(entry,html);
  if(useWebKit) {
    const native=spawnSync(path.join(temp,'webkit'),[entry,String(width)],{encoding:'utf8',timeout:50000,maxBuffer:5e6});
    if(native.status!==0) throw new Error('WebKit failed: '+native.stdout+' '+native.stderr.slice(-600));
    return JSON.parse(native.stdout);
  }
  return chromium(entry,width,screenshot,`${temp}/profile-${mode}-${preview}`);
}
try {
  if(useWebKit) {
    fs.cpSync(path.join(root,'dist/assets'),path.join(temp,'assets'),{recursive:true});
    const compile=spawnSync('xcrun',['swiftc',path.join(root,'scripts/test-workspace-webkit.swift'),'-o',path.join(temp,'webkit')],{encoding:'utf8',timeout:60000});
    if(compile.status!==0) throw new Error(compile.stderr);
  }
  const screenshotAt=process.argv.indexOf('--screenshot');
  const reports=[await run('normal',false,screenshotAt>=0?path.resolve(process.argv[screenshotAt+1]):null)];
  if(useWebKit) {
    for(const mode of ['compact','minimal','edge']) reports.push(await run(mode,true,null));
  }
  if(allDir && !useWebKit) {
    fs.mkdirSync(allDir,{recursive:true});
    for(const mode of ['normal','compact','minimal','edge']) reports.push(await run(mode,true,path.join(allDir,`mode-${mode}.png`)));
    const registration=await run('normal','registration',path.join(allDir,'api-registration.png'));
    const result=await run('normal','result',path.join(allDir,'api-result.png'));
    const noAI=await run('minimal','no-ai',path.join(allDir,'mode-no-ai.png'));
    fs.writeFileSync(path.join(allDir,'api-preview-layout.json'),JSON.stringify({registration:registration.dialog,result:result.dialog},null,2)+'\n');
    const labels={normal:['01 일반','계정 · API 키 · 전체 도구'],compact:['02 컴팩트','계정과 사용량 요약'],minimal:['03 미니멀','사용량만 작게'],edge:['04 엣지','벽 붙임 · 펼친 상태']};
    const figures=reports.slice(1).map(r=>`<figure style="width:${r.width}px"><figcaption><b>${labels[r.mode][0]}</b><span>${labels[r.mode][1]}</span></figcaption><div style="height:${Math.ceil(r.height)}px"><img src="mode-${r.mode}.png" style="width:${r.width}px"></div></figure>`).join('');
    const board=`<!doctype html><html lang="ko"><meta charset="utf-8"><title>switcher · 네 가지 모드</title><style>*{box-sizing:border-box}body{margin:0;background:#e8ecec;color:#182024;font-family:-apple-system,BlinkMacSystemFont,sans-serif;padding:32px 38px}header{display:flex;align-items:baseline;gap:16px;margin-bottom:28px}h1{font-size:26px;letter-spacing:-1px;margin:0}header span{font-size:12px;color:#566568}.views{display:flex;align-items:flex-start;gap:30px}figure{margin:0;flex:none}figcaption{height:56px}b{font-size:15px;display:block}figcaption span{font-size:10px;color:#566568;display:block;margin-top:6px;white-space:nowrap}figure div{overflow:hidden;border-radius:12px;box-shadow:0 12px 32px #17212221}img{display:block}</style><header><h1>switcher</h1><span>실제 프론트 화면 · 예시 계정 · 4개 모드 유지</span></header><div class="views">${figures}</div></html>`;
    const boardPath=path.join(allDir,'four-modes.html');fs.writeFileSync(boardPath,board);
    const height=Math.ceil(Math.max(...reports.slice(1).map(r=>r.height))+170);
    const screen=spawnSync(chrome,['--headless=new','--disable-gpu','--no-first-run',`--user-data-dir=${temp}/board`,`--window-size=1100,${height}`,'--hide-scrollbars','--virtual-time-budget=1000',`--screenshot=${path.join(allDir,'four-modes.png')}`,pathToFileURL(boardPath).href],{encoding:'utf8',timeout:30000});
    if(screen.status!==0) throw new Error('Preview board failed');
    const apiFigure=(label,file,rect)=>`<section class="api-step"><h3>${label}</h3><div class="crop" style="width:${rect.width}px;height:${rect.height}px"><img src="${file}" style="width:420px;transform:translate(${-rect.x}px,${-rect.y}px)"></div></section>`;
    const complete=`<!doctype html><html lang="ko"><meta charset="utf-8"><title>switcher · 전체 개선 화면</title><style>*{box-sizing:border-box}body{margin:0;background:#e8ecec;color:#182024;font-family:-apple-system,BlinkMacSystemFont,sans-serif;padding:32px 38px}header{margin-bottom:30px}h1{font-size:27px;letter-spacing:-1px;margin:0 0 7px}header p{font-size:12px;color:#566568;margin:0}.layout{display:flex;align-items:flex-start;gap:36px}.views{display:flex;align-items:flex-start;gap:24px}figure{margin:0;flex:none}figcaption{height:56px}figcaption b{font-size:15px;display:block}figcaption span{font-size:10px;color:#566568;display:block;margin-top:6px;white-space:nowrap}figure>div,.crop{overflow:hidden;border-radius:12px;box-shadow:0 12px 32px #17212221}img{display:block}.api-column{width:380px;flex:none;border-left:1px solid #cad4d4;padding-left:0}.api-step h3{font-size:13px;margin:0 0 12px}.api-step+.api-step{margin-top:24px}.note{font-size:11px;line-height:1.7;color:#566568;margin-top:20px}.extra{position:absolute;top:770px;left:482px;width:280px;display:flex;align-items:flex-start;gap:16px}.extra img{border-radius:10px;box-shadow:0 8px 24px #17212221}.extra p{font-size:11px;line-height:1.8;color:#566568;margin:6px 0}.extra strong{display:block;color:#182024;font-size:12px;margin-bottom:5px}</style><header><h1>switcher · 전체 개선 화면</h1><p>예시 계정으로 검증한 실제 프론트 · 네 가지 모드 유지 · API 키 등록부터 버튼 전환까지</p></header><div class="layout"><div class="views">${figures}</div><aside class="api-column">${apiFigure('API 01 · 키 등록', 'api-registration.png',registration.dialog)}${apiFigure('API 02 · 저장 후 바로 사용', 'api-result.png',result.dialog)}<p class="note">연결 확인·사용량은 검증용 예시 데이터입니다.<br>실제 키를 등록하면 해당 연결의 결과를 표시합니다.</p></aside></div><div class="extra"><img src="mode-no-ai.png" width="120"><p><strong>AI를 모두 숨겼을 때</strong>SYSTEM과 도구만 남습니다.<br>숨긴 계정의 빈 영역도 사라집니다.<br><br>계정 조작은 같은 위치에:<br>다시 연결 · 이름 변경 · 삭제</p></div></html>`;
    const completePath=path.join(allDir,'complete-preview.html');fs.writeFileSync(completePath,complete);
    const completeShot=spawnSync(chrome,['--headless=new','--disable-gpu','--no-first-run',`--user-data-dir=${temp}/complete-board`,'--window-size=1490,1200','--hide-scrollbars','--virtual-time-budget=1000',`--screenshot=${path.join(allDir,'complete-preview.png')}`,pathToFileURL(completePath).href],{encoding:'utf8',timeout:30000});
    if(completeShot.status!==0) throw new Error('Complete preview failed');
    reports.push(registration,result,noAI);

  }
  const result={reports,passed:reports.reduce((n,r)=>n+r.checks.filter(c=>c.passed).length,0),failed:reports.reduce((n,r)=>n+r.checks.filter(c=>!c.passed).length,0)};
  if(outputAt>=0) fs.writeFileSync(process.argv[outputAt+1],JSON.stringify(result,null,2)+'\n');
  console.log(JSON.stringify(result,null,2));process.exitCode=result.failed?1:0;
} finally { fs.rmSync(temp,{recursive:true,force:true,maxRetries:5,retryDelay:200}); }
