import type { Entry, Project, SessionDetail, SessionMeta } from '../types';

const cwd = '/demo/research-toolkit';
const now = Math.floor(Date.now() / 1000);
export const project: Project = {key:'demo',cwd,sessionCount:3,subagentCount:2,updatedAt:now,runningCount:1,rmuxCount:1,termCount:0};
export const sessions: SessionMeta[] = [
  {id:'demo-main',name:'Prepare a research workflow',isSubagent:false,taskId:null,parentSessionId:null,parentSessionPath:null,running:false,sleeping:false,interrupted:false,inRmux:false,rmuxAttached:false},
  {id:'demo-child',name:'Review the result schema',isSubagent:true,taskId:'demo-review',parentSessionId:'demo-main',parentSessionPath:'/demo/demo-main.jsonl',running:false,sleeping:false,interrupted:false,inRmux:false,rmuxAttached:false},
  {id:'demo-worker',name:'Document the task runner',isSubagent:true,taskId:'demo-docs',parentSessionId:'demo-main',parentSessionPath:'/demo/demo-main.jsonl',running:true,sleeping:false,interrupted:false,inRmux:true,rmuxAttached:false},
].map((s,i)=>({...s,path:`/demo/${s.id}.jsonl`,cwd,firstMessage:s.name,lastMessage:'Synthetic preview session',createdIso:new Date((now-600)*1000).toISOString(),createdAt:now-600,updatedAt:now-i*60,model:'demo-model',messageCount:3,rmuxTarget:s.inRmux?'demo-pane':null,rmuxDead:false,termAlive:false,size:1024}));

function entry(id:string, parentId:string|null, role:string, text:string): Entry {
  return {kind:'message',id,parentId,ts:new Date(now*1000).toISOString(),role,content:[{kind:'text',text}],model:role==='assistant'?'demo-model':null,toolName:null,toolCallId:null,isError:null,summary:null,name:null,label:null};
}
export function detailFor(session: SessionMeta): SessionDetail {
  const entries = session.isSubagent ? [
    entry('c1',null,'user',session.name ?? 'Example task'),
    entry('c2','c1','assistant',session.running ? 'This synthetic worker is shown as running so you can inspect its runtime badge. No process is started by this preview.' : 'Reviewed the sample schema. The task ID, execution status and result fields are documented. This is synthetic example content.'),
  ] : [
    entry('m1',null,'user','Help me prepare a reproducible research workflow. Keep task status separate from research results.'),
    {...entry('m2','m1','assistant','I will review the result schema and document the task runner in two separate tasks.'),content:[{kind:'thinking' as const,thinking:'Synthetic example of a collapsible thinking block.'},{kind:'text' as const,text:'I will review the result schema and document the task runner in two separate tasks.'},{kind:'toolCall' as const,id:'call-demo',name:'subagent',arguments:'{"agent":"reviewer","task":"Review the sample schema"}'}]},
    {...entry('m3','m2','toolResult','Sample schema review complete: task ID, status and results are separate fields.'),toolName:'subagent',toolCallId:'call-demo',isError:false},
    entry('m4','m3','assistant','## Workflow outline\n\n1. Define the research task and input data.\n2. Track execution status independently.\n3. Store results with reproducible settings.\n4. Review results before continuing.\n\nChoose a child session in the sidebar to inspect its conversation.'),
  ];
  return {id:session.id,cwd,createdIso:session.createdIso,path:session.path,taskId:session.taskId,stats:{tokenCount:1200,messageCount:entries.length,model:'demo-model',provider:'synthetic',thinkingLevel:null,contextTokens:1200,contextLimit:32000,costTotal:0},entries,active:entries.map((_,i)=>i),size:1024,updatedAt:now};
}
