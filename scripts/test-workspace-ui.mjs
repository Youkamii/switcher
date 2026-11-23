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
  const emptyAccounts = preview === 'empty-accounts';
  let emptyLayout;
  const activeApi = new Map(preview===true || preview==='api-folded' ? [['claude','project']] : []); let activationFailure = false;
  let codexBlocked = false; let checkFailure = false; let releaseCheck = null; let holdCheck = false;
  // With no billing login the compact balance shows its billing link, so the frame tests can press it.
  let balanceIssue = preview === true || preview === 'toggle' ? 'login_required' : null;
  const consoleCredits = new Map(preview === true ? [['project', { amount_minor:7845,currency:'USD',checked_at:1794300000 }]] : []);
  let balanceMinor = 7845;
  let balanceCurrency = "USD";
  let sessionPhase = null;
  // 'toggle'·'api-ready': the key belongs to the account in use, so one more click can select it.
  const ownKey = preview === 'toggle' || preview === 'api-ready';
  const connections = new Map(preview && !emptyAccounts ? [['claude/project',{provider:'claude',name:'project',account_id:ownKey?'claude-1':'claude-2',check_state:'ready',checked_at:1794300000}]] : []);
  const activeProfile = new Map([['claude','personal'],['codex','personal']]);
  // Toggle scenario controls: a simulated clock (the app measures elapsed time at each press, it runs no
  // timers), how long the fake backend takes on that clock, and a gate that keeps usage requests pending.
  let clockSkew = 0; let backendLatency = 0; let usageGate = null; let openUsage = null;
  // Key-only registration (#187): the account the fixture "finds" for a key when several are saved (null = unknown).
  let autoOwner = null;
  let transferFiles = [{path:'/Users/fixture/Downloads/switcher-transfer-20261123-101500.swx', name:'switcher-transfer-20261123-101500.swx', modified:1795400000}];
  // Handle drag (#188): a window that really moves. The window is the viewport (80x1100) on a wall that is
  // 2100 tall (work area 40..2140), so there is room to travel. buttonHeld/cursorAt answer the native probes.
  const restoreScene = preview === 'edge-restore';
  const dragScene = preview === 'edge-drag' || restoreScene;
  const win = {x:1320, y:500, h:1100}; let buttonHeld = false;
  // What the native side recorded for the press that grabbed the grip: where it went down and where it let go.
  let gripPress = null;
  // A widget that was left on the left wall, a quarter of the way down, with the handle 200px into the panel.
  // The app always starts at the bottom right of the screen, so this is what a restart looks like.
  if(restoreScene) { win.y = 1040; localStorage.setItem('switcher.edgeanchor','0.25000'); localStorage.setItem('switcher.edgetab','200'); localStorage.setItem('switcher.edgeside','left'); }
  const findXY = v => { if(!v || typeof v!=='object') return null; if(typeof v.x==='number' && typeof v.y==='number') return {x:v.x,y:v.y};
    for(const k of Object.keys(v)) { const hit=findXY(v[k]); if(hit) return hit; } return null; };
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
      if(cmd === 'transfer_export' || cmd === 'transfer_import') { calls.push({cmd, args:{...args, password:'[REDACTED]', passwordLength:(args.password ?? '').length}}); }
      else calls.push({cmd, args: cmd === 'api_save' ? {provider:args.provider,accountId:args.accountId,name:args.name,key:'[REDACTED]',replace:args.replace}
        : cmd === 'api_save_auto' ? {provider:args.provider,accountId:args.accountId,name:args.name,key:'[REDACTED]'} : args});
      if(cmd === 'plugin:event|listen') { const ids = events.get(args.event) ?? []; ids.push(args.handler); events.set(args.event,ids); return seq++; }
      if(cmd === 'get_language') return 'ko';
      if(cmd === 'get_accent_theme') return 'deep-green';
      if(cmd === 'get_github_star_prompt_choice') return 'dismissed';
      if(cmd === 'initial_view_mode') return mode;
      if(cmd === 'get_visibility') return visibility;
      if(cmd === 'transfer_export') return {path:'/Users/fixture/Downloads/switcher-transfer-20261123-101500.swx', accounts:3, api_keys:2};
      if(cmd === 'transfer_candidates') return transferFiles;
      if(cmd === 'transfer_reveal') return null;
      if(cmd === 'transfer_import') { if(args.password !== 'right-password') throw '비밀번호가 다르거나 파일이 손상됐습니다'; return {accounts_added:2, accounts_replaced:1, live_replaced:1, api_keys_added:1, api_keys_replaced:2, api_keys_unchanged:1, rejected:0, problems:["API 키 'work': 덮어썼지만 사용 중 상태를 새 키로 맞추지 못했습니다 — 사용을 다시 눌러 주세요"], source_os:'windows'}; }
      if(cmd === 'get_section_order') return ['claude','codex','github','display','system'];
      if(dragScene) {
        if(cmd === 'pointer_button_down') return buttonHeld;
        if(cmd === 'arm_grip_drop') return null;
        if(cmd === 'take_grip_drop') { const press=gripPress; gripPress=null; return press; }
        if(cmd === 'plugin:window|current_monitor') return {name:'fixture',position:{x:0,y:0},size:{width:1400,height:2200},workArea:{position:{x:0,y:40},size:{width:1400,height:2100}},scaleFactor:1};
        if(cmd.endsWith('|outer_position') || cmd.endsWith('|inner_position')) return {x:win.x,y:win.y};
        if(cmd.endsWith('|outer_size') || cmd.endsWith('|inner_size')) return {width:80,height:win.h};
        // Like a real window, the move lands a moment after the call returns: a position read straight after
        // still sees the old place (two placements running at once must not act on that stale reading).
        if(cmd === 'plugin:window|set_position') { const to=findXY(args.value); if(to) delay().then(delay).then(()=>{ win.x=to.x; win.y=to.y; emit('tauri://move',{x:to.x,y:to.y}); }); return null; }
        if(cmd === 'plugin:window|set_size') return null;
      }
      if(cmd === 'demo_mode' || cmd === 'pointer_button_down') return false;
      if(cmd === 'clamshell_mode' || cmd === 'clamshell_cycle') return 0;
      if(cmd === 'tfsd_toggle') return false;
      if(cmd === 'github_list') return {gh_found:true,accounts:[{login:'sample-dev',active:true}]};
      if(cmd === 'display_list') return [{id:0,name:'Built-in display',brightness:65}];
      if(cmd === 'stats_read') return {cpu:18,mem_used:6*1024**3,mem_total:16*1024**3,disk_read:2*1024**2,disk_write:1024**2,net_rx:600*1024,net_tx:200*1024};
      if(cmd === 'list_profiles' && args.provider==='codex' && codexBlocked) throw 'Codex가 파일 외 인증 저장소를 사용합니다';
      if(cmd === 'list_profiles' && emptyAccounts) return {profiles:[],live:null,live_saved:false,api_active:null,api_active_account_id:null};
      if(cmd === 'list_profiles') return { api_active:activeApi.get(args.provider)??null, api_active_account_id:connections.get(args.provider+'/'+activeApi.get(args.provider))?.account_id, live:activeApi.has(args.provider)?null:{id:args.provider+'-1',email:'personal@example.test'},live_saved:!activeApi.has(args.provider),profiles:[
        {name:'personal', id:args.provider+'-1',email:'personal@example.test',plan:args.provider==='claude'?'Max':'Plus',active:!activeApi.has(args.provider)&&activeProfile.get(args.provider)==='personal',saved_at:1,plan_tier:null},
        ...(args.provider==='claude' ? [{name:'work',id:'claude-2',email:'work@example.test',plan:'Pro',active:!activeApi.has('claude')&&activeProfile.get('claude')==='work',saved_at:1,plan_tier:null}] : [])] };
      // Like the backend: selecting a subscription account releases the API selection first.
      if(cmd === 'switch_profile') { clockSkew+=backendLatency; activeApi.delete(args.provider); activeProfile.set(args.provider,args.name); return {backed_up_to:null,switched_to:args.name}; }
      if(cmd === 'codex_session_status') return sessionPhase ? {phase:sessionPhase,reason:null} : null;
      if(cmd === 'fetch_usage') { const usage = { windows:[{key:'five_hour',label:'5 Hours',percent:32,resets_at:null},{key:'weekly',label:'Weekly',percent:54,resets_at:null}],stale:args.profile==='work',stale_age_secs:600,issue:args.profile==='work'?'토큰 갱신 권한이 거부됐습니다 — 계정·조직의 접근 정책을 확인하세요':null }; return usageGate ? usageGate.then(()=>usage) : usage; }
      if(cmd === 'api_list') return [...connections.values()].filter(c=>c.provider===args.provider).map(c=>({...c,active:activeApi.get(c.provider)===c.name}));
      if(cmd === 'start_login') throw 'fixture login stopped';
      if(cmd === 'api_balance') {
        if (args.provider !== 'claude') throw Error('Only Claude has automatic credit lookup');
        return {credit:balanceIssue ? null : {amount_minor:balanceMinor,currency:balanceCurrency,checked_at:1794300000},
          issue:balanceIssue,organization:'Demo Studio',import_ref:args.name+'/fixture',console_credit:consoleCredits.get(args.name)??null};
      }
      if(cmd === 'api_import_console_credit') {
        if(args.reading.organization!=='Demo Studio' || args.reading.import_ref!==args.name+'/fixture') throw 'wrong fixture organization';
        consoleCredits.set(args.name,{amount_minor:args.reading.amount_minor,currency:'USD',checked_at:1794300000}); return;
      }
      if(cmd === 'api_save') {
        if(args.key !== 'sk-dummy-test-key-123456') throw 'test input mismatch';
        if(!args.accountId) throw 'account required';
        let name=args.name;
        if(!args.replace && [...connections.values()].some(c=>c.provider===args.provider && c.account_id===args.accountId && (c.label??c.name)===name)) throw '같은 이름의 API 연결이 있습니다';
        if(!args.replace && connections.has(args.provider+'/'+name)) name+='-2';
        const id=args.provider+'/'+name, previous=connections.get(id);
        if(args.replace && previous?.account_id!==args.accountId) throw 'wrong owner';
        const saved={provider:args.provider,name,account_id:args.accountId,label:previous?.label??args.name,check_state:'unchecked'};
        connections.set(id,saved); return saved;
      }
      if(cmd === 'api_save_auto') {
        // Stands in for the owner lookup: an explicit account wins, a lone account owns the key,
        // otherwise the fixture's "organization name" (autoOwner) points at an account or at nobody.
        if(args.key !== 'sk-dummy-test-key-123456') throw 'test input mismatch';
        const accounts = args.provider==='claude' ? {'claude-1':'personal','claude-2':'work'} : {[args.provider+'-1']:'personal'};
        const ids = Object.keys(accounts);
        const owner = args.accountId || (ids.length===1 ? ids[0] : autoOwner);
        if(!ids.includes(owner)) { if(args.accountId) throw 'API 키를 추가할 계정을 찾을 수 없습니다'; return {status:'needs_account'}; }
        const taken = label => [...connections.values()].some(c=>c.provider===args.provider && c.account_id===owner && (c.label??c.name)===label);
        let label = args.name;
        if(!label) { label='api'; for(let n=2; taken(label); n++) label='api-'+n; }
        else if(taken(label)) throw '이 계정에 같은 이름의 API 키가 있습니다';
        let name = label; for(let n=2; connections.has(args.provider+'/'+name); n++) name=label+'-'+n;
        const saved={provider:args.provider,name,account_id:owner,label,check_state:'unchecked'};
        connections.set(args.provider+'/'+name,saved);
        return {status:'saved',connection:saved,account_id:owner,account:accounts[owner],matched_by:args.accountId?'chosen':ids.length===1?'only_account':'organization_name'};
      }
      if(cmd === 'api_attach') {
        const entry=connections.get(args.provider+'/'+args.name);
        if(!entry || entry.account_id) throw 'fixture key is already linked';
        entry.account_id=args.accountId; return;
      }
      if(cmd === 'api_activate') {
        if(activationFailure) throw 'API 설정 적용 실패';
        clockSkew+=backendLatency;
        activeApi.set(args.provider,args.name); return {...connections.get(args.provider+'/'+args.name),active:true};
      }
      if(cmd === 'api_deactivate') {activeApi.delete(args.provider);return;}
      if(cmd === 'api_check') {
        if(holdCheck) await new Promise(resolve=>{releaseCheck=resolve;});
        const id=args.provider+'/'+args.name;
        const entry={...connections.get(id),check_state:checkFailure?'invalid_key':'ready',checked_at:1794300000};
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
  const settle = async(test) => { for(let i=0;i<120 && !test();i++) await delay(); return test(); };
  const openApi = async(provider='claude', account=provider==='claude'?'work':'personal') => {
    (await ready('.profile-card[data-provider="'+provider+'"][data-name="'+account+'"] .account-api-add')).click();
    return ready('.api-form');
  };
  const saveAndUse = form => form.requestSubmit(form.querySelector('.api-save-use'));
  async function checkEmptyAccounts() {
    const body = {normal:'body:not(.locked)', minimal:'body.minimal', edge:'body.edge'}[mode];
    const systemSelector = body+' #app section[data-key="system"]';
    await ready(systemSelector);
    if(mode === 'edge') {
      emit('edge-hover', true);
      await ready('body.edge-out');
    }
    for(let i=0;i<6;i++) await delay();
    const providers = '#app section[data-key="claude"], #app section[data-key="codex"]';
    const accountElements = [
      '.profile-card', '.compact-card.prov-claude', '.compact-card.prov-codex',
      '.edge-account', '.account-apis', '.compact-api-active', '.api-recovery',
    ].map(selector => '#app '+selector).join(', ');
    record('no-accounts-has-no-key-or-profile-rows-'+mode,
      !document.querySelector(accountElements));
    if(mode === 'normal') {
      const sections = [...document.querySelectorAll(providers)];
      record('no-accounts-keeps-both-registration-entries',
        sections.length === 2 && sections.every(section => section.querySelector('.add-account')?.getBoundingClientRect().width > 0));
      // A key needs an account to live under: with none saved, only "add account" is offered.
      record('no-accounts-offers-no-key-add', !document.querySelector('.add-api-key'));
      return;
    }
    record('no-accounts-omits-provider-sections-'+mode, !document.querySelector(providers));
    if(mode === 'edge') {
      const gauges = [...document.querySelectorAll('#edge-gauge [data-provider="claude"], #edge-gauge [data-provider="codex"]')];
      record('no-accounts-removes-edge-ai-gauges-and-divider',
        gauges.length === 2 && gauges.every(row => row.hidden) && document.querySelector('#edge-gauge').dataset.separatedSystem === 'false');
    }
    const system = () => document.querySelector(systemSelector);
    const geometry = () => {
      const shell = document.querySelector('.shell').getBoundingClientRect();
      return {height: shell.height, systemTop: system().getBoundingClientRect().top - shell.top};
    };
    const before = system();
    const enabled = geometry();
    visibility = {...visibility, claude:false, codex:false};
    emit('visibility-changed');
    for(let i=0;i<100 && system()===before;i++) await delay();
    if(!system() || system() === before) throw Error('Hidden AI layout did not render');
    for(let i=0;i<6;i++) await delay();
    const hidden = geometry();
    emptyLayout = {enabled, hidden};
    record('no-accounts-same-height-and-system-position-as-hidden-'+mode,
      enabled.height > 0 && hidden.height > 0 &&
      Math.abs(enabled.height - hidden.height) < 1 && Math.abs(enabled.systemTop - hidden.systemTop) < 1);
  }
  // While an API key is in use, the collapsed edge handle shows it without opening the panel (#186).
  async function checkFolded() {
    await ready('.edge-account');
    const row = () => document.querySelector('#edge-gauge .edge-gauge-row[data-provider="claude"]');
    await settle(()=>row()?.classList.contains('api') && document.querySelector('#edge-gauge .edge-gauge-row[data-provider="codex"] .edge-gauge-bar'));
    for(let i=0;i<8;i++) await delay();
    const mark = row()?.querySelector('.edge-gauge-api')?.getBoundingClientRect();
    const tab = document.getElementById('edge-tab').getBoundingClientRect();
    const gauge = document.getElementById('edge-gauge').getBoundingClientRect();
    record('collapsed-handle-stays-collapsed', !document.body.classList.contains('edge-out'));
    record('collapsed-api-mark-is-visible', mark && mark.width>0 && mark.height>0 && Number(getComputedStyle(document.getElementById('edge-gauge')).opacity)>0.9);
    record('collapsed-api-mark-fits-the-gauge-column', mark && mark.left>=gauge.left-0.5 && mark.right<=gauge.right+0.5 && mark.top>=gauge.top-0.5 && mark.bottom<=gauge.bottom+0.5);
    record('collapsed-api-mark-fits-inside-the-handle', mark && mark.left>=tab.left && mark.right<=innerWidth+0.5 && mark.top>=tab.top && mark.bottom<=tab.bottom);
    record('collapsed-api-row-keeps-the-usage-row-height', Math.abs(row().getBoundingClientRect().height-19)<0.5);
    const zone = calls.filter(c=>c.cmd==='set_edge_zone').at(-1)?.args.zone?.rect;
    record('collapsed-hover-zone-follows-the-handle', zone && Math.abs(zone[3]-(tab.height+8))<1.5);
  }
  // After a restart the handle is back on the wall and at the height where it was left (#188).
  async function checkEdgeRestore() {
    await ready('.edge-account');
    await settle(()=>!document.querySelector('#edge-gauge .edge-gauge-row[data-provider="system"]')?.hidden);
    const root = document.documentElement;
    const tabY = () => parseFloat(getComputedStyle(root).getPropertyValue('--edge-tab-y'));
    const edge = () => parseFloat(getComputedStyle(root).getPropertyValue('--edge-tab-height'))/2 + 4;
    const kept = () => 40 + edge() + 0.25*(2100 - 2*edge());           // a quarter of the way down the wall
    const state = () => JSON.stringify({x:win.x,y:win.y,tab:tabY(),kept:kept(),cls:document.body.className,side:localStorage.getItem('switcher.edgeside')});
    const check = (id, passed) => record(passed ? id : id+' '+state(), passed);
    let settled = false;
    for(let i=0;i<400 && !settled;i++) { await delay(); settled = win.x===0 && Math.abs(win.y+tabY()-kept())<=1.5 && Math.abs(tabY()-200)<=1; }
    for(let i=0;i<40;i++) await delay();                               // and it stays there
    check('restart-returns-to-the-remembered-wall', win.x===0 && document.body.classList.contains('edge-left'));
    check('restart-returns-the-handle-to-its-height', Math.abs(win.y+tabY()-kept())<=1.5);
    check('restart-keeps-the-users-panel-arrangement', Math.abs(tabY()-200)<=1 && localStorage.getItem('switcher.edgetab')==='200');
    check('restart-does-not-rewrite-what-the-user-chose', localStorage.getItem('switcher.edgeanchor')==='0.25000' && localStorage.getItem('switcher.edgeside')==='left');
    const zone = calls.filter(c=>c.cmd==='set_edge_zone').at(-1)?.args.zone?.rect;
    check('hover-zone-is-on-the-restored-handle', zone && zone[0]<=0 && Math.abs(zone[1]+zone[3]/2-tabY())<=2);
  }
  // The small handle is dragged up and down the wall, and its place is kept (#188).
  async function checkEdgeDrag() {
    await ready('.edge-account');
    await settle(()=>document.querySelector('.edge-account.active .edge-col'));
    for(let i=0;i<10;i++) await delay();
    const wait = async test => { for(let i=0;i<500 && !test();i++) await delay(); return Boolean(test()); };
    const root = document.documentElement;
    const tabY = () => parseFloat(getComputedStyle(root).getPropertyValue('--edge-tab-y'));
    // The handle grows when the SYSTEM row arrives, so its ranges are read when they are needed.
    const tabH = () => parseFloat(getComputedStyle(root).getPropertyValue('--edge-tab-height'));
    const min = () => tabH()/2 + 4;                                    // nearest the handle gets to a window end
    const wallMin = () => 40 + min(), wallMax = () => 2140 - min();    // handle range on the wall (work area 40..2140)
    await settle(()=>!document.querySelector('#edge-gauge .edge-gauge-row[data-provider="system"]')?.hidden);
    const moves = () => calls.filter(c=>c.cmd==='plugin:window|set_position').length;
    const flags = () => calls.filter(c=>c.cmd==='set_edge_drag').map(c=>c.args.active);
    const accountCalls = () => calls.filter(c=>['switch_profile','api_activate','api_deactivate'].includes(c.cmd)).length;
    const stored = key => localStorage.getItem('switcher.'+key);
    const onScreen = () => win.y + tabY();                             // where the handle stands on screen
    // A failed check carries the numbers it saw.
    const state = () => JSON.stringify({y:win.y,x:win.x,h:win.h,tab:tabY(),tabH:tabH(),anchor:stored('edgeanchor'),edgetab:stored('edgetab'),moves:moves(),cls:document.body.className});
    const check = (id, passed) => record(passed ? id : id+' '+state(), passed);
    let pressScreenY = 0;
    const fire = (type, x, y, screenY, buttons, target) => (target ?? document.elementFromPoint(x,y) ?? document.body).dispatchEvent(
      new PointerEvent(type,{bubbles:true,cancelable:true,composed:true,pointerId:1,isPrimary:true,pointerType:'mouse',button:type==='pointermove'?-1:0,buttons,clientX:x,clientY:y,screenX:x,screenY}));
    const press = (x, y, target) => { pressScreenY = win.y + y; buttonHeld = true; fire('pointerdown',x,y,pressScreenY,1,target); };
    const dragTo = (x, y, dy) => fire('pointermove',x,y,pressScreenY+dy,1);
    const release = (x, y, dy=0) => { buttonHeld = false; fire('pointerup',x,y,pressScreenY+dy,0);
      return (document.elementFromPoint(x,y) ?? document.body).dispatchEvent(new MouseEvent('click',{bubbles:true,cancelable:true,clientX:x,clientY:y})); };
    const handleX = innerWidth - 10;

    check('untouched-handle-sits-mid-window-on-the-right-wall', !document.body.classList.contains('edge-left') && Number.isNaN(tabY()) && stored('edgeanchor')===null && win.x===1320 && win.y===500);
    // The pointer reaches the handle, the panel starts to come out, and the press lands before any content shows.
    emit('edge-hover',true); await ready('body.edge-out');
    const startY = innerHeight/2; const baseMoves = moves();
    press(handleX,startY); await delay(); await delay();
    dragTo(handleX,startY-3,-3); await delay();
    check('a-press-that-barely-moves-is-not-a-drag', !document.body.classList.contains('edge-dragging') && document.body.classList.contains('edge-out') && flags().length===0);
    dragTo(handleX,startY-40,-40); await delay(); await delay();
    check('dragging-folds-the-panel-and-shows-the-handle', document.body.classList.contains('edge-dragging') && !document.body.classList.contains('edge-out') && flags().at(-1)===true);
    check('handle-follows-the-pointer-inside-the-window', Math.abs(tabY()-(startY-40))<=1 && moves()===baseMoves && win.y===500);
    const gauge = getComputedStyle(document.getElementById('edge-gauge'));
    check('handle-and-gauge-do-not-animate-while-dragged', gauge.opacity==='1' && gauge.animationName==='none' && getComputedStyle(document.getElementById('edge-tab')).animationName==='none');
    // Past the top of the window the window comes along; at the top of the wall both stop.
    dragTo(handleX,10,-700); await wait(()=>Math.abs(win.y-(1050-700-min()))<=1);
    check('window-follows-once-the-handle-reaches-its-end', Math.abs(win.y-(1050-700-min()))<=1 && Math.abs(tabY()-min())<=1 && Math.abs(onScreen()-(1050-700))<=1);
    dragTo(handleX,10,-5000); await wait(()=>win.y===40);
    check('handle-reaches-the-top-of-the-wall', win.y===40 && Math.abs(onScreen()-wallMin())<=1);
    dragTo(handleX,900,5000); await wait(()=>win.y===1040);
    check('handle-reaches-the-bottom-of-the-wall', win.y===1040 && Math.abs(onScreen()-wallMax())<=1);
    // Back up a little: the handle moves inside the window again and the window stays.
    const before = moves();
    dragTo(handleX,500,450); await delay(); await delay();
    check('coming-back-moves-the-handle-first', win.y===1040 && moves()===before && Math.abs(onScreen()-1500)<=1);
    const calmBefore = accountCalls();
    const clickReached = release(handleX,500,450); await delay(); await delay();
    check('drop-ends-the-drag-and-keeps-the-panel-folded', !document.body.classList.contains('edge-dragging') && !document.body.classList.contains('edge-out') && flags().at(-1)===false && flags().filter(Boolean).length===1);
    check('drop-saves-height-wall-and-offset', Math.abs(Number(stored('edgeanchor'))-(1500-wallMin())/(wallMax()-wallMin()))<0.001 && stored('edgetab')===String(Math.round(1500-1040)) && stored('edgeside')==='right');
    check('drag-never-presses-the-card-under-the-handle', accountCalls()===calmBefore && clickReached===false);
    const zone = calls.filter(c=>c.cmd==='set_edge_zone').at(-1)?.args.zone?.rect;
    check('hover-zone-moves-with-the-handle', zone && Math.abs(zone[1]+zone[3]/2-tabY())<=2);
    // The folded handle under the pointer opens on a plain press, without touching a card.
    press(handleX,tabY()); await delay(); release(handleX,tabY()); await delay();
    check('pressing-the-dropped-handle-opens-the-panel', document.body.classList.contains('edge-out') && accountCalls()===calmBefore);

    // A press that starts before the content is visible is aimed at the handle, not at the invisible card.
    const realNow = Date.now; let skew = 0; Date.now = () => realNow()+skew;
    try {
      emit('edge-hover',false); await wait(()=>!document.body.classList.contains('edge-out'));
      skew += 5000; emit('edge-hover',true); await ready('body.edge-out'); await delay();
      const other = document.querySelector('.edge-account.switchable[data-provider="claude"]');
      const box = other.getBoundingClientRect(); const ox = box.left+box.width/2, oy = box.top+box.height/2;
      press(ox,oy,other); await delay(); release(ox,oy); await delay(); await delay();
      record('press-before-content-shows-does-not-switch-accounts', accountCalls()===calmBefore);
      // Once the content is up, the same press is an ordinary click again.
      skew += 900;
      press(ox,oy,other); await delay(); release(ox,oy);
      record('press-on-visible-card-still-switches', await wait(()=>calls.some(c=>c.cmd==='switch_profile' && c.args.name===other.dataset.name)));
      skew += 5000;
    } finally { Date.now = realNow; }
    await ready('.edge-account'); for(let i=0;i<12;i++) await delay();

    // A panel that changes height leaves the handle where it stands on screen.
    emit('edge-hover',false); await wait(()=>!document.body.classList.contains('edge-out'));
    emit('edge-hover',true); await ready('body.edge-out');
    press(handleX,tabY()); await delay(); dragTo(handleX,900,5000); await wait(()=>win.y===1040 && Math.abs(onScreen()-wallMax())<=1); release(handleX,900,5000); await delay();
    // The fixture changes the window height; a re-render makes the app fit and place the window again.
    const refit = async() => { document.getElementById('refresh').click(); for(let i=0;i<8;i++) await delay(); };
    win.h = 900; await refit(); await wait(()=>win.y===1240);
    const arranged = stored('edgetab');
    check('shorter-panel-keeps-the-handle-on-screen', win.y===1240 && Math.abs(onScreen()-wallMax())<=1);
    // What the squeeze changed is only drawn: the user's own arrangement stays stored and comes back.
    check('a-squeezed-panel-does-not-overwrite-the-users-arrangement', stored('edgetab')===arranged && Math.abs(Number(stored('edgeanchor'))-1)<0.001);
    win.h = 1100; await refit(); await wait(()=>win.y===1040);
    check('taller-panel-keeps-the-handle-on-screen', win.y===1040 && Math.abs(onScreen()-wallMax())<=1 && Math.abs(tabY()-Number(arranged))<=1);

    // Moving the panel with its grip: the small handle ends up where the mouse let go, beside the grip.
    // The app cannot see the pointer during a native window drag, so the fixture plays the native record.
    emit('edge-hover',false); await wait(()=>!document.body.classList.contains('edge-out'));
    emit('edge-hover',true); await ready('body.edge-out');
    press(handleX,tabY()); await delay(); dragTo(handleX,500,-578); await delay(); release(handleX,500,-578); await delay();
    check('setup-handle-low-in-the-panel', win.y===1040 && Math.abs(onScreen()-(wallMax()-578))<=1 && tabY()>400);
    const grip = document.querySelector('.edge-handle');
    // down/up: where the pointer pressed and released (screen px); to: where the native drag left the window.
    const gripDrag = async(to, down, up) => {
      buttonHeld = true; fire('pointerdown',20,20,win.y+20,1,grip); await delay(); await delay();
      if(to) { win.x = to.x; win.y = to.y; emit('tauri://move',to); }
      gripPress = {down, up}; await delay(); buttonHeld = false;
    };
    const lowTab = tabY(); const lowAnchor = stored('edgeanchor'); const lowMoves = moves();
    // A press on the grip that goes nowhere moves nothing.
    await gripDrag(null,[1340,1060],[1340,1061]); for(let i=0;i<40;i++) await delay();
    check('pressing-the-grip-without-dragging-changes-nothing', win.y===1040 && Math.abs(tabY()-lowTab)<=1 && stored('edgeanchor')===lowAnchor && moves()===lowMoves);
    // The reported case: the panel is carried up to the top of the screen. The small handle must be up there too.
    await gripDrag({x:1320,y:40},[1340,1060],[1340,12]);
    await wait(()=>win.y===40 && Math.abs(tabY()-min())<=1);
    check('panel-carried-to-the-top-brings-the-small-handle-to-the-top', win.y===40 && Math.abs(onScreen()-wallMin())<=1 && Math.abs(Number(stored('edgeanchor')))<0.001);
    check('the-handle-sits-beside-the-grip', Math.abs(tabY()-min())<=1 && Number(stored('edgetab'))===min());
    // Carried down to the bottom of the screen: the panel comes back inside, the handle stays at the bottom.
    await gripDrag({x:1320,y:2150},[1340,65],[1340,2175]);
    await wait(()=>win.y===1040 && Math.abs(onScreen()-wallMax())<=1);
    check('panel-carried-to-the-bottom-leaves-the-handle-at-the-bottom', win.y===1040 && Math.abs(onScreen()-wallMax())<=1 && Math.abs(Number(stored('edgeanchor'))-1)<0.001);
    // The panel is already against the top and cannot move, the handle is low: pushing the grip up still counts.
    win.y = 40; emit('tauri://move',{x:1320,y:40}); await wait(()=>Math.abs(win.y+tabY()-wallMax())<=1.5);
    const stuckTab = tabY();
    await gripDrag({x:1320,y:40},[1340,65],[1340,3]);
    await wait(()=>Math.abs(tabY()-min())<=1);
    check('pushing-the-grip-against-the-top-lifts-a-low-handle', stuckTab>400 && win.y===40 && Math.abs(onScreen()-wallMin())<=1);
    // Dropped in the middle: the top of the folded handle is exactly where the mouse let go (user's rule),
    // and the panel hangs from it.
    await gripDrag({x:1320,y:700},[1340,65],[1340,725]);
    await wait(()=>win.y===721 && Math.abs(tabY()-min())<=1);
    check('folded-handle-top-is-where-the-mouse-let-go', win.y===721 && Math.abs(onScreen()-(min()-4)-725)<=1 && moves()>lowMoves);
    // Let go low on the wall where the panel cannot hang below: the handle top is still at the mouse.
    await gripDrag({x:1320,y:1800},[1340,741],[1340,1825]);
    await wait(()=>win.y===1040 && Math.abs(onScreen()-(min()-4)-1825)<=1);
    check('handle-top-at-the-mouse-low-on-the-wall', win.y===1040 && Math.abs(onScreen()-(min()-4)-1825)<=1);
    await gripDrag({x:1320,y:700},[1340,1060],[1340,725]);
    await wait(()=>win.y===721 && Math.abs(tabY()-min())<=1);
    check('and-back-up-from-the-low-place', win.y===721 && Math.abs(onScreen()-(min()-4)-725)<=1);
    // Carried to the other wall: it docks there, remembered, with the handle at the height where it was dropped.
    await gripDrag({x:100,y:600},[1340,741],[120,625]);
    await wait(()=>win.x===0 && document.body.classList.contains('edge-left') && stored('edgeside')==='left');
    check('grip-moves-the-panel-to-the-other-wall-and-remembers-it', win.x===0 && win.y===621 && document.body.classList.contains('edge-left') && stored('edgeside')==='left');
    check('handle-top-is-at-the-mouse-on-the-new-wall', Math.abs(onScreen()-(min()-4)-625)<=1.5);
    // A pointer record that is not this grip press is ignored: the handle goes to the top of the dropped panel.
    const beforeForeign = stored('edgeanchor');
    await gripDrag({x:0,y:300},[500,5],[500,900]);
    await wait(()=>stored('edgeanchor')!==beforeForeign && win.y===300 && Math.abs(tabY()-min())<=1);
    check('foreign-pointer-record-is-ignored', win.y===300 && Math.abs(onScreen()-(300+min()))<=1.5);

    // A window move nobody made with the grip leaves the handle where the user put it.
    const kept = wallMin()+Number(stored('edgeanchor'))*(wallMax()-wallMin());
    win.y = 900; emit('tauri://move',{x:0,y:900});
    const settled = await wait(()=>win.y===300 && Math.abs(win.y+tabY()-kept)<=1.5);
    for(let i=0;i<20;i++) await delay();
    check('other-window-moves-return-to-the-users-place', settled && win.y===300 && Math.abs(onScreen()-kept)<=1.5);
  }
  // One click = account, one more click on the account in use = its API key (#186).
  async function checkToggle() {
    const edge = mode === 'edge';
    const card = name => document.querySelector((edge?'.edge-account':'.compact-card.prov-claude')+'[data-name="'+name+'"]'+(edge?'[data-provider="claude"]':''));
    const activeCard = () => document.querySelector(edge?'.edge-account.active[data-provider="claude"]':'.compact-card.active.prov-claude');
    const regions = () => calls.filter(c=>c.cmd==='set_hit_regions').at(-1)?.args.regions ?? [];
    const center = el => { const r=el.getBoundingClientRect(); return [r.left+r.width/2, r.top+r.height/2]; };
    const hitsAt = el => { const [x,y]=center(el); return regions().filter(({rect})=>x>=rect[0]&&y>=rect[1]&&x<=rect[0]+rect[2]&&y<=rect[1]+rect[3]); };
    const click = (el, detail=1) => el.dispatchEvent(new MouseEvent('click',{bubbles:true,cancelable:true,detail}));
    const activations = () => calls.filter(c=>c.cmd==='api_activate');
    const token = name => getComputedStyle(document.documentElement).getPropertyValue(name).trim().replace(/\s+/g,'');
    const sameRgb = (color, name) => color.replace(/\s+/g,'').startsWith('rgb('+token(name)) || color.replace(/\s+/g,'').startsWith('rgba('+token(name));
    await ready(edge?'.edge-account':'.compact-card');
    if(edge) { emit('edge-hover',true); await ready('body.edge-out'); }
    await settle(()=>activeCard()?.querySelector(edge?'.edge-col':'.compact-row'));
    for(let i=0;i<6;i++) await delay();
    let active = activeCard();
    record('account-in-use-with-key-offers-api-next-'+mode, active?.dataset.name==='personal' && active.classList.contains('api-ready') && active.dataset.apiKey==='project' && active.querySelector('.api-next')?.textContent.includes('API') && active.querySelector('.api-next').getBoundingClientRect().width>0);
    record('api-next-names-the-key-and-separate-billing-'+mode, active.title.includes('project') && active.title.includes('별도 과금') && active.title.includes(edge?'한 번 더':'더블클릭'));
    record('api-next-stays-inside-the-card-'+mode, (()=>{const h=active.querySelector('.api-next').getBoundingClientRect(),c=active.getBoundingClientRect();return h.left>=c.left-0.5&&h.right<=c.right+0.5&&h.bottom<=c.bottom+0.5;})());
    record('other-accounts-and-keyless-providers-have-no-api-next-'+mode, !card('work')?.querySelector('.api-next') && !card('work')?.classList.contains('api-ready') && !document.querySelector(edge?'.edge-account[data-provider="codex"] .api-next':'.compact-card.prov-codex .api-next'));
    record('api-next-uses-the-api-colour-not-the-account-colour-'+mode, token('--api-rgb')!==token('--accent-rgb') && token('--api-rgb')!==token('--accent-alt-rgb') && sameRgb(getComputedStyle(active.querySelector('.api-next')).color,'--api-rgb'));
    // Native hit test: where regions overlap, the first plain control wins, otherwise the first hit.
    const winnerAt = el => { const hits=hitsAt(el); return hits.find(hit=>hit.action===null) ?? hits[0]; };
    if(edge) {
      const own = hitsAt(active);
      record('edge-card-in-use-is-a-hover-target-before-the-whole-panel', own.length>=2 && own[0].action===null && !own[0].api && Math.abs(own[0].rect[3]-active.getBoundingClientRect().height)<1.5);
      click(active); await settle(()=>activations().length===1);
      record('one-more-press-selects-the-accounts-own-key', activations().length===1 && activations()[0].args.provider==='claude' && activations()[0].args.name==='project' && !('key' in activations()[0].args) && !calls.some(c=>c.cmd==='switch_profile'));
      // The card and the collapsed handle change as soon as the backend answers, before usage reloads.
      const marked = await settle(()=>card('personal')?.classList.contains('api-live') || card('personal')?.closest('.account-with-api'));
      record('api-state-shows-at-once-on-card-and-collapsed-handle', Boolean(marked) && document.querySelector('#edge-gauge .edge-gauge-row[data-provider="claude"]')?.classList.contains('api'));
      // The second half of the same double-click must not undo the selection it just made.
      click(card('personal'),2); await delay(); await delay();
      record('tail-of-the-same-gesture-does-not-undo-the-api-selection', !calls.some(c=>c.cmd==='switch_profile'));
      await settle(()=>document.querySelector('.account-with-api[data-name="personal"] .compact-api-active'));
    } else {
      const own = hitsAt(active);
      record('minimal-card-in-use-reports-its-key-for-native-double-click', own.length===1 && own[0].api===true && own[0].action?.[0]==='claude' && own[0].action?.[1]==='project');
      const other = hitsAt(card('work'));
      record('minimal-other-account-still-reports-a-plain-switch', other.length===1 && !other[0].api && other[0].action?.[1]==='work');
      // A failed native activation leaves the subscription in use and says why on the card.
      emit('account-switched',{ok:false,error:'API 키체인을 읽지 못했습니다',provider:'claude',name:'project',api:true});
      await delay();
      record('native-api-failure-is-shown-on-the-card', active.classList.contains('active') && !active.classList.contains('api-live') && active.querySelector('.usage-issue.api-issue')?.title.includes('키체인') && !document.querySelector('.account-with-api'));
      // The native poller applies the stored key, then reports it like an account switch.
      activeApi.set('claude','project');
      emit('account-switched',{ok:true,provider:'claude',name:'project',api:true});
      await delay();
      record('native-api-result-marks-the-card-at-once', card('personal').classList.contains('api-live') || card('personal').closest('.account-with-api')!==null);
      await settle(()=>document.querySelector('.account-with-api[data-name="personal"] .compact-api-active'));
    }
    for(let i=0;i<6;i++) await delay();
    const group = document.querySelector('.account-with-api[data-name="personal"]');
    const owner = card('personal');
    record('api-state-frames-the-owner-account-'+mode, group && owner?.closest('.account-with-api')===group && owner.classList.contains('owns-api') && owner.classList.contains('switchable') && !owner.classList.contains('active') && !activeCard());
    record('api-state-uses-its-own-colour-'+mode, group && sameRgb(getComputedStyle(group).borderTopColor,'--api-rgb') && sameRgb(getComputedStyle(group.querySelector('.compact-api-label strong')).backgroundColor,'--api-rgb') && group.querySelector('.compact-api-label strong').textContent==='API');
    record('api-state-names-the-key-'+mode, group?.querySelector('.compact-api-label span')?.textContent==='project');
    // The pointer usually still rests on the card that was just pressed: hover must not wash the API colour out.
    owner.classList.add('hit-hover');
    record('hover-keeps-the-api-colour-'+mode, sameRgb(getComputedStyle(group).borderTopColor,'--api-rgb') && !sameRgb(getComputedStyle(group).borderTopColor,'--accent-rgb'));
    owner.classList.remove('hit-hover');
    record('api-state-stays-inside-window-'+mode, group && group.getBoundingClientRect().left>=0 && group.getBoundingClientRect().right<=innerWidth+0.5);
    const chip = () => document.querySelector('.account-with-api[data-name="personal"] .compact-api-label strong');
    const link = () => document.querySelector('.account-with-api[data-name="personal"] .api-billing-link');
    const switches = () => calls.filter(c=>c.cmd==='switch_profile');
    // Simulated clock: the app measures elapsed time at each press instead of running timers.
    const realNow=Date.now; Date.now=()=>realNow()+clockSkew;
    const later = ms => { clockSkew+=ms; };
    const holdUsage = () => { usageGate = new Promise(resolve => { openUsage = resolve; }); };
    const releaseUsage = () => { openUsage?.(); usageGate = null; openUsage = null; };
    try {
    if(edge) {
      const row=document.querySelector('#edge-gauge .edge-gauge-row[data-provider="claude"]');
      const mark=row?.querySelector('.edge-gauge-api');
      record('collapsed-handle-shows-api-instead-of-usage-bars', row && !row.hidden && row.classList.contains('api') && mark?.textContent==='API' && !row.querySelector('.edge-gauge-bar') && sameRgb(getComputedStyle(mark).backgroundColor,'--api-rgb'));
      record('collapsed-handle-keeps-other-provider-usage', !!document.querySelector('#edge-gauge .edge-gauge-row[data-provider="codex"] .edge-gauge-bar'));
      // The whole frame is one target: its hover area covers the API block, not just the account row.
      const frame=hitsAt(chip());
      record('api-frame-is-one-hover-and-press-target', frame.length>=2 && frame[0].action===null && Math.abs(frame[0].rect[3]-group.getBoundingClientRect().height)<1.5);
      // The key dialog's owner paragraph style must not leak into the card (type size, colour, privacy blur).
      record('owner-card-keeps-its-own-type-and-spacing-'+mode, !owner.classList.contains('api-owner') && ['fontSize','color','filter'].every(prop=>getComputedStyle(owner)[prop]===getComputedStyle(card('work'))[prop]) && getComputedStyle(owner).marginBottom!=='10px');
      const before=activations().length;
      // A deliberate second press comes back, even when the system still counts it as a double-click.
      later(300); click(chip(),2); await settle(()=>switches().length===1);
      record('press-on-the-api-block-returns-to-the-subscription', switches().length===1 && switches()[0].args.provider==='claude' && switches()[0].args.name==='personal' && activations().length===before);
      await settle(()=>activeCard()?.classList.contains('api-ready') && !document.querySelector('.account-with-api'));
      for(let i=0;i<6;i++) await delay();
      active = activeCard();
      record('returning-to-subscription-restores-usage-handle', !document.querySelector('#edge-gauge .edge-gauge-row.api') && !document.querySelector('.account-with-api') && active?.classList.contains('api-ready') && !!active.querySelector(edge?'.edge-col':'.compact-row'));
      click(active); await delay(); await delay();
      record('api-waits-out-the-previous-account-change', activations().length===before);
      later(700); click(active); await settle(()=>activations().length===before+1);
      record('api-selectable-again-after-the-pause', activations().length===before+1);
      await settle(()=>document.querySelector('.account-with-api[data-name="personal"]'));
      // Press on the account row itself: back again. Press, press, press keeps toggling.
      later(400); click(card('personal')); await settle(()=>switches().length===2);
      await settle(()=>activeCard()?.classList.contains('api-ready') && !document.querySelector('.account-with-api'));
      later(700); click(activeCard()); await settle(()=>activations().length===before+2);
      await settle(()=>document.querySelector('.account-with-api[data-name="personal"]'));
      record('every-press-keeps-toggling-account-and-api', switches().length===2 && activations().length===before+2 && switches().every(c=>c.args.name==='personal'));

      // A steady 0.7 s beat with a backend that takes 0.3 s, while usage is still loading: every press
      // still lands. The "one more = API" mark does not wait for the usage reload, and the wait before
      // the API is measured from the press, not from the end of the change.
      holdUsage(); backendLatency=300;
      later(700); click(card('personal')); await settle(()=>switches().length===3);
      await delay(); await delay();
      const back=activeCard();
      record('one-more-press-is-offered-before-usage-reloads', back?.dataset.name==='personal' && back.classList.contains('api-ready') && back.dataset.apiKey==='project' && back.querySelectorAll('.api-next').length===1 && back.querySelector('.api-next').textContent==='API ›' && !back.classList.contains('owns-api') && !!document.querySelector('.account-with-api.api-ended'));
      record('optimistic-api-mark-is-a-hover-target', hitsAt(back).some(hit=>hit.action===null && Math.abs(hit.rect[3]-back.getBoundingClientRect().height)<1.5));
      record('returned-frame-already-looks-like-the-account-in-use', sameRgb(getComputedStyle(document.querySelector('.account-with-api.api-ended')).borderTopColor,'--accent-rgb'));
      record('optimistic-api-mark-names-the-key-once', (back.title.match(/별도 과금/g)??[]).length===1 && back.title.includes('project') && !back.title.includes('구독으로'));
      later(100); click(back); await delay(); await delay();
      record('press-right-after-the-change-shows-is-not-an-api-decision', activations().length===before+2);
      later(300); click(activeCard()); await settle(()=>activations().length===before+3);
      record('steady-beat-reaches-the-api-while-usage-is-loading', activations().length===before+3);
      await settle(()=>card('personal')?.classList.contains('api-live') || card('personal')?.closest('.account-with-api:not(.api-ended)'));
      later(400); click(card('personal')); await settle(()=>switches().length===4);
      record('steady-beat-comes-back-while-usage-is-loading', switches().length===4 && switches().at(-1).args.name==='personal');
      await delay(); await delay();
      later(400); click(activeCard()); await settle(()=>activations().length===before+4);
      record('steady-beat-keeps-toggling-without-waiting-for-usage', activations().length===before+4 && switches().length===4);
      backendLatency=0; releaseUsage();
      await settle(()=>document.querySelector('.account-with-api[data-name="personal"]:not(.api-ended) .compact-api-active'));
      for(let i=0;i<6;i++) await delay();
      record('held-usage-settles-into-the-real-api-state', card('personal')?.classList.contains('owns-api') && !activeCard() && !!document.querySelector('#edge-gauge .edge-gauge-row.api'));

      // The billing link keeps its own job and does not toggle.
      await settle(()=>link()?.getBoundingClientRect().width>0);
      later(400); const billed=calls.filter(c=>c.cmd==='api_open_billing').length; const switched=switches().length; link().click(); await delay(); await delay();
      record('billing-link-inside-the-frame-opens-billing-without-toggling', calls.filter(c=>c.cmd==='api_open_billing').length===billed+1 && switches().length===switched);
      // Another account: one press leaves the API for that account's subscription.
      click(card('work')); await settle(()=>switches().length===switched+1);
      await settle(()=>activeCard()?.dataset.name==='work');
      record('press-on-another-account-leaves-api-for-that-account', switches().at(-1).args.name==='work' && activeCard()?.dataset.name==='work' && !document.querySelector('.account-with-api') && !activeCard().classList.contains('api-ready'));
      // A failed activation stays on the subscription and says why.
      click(card('personal')); await settle(()=>activeCard()?.dataset.name==='personal' && activeCard().classList.contains('api-ready'));
      activationFailure=true; later(700);
      const tried=activations().length; click(activeCard()); await settle(()=>activations().length===tried+1);
      for(let i=0;i<4;i++) await delay();
      const failed=activeCard();
      record('failed-api-selection-keeps-subscription-and-shows-reason', failed?.dataset.name==='personal' && !failed.classList.contains('api-live') && !document.querySelector('.account-with-api') && failed.querySelector('.usage-issue.api-issue')?.title.includes('API 설정 적용 실패') && !document.querySelector('#edge-gauge .edge-gauge-row.api'));
      activationFailure=false;
    } else {
      // Native double-click: the whole frame switches back, wherever inside it the pointer is.
      for(const [id, el] of [['account-row', owner], ['api-block', chip()]]) {
        const hit = winnerAt(el);
        record('minimal-api-frame-switches-back-from-'+id, hit && !hit.api && hit.action?.[0]==='claude' && hit.action?.[1]==='personal' && Math.abs(hit.rect[3]-group.getBoundingClientRect().height)<1.5);
      }
      record('minimal-billing-link-stays-a-plain-control', link().getBoundingClientRect().width>0 && winnerAt(link())?.action===null);
      record('owner-card-keeps-its-own-type-and-spacing-'+mode, !owner.classList.contains('api-owner') && ['fontSize','color','filter'].every(prop=>getComputedStyle(owner)[prop]===getComputedStyle(card('work'))[prop]) && getComputedStyle(owner).marginBottom!=='10px');
      // The native poller switched back. Usage is still loading: the card must already offer its key again,
      // and report it to the poller, so the next double-click does not wait for the reload.
      holdUsage();
      activeApi.delete('claude');
      emit('account-switched',{ok:true,provider:'claude',name:'personal'});
      await delay(); await delay();
      const back=activeCard();
      const again=back ? winnerAt(back) : null;
      record('native-switch-back-offers-the-key-before-usage-reloads', back?.dataset.name==='personal' && back.classList.contains('api-ready') && back.querySelectorAll('.api-next').length===1 && again?.api===true && again.action?.[0]==='claude' && again.action?.[1]==='project');
      releaseUsage();
      await settle(()=>activeCard()?.classList.contains('api-ready') && !document.querySelector('.account-with-api'));
      record('native-switch-back-restores-the-subscription-card', activeCard()?.dataset.name==='personal' && !document.querySelector('.account-with-api') && !!activeCard().querySelector('.compact-row') && activeCard().querySelectorAll('.api-next').length===1);
      // A plain native switch to another account with no key offers nothing.
      emit('account-switched',{ok:true,provider:'claude',name:'work'});
      await delay(); await delay();
      record('native-switch-to-keyless-account-offers-no-api', !card('work')?.classList.contains('api-ready') && !card('work')?.querySelector('.api-next'));
    }
    } finally { Date.now=realNow; backendLatency=0; releaseUsage(); }
  }
  // Key-only registration (#187): the provider header takes a key without an account and the app files it
  // under the account it belongs to; it asks only when it cannot tell.
  // Move to another PC (#189): one locked file out, the same file and password back in.
  async function checkTransfer() {
    await ready('.profile-card');
    for(let i=0;i<4;i++) await delay();
    const open = document.querySelector('.transfer-row .transfer-open');
    const inWindow = el => { const r=el.getBoundingClientRect(); return r.width>0 && r.left>=0 && r.right<=innerWidth && r.top>=0 && r.bottom<=innerHeight; };
    record('normal-mode-offers-moving-to-another-pc', open?.textContent==='다른 PC로 옮기기' && open.getBoundingClientRect().width>0);
    open.click();
    const dialog = await ready('.api-dialog.transfer-dialog');
    const exportForm = dialog.querySelector('.transfer-export'), importForm = dialog.querySelector('.transfer-import');
    const [first, second] = exportForm.querySelectorAll('input[type="password"]');
    record('opens-on-export-with-two-password-fields', !exportForm.hidden && importForm.hidden && first && second && first.minLength===8 && inWindow(dialog) && inWindow(exportForm.querySelector('button.primary')));
    const sent = cmd => calls.filter(c=>c.cmd===cmd);
    first.value='password-one'; second.value='password-two';
    exportForm.requestSubmit();
    for(let i=0;i<4;i++) await delay();
    record('mismatched-passwords-make-no-file', sent('transfer_export').length===0 && exportForm.querySelector('.api-error').textContent==='두 비밀번호가 다릅니다');
    const storedBefore = JSON.stringify(Object.entries(localStorage).sort());
    second.value='password-one';
    exportForm.requestSubmit();
    await ready('.transfer-export .transfer-done');
    const exported = sent('transfer_export')[0];
    record('export-sends-only-the-password', sent('transfer_export').length===1 && exported.args.passwordLength===12 && Object.keys(exported.args).sort().join()==='password,passwordLength' && exportForm.textContent.includes('설정과 메모는 담지 않습니다'));
    const result = exportForm.querySelector('.transfer-result');
    record('export-result-names-the-file-and-what-it-holds', result.textContent.includes('switcher-transfer-20261123-101500.swx') && result.textContent.includes('계정 3개') && result.textContent.includes('API 키 2개') && !result.textContent.includes('/Users/fixture') && first.value==='' && second.value==='' && inWindow(result.querySelector('.transfer-reveal')));
    result.querySelector('.transfer-reveal').click();
    await delay();
    record('the-file-can-be-shown-in-its-folder', sent('transfer_reveal').length===1 && sent('transfer_reveal')[0].args.path.endsWith('.swx'));
    // Import: the file found in Downloads is offered; a wrong password changes nothing on screen but the error.
    dialog.querySelectorAll('.transfer-tabs button')[1].click();
    await settle(()=>importForm.querySelector('select').options.length===1);
    const filePassword = importForm.querySelector('input[type="password"]');
    record('import-offers-the-file-it-found', exportForm.hidden && !importForm.hidden && importForm.querySelector('select').value==='/Users/fixture/Downloads/switcher-transfer-20261123-101500.swx' && importForm.querySelector('input[type="file"]').accept==='.swx' && inWindow(importForm.querySelector('button.primary')));
    filePassword.value='wrong-password';
    importForm.requestSubmit();
    await settle(()=>importForm.querySelector('.api-error').textContent!=='');
    record('a-wrong-password-is-reported-and-imports-nothing', importForm.querySelector('.api-error').textContent.includes('비밀번호가 다르거나') && !importForm.querySelector('.transfer-done') && JSON.stringify(Object.entries(localStorage).sort())===storedBefore);
    filePassword.value='right-password';
    importForm.requestSubmit();
    await ready('.transfer-import .transfer-done');
    const imported = importForm.querySelector('.transfer-result').textContent;
    record('import-result-says-what-arrived-and-what-was-overwritten', imported.includes('계정 2개 추가 · 1개 덮어씀') && imported.includes('지금 로그인된 계정 1개는 로그인도 바꿨습니다') && imported.includes('API 키 1개 추가 · 2개 덮어씀 · 1개는 이미 같음') && imported.includes("API 키 'work'") && filePassword.value==='' && sent('transfer_import')[1].args.path.endsWith('.swx') && sent('transfer_import')[1].args.data===null);
    record('import-says-up-front-that-it-overwrites', importForm.textContent.includes('파일 것으로 덮어씁니다') && importForm.textContent.includes('파일에 없는 것은 그대로') && importForm.textContent.includes('다시 로그인해야 할 수 있습니다'));
    record('settings-on-this-pc-are-not-touched', JSON.stringify(Object.entries(localStorage).sort())===storedBefore);
    record('transfer-dialog-fits-the-window', inWindow(dialog) && document.documentElement.scrollWidth<=innerWidth);
    transferFiles = [];
    dialog.querySelectorAll('.transfer-tabs button')[0].click(); dialog.querySelectorAll('.transfer-tabs button')[1].click();
    await settle(()=>!importForm.querySelector('select').closest('label') || importForm.querySelector('select').closest('label').hidden);
    record('no-file-found-asks-to-choose-one', importForm.querySelector('select').closest('label').hidden && [...importForm.querySelectorAll('p')].some(p=>!p.hidden && p.textContent.includes('찾지 못했습니다')));
    dialog.querySelector('.api-close').click();
    await settle(()=>!document.querySelector('.api-dialog'));
    record('closing-leaves-no-dialog-behind', !document.querySelector('.api-dialog'));
  }
  async function checkAutoKey() {
    await ready('.profile-card');
    for(let i=0;i<4;i++) await delay();
    const rect = el => el.getBoundingClientRect();
    const header = provider => document.querySelector('.section-title .add-api-key[data-provider="'+provider+'"]');
    const accountAdd = provider => document.querySelector('.section-title .add-account[data-provider="'+provider+'"]');
    const openAuto = async provider => { header(provider).click(); return ready('.api-form'); };
    const keyInput = form => form.querySelector('input[name="api-key"]');
    const nameInput = form => form.querySelector('input[name="connection-name"]');
    const fill = (form, name='') => { keyInput(form).value='sk-dummy-test-key-123456'; nameInput(form).value=name; };
    const keysOf = (provider, account) => [...document.querySelectorAll('.profile-card[data-provider="'+provider+'"][data-name="'+account+'"] .api-card .card-name')].map(el=>el.textContent);
    const autoCalls = () => calls.filter(c=>c.cmd==='api_save_auto');
    const closeDialog = async () => { document.querySelector('.api-dialog .api-close').click(); await settle(()=>!document.querySelector('.api-dialog')); for(let i=0;i<6;i++) await delay(); };
    const stored = () => connections.size;

    record('provider-title-offers-key-add-beside-account-add', ['claude','codex'].every(p=>{ const api=header(p), account=accountAdd(p); return api && account && api.parentElement===account.parentElement && api.textContent==='API 키 추가' && rect(api).width>0 && rect(api).right<=rect(account).left+0.5; }));
    record('title-row-buttons-fit-without-overlap', ['claude','codex'].every(p=>{ const title=header(p).closest('.section-title'), t=rect(title), a=rect(header(p)), b=rect(accountAdd(p)), label=rect(title.querySelector('svg, img, .provider-icon') ?? title); return a.left>=t.left && b.right<=t.right+0.5 && Math.abs(a.top-b.top)<3 && a.left>label.left && title.scrollWidth<=title.clientWidth+1; }));
    record('account-cards-keep-their-own-key-add', document.querySelectorAll('.profile-card .account-api-add').length===3);

    // 1) Several accounts, and the key's organization names one of them: saved there, no question.
    autoOwner='claude-1';
    let before=stored();
    let form = await openAuto('claude');
    const dialog = document.querySelector('.api-dialog');
    record('key-only-dialog-asks-for-no-account', !dialog.querySelector('.api-owner') && dialog.querySelector('.api-auto-note')?.textContent.includes('Claude') && form.querySelector('.api-account-pick').hidden && keyInput(form).required && !nameInput(form).required && document.activeElement===keyInput(form) && rect(form.querySelector('.api-account-pick')).height===0);
    record('key-only-dialog-stays-inside-window', (()=>{ const r=rect(dialog); return r.left>=0 && r.right<=innerWidth && r.top>=0 && r.bottom<=innerHeight; })());
    fill(form);
    form.requestSubmit(form.querySelector('.api-save'));
    await ready('.api-saved');
    record('key-alone-is-filed-under-the-account-it-belongs-to', autoCalls().length===1 && autoCalls()[0].args.provider==='claude' && !autoCalls()[0].args.accountId && autoCalls()[0].args.name==='' && autoCalls()[0].args.key==='[REDACTED]' && !calls.some(c=>c.cmd==='api_save') && !calls.some(c=>c.cmd==='api_activate') && stored()===before+1 && connections.get('claude/api')?.account_id==='claude-1');
    const placed = document.querySelector('.api-dialog .api-placed');
    record('result-names-the-account-and-how-it-was-found', placed?.textContent.includes('personal@example.test') && placed.textContent.includes('조직의 이름') && !document.querySelector('.api-dialog .api-form') && !document.querySelector('.api-dialog input'));
    // Once the owner is known the top line names it, exactly like a dialog opened from that account's card.
    record('result-header-names-the-owner-account', document.querySelector('.api-dialog .api-owner')?.textContent==='Claude · personal@example.test' && !document.querySelector('.api-dialog .api-auto-note'));
    document.body.classList.add('privacy');
    record('privacy-mode-blurs-the-account-in-the-result', getComputedStyle(placed).filter.includes('blur'));
    document.body.classList.remove('privacy');
    await closeDialog();
    await settle(()=>keysOf('claude','personal').includes('api'));
    record('new-key-appears-under-that-account-only', keysOf('claude','personal').includes('api') && !keysOf('claude','work').includes('api'));

    // 2) Nobody is named: nothing is stored, the dialog asks once, and the typed key survives the question.
    autoOwner=null; before=stored();
    form = await openAuto('claude');
    fill(form,'team');
    form.requestSubmit(form.querySelector('.api-save'));
    const pick = form.querySelector('.api-account-pick'), select = pick.querySelector('select');
    await settle(()=>!pick.hidden);
    record('unknown-owner-stores-nothing-and-asks-for-the-account', !pick.hidden && rect(select).height>0 && stored()===before && form.isConnected && pick.textContent.includes('계정을 골라') && form.querySelector('.api-error').textContent==='' && document.activeElement===select && [...select.options].map(o=>o.value).join()===',claude-1,claude-2' && [...select.options].slice(1).map(o=>o.textContent).join()==='personal@example.test,work@example.test' && select.required);
    record('the-question-keeps-the-typed-key-and-name', keyInput(form).value==='sk-dummy-test-key-123456' && nameInput(form).value==='team' && !keyInput(form).readOnly && !form.querySelector('.api-save').disabled && keyInput(form).type==='password');
    record('saving-waits-for-an-account-choice', !form.checkValidity() && autoCalls().length===2);
    // WebKit draws a bare select as a small native button that ignores padding; the list must match the inputs.
    record('account-list-is-as-tall-as-the-inputs', rect(select).height>=34 && Math.abs(rect(select).width-rect(keyInput(form)).width)<1.5 && Math.abs(rect(select).left-rect(keyInput(form)).left)<1.5);
    record('account-question-stays-inside-window', (()=>{ const r=rect(document.querySelector('.api-dialog')); return r.left>=0 && r.right<=innerWidth && r.top>=0 && r.bottom<=innerHeight && rect(select).right<=r.right; })());
    select.value='claude-2'; select.dispatchEvent(new Event('change',{bubbles:true}));
    form.requestSubmit(form.querySelector('.api-save'));
    await ready('.api-saved');
    const chosen = document.querySelector('.api-dialog .api-placed');
    record('chosen-account-receives-the-key', autoCalls().length===3 && autoCalls()[2].args.accountId==='claude-2' && autoCalls()[2].args.name==='team' && autoCalls()[2].args.key==='[REDACTED]' && stored()===before+1 && chosen?.textContent.includes('work@example.test') && !chosen.textContent.includes('조직의 이름'));
    await closeDialog();
    await settle(()=>keysOf('claude','work').includes('team'));
    record('chosen-key-appears-under-the-chosen-account', keysOf('claude','work').includes('team') && !keysOf('claude','personal').includes('team'));

    // 3) One saved account: it owns the key without any question, and "save and use" applies it at once.
    before=stored();
    form = await openAuto('codex');
    record('single-account-dialog-is-the-same-key-only-form', document.querySelector('.api-dialog .api-auto-note')?.textContent.includes('Codex') && form.querySelector('.api-account-pick').hidden);
    fill(form);
    saveAndUse(form);
    await ready('.api-applied');
    const only = document.querySelector('.api-dialog .api-placed');
    record('lone-account-takes-the-key-and-can-use-it-at-once', autoCalls().length===4 && autoCalls()[3].args.provider==='codex' && !autoCalls()[3].args.accountId && stored()===before+1 && only?.textContent.includes('personal@example.test') && !only.textContent.includes('조직의 이름') && calls.some(c=>c.cmd==='api_activate' && c.args.provider==='codex' && c.args.name==='api') && activeApi.get('codex')==='api');
    await closeDialog();

    // 4) A refused save says why, stores nothing, and does not keep the key on screen.
    autoOwner='claude-2'; before=stored();
    form = await openAuto('claude');
    fill(form,'team');
    form.requestSubmit(form.querySelector('.api-save'));
    await settle(()=>form.querySelector('.api-error').textContent.includes('같은 이름'));
    record('refused-save-says-why-and-clears-the-key', form.isConnected && form.querySelector('.api-error').textContent.includes('같은 이름') && keyInput(form).value==='' && stored()===before && form.querySelector('.api-account-pick').hidden && form.querySelector('.api-finding').hidden);
    await closeDialog();
    // The harness script itself spells the fixture key, so look at the page without its scripts.
    const page = document.body.cloneNode(true); page.querySelectorAll('script').forEach(script=>script.remove());
    record('key-text-never-reaches-the-page-or-the-call-log', !page.outerHTML.includes('sk-dummy-test-key-123456') && ![...document.querySelectorAll('input, textarea')].some(field=>field.value.includes('sk-dummy-test-key-123456')) && !JSON.stringify(calls).includes('sk-dummy-test-key-123456'));
  }
  window.addEventListener('load', async () => {
    try {
      if(emptyAccounts) {
        await checkEmptyAccounts();
        record('no-browser-errors-empty-'+mode,errors.length===0);
        return;
      }
      if(preview === 'auto-key-ask') {
        // Screenshot state: the key-only dialog after the app could not tell the owner.
        await ready('.profile-card');
        document.querySelector('.section-title .add-api-key[data-provider="claude"]').click();
        const form = await ready('.api-form');
        form.querySelector('input[name="api-key"]').value='sk-dummy-test-key-123456';
        form.requestSubmit(form.querySelector('.api-save'));
        const pick = form.querySelector('.api-account-pick');
        await settle(()=>!pick.hidden);
        for(let i=0;i<4;i++) await delay();
        record('key-only-dialog-shows-the-account-question', !pick.hidden && errors.length===0);
        return;
      }
      if(preview === 'transfer' || preview === 'transfer-shot') {
        if(preview === 'transfer-shot') {
          await ready('.profile-card');
          document.querySelector('.transfer-row .transfer-open').click();
          await ready('.api-dialog.transfer-dialog');
          for(let i=0;i<4;i++) await delay();
          record('transfer-dialog-opens', errors.length===0);
          return;
        }
        await checkTransfer();
        record('no-browser-errors-transfer-'+mode,errors.length===0);
        return;
      }
      if(preview === 'auto-key') {
        await checkAutoKey();
        record('no-browser-errors-auto-key-'+mode,errors.length===0);
        return;
      }
      if(preview === 'toggle') {
        await checkToggle();
        record('no-browser-errors-toggle-'+mode,errors.length===0);
        return;
      }
      if(preview === 'edge-restore') {
        await checkEdgeRestore();
        record('no-browser-errors-edge-restore',errors.length===0);
        return;
      }
      if(preview === 'edge-drag') {
        await checkEdgeDrag();
        record('no-browser-errors-edge-drag',errors.length===0);
        return;
      }
      if(preview === 'api-folded') {
        await checkFolded();
        record('no-browser-errors-api-folded',errors.length===0);
        return;
      }
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
          record('selected-api-belongs-to-second-claude-account-'+mode, selected?.closest('.profile-card, .account-with-api')?.dataset.name==='work');
          record('selected-api-inside-window-'+mode, selected && box(selected).left>=0 && box(selected).right<=innerWidth);
          const credit=document.querySelector(mode==='normal'?'.api-active .api-balance-value':'.compact-api-active .api-balance-value');
          record('actual-credit-in-all-modes-'+mode,credit?.textContent.includes('78.45') && box(credit).width>0 && box(credit).right<=innerWidth);
          record('copied-credit-shows-source-and-time-'+mode,credit?.parentElement.dataset.state==='console' && credit.parentElement.querySelector('.api-balance-detail').textContent.includes('Console 확인'));
          if(mode!=='normal') {
            const link=credit.parentElement.querySelector('.api-billing-link'), r=box(link);
            const x=r.left+r.width/2, y=r.top+r.height/2;
            const hits=(calls.filter(c=>c.cmd==='set_hit_regions').at(-1)?.args.regions??[]).filter(({rect})=>x>=rect[0] && y>=rect[1] && x<rect[0]+rect[2] && y<rect[1]+rect[3]);
            // Same rule as the native hit test: where regions overlap, a plain control wins over a switch area.
            const winner=hits.find(hit=>hit.action===null) ?? hits[0];
            record('balance-link-native-hit-does-not-switch-subscription-'+mode,hits.length && winner.action===null);
            link.click();await delay();
            record('balance-link-opens-own-billing-'+mode,calls.some(c=>c.cmd==='api_open_billing'&&c.args.provider==='claude') && !calls.some(c=>c.cmd==='switch_profile'));
          }
        }
        if(preview==='registration' || preview==='result') {
          await openApi();
          const form=await ready('.api-form');
          if(preview==='result') {
            form.elements.namedItem('connection-name').value='demo'; form.elements.namedItem('api-key').value='sk-dummy-test-key-123456';
            saveAndUse(form); await ready('.api-applied'); await delay();
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
      document.querySelector('.add-account[data-provider="claude"]').click();await delay();await delay();await delay();
      record('provider-add-starts-account-login',calls.some(c=>c.cmd==='start_login'&&c.args.provider==='claude')&&!document.querySelector('.api-dialog'));
      record('key-registration-is-offered-per-account-and-per-provider',[...document.querySelectorAll('.account-api-add')].length===3 && [...document.querySelectorAll('.account-api-add')].every(button=>button.closest('.profile-card')) && ['claude','codex'].every(p=>document.querySelector('.section-title .add-api-key[data-provider="'+p+'"]')));
      await ready('.profile-card');
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
      record('header-buttons-add-accounts-and-keys',[...document.querySelectorAll('.add-account')].every(button=>button.textContent==='계정 추가') && [...document.querySelectorAll('.add-api-key')].length===2 && [...document.querySelectorAll('.add-api-key')].every(button=>button.textContent==='API 키 추가' && button.closest('.section-title')));
      record('no-empty-api-space',!document.querySelector('.api-panel, .api-card'));
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
      record('key-registration-names-exact-owner',document.querySelector('.api-owner').textContent==='Claude · work@example.test' && !form.querySelector('select, .api-providers'));
      form.querySelector('.api-console').click(); await delay();
      record('key-issuance-page-uses-selected-provider',calls.some(c=>c.cmd==='api_open_console' && c.args.provider==='claude'));
      document.getElementById('refresh').click(); await delay();
      record('refresh-preserves-api-form', form.isConnected && inputs[1].value==='sk-dummy-test-key-123456');
      document.getElementById('pin').click(); await delay();
      record('view-change-protects-api-input', !document.body.classList.contains('locked') && form.isConnected);
      saveAndUse(form); saveAndUse(form);
      await ready('.api-applied'); await delay();
      record('register-sent-once-to-correct-parent',calls.filter(c=>c.cmd==='api_save').length===1 && calls.find(c=>c.cmd==='api_save').args.accountId==='claude-2');
      record('secret-cleared-after-save',inputs[1].value==='');
      record('registration-automatically-checks-connection',document.querySelector('.api-check-result').dataset.state==='ready');
      record('registration-applies-api-without-command-copy',activeApi.get('claude')==='project'&&calls.filter(c=>c.cmd==='api_activate').length===1&&!document.querySelector('.api-launch'));
      record('api-activation-never-sends-key-back-through-ui',calls.filter(c=>c.cmd==='api_activate').every(c=>!('key' in c.args))&&!calls.some(c=>c.cmd==='api_launch_command'));
      const afterSaveDrag=dragCount();mouseDown(document.querySelector('.api-dialog-head h2'));await delay();
      record('saved-result-remains-draggable',dragCount()===afterSaveDrag+1);
      document.querySelector('.api-close').click();
      let api=await ready('.api-card');
      record('registered-api-immediately-visible',box(api).height>0 && !api.closest('.api-panel').hidden);
      record('child-key-only-in-own-account',api.closest('.profile-card')?.dataset.name==='work'&&!document.querySelector('.profile-card[data-name="personal"] .api-card'));
      record('parent-offers-subscription-while-child-api-is-active',api.closest('.profile-card').querySelector('.profile-switch').textContent==='구독 사용');
      const fold=api.closest('.account-apis').querySelector('.account-api-toggle');
      fold.click();record('account-key-list-can-collapse',fold.getAttribute('aria-expanded')==='false'&&api.closest('.api-panel').hidden);
      fold.click();
      record('verification-persists-after-render',api.querySelector('.api-status').dataset.state==='ready');
      const shown = () => api.querySelector('.api-balance-value').textContent;
      const checkBalance = async () => { api.querySelector('.api-check').click(); await delay(); await delay(); };
      record('api-card-shows-organization-prepaid-balance',shown().includes('78.45') && api.textContent.includes('조직 선불 잔액') && !api.querySelector('[role="progressbar"]'));
      record('registration-and-card-no-longer-offer-budget-controls',!form.querySelector('[type="number"]') && !api.querySelector('.api-usage-settings'));
      balanceIssue='login_required'; await checkBalance();
      record('billing-org-visible-without-disclosing-it-in-a-tooltip',api.querySelector('.api-balance-org').textContent.includes('Demo Studio') && !api.querySelector('.api-balance').title.includes('Demo Studio'));
      record('missing-billing-login-clears-old-value',shown()==='—' && api.textContent.includes('키를 발급한 Claude Console에서 확인'));
      api.querySelector('.api-billing-link').click(); await delay();
      record('missing-login-opens-provider-billing',calls.some(c=>c.cmd==='api_open_billing'&&c.args.provider==='claude'));
      balanceIssue='forbidden'; await checkBalance();
      record('denied-balance-is-never-zero',shown()==='—' && api.textContent.includes('권한 없음'));
      api.querySelector('.api-credit-import').click();
      const pasted=await ready('.api-credit-paste');
      pasted.value='Other Studio\n조직 크레딧\nUS$12.34'; pasted.dispatchEvent(new Event('input')); await delay();
      record('wrong-organization-dashboard-rejected',document.querySelector('.api-error')?.textContent.includes('조직 이름') && pasted.value==='' && !calls.some(c=>c.cmd==='api_import_console_credit'));
      pasted.value='Demo Studio\n조직 크레딧\nUS$12.34\n이번 달 지출\nUS$1.25\nPrivate footer fixture'; pasted.dispatchEvent(new Event('input')); await delay(); await delay();
      api=await ready('.api-card');
      record('dashboard-credit-import-labeled-and-dated',shown().includes('12.34') && api.querySelector('.api-balance').dataset.state==='console' && api.querySelector('.api-balance-detail').textContent.includes('Console 확인'));
      const imported=calls.find(c=>c.cmd==='api_import_console_credit')?.args;
      record('clipboard-and-key-list-stay-out-of-native-command',imported?.reading.amount_minor===1234 && Object.keys(imported.reading).sort().join(',')==='amount_minor,import_ref,organization' && !JSON.stringify(imported).includes('Private footer'));
      balanceIssue=null;balanceMinor=0; await checkBalance();
      record('confirmed-zero-is-visible',shown().includes('0.00') && api.querySelector('.api-balance').dataset.state==='ready');
      balanceMinor=500;balanceCurrency='JPY';await checkBalance();
      record('yen-credit-uses-zero-decimal-currency',shown().includes('500') && !shown().includes('.00'));
      balanceCurrency='USD';balanceMinor=7845;
      document.getElementById('refresh').click(); await delay(); await delay(); api=await ready('.api-card');
      record('toolbar-refresh-requests-current-balance',calls.some(c=>c.cmd==='api_balance'&&c.args.name==='project'&&c.args.force===true) && shown().includes('78.45'));

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
      saveAndUse(form); await ready('.api-applied'); await delay();
      record('replacement-is-explicit',calls.filter(c=>c.cmd==='api_save').at(-1).args.replace===true);
      document.querySelector('.api-close').click(); await delay();
      await openApi();form=await ready('.api-form');inputs=form.querySelectorAll('input');inputs[0].value='project';inputs[1].value='sk-dummy-test-key-123456';
      form.requestSubmit(); await delay();
      record('duplicate-does-not-silently-replace-key',form.isConnected && form.querySelector('.api-error').textContent.includes('같은 이름'));
      record('secret-cleared-on-save-error',inputs[1].value==='');
      document.querySelector('.api-close').click(); await delay();
      activationFailure=true;
      await openApi();form=await ready('.api-form');inputs=form.querySelectorAll('input');inputs[0].value='apply-retry';inputs[1].value='sk-dummy-test-key-123456';saveAndUse(form);await ready('.api-apply');
      record('activation-failure-keeps-saved-key-and-offers-retry',connections.has('claude/apply-retry')&&inputs[1].value===''&&!document.querySelector('.api-applied'));
      activationFailure=false;document.querySelector('.api-apply').click();await ready('.api-applied');
      record('activation-retry-uses-saved-key',activeApi.get('claude')==='apply-retry'&&!document.querySelector('.api-apply'));
      document.querySelector('.api-close').click();await delay();
      // A network check may finish after the user closes registration, without freezing the app.
      checkFailure=true;holdCheck=true;
      await openApi('codex');form=await ready('.api-form');inputs=form.querySelectorAll('input');inputs[0].value='secondary';inputs[1].value='sk-dummy-test-key-123456';saveAndUse(form);
      await ready('.api-applied');await delay();document.querySelector('.api-close').click();await delay();
      record('slow-network-check-does-not-trap-dialog',!document.querySelector('.api-dialog'));
      releaseCheck();holdCheck=false;await delay();await delay();
      record('failed-check-persists-after-dialog-close',document.querySelector('.api-status[data-state="invalid_key"]')?.textContent.includes('키 교체'));
      const openaiCard = [...document.querySelectorAll('.api-card')].find(el=>el.textContent.includes('secondary'));
      record('openai-credit-opens-billing-without-inventing-amounts',openaiCard?.querySelector('.api-balance-value')?.textContent==='—' && openaiCard.textContent.includes('결제 화면에서 확인') && !calls.some(c=>c.cmd==='api_balance' && c.args.provider==='codex'));
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
      const activatedBefore=calls.filter(c=>c.cmd==='api_activate').length;
      form=await openApi('claude','personal');inputs=form.querySelectorAll('input');
      inputs[0].value='apply-retry';inputs[1].value='sk-dummy-test-key-123456';form.requestSubmit();
      await ready('.api-saved');await delay();document.querySelector('.api-close').click();await delay();
      record('save-without-use-keeps-current-selection',calls.filter(c=>c.cmd==='api_activate').length===activatedBefore && activeApi.get('claude')==='apply-retry');
      const duplicateLabel=await ready('.profile-card[data-name="personal"] .api-card');
      record('two-accounts-can-display-the-same-key-name',duplicateLabel.querySelector('.card-name').textContent==='apply-retry' && !duplicateLabel.classList.contains('api-active') && document.querySelector('.profile-card[data-name="work"] .api-card.api-active .card-name')?.textContent==='apply-retry');
      duplicateLabel.querySelector('.api-use').click();await delay();await delay();
      record('use-targets-stored-key-of-the-clicked-account',activeApi.get('claude')==='apply-retry-2');
      connections.set('claude/imported',{provider:'claude',name:'imported',check_state:'ready'});
      emit('visibility-changed');const recovery=await ready('.api-recovery');
      recovery.open=true;
      const owner=recovery.querySelector('select'), attach=recovery.querySelector('.api-attach');
      record('legacy-key-awaits-user-picked-account',owner.value==='' && attach.disabled && ![...document.querySelectorAll('.profile-card .api-card')].some(el=>el.textContent.includes('imported')));
      owner.value='claude-2';owner.dispatchEvent(new Event('change'));attach.click();await delay();await delay();
      record('legacy-key-links-to-chosen-account-without-switching',calls.some(c=>c.cmd==='api_attach' && c.args.accountId==='claude-2') && !document.querySelector('.api-recovery') && document.querySelector('.profile-card[data-name="work"] .api-panel').textContent.includes('imported') && activeApi.get('claude')==='apply-retry-2');
      // Show no AI sections when both are disabled, including their API managers.
      visibility={...visibility,claude:false,codex:false};emit('visibility-changed');await delay();await delay();
      record('hidden-ai-leaves-no-provider-panels',!document.querySelector('#app .profile-actions, #app .api-panel'));
      visibility={...visibility,claude:true,codex:true};emit('visibility-changed');await ready('.card');await delay();
      record('no-horizontal-overflow',document.querySelector('#app').scrollWidth<=420);
      codexBlocked=true; emit('visibility-changed'); await delay(); await delay();
      record('blocked-account-store-preserves-existing-key-recovery',document.querySelector('.api-recovery')?.textContent.includes('secondary') && document.querySelector('#app').textContent.includes('파일 외'));
      codexBlocked=false; emit('visibility-changed'); await delay(); await delay();
      record('api-selection-removes-subscription-session-note',!document.querySelector('.codex-session-note'));
      record('no-browser-errors',errors.length===0);
    } catch(e) { errors.push(String(e)); record('completed',false); } finally {
    const output=document.createElement('pre');output.id='ui-test-result';output.hidden=true;
    output.textContent=JSON.stringify({checks,errors,mode,width:document.querySelector('.shell').getBoundingClientRect().width,height:document.querySelector('.shell').getBoundingClientRect().height,emptyLayout});document.body.append(output);
    window.webkit?.messageHandlers?.report?.postMessage(output.textContent);
    }
  });
}
const allAt=process.argv.indexOf('--all-modes');
const allDir=allAt>=0 ? path.resolve(process.argv[allAt+1]) : null;
const outputAt=process.argv.indexOf('--output');
async function chromium(entry, width, screenshot, profile, boardHeight) {
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
    await send('Emulation.setDeviceMetricsOverride',{width,height:boardHeight ?? 1100,deviceScaleFactor:1,mobile:false});
    await send('Page.navigate',{url:pathToFileURL(entry).href});
    if (boardHeight) {
      for (let i=0; i<160; i++) {
        const ready=await send('Runtime.evaluate',{expression:"document.readyState==='complete' && document.images.length>0 && [...document.images].every(im=>im.complete && im.naturalWidth>0)",returnByValue:true});
        if (ready.result.value) {
          const shot=await send('Page.captureScreenshot',{format:'png'});
          fs.writeFileSync(screenshot,Buffer.from(shot.data,'base64')); return;
        }
        await sleep(50);
      }
      throw new Error('Preview images did not load');
    }
    let report;
    for(let i=0;i<600;i++) {
      const value=await send('Runtime.evaluate',{expression:"document.querySelector('#ui-test-result')?.textContent",returnByValue:true});
      if(value.result.value){report=JSON.parse(value.result.value);break;}await sleep(50);
    }
    if(!report) {
      // Say how far the scenario got: the checks recorded so far and any page errors.
      const debug=await send('Runtime.evaluate',{expression:"JSON.stringify(window.__uiDebug ?? null)",returnByValue:true}).catch(()=>null);
      throw new Error('No UI result: '+(debug?.result?.value ?? 'page gave no state'));
    }
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
  const width={normal:420,minimal:120,edge:80}[mode];
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
  // --only <mode>:<scenario> runs one scenario and prints its report (for working on a single flow).
  const onlyAt=process.argv.indexOf('--only');
  if(onlyAt>=0) {
    const [mode,scene]=process.argv[onlyAt+1].split(':');
    const report=await run(mode,scene==='true'?true:scene==='false'?false:scene,null);
    console.log(JSON.stringify(report,null,2));process.exitCode=report.checks.every(c=>c.passed)?0:1;
    throw Object.assign(new Error('only'),{only:true});
  }
  const screenshotAt=process.argv.indexOf('--screenshot');
  const reports=[await run('normal',false,screenshotAt>=0?path.resolve(process.argv[screenshotAt+1]):null)];
  if(useWebKit || !allDir) {
    for(const mode of ['minimal','edge']) reports.push(await run(mode,true,null));
  }
  if(allDir && !useWebKit) {
    fs.mkdirSync(allDir,{recursive:true});
    const labels={normal:['01 일반','계정 · API 키 · 전체 도구'],minimal:['02 미니멀','사용량만 작게'],edge:['03 엣지','벽 붙임 · 펼친 상태']};
    // One figure per screenshot on the comparison board: each mode, then the "one more = API" cue.
    const shots=[];
    const shoot=async(mode,preview,file,label)=>{const report=await run(mode,preview,path.join(allDir,file));reports.push(report);shots.push({report,file,label});};
    for(const mode of ['normal','minimal','edge']) await shoot(mode,true,`mode-${mode}.png`,labels[mode]);
    const registration=await run('normal','registration',path.join(allDir,'api-registration.png'));
    const result=await run('normal','result',path.join(allDir,'api-result.png'));
    const noAI=await run('minimal','no-ai',path.join(allDir,'mode-no-ai.png'));
    // Account in use with a stored key: the "API ›" cue before the second click (#186).
    for(const mode of ['minimal','edge']) await shoot(mode,'api-ready',`mode-${mode}-api-next.png`,[labels[mode][0],'한 번 더 = API 키']);
    fs.writeFileSync(path.join(allDir,'api-preview-layout.json'),JSON.stringify({registration:registration.dialog,result:result.dialog},null,2)+'\n');
    const figures=shots.map(({report:r,file,label})=>`<figure style="width:${r.width}px"><figcaption><b>${label[0]}</b><span>${label[1]}</span></figcaption><div style="height:${Math.ceil(r.height)}px"><img src="${file}" style="width:${r.width}px"></div></figure>`).join('');
    const board=`<!doctype html><html lang="ko"><meta charset="utf-8"><title>switcher · 세 가지 모드</title><style>*{box-sizing:border-box}body{margin:0;background:#e8ecec;color:#182024;font-family:-apple-system,BlinkMacSystemFont,sans-serif;padding:32px 38px}header{display:flex;align-items:baseline;gap:16px;margin-bottom:28px}h1{font-size:26px;letter-spacing:-1px;margin:0}header span{font-size:12px;color:#566568}.views{display:flex;align-items:flex-start;gap:30px}figure{margin:0;flex:none}figcaption{height:56px}b{font-size:15px;display:block}figcaption span{font-size:10px;color:#566568;display:block;margin-top:6px;white-space:nowrap}figure div{overflow:hidden;border-radius:12px;box-shadow:0 12px 32px #17212221}img{display:block}</style><header><h1>switcher</h1><span>실제 프론트 화면 · 예시 계정 · 3개 모드</span></header><div class="views">${figures}</div></html>`;
    const boardPath=path.join(allDir,'view-modes.html');fs.writeFileSync(boardPath,board);
    const height=Math.ceil(Math.max(...shots.map(({report})=>report.height))+170);
    await chromium(boardPath,1100,path.join(allDir,'view-modes.png'),path.join(temp,'board'),height);
    const apiFigure=(label,file,rect)=>`<section class="api-step"><h3>${label}</h3><div class="crop" style="width:${rect.width}px;height:${rect.height}px"><img src="${file}" style="width:420px;transform:translate(${-rect.x}px,${-rect.y}px)"></div></section>`;
    const noAiFigure=`<figure style="width:150px"><figcaption><b>AI를 모두 숨겼을 때</b><span>SYSTEM과 도구만 남는다</span></figcaption><div style="width:120px"><img src="mode-no-ai.png" width="120"></div><p class="note">숨긴 계정의 빈 영역도 사라집니다.<br><br>계정 조작은 같은 위치에:<br>다시 연결 · 이름 변경 · 삭제</p></figure>`;
    const complete=`<!doctype html><html lang="ko"><meta charset="utf-8"><title>switcher · 전체 개선 화면</title><style>*{box-sizing:border-box}body{margin:0;background:#e8ecec;color:#182024;font-family:-apple-system,BlinkMacSystemFont,sans-serif;padding:32px 38px}header{margin-bottom:30px}h1{font-size:27px;letter-spacing:-1px;margin:0 0 7px}header p{font-size:12px;color:#566568;margin:0}.layout{display:flex;align-items:flex-start;gap:36px}.views{display:flex;align-items:flex-start;gap:24px}figure{margin:0;flex:none}figcaption{height:56px}figcaption b{font-size:15px;display:block}figcaption span{font-size:10px;color:#566568;display:block;margin-top:6px;white-space:nowrap}figure>div,.crop{overflow:hidden;border-radius:12px;box-shadow:0 12px 32px #17212221}img{display:block}.api-column{width:380px;flex:none;border-left:1px solid #cad4d4;padding-left:0}.api-step h3{font-size:13px;margin:0 0 12px}.api-step+.api-step{margin-top:24px}.note{font-size:11px;line-height:1.7;color:#566568;margin-top:20px}figure .note{width:150px}</style><header><h1>switcher · 전체 개선 화면</h1><p>예시 계정으로 검증한 실제 프론트 · 세 가지 모드 · API 키 등록부터 버튼 전환까지</p></header><div class="layout"><div class="views">${figures}${noAiFigure}</div><aside class="api-column">${apiFigure('API 01 · 키 등록', 'api-registration.png',registration.dialog)}${apiFigure('API 02 · 저장 후 바로 사용', 'api-result.png',result.dialog)}<p class="note">연결 확인·사용량은 검증용 예시 데이터입니다.<br>실제 키를 등록하면 해당 연결의 결과를 표시합니다.</p></aside></div></html>`;
    const completePath=path.join(allDir,'complete-preview.html');fs.writeFileSync(completePath,complete);
    await chromium(completePath,1600,path.join(allDir,'complete-preview.png'),path.join(temp,'complete-board'),1200);
    reports.push(registration,result,noAI);

  }
  for(const mode of ['normal','minimal','edge']) reports.push(await run(mode,'empty-accounts',null));
  for(const mode of ['minimal','edge']) reports.push(await run(mode,'toggle',null));
  reports.push(await run('normal','auto-key',null));
  reports.push(await run('normal','transfer',null));
  if(allDir && !useWebKit) reports.push(await run('normal','transfer-shot',path.join(allDir,'transfer.png')));
  if(allDir && !useWebKit) reports.push(await run('normal','auto-key-ask',path.join(allDir,'api-key-only.png')));
  reports.push(await run('edge','api-folded',allDir && !useWebKit ? path.join(allDir,'mode-edge-api-folded.png') : null));
  reports.push(await run('edge','edge-drag',null));
  reports.push(await run('edge','edge-restore',null));
  const result={reports,passed:reports.reduce((n,r)=>n+r.checks.filter(c=>c.passed).length,0),failed:reports.reduce((n,r)=>n+r.checks.filter(c=>!c.passed).length,0)};
  if(outputAt>=0) fs.writeFileSync(process.argv[outputAt+1],JSON.stringify(result,null,2)+'\n');
  console.log(JSON.stringify(result,null,2));process.exitCode=result.failed?1:0;
} catch(error) { if(!error?.only) throw error; } finally { fs.rmSync(temp,{recursive:true,force:true,maxRetries:5,retryDelay:200}); }
