import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { AiSessionSummary, AppConfig, ScanResult, Snapshot } from "../../src/bindings";
import type { CleanResult, HwSnapshot, FanHw, FanControlStatus } from "../../src/bindings";
import { emit } from "@tauri-apps/api/event";

// Only this test entry imports the mock. No native commands or real files are accessed.
const config: AppConfig = { schemaVersion: 1, general: { sampleIntervalMs: 1000, adaptiveInterval: false }, overlay: { items: [] }, network: { monitorMode: "all", monitorInterfaces: [], includeLoopback: false, includeVirtual: false }, history: { retainDays: 30 } };
let sessions: AiSessionSummary[] = Array.from({length: 10000}, (_, i) => ({ sessionId: `fixture-${i}`, title: `验收会话 ${String(i).padStart(5, '0')}`, sessionType: 'codex', source: i % 2 ? 'claude' : 'codex', workspaceHash: 'fixture-workspace', workspaceDirectory: 'C:/Synthetic/Fixture', messageCount: 80, fileSize: 1000, createdAt: Date.now() - i * 1000, modifiedAt: Date.now() - i * 1000 }));
let sessionWorkspaces = ['fixture-workspace'];
if (new URLSearchParams(location.search).has('session-sidebar')) {
  sessionWorkspaces = ['project-a', 'project-b', 'codex:C:/Synthetic/Empty', 'project-readonly'];
  sessions = sessions.slice(0, 30).map((s, i) => ({...s, workspaceHash:'project-a', workspaceDirectory:'C:/Synthetic/Work/SysPulse',
    title: i === 0 ? 'codex://threads/01a14a07-57f0-7ff0-adcd-58f5f2da1011改进硬件数据采集与会话管理的交互体验' : i === 1 ? '长标题'.repeat(45) + '尾部检索目标' : `项目会话 ${i}：检查请求、切换与分页`,
    modifiedAt:Date.now()-i*3600000 }));
  sessions.push({...sessions[0],sessionId:'project-b-1',workspaceHash:'project-b',workspaceDirectory:'D:/Synthetic/Archive/SysPulse',title:'归档项目的独立会话'});
  sessions.push({...sessions[0],sessionId:'@readonly/1',source:'antigravity-backup',workspaceHash:'project-readonly',workspaceDirectory:'C:/Synthetic/Backups',title:'只读备份记录'});
}
export const sessionFixture = {
  listCalls: 0, loadCalls: [] as Array<{sessionId:string;page:number}>, deleteCalls: 0,
  listDelayMs: 0, loadDelayMs: 0, failNextList: false, failNextLoad: false,
  get remaining() { return sessions.length; },
};
const scan: ScanResult = {scanId: 'fixture-scan', categories: [{id:'node-cache',name:'Node 缓存（测试）',description:'内存模拟数据',sizeBytes:1024,fileCount:2,riskLevel:'safe',defaultSelected:true,minAgeDays:7,paths:[{path:'C:/Synthetic/Fixture/cache',sizeBytes:1024,fileCount:2,matchedRule:'fixture',source:'fixture',volumeSerial:1,fileId:1}]}],totalSizeBytes:1024,totalFileCount:2,scannedAtMs:Date.now(),durationMs:100,scannedPaths:1,skippedPaths:0,ignoredPaths:0,hotspotCount:0,scoutWorkers:1,engineerWorkers:1,scoutTasks:1,engineerTasks:1,projectRoots:[],warnings:[],complete:true,cancelled:false};
const failConfig = new URLSearchParams(location.search).has('config-failure');
const backgroundCleanup = new URLSearchParams(location.search).has('cleanup-background');
let pendingScan: ((result: ScanResult) => void) | null = null;
let pendingClean: ((result: CleanResult) => void) | null = null;
let hardwareSnapshot: HwSnapshot | null = null;
let hardwareFanControl: FanControlStatus = {fuseHold:false,fuseReason:null,maxTempC:null,entries:[]};
export const hardwareFixture = {
  fanWrites: [] as Array<{fanId:string;mode:string;pwm?:number}>,
  fanWriteDelayMs: 150,
  failNextFanWrite: false,
  async publish(fans?: FanHw[]) {
    hardwareSnapshot = {
      timestampMs: Date.now(), cpu: { name: '模拟 CPU', packageTempC: 62, totalUsage: null,
        perCoreUsage: [null, 35], perCoreTempsC: [null, 64], frequencyMhz: 3500, powerW: 45 },
      memory: null, gpus: [], disks: [], motherboard: null,
      fans: fans ?? [{ id: '/fixture/fan/0', name: '模拟风扇', rpm: 900, pwmPercent: 35, controllable: true }],
    };
    await emit('hw:helper-status', { status: 'running', reason: null });
    await emit('hw:update', hardwareSnapshot);
  },
};
export const cleanupFixture = {
  directoryChoices: [] as string[],
  scanCalls: 0,
  cleanCalls: 0,
  async progress(scannedPaths: number) {
    await emit('cleanup:scan-progress', {stage:'scout',phase:'模拟深度扫描',scannedPaths,
      skippedPaths:0,ignoredPaths:0,scoutTasks:3,engineerTasks:2,currentPath:'C:/Synthetic/Fixture',done:false,cancelled:false});
  },
  finishScan(cancelled = false) {
    const resolve = pendingScan;
    pendingScan = null;
    resolve?.({...scan,scanId:`fixture-scan-${this.scanCalls}`,scannedAtMs:Date.now(),
      complete:!cancelled,cancelled});
  },
  finishClean() {
    const resolve = pendingClean;
    pendingClean = null;
    resolve?.({freedBytes:512,deletedFiles:1,errors:['C:/Synthetic/Fixture/cache/locked.tmp: 文件正在使用，已跳过']});
  },
};
let configAttempts = 0;
let authorized = false;
const oauthStatus = () => ({oauthSupported:true,authorized,expiresAt:Math.floor(Date.now()/1000)+3600,expiringSoon:false,expired:false,refreshFailed:false,needsReauth:false,message:null});
export const mcpFixture = {
  listCalls: [] as string[], overviewCalls: 0, oauthCalls: 0, toggleCalls: 0, readDelayMs: 0, failNextRead: false,
};
let mcpDisabled = false;
mockWindows('config');
mockIPC(async (cmd, payload) => {
  const args = (payload as { args?: Record<string, any> } | undefined)?.args ?? {};
  switch (cmd) {
    case 'get_config':
      if (failConfig && ++configAttempts <= 2) throw new Error('模拟配置读取失败');
      await new Promise(resolve => setTimeout(resolve, 300));
      return config;
    case 'get_realtime_stats': case 'plugin:updater|check': return null;
    case 'get_hw_snapshot': return hardwareSnapshot && Date.now() - hardwareSnapshot.timestampMs < 5000 ? hardwareSnapshot : null;
    case 'get_helper_status': return hardwareSnapshot ? 'running' : 'unavailable';
    case 'get_fan_control_state': return hardwareFanControl;
    case 'set_fan_manual': case 'set_fan_curve': case 'reset_fan_control': {
      const mode = cmd==='set_fan_curve' ? 'curve' : cmd==='set_fan_manual' ? 'manual' : 'bios';
      hardwareFixture.fanWrites.push({fanId:args.fanId,mode,pwm:args.pwm});
      await new Promise(r=>setTimeout(r,hardwareFixture.fanWriteDelayMs));
      if (hardwareFixture.failNextFanWrite) {hardwareFixture.failNextFanWrite=false;throw new Error('模拟风扇写入失败');}
      hardwareFanControl = {...hardwareFanControl,entries:hardwareFanControl.entries.filter(e=>e.fanId!==args.fanId)};
      if (mode!=='bios') hardwareFanControl.entries.push({fanId:args.fanId,mode,manualPwm:args.pwm??40,curve:args.curve??[],lastWrittenPwm:args.pwm??null,writeFailures:0});
      return hardwareFanControl;
    }
    case 'get_app_info': return {name:'SysPulse 隔离验收',version:'0.1.27',os:'Windows',arch:'x64',configDir:'C:/Synthetic',logsDir:'C:/Synthetic',dbFile:'C:/Synthetic/fixture.db'};
    case 'is_admin': return true;
    case 'autostart_is_enabled': return false;
    case 'query_traffic_history': return [];
    case 'ai_get_mcp_overview': mcpFixture.overviewCalls++; return {totalServers:1,enabledServers:1,clients:[{client:'codex',totalServers:1,enabledServers:1}]};
    case 'ai_get_mcp_servers':
      mcpFixture.listCalls.push(args.client);
      await new Promise(resolve => setTimeout(resolve, mcpFixture.readDelayMs));
      if (mcpFixture.failNextRead) { mcpFixture.failNextRead = false; throw new Error('模拟 MCP 读取失败'); }
      return args.client==='codex' ? [{name:'隔离 OAuth',client:'codex',type:'http',detail:'https://example.invalid/mcp',disabled:mcpDisabled}] : [];
    case 'ai_mcp_oauth_status': mcpFixture.oauthCalls++; return oauthStatus();
    case 'ai_toggle_mcp_server': mcpFixture.toggleCalls++; mcpDisabled = args.disabled; return null;
    case 'plugin:dialog|open': return cleanupFixture.directoryChoices.shift() ?? null;
    case 'ai_mcp_oauth_authorize': authorized=true; return null;
    case 'ai_mcp_oauth_refresh': return oauthStatus();
    case 'ai_mcp_oauth_revoke': authorized=false; return '已撤销模拟授权';
    case 'ai_list_session_tree': {
      sessionFixture.listCalls++;
      const result = {workspaces:[...sessionWorkspaces],sessionsByWorkspace:Object.fromEntries(sessionWorkspaces.map(hash=>[hash,sessions.filter(s=>s.workspaceHash===hash)]))};
      await new Promise(r=>setTimeout(r,sessionFixture.listDelayMs));
      if (sessionFixture.failNextList) { sessionFixture.failNextList=false; throw new Error('模拟会话列表读取失败'); }
      return result;
    }
    case 'ai_load_session': {
      sessionFixture.loadCalls.push({sessionId:args.sessionId,page:args.page});
      const summary = sessions.find(s=>s.sessionId===args.sessionId && s.workspaceHash===args.workspaceHash)!;
      await new Promise(r=>setTimeout(r, sessionFixture.loadDelayMs || (args.sessionId==='fixture-1' ? 300 : 20)));
      if (sessionFixture.failNextLoad) { sessionFixture.failNextLoad=false; throw new Error('模拟会话正文读取失败'); }
      return {sessionId:summary.sessionId,title:summary.title,sessionType:'codex',workspaceDirectory:summary.workspaceDirectory,conversationSummary:null,totalMessages:80,page:args.page,pageSize:args.pageSize,history:Array.from({length:Math.min(args.pageSize,80-(args.page-1)*args.pageSize)},(_,i)=>({message:{role:i%2?'assistant':'user',id:`${args.page}-${i}`,isHidden:false,content:[{type:'text',text:`${summary.title} 正文 ${(args.page-1)*args.pageSize+i+1}`}]}}))};
    }
    case 'ai_delete_session': sessionFixture.deleteCalls++; sessions = sessions.filter(s=>s.sessionId!==args.sessionId || s.workspaceHash!==args.workspaceHash); return null;
    case 'ai_delete_workspace': sessionFixture.deleteCalls++; sessions = sessions.filter(s=>s.workspaceHash!==args.workspaceHash); sessionWorkspaces=sessionWorkspaces.filter(hash=>hash!==args.workspaceHash); return null;
    case 'ai_refresh_session_cache': return null;
    case 'scan_cleanup':
      cleanupFixture.scanCalls++;
      return backgroundCleanup ? new Promise<ScanResult>(resolve => { pendingScan = resolve; }) : scan;
    case 'cancel_cleanup_scan': cleanupFixture.finishScan(true); return true;
    case 'clean_categories':
      cleanupFixture.cleanCalls++;
      return backgroundCleanup ? new Promise<CleanResult>(resolve => { pendingClean = resolve; })
        : {freedBytes:1024,deletedFiles:2,errors:[]};
    default: throw new Error(`隔离验收未实现命令: ${cmd}`);
  }
}, { shouldMockEvents: true });
const { useLiveStore } = await import('../../src/stores/liveStore');
for (let i = 0; i < 12; i++) {
  const snapshot: Snapshot = {timestampMs:Date.now()-12000+i*1000,cpu:{usagePercent:20+i,perCore:null,model:'Fixture CPU',physicalCores:4},memory:{totalBytes:8e9,usedBytes:4e9,usedPercent:50,swapTotalBytes:0,swapUsedBytes:0},network:{sampleIntervalMs:1000,interfaces:[],total:{luid:0,name:'Fixture',description:'模拟网卡',isUp:true,isPhysical:true,bytesSentTotal:1024,bytesRecvTotal:2048,acceptedBytesSentTotal:1024,acceptedBytesRecvTotal:2048,bytesSentPerSec:100+i*50,bytesRecvPerSec:200+i*100}}};
  useLiveStore.getState().push(snapshot);
}
await import('../../src/main');
