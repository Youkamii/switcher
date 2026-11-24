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
  const transferText = 'SWX2.1200.' + 'QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo0'.repeat(34).slice(0,1200);
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
      if(cmd === 'transfer_export') return {text:transferText, chars:transferText.length, accounts:3, expires_at:Math.floor(Date.now()/1000)+600};
      if(cmd === 'transfer_save') return {path:'/Users/fixture/Downloads/switcher-transfer-20261123-101500.swx', name:'switcher-transfer-20261123-101500.swx'};
      if(cmd === 'transfer_candidates') return transferFiles;
      if(cmd === 'transfer_reveal') return null;
      if(cmd === 'transfer_import') { if(args.password !== 'right-password') throw '비밀번호가 다르거나 내용이 손상됐습니다'; return {accounts_added:2, accounts_replaced:1, live_replaced:1, rejected:0, problems:["계정 'work': 이름을 쓸 수 없습니다"], source_os:'windows'}; }
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
  // Widget modes switch accounts and nothing else: a click (edge) or a native double-click (minimal) on
  // another account selects it; the account in use is not a target. There is no API key state (#191).
  async function checkSwitch() {
    const edge = mode === 'edge';
    const card = name => document.querySelector((edge?'.edge-account':'.compact-card.prov-claude')+'[data-name="'+name+'"]'+(edge?'[data-provider="claude"]':''));
    const activeCard = () => document.querySelector(edge?'.edge-account.active[data-provider="claude"]':'.compact-card.active.prov-claude');
    const regions = () => calls.filter(c=>c.cmd==='set_hit_regions').at(-1)?.args.regions ?? [];
    const hitsAt = el => { const r=el.getBoundingClientRect(), x=r.left+r.width/2, y=r.top+r.height/2; return regions().filter(({rect})=>x>=rect[0]&&y>=rect[1]&&x<=rect[0]+rect[2]&&y<=rect[1]+rect[3]); };
    const click = el => el.dispatchEvent(new MouseEvent('click',{bubbles:true,cancelable:true,detail:1}));
    const switches = () => calls.filter(c=>c.cmd==='switch_profile');
    await ready(edge?'.edge-account':'.compact-card');
    if(edge) { emit('edge-hover',true); await ready('body.edge-out'); }
    await settle(()=>activeCard()?.querySelector(edge?'.edge-col':'.compact-row'));
    for(let i=0;i<6;i++) await delay();
    record('no-api-marks-or-calls-in-widget-mode-'+mode, !document.querySelector('.api-next, .api-ready, .api-live, .owns-api, .account-with-api, .compact-api-active, .edge-gauge-api') && regions().every(region=>!('api' in region)) && !calls.some(c=>c.cmd.startsWith('api_')));
    if(edge) {
      // The account in use is not a click target; another account is.
      click(activeCard()); for(let i=0;i<4;i++) await delay();
      record('press-on-the-account-in-use-does-nothing', switches().length===0 && activeCard()?.dataset.name==='personal');
      const other = hitsAt(card('work'));
      record('edge-other-account-is-a-hover-target-before-the-whole-panel', other.length>=2 && other[0].action===null);
      click(card('work')); await settle(()=>switches().length===1);
      await settle(()=>activeCard()?.dataset.name==='work');
      record('press-on-another-account-switches-to-it', switches().length===1 && switches()[0].args.provider==='claude' && switches()[0].args.name==='work' && activeCard()?.dataset.name==='work');
      await settle(()=>card('personal')?.classList.contains('switchable'));
      click(card('personal')); await settle(()=>switches().length===2);
      await settle(()=>activeCard()?.dataset.name==='personal');
      record('press-on-the-first-account-switches-back', switches().length===2 && switches()[1].args.name==='personal' && activeCard()?.dataset.name==='personal');
    } else {
      record('minimal-account-in-use-reports-no-switch-region', hitsAt(activeCard()).every(hit=>hit.action===null));
      const other = hitsAt(card('work'));
      record('minimal-other-account-reports-a-plain-switch', other.length===1 && other[0].action?.[0]==='claude' && other[0].action?.[1]==='work');
      // The native poller switches, then reports it: the mark moves at once.
      await window.__TAURI_INTERNALS__.invoke('switch_profile',{provider:'claude',name:'work'});
      emit('account-switched',{ok:true,provider:'claude',name:'work'});
      await settle(()=>activeCard()?.dataset.name==='work');
      record('native-switch-moves-the-active-mark', activeCard()?.dataset.name==='work');
    }
  }
  // Move to another PC (#189): locked text that is copied and pasted, or the same text as a file. Valid for 10 minutes.
  async function checkTransfer() {
    await ready('.profile-card');
    for(let i=0;i<4;i++) await delay();
    const open = document.querySelector('.transfer-row .transfer-open');
    const inWindow = el => { const r=el.getBoundingClientRect(); return r.width>0 && r.left>=0 && r.right<=innerWidth && r.top>=0 && r.bottom<=innerHeight; };
    const stored = () => JSON.stringify(Object.entries(localStorage).sort());
    const storedBefore = stored();
    record('normal-mode-offers-moving-to-another-pc', open?.textContent==='다른 PC로 옮기기' && open.getBoundingClientRect().width>0);
    open.click();
    const dialog = await ready('.api-dialog.transfer-dialog');
    const exportForm = dialog.querySelector('.transfer-export'), importForm = dialog.querySelector('.transfer-import');
    const [first, second] = exportForm.querySelectorAll('input[type="password"]');
    record('opens-on-export-with-two-password-fields', !exportForm.hidden && importForm.hidden && first && second && first.minLength===8 && inWindow(dialog) && inWindow(exportForm.querySelector('button.primary')));
    record('export-says-what-is-included-and-the-ten-minute-limit', exportForm.textContent.includes('구독 계정 로그인을 비밀번호로') && !exportForm.textContent.includes('API') && exportForm.textContent.includes('10분 안에만') && exportForm.textContent.includes('설정과 메모는 담지 않습니다'));
    const sent = cmd => calls.filter(c=>c.cmd===cmd);
    first.value='password-one'; second.value='password-two';
    exportForm.requestSubmit();
    for(let i=0;i<4;i++) await delay();
    record('mismatched-passwords-make-nothing', sent('transfer_export').length===0 && exportForm.querySelector('.api-error').textContent==='두 비밀번호가 다릅니다');
    second.value='password-one';
    exportForm.requestSubmit();
    await ready('.transfer-export .transfer-done');
    const exported = sent('transfer_export')[0];
    record('export-sends-only-the-password', sent('transfer_export').length===1 && exported.args.passwordLength===12 && Object.keys(exported.args).sort().join()==='password,passwordLength');
    const result = exportForm.querySelector('.transfer-result');
    const box = result.querySelector('textarea.transfer-text');
    const copy = result.querySelector('.transfer-copy'), save = result.querySelector('.transfer-save');
    record('result-shows-what-it-holds-its-length-and-when-it-expires', result.textContent.includes('계정 3개 · 글 1,210자') && /\d{1,2}:\d{2}까지, 10분 동안만 가져올 수 있습니다/.test(result.querySelector('.transfer-until').textContent) && first.value==='' && second.value==='');
    record('both-ways-out-are-offered-side-by-side', copy.textContent==='복사' && save.textContent==='파일로 저장' && inWindow(copy) && inWindow(save) && Math.abs(copy.getBoundingClientRect().top-save.getBoundingClientRect().top)<=1 && copy.getBoundingClientRect().right<=save.getBoundingClientRect().left);
    record('the-text-is-shown-read-only-inside-its-box', box.readOnly && box.value===transferText && inWindow(box) && box.scrollWidth<=box.clientWidth+1 && getComputedStyle(box).userSelect!=='none');
    // Copy: the clipboard gets the whole text.
    let copied=null; const nativeClipboard=Object.getOwnPropertyDescriptor(Navigator.prototype,'clipboard');
    Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async value=>{ copied=value; }}});
    copy.click();
    await settle(()=>result.querySelector('.transfer-note').textContent!=='');
    record('copy-puts-the-whole-text-on-the-clipboard', copied===transferText && result.querySelector('.transfer-note').textContent.includes('복사했습니다') && !result.querySelector('.transfer-note').classList.contains('transfer-warn'));
    // A webview that refuses clipboard access: the box is selected and copied the old way.
    let oldWay=null; const nativeExec=document.execCommand;
    Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async()=>{ throw new Error('denied'); }}});
    document.execCommand = command => { oldWay={command, all:document.activeElement===box && box.selectionStart===0 && box.selectionEnd===box.value.length}; return true; };
    result.querySelector('.transfer-note').textContent='';
    copy.click();
    await settle(()=>result.querySelector('.transfer-note').textContent!=='');
    record('copy-falls-back-to-selecting-the-box', oldWay?.command==='copy' && oldWay.all && result.querySelector('.transfer-note').textContent.includes('복사했습니다'));
    document.execCommand = () => false;
    result.querySelector('.transfer-note').textContent='';
    copy.click();
    await settle(()=>result.querySelector('.transfer-note').textContent!=='');
    record('a-failed-copy-says-so-and-points-at-the-text', result.querySelector('.transfer-note').textContent.includes('복사하지 못했습니다') && result.querySelector('.transfer-note').classList.contains('transfer-warn'));
    document.execCommand = nativeExec;
    if(nativeClipboard) delete navigator.clipboard;
    // File: the same text is saved to Downloads.
    save.click();
    await ready('.transfer-export .transfer-reveal');
    const note = result.querySelector('.transfer-note');
    record('the-same-text-is-saved-as-a-file', sent('transfer_save').length===1 && sent('transfer_save')[0].args.text===transferText && note.textContent.includes('다운로드 폴더에 저장했습니다') && note.textContent.includes('switcher-transfer-20261123-101500.swx') && !note.textContent.includes('/Users/fixture') && !note.classList.contains('transfer-warn') && inWindow(note.querySelector('.transfer-reveal')));
    note.querySelector('.transfer-reveal').click();
    await delay();
    record('the-file-can-be-shown-in-its-folder', sent('transfer_reveal').length===1 && sent('transfer_reveal')[0].args.path.endsWith('.swx'));
    record('export-result-fits-the-window', inWindow(dialog) && document.documentElement.scrollWidth<=innerWidth && dialog.scrollWidth<=dialog.clientWidth+1);

    // Import: paste box first, then files. Nothing found and nothing pasted is refused before any call.
    const tabsButtons = dialog.querySelectorAll('.transfer-tabs button');
    const realFiles = transferFiles; transferFiles = [];
    tabsButtons[1].click();
    for(let i=0;i<6;i++) await delay();
    const paste = importForm.querySelector('textarea.transfer-paste'), picker = importForm.querySelector('input[type="file"]');
    const foundLabel = importForm.querySelector('select').closest('label');
    const filePassword = importForm.querySelector('input[type="password"]');
    record('import-offers-paste-first-and-a-file-as-well', exportForm.hidden && !importForm.hidden && paste && document.activeElement===paste && paste.placeholder.includes('붙여넣으세요') && picker.accept==='.swx' && foundLabel.hidden && paste.getBoundingClientRect().top<picker.getBoundingClientRect().top && inWindow(importForm.querySelector('button.primary')));
    record('import-says-up-front-that-it-overwrites', importForm.textContent.includes('받은 것으로 덮어씁니다') && importForm.textContent.includes('받은 것에 없는 계정은 그대로') && !importForm.textContent.includes('API') && importForm.textContent.includes('다시 로그인해야 할 수 있습니다'));
    filePassword.value='right-password';
    importForm.requestSubmit();
    for(let i=0;i<4;i++) await delay();
    record('nothing-to-import-is-refused', sent('transfer_import').length===0 && importForm.querySelector('.api-error').textContent==='복사한 글을 붙여넣거나 파일을 고르세요');
    // A pasted text with a wrong password: reported, nothing arrives.
    paste.value='  '+transferText+'\n';
    filePassword.value='wrong-password';
    importForm.requestSubmit();
    await settle(()=>importForm.querySelector('.api-error').textContent.includes('비밀번호'));
    record('a-wrong-password-is-reported-and-imports-nothing', importForm.querySelector('.api-error').textContent.includes('비밀번호가 다르거나') && !importForm.querySelector('.transfer-done') && paste.value.trim()===transferText);
    filePassword.value='right-password';
    importForm.requestSubmit();
    await ready('.transfer-import .transfer-done');
    const pastedCall = sent('transfer_import')[1];
    const imported = importForm.querySelector('.transfer-result').textContent;
    record('the-pasted-text-is-what-gets-imported', pastedCall.args.text===transferText && pastedCall.args.path===null && filePassword.value==='' && paste.value==='');
    record('import-result-says-what-arrived-and-what-was-overwritten', imported.includes('계정 2개 추가 · 1개 덮어씀') && imported.includes('지금 로그인된 계정 1개는 로그인도 바꿨습니다') && !imported.includes('API') && imported.includes("계정 'work'"));
    // A file found in Downloads is used when nothing is pasted.
    transferFiles = realFiles;
    tabsButtons[0].click(); tabsButtons[1].click();
    await settle(()=>importForm.querySelector('select').options.length===1 && !foundLabel.hidden);
    filePassword.value='right-password';
    importForm.requestSubmit();
    await settle(()=>sent('transfer_import').length===3);
    await ready('.transfer-import .transfer-done');
    const fileCall = sent('transfer_import')[2];
    record('a-found-file-is-imported-when-nothing-is-pasted', fileCall.args.text===null && fileCall.args.path==='/Users/fixture/Downloads/switcher-transfer-20261123-101500.swx' && inWindow(foundLabel));
    record('settings-on-this-pc-are-not-touched', stored()===storedBefore);
    record('import-fits-the-window', inWindow(dialog) && document.documentElement.scrollWidth<=innerWidth && dialog.scrollWidth<=dialog.clientWidth+1);
    dialog.querySelector('.api-close').click();
    await settle(()=>!document.querySelector('.api-dialog'));
    record('closing-leaves-no-dialog-behind', !document.querySelector('.api-dialog'));
  }
  window.addEventListener('load', async () => {
    try {
      if(emptyAccounts) {
        await checkEmptyAccounts();
        record('no-browser-errors-empty-'+mode,errors.length===0);
        return;
      }
      if(preview === 'transfer' || preview === 'transfer-shot') {
        if(preview === 'transfer-shot') {
          await ready('.profile-card');
          document.querySelector('.transfer-row .transfer-open').click();
          const form = (await ready('.api-dialog.transfer-dialog')).querySelector('.transfer-export');
          form.querySelectorAll('input[type="password"]').forEach(input=>{ input.value='password-one'; });
          form.requestSubmit();
          await ready('.transfer-export .transfer-done');
          for(let i=0;i<4;i++) await delay();
          record('transfer-dialog-shows-the-result', errors.length===0);
          return;
        }
        await checkTransfer();
        record('no-browser-errors-transfer-'+mode,errors.length===0);
        return;
      }
      if(preview === 'switch') {
        await checkSwitch();
        record('no-browser-errors-switch-'+mode,errors.length===0);
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
        record('no-api-key-ui-'+mode,!document.querySelector('.add-api-key, .account-api-add, .api-panel, .api-card, .api-next, .compact-api-active, .account-with-api, .edge-gauge-api') && !calls.some(c=>c.cmd.startsWith('api_')));
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
      record('no-api-key-ui',!document.querySelector('.add-api-key, .account-api-add, .api-panel, .api-card, .api-keys') && !calls.some(c=>c.cmd.startsWith('api_')));
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
      record('header-buttons-add-accounts',[...document.querySelectorAll('.add-account')].length===2 && [...document.querySelectorAll('.add-account')].every(button=>button.textContent==='계정 추가'));
      record('no-empty-api-space',!document.querySelector('.api-panel, .api-card'));
      // Show no AI sections when both are disabled.
      visibility={...visibility,claude:false,codex:false};emit('visibility-changed');await delay();await delay();
      record('hidden-ai-leaves-no-provider-panels',!document.querySelector('#app .profile-actions, #app .api-panel'));
      visibility={...visibility,claude:true,codex:true};emit('visibility-changed');await ready('.card');await delay();
      record('no-horizontal-overflow',document.querySelector('#app').scrollWidth<=420);
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
    // Chromium. Wait for its actual readiness signal: the port line in this file.
    // The file can exist while Chromium still holds it open (Windows reports EBUSY)
    // or before the port is written, so keep reading until the line is there.
    let port=NaN;
    for(let i=0;i<400 && !Number.isInteger(port);i++) {
      if(child.exitCode !== null) throw new Error('Chromium exited before readiness: '+child.exitCode);
      try {
        const first=fs.readFileSync(portFile,'utf8').split('\n')[0].trim();
        if(/^\d+$/.test(first)) port=Number(first);
      } catch(error) {
        if(!['ENOENT','EBUSY','EPERM','EACCES'].includes(error.code)) throw error;
      }
      if(!Number.isInteger(port)) await sleep(50);
    }
    if(!Number.isInteger(port)) throw new Error('Chromium did not become ready within 20 seconds');
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
    const labels={normal:['01 일반','계정 · 전체 도구'],minimal:['02 미니멀','사용량만 작게'],edge:['03 엣지','벽 붙임 · 펼친 상태']};
    // One figure per screenshot on the comparison board: each mode.
    const shots=[];
    const shoot=async(mode,preview,file,label)=>{const report=await run(mode,preview,path.join(allDir,file));reports.push(report);shots.push({report,file,label});};
    for(const mode of ['normal','minimal','edge']) await shoot(mode,true,`mode-${mode}.png`,labels[mode]);
    const noAI=await run('minimal','no-ai',path.join(allDir,'mode-no-ai.png'));
    const figures=shots.map(({report:r,file,label})=>`<figure style="width:${r.width}px"><figcaption><b>${label[0]}</b><span>${label[1]}</span></figcaption><div style="height:${Math.ceil(r.height)}px"><img src="${file}" style="width:${r.width}px"></div></figure>`).join('');
    const board=`<!doctype html><html lang="ko"><meta charset="utf-8"><title>switcher · 세 가지 모드</title><style>*{box-sizing:border-box}body{margin:0;background:#e8ecec;color:#182024;font-family:-apple-system,BlinkMacSystemFont,sans-serif;padding:32px 38px}header{display:flex;align-items:baseline;gap:16px;margin-bottom:28px}h1{font-size:26px;letter-spacing:-1px;margin:0}header span{font-size:12px;color:#566568}.views{display:flex;align-items:flex-start;gap:30px}figure{margin:0;flex:none}figcaption{height:56px}b{font-size:15px;display:block}figcaption span{font-size:10px;color:#566568;display:block;margin-top:6px;white-space:nowrap}figure div{overflow:hidden;border-radius:12px;box-shadow:0 12px 32px #17212221}img{display:block}</style><header><h1>switcher</h1><span>실제 프론트 화면 · 예시 계정 · 3개 모드</span></header><div class="views">${figures}</div></html>`;
    const boardPath=path.join(allDir,'view-modes.html');fs.writeFileSync(boardPath,board);
    const height=Math.ceil(Math.max(...shots.map(({report})=>report.height))+170);
    await chromium(boardPath,1100,path.join(allDir,'view-modes.png'),path.join(temp,'board'),height);
    reports.push(noAI);

  }
  for(const mode of ['normal','minimal','edge']) reports.push(await run(mode,'empty-accounts',null));
  for(const mode of ['minimal','edge']) reports.push(await run(mode,'switch',null));
  reports.push(await run('normal','transfer',null));
  if(allDir && !useWebKit) reports.push(await run('normal','transfer-shot',path.join(allDir,'transfer.png')));
  reports.push(await run('edge','edge-drag',null));
  reports.push(await run('edge','edge-restore',null));
  const result={reports,passed:reports.reduce((n,r)=>n+r.checks.filter(c=>c.passed).length,0),failed:reports.reduce((n,r)=>n+r.checks.filter(c=>!c.passed).length,0)};
  if(outputAt>=0) fs.writeFileSync(process.argv[outputAt+1],JSON.stringify(result,null,2)+'\n');
  console.log(JSON.stringify(result,null,2));process.exitCode=result.failed?1:0;
} catch(error) { if(!error?.only) throw error; } finally { fs.rmSync(temp,{recursive:true,force:true,maxRetries:5,retryDelay:200}); }
