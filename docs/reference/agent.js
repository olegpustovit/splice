"use strict";
/* ══ MCP surface: 19 tools over the same Store and registry. In the product this is splice_mcp (stdio shim → app socket). ══ */
const PROJECT_ID = "prj_field-notes";
const rid = () => "r"+Store.rev;
const revNum = v => typeof v==="number"?v:parseInt(String(v||"").replace(/^r/,""),10);
const JOBS = {};
const EXPORTED = new Set();
const EXISTED = p => EXPORTED.has(p);
function runJob(j){const tick=()=>{if(j.state!=="running")return;j.progress=Math.min(1,+(j.progress+.12).toFixed(2));if(j.progress>=1){j.state="done";j.output=j.settings.path;}else setTimeout(tick,350);if(document.body.dataset.view==="mcp")renderLab();};setTimeout(tick,350);}

function needAccess(level){
  const a=proj().access;
  if(a==="off")throw fail("PERMISSION_DENIED","Agent access is off for this project.","Ask the human to set Agent access to read or live in the Canvas inspector.");
  if(level==="live"&&a!=="live")throw fail("PERMISSION_DENIED","This project is read-only for agents.","Ask the human to set Agent access to live.");
}
function needProject(a){if(a.projectId&&a.projectId!==PROJECT_ID)throw fail("NOT_FOUND",`No project ${a.projectId}.`,`Open projects: ${PROJECT_ID}.`);}
function needSession(a){
  if(!a.sessionId)throw fail("NO_SESSION","Writes need a sessionId.","Call session.start first; pass its sessionId on every write.");
  const s=Store.sessions[a.sessionId];if(!s)throw fail("NOT_FOUND",`No session ${a.sessionId}.`,"session.start returns a sessionId.");
  if(!s.open)throw fail("NO_SESSION",`Session ${a.sessionId} has ended.`,"Start a new session.");
  return s;
}
function concise(p,x){const s=startOf(p,x);return {id:x.id,label:labelOf(p,x.id),from:s,to:s+lengthOf(p,x)};}
function full(p,x){
  const s=startOf(p,x), o={...copy(x),label:labelOf(p,x.id),resolved:{from:s,to:s+lengthOf(p,x)}};
  if(isClip(o)){o.media=mediaIds[x.asset]||x.asset;delete o.asset;o.children=childrenOf(p,x.id).map(c=>c.id);}
  if(isItem(o)&&o.asset){o.media=mediaIds[o.asset]||o.asset;delete o.asset;}
  if(isItem(x)&&x.kind==="caption"){o.text=cueText(x);o.tokens=cueTokens(p,x).map(t=>({id:t.id,text:t.text,from:t.f0,to:t.f1,...(t.emph?{emph:true}:{}),...(t.emoji?{emoji:t.emoji}:{}),...(t.hidden?{hidden:true}:{})}));}
  if(isClip(x)||isItem(x))o.effectsResolved=Object.fromEntries(Object.keys(EFFECT_TYPES).map(t=>[t,resolveFx(p,x,t)]).filter(([,r])=>r).map(([t,r])=>[t,{enabled:r.enabled,params:r.params,from:r.from,scopes:r.scopes}]));
  return o;
}

/* Caption cues as data or subtitle text. Times come from the timeline, so the export always matches the cut. */
function captionsOut(p,format,laneId){
  if(!["json","srt","vtt"].includes(format))throw fail("BAD_ARGS","captions must be json, srt or vtt.","Omit captions to page through words.");
  const cues=p.items.filter(i=>i.kind==="caption"&&(!laneId||i.lane===laneId)).sort((a,b)=>startOf(p,a)-startOf(p,b));
  const ts=(f,sep)=>{const ms=Math.round(f/FPS*1000),h=Math.floor(ms/36e5),m=Math.floor(ms/6e4)%60,s=Math.floor(ms/1e3)%60;return [h,m,s].map(v=>String(v).padStart(2,"0")).join(":")+sep+String(ms%1000).padStart(3,"0");};
  if(format==="json")return {revision:rid(),hash:TRANSCRIPT_HASH,lanes:lanesOfKind(p,"captions").map(l=>({id:l.id,name:l.name,style:resolveFx(p,l,"captions")?.params||null})),
    cues:cues.map(c=>({id:c.id,lane:c.lane,from:startOf(p,c),to:startOf(p,c)+c.length,text:cueText(c),words:cueTokens(p,c).map(t=>({id:t.id,text:t.text,from:t.f0,to:t.f1,...(t.emph?{emph:true}:{}),...(t.emoji?{emoji:t.emoji}:{}),...(t.hidden?{hidden:true}:{})})),...(c.effects?.length?{styleOverrides:c.effects.find(e=>e.type==="captions")?.params}:{})}))};
  const sep=format==="srt"?",":".";
  const body=cues.map((c,i)=>(format==="srt"?(i+1)+"\n":"")+ts(startOf(p,c),sep)+" --> "+ts(startOf(p,c)+c.length,sep)+"\n"+cueText(c)).join("\n\n");
  return {revision:rid(),format,cues:cues.length,text:(format==="vtt"?"WEBVTT\n\n":"")+body+"\n"};
}
const TOOLS = {
  "project.info":{group:"Perception",hint:"read",desc:"Opens a project by path (create:true makes a new one) or by projectId, and returns its metadata: fps, aspect, duration, lanes, media, access level, revision, transcript state.",
    example:()=>({path:"~/Movies/Field Notes.splice"}),
    run(a){if(a.path!==undefined&&!/\.splice\/?$/.test(a.path))throw fail("BAD_ARGS","path must name a .splice project package.","Pass path:\"…/Name.splice\", with create:true to make a new one.",{valid:["*.splice"]});needProject(a);needAccess("read");const p=proj();
      return {projectId:PROJECT_ID,revision:rid(),fps:FPS,aspect:p.aspect,durationFrames:endOf(p),access:p.access,
        lanes:[...p.lanes.map(l=>({id:l.id,name:l.name,kind:l.kind,items:p.items.filter(i=>i.lane===l.id).length})),{id:"spine",name:"Video",kind:"spine",items:p.clips.length}],
        media:Object.entries(mediaIds).map(([k,m])=>({id:m,name:assets[k].name,type:assets[k].type,frames:assets[k].frames>1e8?null:assets[k].frames})),
        transcript:{hash:TRANSCRIPT_HASH,words:WORDS.length,state:"ready"},
        effects:{project:p.effects||[],lan_spine:p.spine.effects||[],...Object.fromEntries(p.lanes.filter(l=>l.effects?.length).map(l=>[l.id,l.effects]))},openSession:Store.openSession()?.id??null};}},
  "timeline.snapshot":{group:"Perception",hint:"read",desc:"Lanes and items in a frame range. detail: concise (ID, label, range) or full (all fields). Paged with limit/cursor.",
    example:()=>({projectId:PROJECT_ID,from:0,to:endOf(proj()),detail:"concise"}),
    run(a){needProject(a);needAccess("read");const p=proj(), from=a.from??0, to=a.to??endOf(p), f=a.detail==="full"?full:concise;
      const inR=x=>startOf(p,x)<to&&startOf(p,x)+lengthOf(p,x)>from;
      const lanes=p.lanes.map(l=>({id:l.id,name:l.name,kind:l.kind,items:p.items.filter(i=>i.lane===l.id&&inR(i)).sort((a,b)=>startOf(p,a)-startOf(p,b)).map(i=>({...f(p,i),anchor:i.anchor}))}));
      const spine=[...p.clips].sort((a,b)=>a.start-b.start).filter(inR).map(c=>f(p,c));
      const limit=a.limit??200, all=[...spine,...lanes.flatMap(l=>l.items)];
      return {revision:rid(),fps:FPS,range:{from,to},lanes,spine,markers:p.markers.filter(m=>m.frame>=from&&m.frame<to),total:all.length,nextCursor:all.length>limit?"c"+limit:null};}},
  "items.inspect":{group:"Perception",hint:"read",desc:"Full state of up to 50 IDs: inspector parity, plus anchor, resolved frames, children and links.",
    example:()=>({projectId:PROJECT_ID,ids:["clp_take","ovl_broll"]}),
    run(a){needProject(a);needAccess("read");const p=proj(), ids=list(a.ids);if(ids.length>50)throw fail("BAD_ARGS","Up to 50 IDs per call.","Page the request.");
      return {revision:rid(),items:ids.map(id=>{const x=byId(p,id);if(!x)return {id,error:"NOT_FOUND"};if(!isClip(x)&&!isItem(x))return {...x,label:labelOf(p,id)};return full(p,x);})};}},
  "timeline.find":{group:"Perception",hint:"read",desc:"Search by text, transcript word, marker label, kind, color or lane. kind:\"edit\" returns edit points. Returns IDs and frames.",
    example:()=>({projectId:PROJECT_ID,text:"frame"}),
    run(a){needProject(a);needAccess("read");const p=proj(), q=(a.text||"").toLowerCase(), out=[];
      const want=k=>!a.kind||a.kind===k;
      for(const c of p.clips)if(want("clip")&&(!q||nameOf(c).toLowerCase().includes(q))&&(!a.color||c.color===a.color))out.push({id:c.id,kind:"clip",label:labelOf(p,c.id),frame:c.start});
      for(const i of p.items)if(want(i.kind)&&(!a.lane||i.lane===a.lane)&&(!q||nameOf(i).toLowerCase().includes(q))&&(!a.color||i.color===a.color))out.push({id:i.id,kind:i.kind,lane:i.lane,label:labelOf(p,i.id),frame:startOf(p,i)});
      for(const m of p.markers)if(want("marker")&&(!q||m.name.toLowerCase().includes(q)))out.push({id:m.id,kind:"marker",label:m.name,frame:m.frame});
      if(want("word")&&q)for(const w of WORDS)if(w.text.toLowerCase().includes(q)){let f=null;try{f=resolveFrame(p,{word:w.id},{resolve:v=>v});}catch{}out.push({id:w.id,kind:"word",label:w.text,frame:f});}
      if(a.kind==="edit")for(const f of editPoints())out.push({kind:"edit",frame:f});
      return {revision:rid(),matches:out.slice(0,a.limit??100),total:out.length};}},
  "takes.events":{group:"Perception",hint:"read",desc:"A take's clicks and keystrokes (category and text) on the timeline clock. Event indexes feed keys.hide.",
    example:()=>({projectId:PROJECT_ID,clip:"clp_take"}),
    run(a){needProject(a);needAccess("read");const p=proj(), c=needClip(p,a.clip), A=assets[c.asset];
      const vis=s=>s>=c.source&&s<srcEnd(c);
      return {revision:rid(),clip:c.id,clicks:(A.clicks||[]).filter(vis).map(s=>({sourceFrame:s,frame:Math.round(tlAt(c,s))})),
        keys:(A.keys||[]).map((e,i)=>({index:i,text:e.text,category:/[⌘⇧⌥]/.test(e.text)?"shortcut":e.text==="↩"?"control":"typing",sourceFrame:e.at,frame:vis(e.at)?Math.round(tlAt(c,e.at)):null,hidden:c.keyStyle.hidden.includes(i)}))};}},
  "transcript.get":{group:"Perception",hint:"read",desc:"Words with IDs and timeline frames, phrases and fillers. Paged. Pass the hash back with word anchors and cut.words. captions:\"json\"|\"srt\"|\"vtt\" returns the caption cues instead, with per-word frames and display edits.",
    example:()=>({projectId:PROJECT_ID,limit:16}),
    run(a){needProject(a);needAccess("read");const p=proj(), start=a.cursor?parseInt(a.cursor.slice(1)):0, limit=a.limit??60;
      if(a.captions!==undefined)return captionsOut(p,a.captions,a.lane);
      const words=WORDS.slice(start,start+limit).map(w=>{let f=null;try{f=resolveFrame(p,{word:w.id},{resolve:v=>v});}catch{}return {id:w.id,text:w.text,frame:f,length:w.dur,filler:w.filler||undefined};});
      return {revision:rid(),hash:TRANSCRIPT_HASH,clip:"clp_take",words,fillers:WORDS.filter(w=>w.filler).map(w=>w.id),nextCursor:start+limit<WORDS.length?"w"+(start+limit):null};}},
  "audio.summary":{group:"Perception",hint:"read",desc:"Per component: loudness, peaks and silences (from the transcript's gaps).",
    example:()=>({projectId:PROJECT_ID,clip:"clp_take"}),
    run(a){needProject(a);needAccess("read");const p=proj(), c=needClip(p,a.clip);
      const gaps=[];for(let i=1;i<WORDS.length;i++){const g0=WORDS[i-1].at+WORDS[i-1].dur,g1=WORDS[i].at;if(g1-g0>=15&&g0>=c.source&&g1<=srcEnd(c))gaps.push({from:Math.round(tlAt(c,g0)),to:Math.round(tlAt(c,g1))});}
      const comps=Object.keys(c.enabled).filter(k=>k==="mic"||k==="system").map(k=>({component:k,enabled:c.enabled[k],volume:c.volume[k],integratedLufs:k==="mic"?-17.8+20*Math.log10(c.volume[k]||1e-3):-31.2+20*Math.log10(c.volume[k]||1e-3),peakDbfs:k==="mic"?-3.1:-14.6}));
      return {revision:rid(),clip:c.id,components:comps,silences:gaps};}},
  "view.frames":{group:"Perception",hint:"read",desc:"Rendered frames at given frames, or a filmstrip (count). Images inline, with a resource_link for each.",
    example:()=>({projectId:PROJECT_ID,at:[200,372,560]}),
    run(a){needProject(a);needAccess("read");const p=a.preview||proj(), ats=a.count?Array.from({length:a.count},(_,i)=>Math.round(i*(endOf(p)-1)/(a.count-1||1))):list(a.at);
      if(!ats.length)throw fail("BAD_ARGS","Pass at:[frames] or count.","Frames are integers.");
      const cv=document.createElement("canvas");
      const frames=ats.map(f=>{if(!Number.isInteger(f))throw fail("BAD_ARGS","at must be integer frames.","Multiply seconds by fps.");draw(cv,p,clamp(f,0,endOf(p)-1));const s=document.createElement("canvas");s.width=Math.round(cv.width/4);s.height=Math.round(cv.height/4);s.getContext("2d").drawImage(cv,0,0,s.width,s.height);return {frame:f,uri:`splice://${PROJECT_ID}/${rid()}/frame/${f}.jpg`,_img:s.toDataURL("image/jpeg",.7)};});
      return {revision:rid(),frames};}},
  "commands.describe":{group:"Perception",hint:"read",desc:"Registry search and schemas with examples: the catalog for edit.apply. Pass id for one schema, or query to search.",
    example:()=>({query:"lane"}),
    run(a){
      if(a.id){const c=COMMANDS[a.id];if(!c)throw fail("UNKNOWN_COMMAND",`No command ${a.id}.`,"Call commands.describe with a query.");return {id:c.id,title:c.title,group:c.group,risk:c.risk,shortcut:c.shortcut||null,planned:!!c.planned||undefined,params:c.params,ui:c.ui,example:{cmd:c.id,...c.example(proj())},...(c.id.startsWith("caption")?{captionModel:"A cue holds transcript word IDs; its timing is derived from those words on the timeline. Display edits per word never change the transcript. Style: effect type captions (project › lane › cue).",presets:CAPTION_PRESETS}:{}),...(c.id.startsWith("effect.")?{effectTypes:Object.fromEntries(Object.entries(EFFECT_TYPES).map(([k,t])=>[k,{title:t.title,appliesTo:t.applies==="take"?"screen takes":t.applies==="caption"?"captions lanes, caption cues and the project":"spine clips and media overlays",params:t.params}])),resolution:"project › lane › item, per parameter; enabled:false at a scope turns an inherited effect off"}:{})};}
      const q=(a.query||"").toLowerCase();
      return {commands:Object.values(COMMANDS).filter(c=>!q||(c.id+" "+c.title+" "+c.group).toLowerCase().includes(q)).map(c=>({id:c.id,title:c.title,risk:c.risk,planned:!!c.planned||undefined}))};}},
  "session.start":{group:"Session and history",hint:"edit",desc:"Opens an agent session: returns a server-minted sessionId and the checkpoint revision. Every write carries the sessionId.",
    example:()=>({projectId:PROJECT_ID,agent:"claude"}),
    run(a){needProject(a);needAccess("live");const open=Store.openSession();if(open)return {sessionId:open.id,checkpoint:"r"+open.checkpoint,revision:rid(),resumed:true};const s=Store.startSession(a.agent);return {sessionId:s.id,checkpoint:"r"+s.checkpoint,revision:rid(),access:proj().access};}},
  "edit.apply":{group:"Session and history",hint:"edit",idempotent:true,desc:"Atomic batch of any registry commands, committed live as one undo step. Needs sessionId and ifRevision; dryRun returns the diff without committing; idempotencyKey makes retries safe.",
    example:()=>({projectId:PROJECT_ID,sessionId:Store.openSession()?.id??"ses_…",ifRevision:rid(),dryRun:true,ops:[{cmd:"item.split",item:"clp_take",at:372,ref:"$cut"},{cmd:"overlay.add",kind:"media",media:"med_broll",anchor:{item:"$cut.right",edge:"start"},length:90},{cmd:"item.delete",items:["ovl_label"],ripple:false}]}),
    run(a){
      needProject(a);needAccess("live");const s=needSession(a);
      if(a.idempotencyKey&&Store.idem[a.idempotencyKey])return {...Store.idem[a.idempotencyKey],replayed:true};
      if(a.ifRevision===undefined)throw fail("BAD_ARGS","ifRevision is required.","Pass the revision from your last read (project.info or any result).");
      if(revNum(a.ifRevision)!==Store.rev){const since=Store.log.filter(e=>e.rev>revNum(a.ifRevision)).map(e=>`${"r"+e.rev}: ${e.title} (${e.source})`);throw fail("STALE_REVISION",`ifRevision ${a.ifRevision} is stale; current is ${rid()}.`,"Re-read what you need, then retry with the current revision.",{current:rid(),changedSince:since.slice(-8)});}
      if(!Array.isArray(a.ops)||!a.ops.length)throw fail("BAD_ARGS","ops must be a non-empty array.","[{cmd:\"item.split\", item, at}]");
      const label=a.summary?"Agent: "+a.summary:null;
      const r=Store.apply(a.ops,{source:"agent",sessionId:s.id,dryRun:!!a.dryRun,label});
      const out={...r};delete out.preview;
      if(a.dryRun)out.diff=timelineDiff(proj(),r.preview);
      if(a.idempotencyKey&&!a.dryRun)Store.idem[a.idempotencyKey]=out;
      return out;
    }},
  "history.list":{group:"Session and history",hint:"read",desc:"The session's batches with summaries, revisions and affected IDs.",
    example:()=>({projectId:PROJECT_ID,sessionId:Store.openSession()?.id??Object.keys(Store.sessions).at(-1)??"ses_…"}),
    run(a){const s=Store.sessions[a.sessionId];if(!s)throw fail("NOT_FOUND",`No session ${a.sessionId}.`,"session.start returns a sessionId.");
      return {sessionId:s.id,open:s.open,checkpoint:"r"+s.checkpoint,revision:rid(),batches:s.batches.map(i=>{const e=Store.log[i];return {revision:"r"+e.rev,summary:e.title,state:e.reverted?"reverted":e.undone?"undone":"live",created:e.changes.flatMap(c=>c.created||[]),sideEffects:e.sideEffects.length};}),humanEditsSince:Store.log.filter((e,i)=>e.source==="human"&&e.rev>s.checkpoint&&!e.revertOf).length};}},
  "history.undo":{group:"Session and history",hint:"edit",desc:"scope:last reverts the session's last batch; scope:session reverts everything since the checkpoint in one step (later human edits are replayed on top; any that no longer apply are listed); scope:to with revision restores that exact state. Every undo is a new revision, so an undo can itself be undone.",
    example:()=>({projectId:PROJECT_ID,sessionId:Store.openSession()?.id??Object.keys(Store.sessions).at(-1)??"ses_…",scope:"last"}),
    run(a){needProject(a);
      if(a.scope==="to"){if(a.revision===undefined)throw fail("BAD_ARGS","scope:to needs a revision.","Pass revision, e.g. \"r12\"; history.list shows them.");return Store.restoreTo(revNum(a.revision),a.sessionId);}
      if(!["last","session"].includes(a.scope))throw fail("BAD_ARGS","scope must be last, session or to.","Pass scope:\"last\", \"session\", or \"to\" with a revision.",{valid:["last","session","to"]});return Store.undoSession(a.sessionId,a.scope);}},
  "ui.show":{group:"View",hint:"edit (view only)",desc:"Moves the playhead, selects, frames a range, expands components, sets viewer zoom (viewer:{zoom:\"fit\"|1}) or switches view. Never edits the project.",
    example:()=>({at:372,select:["ovl_broll"]}),
    run(a){
      if(a.view)setView(a.view==="editor"?"edit":a.view);
      if(a.select){const ids=list(a.select).filter(id=>byId(proj(),id));if(ids.length===1)select(ids[0]);else selectGroup(new Set(ids));}
      if(a.expand){expanded.add(a.expand);renderTimeline();}
      if(a.range){const W=laneWidth(),dur=Math.max(2,a.range.to-a.range.from)/FPS;pps=clamp(W/(dur*1.2),minPps(),MAX_PPS);scrollX=a.range.from*ppf()-W/12;clampScroll();userZoomed=true;renderTimeline();}
      if(a.at!==undefined)seek(resolveFrame(proj(),a.at,{resolve:v=>v}));
      if(a.viewer){if(a.viewer.zoom==="fit")fitViewer();else if(typeof a.viewer.zoom==="number")zoomViewer(a.viewer.zoom);}
      return {revision:rid(),playhead:frame,selection:selIds(),edited:false};}},
  "export.start":{group:"Jobs",hint:"edit",desc:"Starts an export job. No approval: agents export freely. Writes <project>.splice/exports/<name>-r<rev>.mp4 by default and never overwrites unless overwrite:true.",
    example:()=>({projectId:PROJECT_ID,preset:"social-hevc"}),
    run(a){needProject(a);needAccess("read");
      const sizes={"9:16":[1080,1920],"1:1":[1080,1080],"16:9":[1920,1080]}, [w,h]=sizes[proj().aspect];
      const path=a.path||`Field Notes.splice/exports/Field Notes-${rid()}.mp4`;
      if(EXISTED(path)&&!a.overwrite)throw fail("EXISTS",`${path} already exists.`,"Pass overwrite:true, or a different path.",{current:path});
      if(Object.values(JOBS).some(j=>["running","recording"].includes(j.state)))throw fail("BUSY","Another job is already running.","Wait for jobs.status to finish, or jobs.cancel it.");
      const id="job_"+(SERIAL++).toString(36);
      JOBS[id]={id,kind:"export",state:"running",progress:0,revision:rid(),settings:{width:w,height:h,fps:FPS,codec:a.preset==="social-h264"?"H.264":"HEVC",container:"MP4",encoder:"VideoToolbox (hardware)",path,durationFrames:endOf(proj())}};
      EXPORTED.add(path);runJob(JOBS[id]);
      return {jobId:id,state:"running",settings:JOBS[id].settings};}},
  "record.start":{group:"Jobs",hint:"edit",desc:"Starts a headless recording of a display, window or area (with mic, system audio, camera and input events) into the project. Runs in Splice.app's capture agent, so grants stay with the signed bundle. dryRun:true lists sources and permission states. Simulated here; built in M1 (PLAN.md T8).",
    example:()=>({projectId:PROJECT_ID,display:"main",mic:true,events:true}),
    run(a){needProject(a);needAccess("live");
      if(a.dryRun)return {sources:{displays:[{id:"main",width:3024,height:1964}],windows:[{id:"win_41",app:"com.apple.Safari",title:"Docs"}]},permissions:{screen:"granted",microphone:"granted",camera:"notDetermined",inputMonitoring:"granted"},identity:{path:"~/Applications/Splice.app",team:"7ZH3FYV964",requirementMatches:true}};
      if(Object.values(JOBS).some(j=>j.state==="recording"))throw fail("BUSY","A recording is already running.","record.stop it first.");
      const id="job_"+(SERIAL++).toString(36);
      JOBS[id]={id,kind:"record",state:"recording",progress:0,settings:{display:a.display??"main",window:a.window??null,mic:!!a.mic,systemAudio:!!a.systemAudio,camera:!!a.camera,events:a.events!==false}};
      return {jobId:id,state:"recording",settings:JOBS[id].settings};}},
  "record.stop":{group:"Jobs",hint:"edit",desc:"Finishes a recording. In the product the take lands as a spine clip with its components (mic, system, cursor, keys); here the job just completes.",
    example:()=>({jobId:Object.values(JOBS).find(j=>j.kind==="record")?.id??"job_…"}),
    run(a){const j=JOBS[a.jobId];if(!j||j.kind!=="record")throw fail("NOT_FOUND",`No recording ${a.jobId}.`,"record.start returns a jobId.");if(j.state==="recording"){j.state="done";j.progress=1;}return {...copy(j)};}},
  "jobs.status":{group:"Jobs",hint:"read",desc:"Progress for export, transcription and recording jobs.",
    example:()=>({jobId:Object.keys(JOBS).at(-1)??"job_…"}),
    run(a){const j=JOBS[a.jobId];if(!j)throw fail("NOT_FOUND",`No job ${a.jobId}.`,"export.start returns a jobId.");return {...copy(j)};}},
  "jobs.cancel":{group:"Jobs",hint:"edit",desc:"Cancels a running or pending job.",
    example:()=>({jobId:Object.keys(JOBS).at(-1)??"job_…"}),
    run(a){const j=JOBS[a.jobId];if(!j)throw fail("NOT_FOUND",`No job ${a.jobId}.`,"export.start returns a jobId.");if(["done","cancelled"].includes(j.state))return {...copy(j)};j.state="cancelled";return {...copy(j)};}}
};
function callTool(name,args){
  const t=TOOLS[name];
  try{
    if(!t)throw fail("UNKNOWN_COMMAND",`No tool ${name}.`,"tools/list returns the 19 tools.");
    const r=t.run(args||{});
    return {isError:false,structuredContent:r};
  }catch(e){
    if(!e.code)throw e;
    const err={code:e.code,message:e.message,fix:e.fix};for(const k of ["current","changedSince","valid","op"])if(e[k]!==undefined)err[k]=e[k];
    return {isError:true,structuredContent:{error:err}};
  }
}

/* ── Agent tab ── */
let tool="edit.apply", callLog=[], shown=null;
function textMirror(name,r){
  const s=r.structuredContent;
  if(r.isError)return `${s.error.code}: ${s.error.message} ${s.error.fix||""}`;
  if(name==="edit.apply")return `${s.dryRun?"Dry run":"Committed "+s.revision}: ${s.changes.map(c=>c.summary).join("; ")}.${s.sideEffects.length?" Side effects: "+s.sideEffects.map(x=>x.label+(x.deleted?" deleted":x.bumped?" bumped":" re-anchored")).join(", ")+".":""}`;
  return JSON.stringify(s).slice(0,160);
}
function record(name,args){
  const r=callTool(name,args);
  const imgs=r.structuredContent?.frames?.map(f=>f._img)||[];
  if(r.structuredContent?.frames)r.structuredContent.frames=r.structuredContent.frames.map(({_img,...f})=>f);
  const entry={name,args,result:{content:[{type:"text",text:textMirror(name,r)},...imgs.map(()=>({type:"image",mimeType:"image/jpeg",data:"…"}))],structuredContent:r.structuredContent,isError:r.isError},imgs,time:new Date()};
  callLog.unshift(entry);shown=entry;
  return entry;
}
function renderLab(){
  const groups={};for(const [n,t] of Object.entries(TOOLS))(groups[t.group]??=[]).push(n);
  $("tool-list").innerHTML=Object.entries(groups).map(([g,ns])=>`<div class="tool-group">${g}</div>`+ns.map(n=>`<button data-tool="${n}" aria-pressed="${n===tool}"><span>${n}</span><small>${TOOLS[n].hint}</small></button>`).join("")).join("");
  $("tool-name").textContent=tool;
  const t=TOOLS[tool];
  $("tool-desc").textContent=t.desc;
  $("tool-annot").innerHTML=`<span class="pill">${t.hint==="read"?"readOnlyHint":"destructiveHint: "+(tool==="edit.apply"?"per op":"false")}</span>${t.idempotent?'<span class="pill">idempotentHint (with key)</span>':""}<span class="pill">rev ${rid()}</span>`;
  const s=Store.openSession();
  $("session-state").innerHTML=s?`<span class="dot live"></span>${s.id} · checkpoint r${s.checkpoint} · ${s.batches.length} batch(es)`:`<span class="dot"></span>no open session · access ${proj().access}`;
  if(shown){
    $("result").textContent=JSON.stringify(shown.result.structuredContent,null,2);
    $("result").classList.toggle("is-error",shown.result.isError);
    $("result-mirror").textContent=shown.result.content[0].text;
    $("result-frames").innerHTML=shown.imgs.map((src,i)=>`<figure><img src="${src}" alt="Frame ${shown.result.structuredContent.frames[i].frame}"><figcaption>${shown.result.structuredContent.frames[i].frame} f</figcaption></figure>`).join("");
    $("result-title").textContent=(shown.result.isError?"Error · ":"Result · ")+shown.name;
  }else{$("result").textContent="Run the call to see its structured result.";$("result-mirror").textContent="";$("result-frames").innerHTML="";$("result-title").textContent="Result";$("result").classList.remove("is-error");}
  const diff=shown?.name==="edit.apply"&&!shown.result.isError?shown.result.structuredContent.diff:null;
  $("dry-diff").innerHTML=diff?.length?'<h3>Dry-run diff</h3>'+diff.map(d=>`<div class="change">${esc(d.name)} <small>${d.id}</small> · ${esc(d.before)} → ${esc(d.after)}</div>`).join(""):"";
  $("call-log").innerHTML=callLog.slice(0,30).map((c,i)=>`<button data-call="${i}" class="${c===shown?"on":""}"><span class="${c.result.isError?"error":"ok"}">${c.result.isError?"✗":"✓"}</span><code>${c.name}</code><small>${esc(c.result.content[0].text.slice(0,70))}</small></button>`).join("")||'<p class="dim">No calls yet.</p>';
  const sess=Object.values(Store.sessions).at(-1);
  $("activity").innerHTML=sess?`<div class="act-head"><span>${sess.id} · ${sess.open?"open":"ended"} · checkpoint r${sess.checkpoint}</span><button id="act-undo-all" ${sess.batches.some(i=>!Store.log[i].reverted&&!Store.log[i].undone)?"":"disabled"}>Undo all agent edits</button></div>`+
    (sess.batches.slice().reverse().map(i=>{const e=Store.log[i];const st=e.reverted?"reverted":e.undone?"undone":"live";return `<div class="act-row ${st}"><span>r${e.rev}</span><span class="grow">${esc(e.title.replace(/^Agent: /,""))}${e.sideEffects.length?` <small>· ${e.sideEffects.length} side effect(s)</small>`:""}</span><button data-revert="${i}" ${st!=="live"?"disabled":""}>${st==="live"?"Undo":st}</button></div>`;}).join("")||'<p class="dim">No batches yet.</p>'):'<p class="dim">Start a session to see agent batches here.</p>';
  const ua=$("act-undo-all");if(ua)ua.onclick=()=>Store.undoSession(sess.id,"session");
}
let lastExample="";
function loadExample(){const ex=TOOLS[tool].example();$("request").value=lastExample=JSON.stringify(ex,null,2);}
function setTool(name){tool=name;loadExample();renderLab();}
$("tool-list").onclick=e=>{const b=e.target.closest("[data-tool]");if(b)setTool(b.dataset.tool);};
$("reset-request").onclick=()=>{loadExample();};
$("sync-rev").onclick=()=>{try{const a=JSON.parse($("request").value);if("ifRevision" in a)a.ifRevision=rid();if("sessionId" in a&&Store.openSession())a.sessionId=Store.openSession().id;$("request").value=JSON.stringify(a,null,2);}catch{}};
$("run-tool").onclick=()=>{
  let args;try{args=JSON.parse($("request").value||"{}");}catch(e){$("result").textContent="Request is not valid JSON: "+e.message;$("result").classList.add("is-error");return;}
  record(tool,args);renderLab();
};
$("call-log").onclick=e=>{const b=e.target.closest("[data-call]");if(!b)return;shown=callLog[+b.dataset.call];tool=shown.name;$("request").value=JSON.stringify(shown.args,null,2);renderLab();};
$("activity").addEventListener("click",e=>{const b=e.target.closest("[data-revert]");if(!b)return;const i=+b.dataset.revert;Store.revert([i],"Undo agent batch: "+Store.log[i].title.replace(/^Agent: /,""),{sessionId:Store.log[i].sessionId,source:"human"});});
$("request").addEventListener("keydown",e=>{if((e.metaKey||e.ctrlKey)&&e.key==="Enter"){e.preventDefault();$("run-tool").click();}});

/* Scripted session: what a real agent run looks like, step by step, live in the editor. */
let scenarioRunning=false;
async function runScenario(watch){
  if(scenarioRunning)return;scenarioRunning=true;$("scenario").disabled=$("scenario-watch").disabled=true;
  const wait=ms=>new Promise(r=>setTimeout(r,ms));
  if(watch)setView("edit");
  try{
    const st=record("session.start",{projectId:PROJECT_ID,agent:"claude"}).result.structuredContent;
    if(st.error){setNote("Scenario stopped: "+st.error.message);return;}
    const sid=st.sessionId;await wait(600);
    record("transcript.get",{projectId:PROJECT_ID,limit:20});await wait(500);
    const fill=WORDS.filter(w=>w.filler).map(w=>w.id);
    const ops=[{cmd:"title.add",at:{word:"w12",transcriptHash:TRANSCRIPT_HASH},length:60,text:"⌘K · command palette"},{cmd:"overlay.add",kind:"media",media:"med_broll",at:{marker:"mrk_2"},length:60,placement:"corner-br"},{cmd:"marker.add",at:{word:"w20"},name:"Save beat"}];
    record("edit.apply",{projectId:PROJECT_ID,sessionId:sid,ifRevision:rid(),dryRun:true,ops});await wait(700);
    record("edit.apply",{projectId:PROJECT_ID,sessionId:sid,ifRevision:rid(),ops,summary:"Callouts for the palette and save beats",idempotencyKey:"demo-callouts-"+Store.rev});await wait(900);
    record("edit.apply",{projectId:PROJECT_ID,sessionId:sid,ifRevision:rid(),ops:[{cmd:"cut.words",words:fill,transcriptHash:TRANSCRIPT_HASH}],summary:"Cut filler words"});await wait(700);
    record("ui.show",{at:{word:"w12"},select:[]});await wait(300);
    record("view.frames",{projectId:PROJECT_ID,at:[frame,Math.min(endOf(proj())-1,frame+90)]});
    setNote("Agent session "+sid+" made 2 live batches. Undo all agent edits (⌥⌘Z) reverts both in one step.");
  }finally{scenarioRunning=false;$("scenario").disabled=$("scenario-watch").disabled=false;renderLab();}
}
$("scenario").onclick=()=>runScenario(false);$("scenario-watch").onclick=()=>runScenario(true);
