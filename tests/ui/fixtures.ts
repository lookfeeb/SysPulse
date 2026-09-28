import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { AiSessionSummary, AppConfig, ScanResult, Snapshot } from "../../src/bindings";

// Only this test entry imports the mock. No native commands or real files are accessed.
const config: AppConfig = { schemaVersion: 1, general: { sampleIntervalMs: 1000, adaptiveInterval: false }, overlay: { items: [] }, network: { monitorMode: "all", monitorInterfaces: [], includeLoopback: false, includeVirtual: false }, history: { retainDays: 30 } };
let sessions: AiSessionSummary[] = Array.from({length: 10000}, (_, i) => ({ sessionId: `fixture-${i}`, title: `验收会话 ${String(i).padStart(5, '0')}`, sessionType: 'codex', source: i % 2 ? 'claude' : 'codex', workspaceHash: 'fixture-workspace', workspaceDirectory: 'C:/Synthetic/Fixture', messageCount: 80, fileSize: 1000, createdAt: Date.now() - i * 1000, modifiedAt: Date.now() - i * 1000 }));
const scan: ScanResult = {scanId: 'fixture-scan', categories: [{id:'node-cache',name:'Node 缓存（测试）',description:'内存模拟数据',sizeBytes:1024,fileCount:2,riskLevel:'safe',defaultSelected:true,minAgeDays:7,paths:[{path:'C:/Synthetic/Fixture/cache',sizeBytes:1024,fileCount:2,matchedRule:'fixture',source:'fixture',volumeSerial:1,fileId:1}]}],totalSizeBytes:1024,totalFileCount:2,scannedAtMs:Date.now(),expiresAtMs:Date.now()+3600000,durationMs:100,scannedPaths:1,skippedPaths:0,ignoredPaths:0,hotspotCount:0,scoutWorkers:1,engineerWorkers:1,scoutTasks:1,engineerTasks:1,projectRoots:[],warnings:[],complete:true,cancelled:false};
const failConfig = new URLSearchParams(location.search).has('config-failure');
let configAttempts = 0;
let authorized = false;
const oauthStatus = () => ({oauthSupported:true,authorized,expiresAt:Math.floor(Date.now()/1000)+3600,expiringSoon:false,expired:false,refreshFailed:false,needsReauth:false,message:null});
mockWindows('config');
mockIPC(async (cmd, payload) => {
  const args = (payload as { args?: Record<string, any> } | undefined)?.args ?? {};
  switch (cmd) {
    case 'get_config':
      if (failConfig && ++configAttempts <= 2) throw new Error('模拟配置读取失败');
      await new Promise(resolve => setTimeout(resolve, 300));
      return config;
    case 'get_realtime_stats': case 'get_hw_snapshot': case 'plugin:updater|check': return null;
    case 'get_helper_status': return 'unavailable';
    case 'get_fan_control_state': return {fuseHold:false,fuseReason:null,maxTempC:null,entries:[]};
    case 'get_app_info': return {name:'SysPulse 隔离验收',version:'0.1.27',os:'Windows',arch:'x64',configDir:'C:/Synthetic',logsDir:'C:/Synthetic',dbFile:'C:/Synthetic/fixture.db'};
    case 'is_admin': return true;
    case 'autostart_is_enabled': return false;
    case 'query_traffic_history': return [];
    case 'ai_get_mcp_overview': return {totalServers:1,enabledServers:1,clients:[{client:'codex',totalServers:1,enabledServers:1}]};
    case 'ai_get_mcp_servers': return args.client==='codex' ? [{name:'隔离 OAuth',client:'codex',type:'http',detail:'https://example.invalid/mcp',disabled:false}] : [];
    case 'ai_mcp_oauth_status': return oauthStatus();
    case 'ai_mcp_oauth_authorize': authorized=true; return null;
    case 'ai_mcp_oauth_refresh': return oauthStatus();
    case 'ai_mcp_oauth_revoke': authorized=false; return '已撤销模拟授权';
    case 'ai_list_session_tree': return {workspaces:['fixture-workspace'],sessionsByWorkspace:{'fixture-workspace':sessions}};
    case 'ai_load_session': {
      const summary = sessions.find(s=>s.sessionId===args.sessionId)!;
      await new Promise(r=>setTimeout(r, args.sessionId==='fixture-1' ? 300 : 20));
      return {sessionId:summary.sessionId,title:summary.title,sessionType:'codex',workspaceDirectory:summary.workspaceDirectory,conversationSummary:null,totalMessages:80,page:args.page,pageSize:args.pageSize,history:Array.from({length:Math.min(args.pageSize,80-(args.page-1)*args.pageSize)},(_,i)=>({message:{role:i%2?'assistant':'user',id:`${args.page}-${i}`,isHidden:false,content:[{type:'text',text:`${summary.title} 正文 ${(args.page-1)*args.pageSize+i+1}`}]}}))};
    }
    case 'ai_delete_session': sessions = sessions.filter(s=>s.sessionId!==args.sessionId); return null;
    case 'ai_refresh_session_cache': return null;
    case 'scan_cleanup': return scan;
    case 'clean_categories': return {freedBytes:1024,deletedFiles:2,errors:[]};
    default: throw new Error(`隔离验收未实现命令: ${cmd}`);
  }
}, { shouldMockEvents: true });
const { useLiveStore } = await import('../../src/stores/liveStore');
for (let i = 0; i < 12; i++) {
  const snapshot: Snapshot = {timestampMs:Date.now()-12000+i*1000,cpu:{usagePercent:20+i,perCore:null,model:'Fixture CPU',physicalCores:4},memory:{totalBytes:8e9,usedBytes:4e9,usedPercent:50,swapTotalBytes:0,swapUsedBytes:0},network:{sampleIntervalMs:1000,interfaces:[],total:{luid:0,name:'Fixture',description:'模拟网卡',isUp:true,isPhysical:true,bytesSentTotal:1024,bytesRecvTotal:2048,acceptedBytesSentTotal:1024,acceptedBytesRecvTotal:2048,bytesSentPerSec:100+i*50,bytesRecvPerSec:200+i*100}}};
  useLiveStore.getState().push(snapshot);
}
await import('../../src/main');
