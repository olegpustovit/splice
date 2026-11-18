"use strict";
/* ══ Splice core (prototype of the Rust core) ══
   Pure model + command registry + apply engine. No DOM here.
   Everything that changes the project goes through COMMANDS via Core.apply(). */
const FPS = 30;
const clamp = (x,a,b) => Math.min(b,Math.max(a,x));
const copy = v => structuredClone(v);
const parts = {screen:"Screen video",camera:"Camera video",mic:"Microphone",system:"System audio",pointer:"Cursor",keys:"Keystrokes"};

/* Synthetic capture events and transcript on the screen take's source clock. */
const KEY_EVENTS = [{at:24,dur:40,text:"⌘ K"},{at:96,dur:66,text:"frame study"},{at:186,dur:34,text:"⌘ S"},{at:262,dur:44,text:"⇧ ⌘ P"},{at:330,dur:30,text:"↩"},{at:402,dur:50,text:"rotate 15"},{at:478,dur:36,text:"⌘ Z"}];
const CLICK_EVENTS = [40,92,150,214,268,318,376,436,494];
const WORDS = (() => {
  const text = "Today we build a frame study. Um, first open the command palette and search frame study. Save it. Then open the project panel, hit return and rotate it fifteen degrees. Uh, undo if it looks wrong.";
  let at = 6; const out = [];
  text.split(" ").forEach((w,i) => {
    const filler = /^(um|uh),?$/i.test(w);
    const dur = Math.max(5, Math.round(w.replace(/\W/g,"").length*2.2));
    out.push({id:"w"+(i+1), text:w.replace(/[.,]/g,""), at, dur, filler, end:/[.]$/.test(w)||undefined, pause:/,$/.test(w)||undefined});
    at += dur + (/[.]$/.test(w) ? 22 : /,$/.test(w) ? 9 : 3);
  });
  return out;
})();
const assets = {
  opener:{name:"Opening motion",type:"graphic",frames:120,mark:"◈"},
  screen:{name:"Screen take 01",type:"take",frames:540,native:{w:640,h:400},mark:"▣",parts:["screen","camera","mic","system","pointer","keys"],keys:KEY_EVENTS,clicks:CLICK_EVENTS,words:WORDS},
  webcam:{name:"Webcam take 01",type:"take",frames:300,mark:"◉",parts:["camera","mic"],keys:[],clicks:[],words:[]},
  closing:{name:"End card",type:"graphic",frames:120,mark:"▥"},
  broll:{name:"Desk b-roll",type:"graphic",frames:240,mark:"▤",style:"broll",native:{w:640,h:360}},
  logo:{name:"Logo bug",type:"image",frames:1e9,mark:"◩"},
  gap:{name:"Gap",type:"gap",frames:1e9,mark:"·"}
};
const mediaIds = {opener:"med_opener",screen:"med_screen",webcam:"med_webcam",closing:"med_endcard",broll:"med_broll",logo:"med_logo"};
const assetOfMedia = m => Object.keys(mediaIds).find(k=>mediaIds[k]===m) || (assets[m]?m:null);

/* ── Errors: every refusal is a typed result with a fix hint ── */
const ERRORS = {
  STALE_REVISION:"ifRevision doesn't match. Returns the current revision and a summary of what changed",
  NOT_FOUND:"The ID doesn't exist",
  WRONG_KIND:"The command doesn't apply to this kind of item (e.g. overlay.set on a spine clip)",
  OUT_OF_SOURCE:"The trim or extension exceeds the available media",
  TOO_SHORT:"The result would be under 3 frames",
  OVERLAP:"Only when ripple:false and a spine move collides with another clip",
  LANE_CONFLICT:"Only when the caller names an explicit lane that is occupied; otherwise the item is auto-bumped",
  LANE_NOT_EMPTY:"lane.delete on the last lane of its kind while it still holds items",
  TRANSCRIPT_CHANGED:"The transcript hash doesn't match",
  BAD_ARGS:"A parameter is missing, has the wrong type, or is not an integer frame",
  UNKNOWN_COMMAND:"No registry command has this id; the error lists close matches",
  PLANNED:"The command is in the registry but not built in this prototype",
  EXISTS:"The export path exists; pass overwrite:true or another path",
  BUSY:"Another job is already running",
  PERMISSION_DENIED:"The project's access level doesn't allow this call",
  NO_SESSION:"Writes need a sessionId from session.start"
};
function fail(code,message,fix,extra){const e=Error(message);e.code=code;e.fix=fix;Object.assign(e,extra||{});return e;}

/* ── Sample project ── */
function clipOf(id, asset, start, length = assets[asset].frames, extra = {}) {
  return {id, asset, start, length, source:0, speed:1, fadeIn:0, fadeOut:0, color:null, name:null,
    enabled:Object.fromEntries((assets[asset].parts||[]).map(k=>[k,true])),
    volume:{mic:1,system:.8}, keyStyle:{hidden:[]}, crop:null, fit:"fit", effects:[], ...extra};
}
function itemOf(id, kind, lane, anchor, length, extra = {}) {
  return {id, kind, lane, anchor, length, fadeIn:0, fadeOut:0, color:null, name:null, effects:[], ...extra};
}
function sampleProject(){
  const p={
    fps:FPS, aspect:"9:16", access:"live",
    effects:[{id:"fx_proj_win",type:"window",params:{padding:.07,corner:.03,shadow:.6}},{id:"fx_proj_cur",type:"cursor",params:{size:1.5,clickRings:true}}],
    spine:{id:"lan_spine",name:"Video",effects:[]},
    lanes:[
      {id:"lan_cap",name:"Captions",kind:"captions",effects:[{id:"fx_cap_style",type:"captions",params:{preset:"bold"}}]},
      {id:"lan_gfx",name:"Graphics",kind:"graphics"},
      {id:"lan_ov2",name:"Overlay 2",kind:"overlay",effects:[{id:"fx_ov2_win",type:"window",params:{padding:0,corner:.06,shadow:.8,background:"none"}}]},
      {id:"lan_ov1",name:"Overlay 1",kind:"overlay"},
      {id:"lan_mo",name:"Motion",kind:"motion"}
    ],
    clips:[clipOf("clp_open","opener",0,120,{fadeIn:15,effects:[{id:"fx_open_win",type:"window",enabled:false,params:{}}]}),clipOf("clp_take","screen",120,420,{crop:{x:.36,y:.1,w:.46,h:.8},effects:[{id:"fx_take_win",type:"window",params:{padding:.05,color:"#6b5a7d",color2:"#1e1a26"}}]}),clipOf("clp_end","closing",540,120,{fadeOut:24,effects:[{id:"fx_end_win",type:"window",enabled:false,params:{}}]})],
    items:[
      itemOf("ovl_hook","title","lan_gfx",{clip:"clp_open",sourceFrame:0},120,{text:"Make it move.",fadeIn:8,fadeOut:8}),
      itemOf("ovl_label","title","lan_gfx",{clip:"clp_take",sourceFrame:18},105,{text:"01 / Build the frame",fadeIn:6,fadeOut:6}),
      itemOf("ovl_detail","motion","lan_mo",{clip:"clp_take",sourceFrame:60},150,{from:1,to:1.16}),
      itemOf("ovl_logo","media","lan_ov1",{pinned:true,frame:0},660,{asset:"logo",source:0,placement:"corner-tr",muted:true,behindPresenter:false,name:"Watermark"}),
      itemOf("ovl_broll","media","lan_ov2",{clip:"clp_take",sourceFrame:300},75,{asset:"broll",source:0,placement:"card",muted:true,behindPresenter:false,crop:null,fit:"fill"})
    ],
    markers:[{id:"mrk_1",frame:120,name:"Hook ends"},{id:"mrk_2",frame:372,name:"Demo beat"}]
  };
  /* Captions generated from the take's transcript, three words a cue, one emphasized word with an emoji. */
  cueGroups(p,visibleWords(p,p.clips,false),3).forEach((g,i)=>p.items.push(itemOf("ovl_cap"+(i+1),"caption","lan_cap",{clip:g[0].clip,sourceFrame:g[0].w.at},3,{words:g.map(o=>o.w.id),edits:{}})));
  const cue=p.items.find(i=>i.kind==="caption"&&i.words.includes("w5"));if(cue)cue.edits.w5={emph:true,emoji:"🎯"};
  syncCaptions(p,[]);
  return p;
}

/* ── Queries ── */
const spd = c => c.speed || 1;
const kindOfId = id => id?.startsWith("clp_")?"clip":id?.startsWith("ovl_")?"item":id?.startsWith("mrk_")?"marker":id?.startsWith("lan_")?"lane":null;
const clipById = (p,id) => p.clips.find(c=>c.id===id);
const itemById = (p,id) => p.items.find(i=>i.id===id);
const byId = (p,id) => clipById(p,id)||itemById(p,id)||p.markers.find(m=>m.id===id);
const laneById = (p,id) => p.lanes.find(l=>l.id===id);
const isClip = x => !!x && "asset" in x && !("kind" in x);
const isItem = x => !!x && "anchor" in x;
const srcAt = (c,f) => c.source+(f-c.start)*spd(c);
const tlAt = (c,s) => c.start+(s-c.source)/spd(c);
const srcEnd = c => c.source+c.length*spd(c);
function startOf(p,x){
  if(isClip(x)) return x.start;
  if(!isItem(x)) return x.frame;
  if(x.anchor.pinned) return x.anchor.frame;
  const c=clipById(p,x.anchor.clip); if(!c) return 0;
  return Math.round(tlAt(c,x.anchor.sourceFrame));
}
const lengthOf = (p,x) => x.length ?? 0;
const endOf = p => Math.max(1,...p.clips.map(c=>c.start+c.length),...p.items.map(i=>startOf(p,i)+i.length));
const spineEnd = p => Math.max(0,...p.clips.map(c=>c.start+c.length));
const active = (p,x,f) => f>=startOf(p,x)&&f<startOf(p,x)+lengthOf(p,x);
const clipAt = (p,f) => p.clips.find(c=>f>=c.start&&f<c.start+c.length);
const childrenOf = (p,cid) => p.items.filter(i=>i.anchor.clip===cid);
const isTake = c => isClip(c)&&Object.keys(c.enabled||{}).length>0;
const laneKindFor = kind => kind==="title"?"graphics":kind==="motion"?"motion":kind==="caption"?"captions":"overlay";
const clock = f => {f=Math.max(0,Math.round(f));return [Math.floor(f/1800),Math.floor(f/30)%60,f%30].map(v=>String(v).padStart(2,"0")).join(":");};
const sec = f => (f/FPS).toFixed(2);
const fmtScale = v => String(+(+v).toFixed(2));
const fmtSpeed = s => fmtScale(s)+"×";
function nameOf(x){
  if(!x) return "";
  if(isClip(x)) return x.name||(assets[x.asset]?.name??x.asset);
  if(isItem(x)) return x.kind==="title"?x.text:x.kind==="motion"?(x.name||"Scale animation"):x.kind==="caption"?(x.name||cueText(x)):(x.name||assets[x.asset]?.name||"Overlay");
  return x.name||x.id;
}
function labelOf(p,id){
  const x=byId(p,id); if(!x) return id;
  if(x.frame!==undefined&&!isItem(x)&&!isClip(x)) return nameOf(x)+" · "+clock(x.frame);
  const s=startOf(p,x); return nameOf(x)+" · "+clock(s)+"–"+clock(s+lengthOf(p,x));
}
function hashWords(ws){let h=2166136261;for(const w of ws){for(const ch of w.id+w.text){h^=ch.charCodeAt(0);h=Math.imul(h,16777619)>>>0;}}return "tx_"+h.toString(36);}
const TRANSCRIPT_HASH = hashWords(WORDS);

/* ── Anchoring ── */
function anchorAt(p,f){
  f=Math.round(f);
  let c=clipAt(p,f);
  if(!c&&p.clips.length){
    const sorted=[...p.clips].sort((a,b)=>a.start-b.start);
    c=sorted.filter(x=>x.start<=f).at(-1)||sorted[0];
    const sf=f<c.start?c.source:Math.floor(c.source+(c.length-1)*spd(c));
    return {clip:c.id,sourceFrame:Math.round(sf)};
  }
  if(!c) return {pinned:true,frame:Math.max(0,f)};
  return {clip:c.id,sourceFrame:Math.round(srcAt(c,f))};
}
/* Trim or range delete removed the anchor frame: re-anchor at the new edge, never delete. */
function fixAnchors(p,fx){
  for(const it of p.items){
    if(it.anchor.pinned) continue;
    const c=clipById(p,it.anchor.clip); if(!c) continue;
    const lo=c.source, hi=Math.floor(c.source+(c.length-1)*spd(c));
    if(it.anchor.sourceFrame<lo||it.anchor.sourceFrame>hi){
      it.anchor.sourceFrame=clamp(it.anchor.sourceFrame,lo,hi);
      fx.push({id:it.id,label:nameOf(it),reanchored:{edge:it.anchor.sourceFrame===lo?"start":"end",clip:c.id}});
    }
  }
}
/* Nothing ever overwrites: an overlap in a lane bumps the later item to the nearest free lane above. */
function lanesOfKind(p,kind){return p.lanes.filter(l=>l.kind===kind);}
function laneFree(p,laneId,s,e,exclude){
  return !p.items.some(o=>o.lane===laneId&&!exclude.has(o.id)&&startOf(p,o)<e&&startOf(p,o)+o.length>s);
}
function createLane(p,kind,mint,name){
  const same=lanesOfKind(p,kind);
  const base=kind==="graphics"?"Graphics":kind==="motion"?"Motion":kind==="captions"?"Captions":"Overlay";
  const lane={effects:[],id:mint("lan"),name:name||(kind==="overlay"?base+" "+(same.length+1):same.length?base+" "+(same.length+1):base),kind};
  const stack=["captions","graphics","overlay"], rank=k=>stack.indexOf(k);
  const top=same.length?p.lanes.indexOf(same[0]):kind==="motion"?p.lanes.length:p.lanes.filter(l=>rank(l.kind)>=0&&rank(l.kind)<rank(kind)).length;
  p.lanes.splice(top,0,lane);
  return lane;
}
function settle(p,fx,mint,pinned,base){
  /* Items this batch did not touch keep their lane; touched items are the ones that bump. */
  const touched=it=>{if(!base)return 0;const o=itemById(base,it.id);return !o||o.lane!==it.lane||startOf(base,o)!==startOf(p,it)||o.length!==it.length?1:0;};
  const order=[...p.items].sort((a,b)=>touched(a)-touched(b)||startOf(p,a)-startOf(p,b)||a.id.localeCompare(b.id));
  for(const it of order){
    if(!laneById(p,it.lane)){const l=lanesOfKind(p,laneKindFor(it.kind)).at(-1)||createLane(p,laneKindFor(it.kind),mint);it.lane=l.id;}
    const s=startOf(p,it), e=s+it.length, ex=new Set([it.id]);
    const placed=new Set(order.slice(0,order.indexOf(it)).map(o=>o.id));
    const clash=lane=>p.items.some(o=>o.lane===lane&&o.id!==it.id&&placed.has(o.id)&&startOf(p,o)<e&&startOf(p,o)+o.length>s);
    if(!clash(it.lane)) continue;
    if(pinned?.has(it.id)) throw fail("LANE_CONFLICT",`${laneById(p,it.lane).name} is occupied at ${clock(s)}.`,"Omit lane to let the item bump to a free lane, or pick another lane.");
    const from=it.lane, kind=laneById(p,from).kind, same=lanesOfKind(p,kind);
    let idx=same.findIndex(l=>l.id===from), target=null;
    for(let i=idx-1;i>=0;i--) if(!clash(same[i].id)){target=same[i];break;}
    if(!target) target=createLane(p,kind,mint);
    it.lane=target.id;
    fx.push({id:it.id,label:nameOf(it),bumped:{from,to:target.id}});
  }
}

/* ── Frame resolution: integer frames, or anchor forms anywhere a frame is accepted ── */
function resolveFrame(p,v,refs,name="at"){
  if(v===undefined||v===null) return undefined;
  if(typeof v==="number"){
    if(!Number.isInteger(v)) throw fail("BAD_ARGS",`${name} must be an integer frame at ${FPS} fps, got ${v}.`,"Frames are integers at the sequence rate. Seconds are never accepted: multiply by fps from project.info.");
    return v;
  }
  if(typeof v==="string"&&v.startsWith("$")) return resolveFrame(p,{ref:v},refs,name);
  if(v.marker){const m=p.markers.find(m=>m.id===v.marker);if(!m)throw fail("NOT_FOUND",`No marker ${v.marker}.`,"timeline.find {kind:\"marker\"} lists markers.");return m.frame+(v.offset||0);}
  if(v.item||v.ref){
    const id=v.ref?refs.resolve(v.ref):refs.resolve(v.item);
    const x=byId(p,id);if(!x)throw fail("NOT_FOUND",`No item ${id}.`,"Call timeline.snapshot for current IDs.");
    const s=startOf(p,x);return (v.edge==="end"?s+lengthOf(p,x):s)+(v.offset||0);
  }
  if(v.word){
    const w=WORDS.find(w=>w.id===v.word);if(!w)throw fail("NOT_FOUND",`No word ${v.word}.`,"transcript.get returns word IDs.");
    if(v.transcriptHash&&v.transcriptHash!==TRANSCRIPT_HASH)throw fail("TRANSCRIPT_CHANGED","Transcript changed since you read it.","Call transcript.get again and use the new hash.",{current:TRANSCRIPT_HASH});
    const c=p.clips.find(c=>c.asset==="screen"&&w.at>=c.source&&w.at<srcEnd(c));
    if(!c)throw fail("NOT_FOUND",`Word ${v.word} (“${w.text}”) is cut from the timeline.`,"Pick a word that is still on the timeline.");
    return Math.round(tlAt(c,w.at))+(v.edge==="end"?Math.round(w.dur/spd(c)):0)+(v.offset||0);
  }
  throw fail("BAD_ARGS",`${name} must be a frame or an anchor form.`,"Use an integer, {marker}, {item, edge}, {word} or {ref}.");
}
function needClip(p,id){const c=clipById(p,id);if(!c)throw fail(byId(p,id)?"WRONG_KIND":"NOT_FOUND",byId(p,id)?`${id} is not a spine clip.`:`No clip ${id}.`,"Spine clip IDs start with clp_. timeline.snapshot lists them.");return c;}
function needItem(p,id,kinds){
  const it=itemById(p,id);
  if(!it)throw fail(byId(p,id)?"WRONG_KIND":"NOT_FOUND",byId(p,id)?`${id} is not an overlay item.`:`No overlay ${id}.`,"Overlay IDs start with ovl_.");
  if(kinds&&!kinds.includes(it.kind))throw fail("WRONG_KIND",`${id} is a ${it.kind}, expected ${kinds.join(" or ")}.`,"items.inspect shows each item's kind.");
  return it;
}
function needAny(p,id){const x=byId(p,id);if(!x)throw fail("NOT_FOUND",`No item ${id}.`,"Call timeline.find or timeline.snapshot to get current IDs.");return x;}
const list = v => v===undefined?[]:Array.isArray(v)?v:[v];

/* ── Spine helpers ── */
function shiftAfter(p,at,delta,except){for(const c of p.clips)if(c!==except&&c.start>=at)c.start+=delta;}
function clampFades(x){const L=x.length;x.fadeIn=Math.min(x.fadeIn||0,L);x.fadeOut=Math.min(x.fadeOut||0,L-(x.fadeIn||0));}
function checkSource(c){
  const a=assets[c.asset];
  if(c.source<0||srcEnd(c)>a.frames+1e-6)throw fail("OUT_OF_SOURCE",`${nameOf(c)} has ${a.frames} source frames; that edit needs ${Math.ceil(srcEnd(c))}.`,"Use a smaller trim or extension; items.inspect shows sourceIn and the media length.");
}
function checkLength(x){if(x.length<3)throw fail("TOO_SHORT",`${nameOf(x)} would be ${x.length} frames.`,"Items need at least 3 frames.");}
function checkOverlap(p,c){
  const o=p.clips.find(x=>x!==c&&x.start<c.start+c.length&&x.start+x.length>c.start);
  if(o)throw fail("OVERLAP",`${nameOf(c)} would overlap ${nameOf(o)} (${o.id}).`,"Pass ripple:true to push later clips, or move into the gap.");
}
function recordFrames(p,items){return new Map(items.map(i=>[i.id,startOf(p,i)]));}
function reanchorAt(p,map,fx,why){
  for(const [id,f] of map){const it=itemById(p,id);if(!it||it.anchor.pinned)continue;const before=it.anchor.clip;it.anchor=anchorAt(p,f);if(it.anchor.clip!==before)fx.push({id,label:nameOf(it),reanchored:{from:before,to:it.anchor.clip??"pinned",why}});}
}
function splitClip(p,c,f,mint,fx){
  if(f<c.start+2||f>c.start+c.length-2)throw fail("BAD_ARGS",`Frame ${f} is not inside ${nameOf(c)} (${c.start}–${c.start+c.length}).`,"Split needs a frame at least 2 frames inside the clip.");
  const cut=f-c.start, right={...copy(c),id:mint("clp"),start:f,source:c.source+cut*spd(c),length:c.length-cut,fadeIn:0};
  c.length=cut;c.fadeOut=0;p.clips.push(right);
  for(const it of childrenOf(p,c.id))if(it.anchor.sourceFrame>=right.source){it.anchor.clip=right.id;fx.push({id:it.id,label:nameOf(it),reanchored:{from:c.id,to:right.id}});}
  return right;
}
function removeClip(p,c,ripple,keep,mint,fx){
  const kids=childrenOf(p,c.id);
  if(keep){
    const gap={...clipOf(mint("clp"),"gap",c.start,c.length),enabled:{}};
    p.clips.splice(p.clips.indexOf(c),1,gap);
    for(const it of kids){it.anchor={clip:gap.id,sourceFrame:Math.round((it.anchor.sourceFrame-c.source)/spd(c))};fx.push({id:it.id,label:nameOf(it),reanchored:{from:c.id,to:gap.id,why:"gap keeps overlays"}});}
    return;
  }
  for(const it of kids){p.items.splice(p.items.indexOf(it),1);fx.push({id:it.id,label:nameOf(it),deleted:true,cascadeFrom:c.id});}
  p.clips.splice(p.clips.indexOf(c),1);
  if(ripple)shiftAfter(p,c.start+c.length,-c.length);
}
function addItem(p,a,kind,x){
  const lanePinned=a.lane!==undefined;
  if(lanePinned){const l=laneById(p,a.lane);if(!l)throw fail("NOT_FOUND",`No lane ${a.lane}.`,"project.info lists lanes.");if(l.kind!==laneKindFor(kind))throw fail("WRONG_KIND",`${l.name} holds ${l.kind} items, not ${kind}.`,`Use a ${laneKindFor(kind)} lane or omit lane.`);}
  let anchor;
  if(a.anchor&&(a.anchor.clip||a.anchor.pinned)){
    anchor=a.anchor.pinned?{pinned:true,frame:resolveFrame(p,a.anchor.frame??0,x.refs,"anchor.frame")}:{clip:needClip(p,x.refs.resolve(a.anchor.clip)).id,sourceFrame:a.anchor.sourceFrame|0};
  } else anchor=anchorAt(p,resolveFrame(p,a.at??a.anchor,x.refs)??0);
  const lane=a.lane||(lanesOfKind(p,laneKindFor(kind)).at(-1)||createLane(p,laneKindFor(kind),x.mint)).id;
  const length=a.length??(kind==="title"?90:kind==="motion"?120:Math.min(90,assets[assetOfMedia(a.media)]?.frames??90));
  if(!Number.isInteger(length))throw fail("BAD_ARGS","length must be an integer frame count.","Frames are integers.");
  const it=itemOf(x.mint("ovl"),kind,lane,anchor,length);
  if(kind==="title")Object.assign(it,{text:a.text??"Your next move",fadeIn:6,fadeOut:6});
  if(kind==="motion")Object.assign(it,{from:a.from??1,to:a.to??1.32});
  if(kind==="media"){const asset=assetOfMedia(a.media);if(!asset)throw fail("NOT_FOUND",`No media ${a.media}.`,"project.info lists media IDs (med_…).");Object.assign(it,{asset,source:a.source|0,placement:a.placement||"card",muted:a.muted??true,behindPresenter:!!a.behindPresenter,crop:a.crop??null,fit:a.fit||"fill"});}
  checkLength(it);
  p.items.push(it);x.created.push(it.id);
  if(lanePinned)x.pinnedLane.add(it.id);
  return it;
}

/* ══ Command registry ══
   One entry per command. The UI, command palette, shortcuts, CLI and MCP edit.apply all call these.
   params: name → "type · description"; a leading ! marks required. */
const P = (o) => o;
const COMMANDS = {};
function cmd(id,def){COMMANDS[id]={id,...def};}

cmd("item.split",{title:"Split at Playhead",group:"Spine and any item",shortcut:"C",risk:"edit",status:"✗",
  ui:["Toolbar ✂","Shortcut C","Blade S / hold ⌥ + click","Clip menu ▸ Split at Playhead"],
  params:P({item:"!id · clip or overlay",at:"!frame"}),
  example:p=>({item:"clp_take",at:372}),
  run(p,a,x){
    const v=needAny(p,x.refs.resolve(a.item)), at=resolveFrame(p,a.at,x.refs);
    if(isClip(v)){const r=splitClip(p,v,at,x.mint,x.fx);x.created.push(r.id);x.primary=v.id;return `Split ${nameOf(v)} at ${clock(at)}`;}
    if(!isItem(v))throw fail("WRONG_KIND","Markers can't be split.","Split a clip or overlay.");noCaption(v,"item.split");
    const s=startOf(p,v);if(at<s+2||at>s+v.length-2)throw fail("BAD_ARGS",`Frame ${at} is not inside ${nameOf(v)}.`,"Pick a frame inside the item.");
    const cut=at-s, right={...copy(v),id:x.mint("ovl"),length:v.length-cut,fadeIn:0,anchor:v.anchor.pinned?{pinned:true,frame:at}:anchorAt(p,at)};
    if(v.kind==="motion"){const mid=v.from+(v.to-v.from)*cut/v.length;right.from=mid;v.to=mid;}
    if(v.kind==="media")right.source=(v.source||0)+cut;
    v.length=cut;v.fadeOut=0;p.items.push(right);x.created.push(right.id);x.primary=v.id;
    return `Split ${nameOf(v)} at ${clock(at)}`;
  }});
cmd("item.trim",{title:"Trim",group:"Spine and any item",shortcut:"Q / W",risk:"edit",status:"◐",
  ui:["Drag edge grip","Shortcut Q (start) / W (end) to playhead","Inspector Start / End"],
  params:P({item:"!id",edge:"!in|out",to:"frame · new edge position",delta:"int · frames to move the edge",ripple:"bool = true"}),
  example:p=>({item:"clp_take",edge:"out",delta:-60,ripple:true}),
  run(p,a,x){
    const v=needAny(p,x.refs.resolve(a.item)), ripple=a.ripple!==false;noCaption(v,"item.trim");
    if(!["in","out"].includes(a.edge))throw fail("BAD_ARGS","edge must be in or out.","Pass edge:\"in\" or edge:\"out\".");
    const s=startOf(p,v), e=s+lengthOf(p,v);
    const to=a.to!==undefined?resolveFrame(p,a.to,x.refs,"to"):(a.edge==="in"?s:e)+(a.delta|0);
    if(a.to===undefined&&a.delta===undefined)throw fail("BAD_ARGS","Pass to or delta.","to is an absolute frame; delta moves the edge by frames.");
    if(isClip(v)){
      if(a.edge==="out"){
        const d=to-e;v.length+=d;checkLength(v);checkSource(v);
        if(ripple)shiftAfter(p,e,d,v);else checkOverlap(p,v);
      }else{
        const d=to-s;v.source+=d*spd(v);v.length-=d;checkLength(v);checkSource(v);
        if(ripple)shiftAfter(p,e,-d,v);else{v.start+=d;checkOverlap(p,v);}
      }
      clampFades(v);return `Trimmed ${a.edge==="in"?"start":"end"} of ${nameOf(v)} to ${clock(to)}`+(ripple?" (ripple)":"");
    }
    if(!isItem(v))throw fail("WRONG_KIND","Markers have no length.","Use marker.set to move a marker.");
    if(a.edge==="out")v.length=to-s;
    else{v.length=e-to;if(v.kind==="media")v.source=Math.max(0,(v.source||0)+(to-s));v.anchor=v.anchor.pinned?{pinned:true,frame:to}:anchorAt(p,to);}
    checkLength(v);clampFades(v);return `Trimmed ${nameOf(v)}`;
  }});
cmd("item.move",{title:"Move",group:"Spine and any item",shortcut:", . nudge",risk:"edit",status:"✗",
  ui:["Drag horizontally (re-anchors overlays)","Hold ` while dragging a spine clip (overlays stay)","Shortcut , . (1 frame) ⇧, ⇧. (10)","Inspector Start (overlay)"],
  params:P({items:"!id[]",to:"frame · new start (first item)",delta:"int · frames",carryOverlays:"bool = true · false = grave-key override",ripple:"bool = true · spine reorders magnetically"}),
  example:p=>({items:["ovl_broll"],delta:30}),
  run(p,a,x){
    const ids=list(a.items).map(x.refs.resolve), objs=ids.map(id=>needAny(p,id));objs.forEach(o=>noCaption(o,"item.move"));
    if(a.to===undefined&&a.delta===undefined)throw fail("BAD_ARGS","Pass to or delta.","to is the new start frame; delta is a frame offset.");
    const delta=a.to!==undefined?resolveFrame(p,a.to,x.refs,"to")-startOf(p,objs[0]):(a.delta|0);
    const ripple=a.ripple!==false, carry=a.carryOverlays!==false;
    const clips=objs.filter(isClip), items=objs.filter(isItem), marks=objs.filter(o=>!isClip(o)&&!isItem(o));
    if(clips.length){
      const kept=carry?null:recordFrames(p,clips.flatMap(c=>childrenOf(p,c.id)));
      if(clips.length===1&&ripple){
        const n=clips[0], start=Math.max(0,n.start+delta);
        const overlap=p.clips.some(o=>o!==n&&o.start<start+n.length&&o.start+o.length>start);
        if(!overlap)n.start=start;
        else{
          const ordered=[...p.clips].sort((a,b)=>a.start-b.start), gaps=ordered.map((o,i)=>i?o.start-ordered[i-1].start-ordered[i-1].length:o.start);
          const rest=ordered.filter(o=>o!==n), center=start+n.length/2, idx=rest.findIndex(o=>center<o.start+o.length/2);
          rest.splice(idx<0?rest.length:idx,0,n);let cur=0;rest.forEach((o,i)=>{o.start=cur+gaps[i];cur=o.start+o.length;});
        }
      }else{for(const c of clips){c.start+=delta;if(c.start<0)throw fail("BAD_ARGS","Clips can't start before frame 0.","Use a smaller delta.");}for(const c of clips)checkOverlap(p,c);}
      if(kept)reanchorAt(p,kept,x.fx,"carryOverlays:false");
    }
    for(const it of items){const s=startOf(p,it)+delta;if(s<0)throw fail("BAD_ARGS","Items can't start before frame 0.","Use a smaller delta.");const before=it.anchor.clip;it.anchor=it.anchor.pinned?{pinned:true,frame:s}:anchorAt(p,s);if(before&&it.anchor.clip!==before)x.fx.push({id:it.id,label:nameOf(it),reanchored:{from:before,to:it.anchor.clip}});}
    for(const m of marks)m.frame=Math.max(0,m.frame+delta);
    return `Moved ${objs.length===1?nameOf(objs[0]):objs.length+" items"} ${delta>=0?"+":"−"}${Math.abs(delta)} f`;
  }});
cmd("item.delete",{title:"Delete",group:"Spine and any item",shortcut:"⌫ / ⇧⌫",risk:"destructive",status:"◐",
  ui:["Shortcut ⌫ (ripple) · ⇧⌫ (leave gap)","Toolbar 🗑","Clip menu ▸ Delete / Delete, Keep Overlays","Marker menu ▸ Delete"],
  params:P({items:"!id[]",ripple:"bool = true",overlays:"delete|keep = delete · keep leaves a gap clip carrying them"}),
  example:p=>({items:["ovl_label"],ripple:false}),
  run(p,a,x){
    const ids=list(a.items).map(x.refs.resolve), objs=ids.map(id=>needAny(p,id)), ripple=a.ripple!==false, keep=a.overlays==="keep";
    for(const c of objs.filter(isClip).sort((a,b)=>b.start-a.start))removeClip(p,c,ripple,keep,x.mint,x.fx);
    for(const it of objs.filter(isItem))if(p.items.includes(it))p.items.splice(p.items.indexOf(it),1);
    p.markers=p.markers.filter(m=>!ids.includes(m.id));
    return `Deleted ${objs.length===1?nameOf(objs[0]):objs.length+" items"}`+(keep?", kept overlays on a gap":objs.some(isClip)&&ripple?" and closed the gap":"");
  }});
cmd("item.duplicate",{title:"Duplicate",group:"Spine and any item",shortcut:"⌘D",risk:"edit",status:"✗",
  ui:["Shortcut ⌘D","Clip menu ▸ Duplicate","Copy / Paste ⌘C ⌘V"],
  params:P({items:"!id[]",at:"frame · paste position (default: right after the original)"}),
  example:p=>({items:["ovl_hook"]}),
  run(p,a,x){
    const objs=list(a.items).map(x.refs.resolve).map(id=>needAny(p,id));objs.forEach(o=>noCaption(o,"item.duplicate"));
    const at=resolveFrame(p,a.at,x.refs);
    const clips=objs.filter(isClip).sort((a,b)=>a.start-b.start);
    if(clips.length){
      const span=clips.at(-1).start+clips.at(-1).length-clips[0].start;
      let pos=at??clips.at(-1).start+clips.at(-1).length;
      const under=clipAt(p,pos);if(under&&pos!==under.start)pos=pos-under.start<under.length/2?under.start:under.start+under.length;
      shiftAfter(p,pos,span);
      for(const c of clips){
        const kids=childrenOf(p,c.id);
        const n={...copy(c),id:x.mint("clp"),start:pos+(c.start-clips[0].start)};p.clips.push(n);x.created.push(n.id);
        for(const k of kids){const d={...copy(k),id:x.mint("ovl"),anchor:{...k.anchor,clip:n.id}};p.items.push(d);x.created.push(d.id);}
      }
    }
    const items=objs.filter(isItem);
    if(items.length){
      const base=Math.min(...items.map(i=>startOf(p,i))), last=Math.max(...items.map(i=>startOf(p,i)+i.length)), pos=at??last;
      for(const it of items){const d={...copy(it),id:x.mint("ovl")};const f=pos+startOf(p,it)-base;d.anchor=it.anchor.pinned?{pinned:true,frame:f}:anchorAt(p,f);p.items.push(d);x.created.push(d.id);}
    }
    for(const m of objs.filter(o=>!isClip(o)&&!isItem(o))){const n={...m,id:x.mint("mrk"),frame:(at??m.frame+30)};p.markers.push(n);x.created.push(n.id);}
    return `Duplicated ${objs.length===1?nameOf(objs[0]):objs.length+" items"}`;
  }});
cmd("item.set",{title:"Set Properties",group:"Spine and any item",shortcut:"",risk:"edit",status:"◐",
  ui:["Inspector Name / Fade in / Fade out","Drag fade handle","Clip menu ▸ Color ▸ / Fades ▸"],
  params:P({items:"!id[]",name:"string|null",color:"red|orange|yellow|green|blue|purple|gray|null",fadeIn:"frames",fadeOut:"frames"}),
  example:p=>({items:["clp_take"],color:"blue",fadeOut:15}),
  run(p,a,x){
    const objs=list(a.items).map(x.refs.resolve).map(id=>needAny(p,id));
    for(const o of objs){
      if("name" in a)o.name=a.name||null;
      if("color" in a)o.color=a.color||null;
      if(a.fadeIn!==undefined){if(!Number.isInteger(a.fadeIn))throw fail("BAD_ARGS","fadeIn is in frames.","Use an integer frame count.");o.fadeIn=clamp(a.fadeIn,0,o.length-(a.fadeOut??o.fadeOut??0));}
      if(a.fadeOut!==undefined){if(!Number.isInteger(a.fadeOut))throw fail("BAD_ARGS","fadeOut is in frames.","Use an integer frame count.");o.fadeOut=clamp(a.fadeOut,0,o.length-(o.fadeIn||0));}
    }
    return `Updated ${objs.length===1?nameOf(objs[0]):objs.length+" items"}`;
  }});
cmd("clip.speed",{title:"Speed",group:"Spine and any item",shortcut:"",risk:"edit",status:"✅",
  ui:["Clip menu ▸ Speed ▸","Inspector Speed / Custom"],
  params:P({items:"!id[]",speed:"!number 0.25–16",ripple:"bool = true"}),
  example:p=>({items:["clp_take"],speed:1.5}),
  run(p,a,x){
    const s=+a.speed;if(!(s>=.25&&s<=16))throw fail("BAD_ARGS","speed must be 0.25–16.","Pass a number such as 1.5.");
    const clips=list(a.items).map(x.refs.resolve).map(id=>needClip(p,id)).sort((a,b)=>b.start-a.start);
    for(const c of clips){
      const length=Math.max(3,Math.round(c.length*spd(c)/s)), d=length-c.length, oldEnd=c.start+c.length;
      c.speed=s;c.length=length;
      if(a.ripple!==false)shiftAfter(p,oldEnd,d,c);else checkOverlap(p,c);
      clampFades(c);
    }
    return `Speed ${fmtSpeed(s)} on ${clips.length===1?nameOf(clips[0]):clips.length+" clips"}`;
  }});
cmd("clip.insert",{title:"Insert Media",group:"Spine and any item",shortcut:"",risk:"edit",status:"◐",
  ui:["Media ▸ Append","Spine lane menu ▸ Append …"],
  params:P({media:"!med_…",at:"frame (default: end of spine)",ripple:"bool = true"}),
  example:p=>({media:"med_webcam",at:{item:"clp_take",edge:"end"}}),
  run(p,a,x){
    const asset=assetOfMedia(a.media);if(!asset)throw fail("NOT_FOUND",`No media ${a.media}.`,"project.info lists media.");
    let at=resolveFrame(p,a.at,x.refs)??spineEnd(p);
    const under=clipAt(p,at);if(under&&at!==under.start)at=at-under.start<under.length/2?under.start:under.start+under.length;
    const c=clipOf(x.mint("clp"),asset,at);
    if(a.ripple!==false)shiftAfter(p,at,c.length);
    p.clips.push(c);if(a.ripple===false)checkOverlap(p,c);
    x.created.push(c.id);return `Inserted ${assets[asset].name} at ${clock(at)}`;
  }});
cmd("clip.lift",{title:"Lift to Overlay",group:"Lanes and anchoring",shortcut:"⌥⌘↑",risk:"edit",status:"★",
  ui:["Shortcut ⌥⌘↑","Clip menu ▸ Lift to Overlay"],
  params:P({items:"!clip id[]",lane:"lan_… (default: lowest free overlay lane)"}),
  example:p=>({items:["clp_end"]}),
  run(p,a,x){
    const clips=list(a.items).map(x.refs.resolve).map(id=>needClip(p,id));
    for(const c of clips){
      if(c.asset==="gap")throw fail("WRONG_KIND","A gap can't become an overlay.","Delete the gap instead.");
      const at=c.start, kids=recordFrames(p,childrenOf(p,c.id));
      p.clips.splice(p.clips.indexOf(c),1);shiftAfter(p,at+c.length,-c.length);
      reanchorAt(p,kids,x.fx,"parent lifted");
      const it=addItem(p,{media:c.asset,at,length:c.length,lane:a.lane,placement:"full",muted:false,source:Math.round(c.source),crop:c.crop??null,fit:c.fit||"fit"},"media",x);
      if(c.name)it.name=c.name;
    }
    return `Lifted ${clips.length===1?nameOf(clips[0]):clips.length+" clips"} to an overlay lane`;
  }});
cmd("overlay.replaceSpine",{title:"Replace Spine Clip",group:"Lanes and anchoring",shortcut:"⌥⌘↓",risk:"edit",status:"★",
  ui:["Shortcut ⌥⌘↓","Overlay menu ▸ Replace Spine Clip"],
  params:P({item:"!ovl_… media overlay",target:"clp_… (default: its anchor clip)"}),
  example:p=>({item:"ovl_broll"}),
  run(p,a,x){
    const it=needItem(p,x.refs.resolve(a.item),["media"]);
    const target=needClip(p,a.target?x.refs.resolve(a.target):it.anchor.clip||clipAt(p,startOf(p,it))?.id);
    const kids=recordFrames(p,childrenOf(p,target.id).filter(k=>k!==it));
    const n=clipOf(x.mint("clp"),it.asset,target.start,it.length,{source:it.source||0,crop:it.crop??null,fit:it.fit==="fill"?"fill":"fit"});
    if(assets[it.asset].type==="take")Object.assign(n,{enabled:Object.fromEntries(assets[it.asset].parts.map(k=>[k,true]))});
    p.clips.splice(p.clips.indexOf(target),1,n);
    shiftAfter(p,target.start+target.length,n.length-target.length,n);
    p.items.splice(p.items.indexOf(it),1);
    reanchorAt(p,kids,x.fx,"spine clip replaced");
    x.fx.push({id:target.id,label:nameOf(target),deleted:true,replacedBy:n.id});
    x.created.push(n.id);return `Replaced ${nameOf(target)} with ${nameOf(it)}`;
  }});
cmd("overlay.add",{title:"Add as Overlay",group:"Lanes and anchoring",shortcut:"",risk:"edit",status:"★",
  ui:["Media ▸ Add as Overlay","Overlay lane menu ▸ Add …"],
  params:P({kind:"!title|motion|media",media:"med_… when kind=media",at:"frame or anchor form",anchor:"{clip, sourceFrame} | {pinned:true, frame}",lane:"lan_… (explicit lane may raise LANE_CONFLICT)",length:"frames",text:"title text",from:"scale",to:"scale",placement:"full|card|corner-tl|corner-tr|corner-bl|corner-br",behindPresenter:"bool",muted:"bool = true"}),
  example:p=>({kind:"media",media:"med_broll",at:{marker:"mrk_2"},length:60,placement:"corner-br"}),
  run(p,a,x){
    if(!["title","motion","media"].includes(a.kind))throw fail("BAD_ARGS","kind must be title, motion or media.","Pass kind.");
    const it=addItem(p,a,a.kind,x);return `Added ${a.kind==="media"?"overlay “"+nameOf(it)+"”":a.kind==="title"?"title “"+it.text+"”":"scale animation"} on ${laneById(p,it.lane).name}`;
  }});
cmd("title.add",{title:"Add Title",group:"Titles, motion and effects",shortcut:"T",risk:"edit",status:"✅",
  ui:["Shortcut T","Toolbar T","Graphics lane menu ▸ Add Title Here"],
  params:P({at:"!frame or anchor form",length:"frames = 90",text:"string",lane:"lan_…"}),
  example:p=>({at:{word:"w12"},length:60,text:"Search “frame study”"}),
  run(p,a,x){const it=addItem(p,a,"title",x);return `Added title “${it.text}” on ${laneById(p,it.lane).name}`;}});
cmd("title.set",{title:"Edit Title",group:"Titles, motion and effects",shortcut:"",risk:"edit",status:"◐",
  ui:["Inspector Text","Double-click title"],
  params:P({item:"!ovl_… title",text:"!string"}),
  example:p=>({item:"ovl_hook",text:"Make it move!"}),
  run(p,a,x){const it=needItem(p,x.refs.resolve(a.item),["title"]);it.text=String(a.text);return `Title text “${it.text}”`;}});
cmd("motion.add",{title:"Add Scale Animation",group:"Titles, motion and effects",shortcut:"Z",risk:"edit",status:"✅",
  ui:["Shortcut Z","Toolbar ◆","Motion lane menu ▸ Add Scale Animation Here"],
  params:P({at:"!frame or anchor form",length:"frames = 120",from:"scale = 1",to:"scale = 1.32"}),
  example:p=>({at:{item:"clp_take",edge:"start",offset:200},length:90,from:1,to:1.4}),
  run(p,a,x){const it=addItem(p,a,"motion",x);return `Added scale animation ${fmtScale(it.from)}→${fmtScale(it.to)}`;}});
cmd("motion.set",{title:"Edit Scale Keyframes",group:"Titles, motion and effects",shortcut:"",risk:"edit",status:"✅",
  ui:["Inspector ◆ Start / ◆ End","Motion menu ▸ Reverse Scale"],
  params:P({item:"!ovl_… motion",from:"0.5–2",to:"0.5–2"}),
  example:p=>({item:"ovl_detail",from:1.16,to:1}),
  run(p,a,x){const it=needItem(p,x.refs.resolve(a.item),["motion"]);if(a.from!==undefined)it.from=clamp(+a.from,.5,2);if(a.to!==undefined)it.to=clamp(+a.to,.5,2);return `Scale ${fmtScale(it.from)}→${fmtScale(it.to)}`;}});
cmd("item.anchor",{title:"Pin / Unpin",group:"Lanes and anchoring",shortcut:"",risk:"edit",status:"★",
  ui:["Overlay menu ▸ Pin to Timeline / Anchor to Clip","Inspector Anchor"],
  params:P({item:"!ovl_…",pinned:"bool · pin at the current frame, or re-anchor to the clip under it",anchor:"{clip, sourceFrame} | {pinned:true, frame}"}),
  example:p=>({item:"ovl_broll",pinned:true}),
  run(p,a,x){
    const it=needItem(p,x.refs.resolve(a.item)), f=startOf(p,it);noCaption(it,"item.anchor");
    if(a.anchor)it.anchor=a.anchor.pinned?{pinned:true,frame:resolveFrame(p,a.anchor.frame??f,x.refs)}:{clip:needClip(p,x.refs.resolve(a.anchor.clip)).id,sourceFrame:a.anchor.sourceFrame|0};
    else if(a.pinned)it.anchor={pinned:true,frame:f};
    else it.anchor=anchorAt(p,f);
    return it.anchor.pinned?`Pinned ${nameOf(it)} at ${clock(startOf(p,it))}`:`Anchored ${nameOf(it)} to ${nameOf(clipById(p,it.anchor.clip))}`;
  }});
cmd("item.lane",{title:"Move to Lane",group:"Lanes and anchoring",shortcut:"⌥↑ / ⌥↓",risk:"edit",status:"★",
  ui:["Drag vertically","Shortcut ⌥↑ / ⌥↓","Overlay menu ▸ Lane Up / Lane Down"],
  params:P({items:"!ovl_…[]",lane:"lan_… explicit (LANE_CONFLICT if occupied)",dir:"up|down"}),
  example:p=>({items:["ovl_broll"],dir:"up"}),
  run(p,a,x){
    const its=list(a.items).map(x.refs.resolve).map(id=>needItem(p,id));
    for(const it of its){
      if(a.lane){const l=laneById(p,a.lane);if(!l)throw fail("NOT_FOUND",`No lane ${a.lane}.`,"project.info lists lanes.");if(l.kind!==laneKindFor(it.kind))throw fail("WRONG_KIND",`${l.name} holds ${l.kind} items.`,`Pick a ${laneKindFor(it.kind)} lane.`);it.lane=l.id;x.pinnedLane.add(it.id);continue;}
      const same=lanesOfKind(p,laneKindFor(it.kind)), i=same.findIndex(l=>l.id===it.lane);
      if(a.dir==="up"){
        const s=startOf(p,it),e=s+it.length;let t=null;for(let j=i-1;j>=0;j--)if(laneFree(p,same[j].id,s,e,new Set([it.id]))){t=same[j];break;}
        it.lane=(t||createLane(p,laneKindFor(it.kind),x.mint)).id;
      }else if(a.dir==="down"){
        const s=startOf(p,it),e=s+it.length;let t=null;for(let j=i+1;j<same.length;j++)if(laneFree(p,same[j].id,s,e,new Set([it.id]))){t=same[j];break;}
        if(!t)throw fail("LANE_CONFLICT","No free lane below.","This is the lowest free lane of its kind.");
        it.lane=t.id;
      }else throw fail("BAD_ARGS","Pass lane or dir.","dir is up or down.");
    }
    return `Moved ${its.length===1?nameOf(its[0]):its.length+" items"} to ${laneById(p,its[0].lane).name}`;
  }});
cmd("overlay.set",{title:"Overlay Settings",group:"Lanes and anchoring",shortcut:"",risk:"edit",status:"★",
  ui:["Inspector Placement / Behind Presenter / Audio"],
  params:P({item:"!ovl_… media",placement:"full|card|corner-tl|corner-tr|corner-bl|corner-br",behindPresenter:"bool",muted:"bool"}),
  example:p=>({item:"ovl_broll",placement:"full",behindPresenter:true}),
  run(p,a,x){
    const it=needItem(p,x.refs.resolve(a.item),["media"]);
    if(a.placement){if(!["full","card","corner-tl","corner-tr","corner-bl","corner-br"].includes(a.placement))throw fail("BAD_ARGS","Unknown placement.","full, card or corner-tl/tr/bl/br.");it.placement=a.placement;}
    if(a.behindPresenter!==undefined)it.behindPresenter=!!a.behindPresenter;
    if(a.muted!==undefined)it.muted=!!a.muted;
    return `Updated ${nameOf(it)}`;
  }});
cmd("lane.create",{title:"New Lane",group:"Lanes and anchoring",shortcut:"",risk:"edit",status:"★",
  ui:["Lane header menu ▸ New Lane Above"],
  params:P({kind:"!graphics|overlay|motion|captions",name:"string"}),
  example:p=>({kind:"overlay",name:"Callouts"}),
  run(p,a,x){if(!["graphics","overlay","motion","captions"].includes(a.kind))throw fail("BAD_ARGS","kind must be graphics, overlay, motion or captions.","Pass kind.");const l=createLane(p,a.kind,x.mint,a.name);x.created.push(l.id);return `Created lane ${l.name}`;}});
cmd("lane.rename",{title:"Rename Lane",group:"Lanes and anchoring",shortcut:"",risk:"edit",status:"★",
  ui:["Lane header menu ▸ Rename…","Double-click lane name"],
  params:P({lane:"!lan_…",name:"!string"}),
  example:p=>({lane:"lan_ov2",name:"B-roll"}),
  run(p,a,x){const l=laneById(p,a.lane);if(!l)throw fail("NOT_FOUND",`No lane ${a.lane}.`,"project.info lists lanes.");l.name=String(a.name||l.name);return `Renamed lane to ${l.name}`;}});
cmd("lane.delete",{title:"Delete Lane",group:"Lanes and anchoring",shortcut:"",risk:"destructive",status:"★",
  ui:["Lane header menu ▸ Delete Lane"],
  params:P({lane:"!lan_…"}),
  example:p=>({lane:"lan_ov2"}),
  run(p,a,x){
    const l=laneById(p,a.lane);if(!l)throw fail("NOT_FOUND",`No lane ${a.lane}.`,"project.info lists lanes.");
    const same=lanesOfKind(p,l.kind), held=p.items.filter(i=>i.lane===l.id);
    if(same.length===1&&held.length)throw fail("LANE_NOT_EMPTY",`${l.name} is the only ${l.kind} lane and holds ${held.length} item(s).`,"Delete or move its items first.");
    const into=same[same.indexOf(l)+1]||same[same.indexOf(l)-1];
    for(const it of held){it.lane=into.id;x.fx.push({id:it.id,label:nameOf(it),bumped:{from:l.id,to:into.id}});}
    p.lanes.splice(p.lanes.indexOf(l),1);return `Deleted lane ${l.name}`;
  }});
cmd("component.set",{title:"Show / Hide Component",group:"Take components",shortcut:"",risk:"edit",status:"✗",
  ui:["Component lane eye","Inspector component switch","Clip menu ▸ Mute Audio"],
  params:P({clip:"!clp_…",component:"!screen|camera|mic|system|pointer|keys",enabled:"!bool"}),
  example:p=>({clip:"clp_take",component:"keys",enabled:false}),
  run(p,a,x){const c=needClip(p,x.refs.resolve(a.clip));if(!(a.component in (c.enabled||{})))throw fail("WRONG_KIND",`${nameOf(c)} has no ${a.component} component.`,"items.inspect lists components.");c.enabled[a.component]=!!a.enabled;return `${parts[a.component]} ${a.enabled?"shown":"hidden"} on ${nameOf(c)}`;}});
cmd("audio.set",{title:"Volume",group:"Take components",shortcut:"",risk:"edit",status:"✗",
  ui:["Component menu ▸ Volume ▸","Inspector Volume"],
  params:P({clip:"!clp_…",component:"!mic|system",volume:"!0–2"}),
  example:p=>({clip:"clp_take",component:"system",volume:.5}),
  run(p,a,x){const c=needClip(p,x.refs.resolve(a.clip));if(!["mic","system"].includes(a.component)||!(a.component in c.enabled))throw fail("WRONG_KIND","No such audio component.","mic or system.");c.volume[a.component]=clamp(+a.volume,0,2);return `${parts[a.component]} volume ${Math.round(c.volume[a.component]*100)}%`;}});
cmd("cursor.set",{title:"Cursor Settings",group:"Take components",shortcut:"",risk:"edit",status:"◐",
  ui:["Inspector Size / Click Rings","Cursor menu ▸ Click Rings / Cursor Size ▸","Sugar for effect.set {target:clip, type:\"cursor\"}"],
  params:P({clip:"!clp_…",size:"0.5–4",clickRings:"bool"}),
  example:p=>({clip:"clp_take",size:2,clickRings:false}),
  run(p,a,x){const c=needClip(p,x.refs.resolve(a.clip)), q={};if(a.size!==undefined)q.size=clamp(+a.size,.5,4);if(a.clickRings!==undefined)q.clickRings=!!a.clickRings;upsertFx(p,c,c.id,"cursor",q,undefined,x);return `Cursor updated on ${nameOf(c)} (clip Cursor effect)`;}});
cmd("keys.set",{title:"Keystroke Chip Settings",group:"Take components",shortcut:"",risk:"edit",status:"✗",
  ui:["Inspector Chip size"],
  params:P({clip:"!clp_…",size:"0.5–3"}),
  example:p=>({clip:"clp_take",size:1.4}),
  run(p,a,x){const c=needClip(p,x.refs.resolve(a.clip));upsertFx(p,c,c.id,"keystrokes",{size:clamp(+a.size,.5,3)},undefined,x);return `Chip size ${fmtScale(clamp(+a.size,.5,3))}× (clip Keystroke Chips effect)`;}});
cmd("keys.hide",{title:"Hide Keystrokes",group:"Take components",shortcut:"",risk:"edit",status:"✗",
  ui:["Chip menu ▸ Hide This Keystroke","Inspector keystroke checklist","Keys menu ▸ Show All Keystrokes"],
  params:P({clip:"!clp_…",events:"!int[] · event indexes from takes.events",hidden:"!bool"}),
  example:p=>({clip:"clp_take",events:[2],hidden:true}),
  run(p,a,x){const c=needClip(p,x.refs.resolve(a.clip));const ev=list(a.events);const h=new Set(c.keyStyle.hidden);for(const i of ev){if(!assets[c.asset].keys[i])throw fail("NOT_FOUND",`No keystroke event ${i}.`,"takes.events lists indexes.");a.hidden?h.add(i):h.delete(i);}c.keyStyle.hidden=[...h].sort((a,b)=>a-b);return `${a.hidden?"Hid":"Showed"} ${ev.length} keystroke(s)`;}});
cmd("marker.add",{title:"Add Marker",group:"Markers, project, cutting",shortcut:"M",risk:"edit",status:"✅",
  ui:["Shortcut M","Toolbar ⬇","Ruler / lane menu ▸ Add Marker Here"],
  params:P({at:"!frame or anchor form",name:"string"}),
  example:p=>({at:{word:"w20"},name:"Save beat"}),
  run(p,a,x){const f=resolveFrame(p,a.at,x.refs);if(p.markers.some(m=>m.frame===f))throw fail("BAD_ARGS",`A marker already sits at ${clock(f)}.`,"Pick another frame.");const m={id:x.mint("mrk"),frame:f,name:a.name||"Marker "+(p.markers.length+1)};p.markers.push(m);p.markers.sort((a,b)=>a.frame-b.frame);x.created.push(m.id);return `Marker “${m.name}” at ${clock(f)}`;}});
cmd("marker.set",{title:"Edit Marker",group:"Markers, project, cutting",shortcut:"",risk:"edit",status:"✅",
  ui:["Inspector Name / Time","Drag marker in ruler"],
  params:P({marker:"!mrk_…",name:"string",at:"frame"}),
  example:p=>({marker:"mrk_2",name:"Demo beat (tight)"}),
  run(p,a,x){const m=p.markers.find(m=>m.id===a.marker);if(!m)throw fail("NOT_FOUND",`No marker ${a.marker}.`,"timeline.find {kind:\"marker\"}.");if(a.name!==undefined)m.name=a.name||"Marker";if(a.at!==undefined)m.frame=Math.max(0,resolveFrame(p,a.at,x.refs));p.markers.sort((a,b)=>a.frame-b.frame);return `Marker “${m.name}” at ${clock(m.frame)}`;}});
cmd("project.set",{title:"Output Shape",group:"Markers, project, cutting",shortcut:"",risk:"edit",status:"◐",
  ui:["Canvas inspector ▸ Output shape","Canvas inspector ▸ Agent access"],
  params:P({aspect:"9:16|4:5|1:1|4:3|16:9",access:"off|read|live"}),
  example:p=>({aspect:"1:1"}),
  run(p,a,x){if(a.aspect){if(!["9:16","4:5","1:1","4:3","16:9"].includes(a.aspect))throw fail("BAD_ARGS","aspect must be one of 9:16, 4:5, 1:1, 4:3, 16:9.","Pass one of those.");p.aspect=a.aspect;}if(a.access){if(x.source==="agent")throw fail("PERMISSION_DENIED","Agents can't change their own access level.","Ask the human to change it in the Canvas inspector.");p.access=a.access;}return a.aspect?`Canvas ${p.aspect}`:`Agent access ${p.access}`;}});
cmd("clip.crop",{title:"Crop",group:"Framing",shortcut:"⇧C",risk:"edit",status:"★",
  ui:["Viewer crop overlay (⇧C) ▸ Apply","Clip / overlay menu ▸ Crop…","Inspector ▸ Framing: Crop… / Reset / Fit · Fill","Canvas inspector ▸ Crop clip at playhead…"],
  params:P({items:"!id[] · spine clips or media overlays",rect:"{x,y,w,h} normalized to the source frame (0–1), or null for the full frame",fit:"fit|fill · how the cropped source sits in the canvas (or overlay box)"}),
  example:p=>({items:["clp_take"],rect:{x:.3,y:0,w:.4,h:1},fit:"fill"}),
  run(p,a,x){
    const objs=list(a.items).map(x.refs.resolve).map(id=>needAny(p,id));
    if(a.rect===undefined&&a.fit===undefined)throw fail("BAD_ARGS","Pass rect, fit or both.","rect is normalized {x,y,w,h}; null resets to the full frame.");
    if(a.rect){const r=a.rect;for(const k of ["x","y","w","h"])if(typeof r[k]!=="number"||!(r[k]>=0&&r[k]<=1))throw fail("BAD_ARGS",`rect.${k} must be a number from 0 to 1.`,"Coordinates are fractions of the source frame, so they survive proxies and resolution changes.");
      if(r.w<.05||r.h<.05||r.x+r.w>1.0001||r.y+r.h>1.0001)throw fail("BAD_ARGS","rect must stay inside the frame and be at least 5% on each side.","Clamp x+w and y+h to 1.");}
    if(a.fit!==undefined&&!["fit","fill"].includes(a.fit))throw fail("BAD_ARGS","fit must be fit or fill.","fit letterboxes; fill covers and trims the excess.");
    for(const o of objs){
      const asset=isClip(o)?o.asset:isItem(o)&&o.kind==="media"?o.asset:null;
      if(!asset||!(assets[asset].native||assets[asset].type==="file"))throw fail("WRONG_KIND",`${nameOf(o)} has no fixed source frame to crop.`,"Crop works on screen takes, video files and b-roll; generated graphics and titles adapt to the canvas instead.");
      if(a.rect!==undefined)o.crop=a.rect?{x:+a.rect.x.toFixed(4),y:+a.rect.y.toFixed(4),w:+a.rect.w.toFixed(4),h:+a.rect.h.toFixed(4)}:null;
      if(a.fit!==undefined)o.fit=a.fit;
    }
    return `${a.rect===null?"Reset crop on":a.rect?"Cropped":"Framing "+a.fit+" on"} ${objs.length===1?nameOf(objs[0]):objs.length+" items"}`;
  }});
cmd("cut.range",{title:"Delete Range",group:"Markers, project, cutting",shortcut:"",risk:"destructive",status:"✅",
  ui:["Ruler menu ▸ Delete Range In→Out","Command palette"],
  params:P({from:"!frame",to:"!frame",ripple:"bool = true"}),
  example:p=>({from:300,to:330}),
  run(p,a,x){
    const from=resolveFrame(p,a.from,x.refs,"from"), to=resolveFrame(p,a.to,x.refs,"to");
    if(!(to>from))throw fail("BAD_ARGS","to must be after from.","Pass an increasing range.");
    cutRange(p,from,to,a.ripple!==false,x);return `Deleted ${clock(from)}–${clock(to)} (${to-from} f)`;
  }});
cmd("cut.words",{title:"Delete Words",group:"Markers, project, cutting",shortcut:"",risk:"destructive",status:"✅",
  ui:["Spine lane menu ▸ Cut Filler Words","Command palette","Transcript panel ▸ strike words (M6)"],
  params:P({words:"!word id[]",transcriptHash:"string from transcript.get"}),
  example:p=>({words:WORDS.filter(w=>w.filler).map(w=>w.id),transcriptHash:TRANSCRIPT_HASH}),
  run(p,a,x){
    if(a.transcriptHash&&a.transcriptHash!==TRANSCRIPT_HASH)throw fail("TRANSCRIPT_CHANGED","Transcript changed since you read it.","Call transcript.get again.",{current:TRANSCRIPT_HASH});
    const ranges=[];
    for(const id of list(a.words)){const w=WORDS.find(w=>w.id===id);if(!w)throw fail("NOT_FOUND",`No word ${id}.`,"transcript.get lists words.");
      for(const c of p.clips.filter(c=>c.asset==="screen")){const s0=Math.max(w.at-2,c.source),s1=Math.min(w.at+w.dur+2,srcEnd(c));if(s1>s0)ranges.push([Math.round(tlAt(c,s0)),Math.round(tlAt(c,s1))]);}}
    ranges.sort((a,b)=>b[0]-a[0]);
    for(const [f,t] of ranges)if(t-f>=1)cutRange(p,f,t,true,x);
    return `Deleted ${list(a.words).length} word(s)`;
  }});
function cutRange(p,from,to,ripple,x){
  for(const f of [from,to]){const c=clipAt(p,f);if(c&&f>c.start&&f<c.start+c.length){if(f-c.start<1||c.start+c.length-f<1)continue;const cut=f-c.start,right={...copy(c),id:x.mint("clp"),start:f,source:c.source+cut*spd(c),length:c.length-cut,fadeIn:0};c.length=cut;c.fadeOut=0;p.clips.push(right);for(const it of childrenOf(p,c.id))if(it.anchor.sourceFrame>=right.source)it.anchor.clip=right.id;}}
  const inside=p.clips.filter(c=>c.start>=from&&c.start+c.length<=to);
  const kids=recordFrames(p,inside.flatMap(c=>childrenOf(p,c.id)));
  for(const c of inside)p.clips.splice(p.clips.indexOf(c),1);
  if(ripple)shiftAfter(p,to,-(to-from));
  for(const [id] of kids){const it=itemById(p,id);it.anchor=anchorAt(p,from);x.fx.push({id,label:nameOf(it),reanchored:{to:it.anchor.clip??"pinned",edge:"range removed its anchor frame"}});}
}
/* ══ Effects: Recorder's treatments as additive, stackable effects at clip, lane or project scope ══
   Components are what was recorded (screen, camera, mic, cursor path, keystrokes). Effects are how it is treated.
   A lane or project effect is a default; a clip overrides individual parameters (per-parameter diff), so a later
   change to an unrelated lane parameter still reaches the clip. Resolution order: project › lane › clip. */
const CAPTION_PRESETS = ["bold","karaoke","pop","box","clean","typewriter"];
const EFFECT_TYPES = {
  window:{title:"Window",group:"Frame",applies:"visual",desc:"Padding, background, rounded corners, shadow and inset around the picture (Recorder's Background + Frame). Applied after crop; the background stays put while zooms move the window.",
    params:{padding:{type:"number",def:.07,min:0,max:.3,step:.01,label:"Padding"},corner:{type:"number",def:.03,min:0,max:.12,step:.005,label:"Corners"},shadow:{type:"number",def:.6,min:0,max:1,step:.05,label:"Shadow"},inset:{type:"number",def:0,min:0,max:.02,step:.001,label:"Inset"},
      background:{type:"enum",def:"gradient",options:["gradient","color","none"],label:"Background"},color:{type:"color",def:"#5b6f63",label:"Color"},color2:{type:"color",def:"#1d2420",label:"Color 2"}}},
  cursor:{title:"Cursor",group:"Cursor",applies:"take",desc:"Scales the recorded cursor, adds click rings or hides it. Without it the cursor renders as recorded.",
    params:{size:{type:"number",def:1.5,min:.5,max:4,step:.1,label:"Size"},clickRings:{type:"bool",def:true,label:"Click rings"},hidden:{type:"bool",def:false,label:"Hide cursor"}}},
  clickZoom:{title:"Click Zoom",group:"Cursor",applies:"take",desc:"Zooms the window toward each recorded click and back (Recorder auto-zoom). The background stays put.",
    params:{scale:{type:"number",def:1.6,min:1.05,max:2.5,step:.05,label:"Scale"},hold:{type:"number",def:45,min:15,max:120,step:1,label:"Hold (f)"}}},
  camera:{title:"Camera Layout",group:"Camera",applies:"take",desc:"Places the camera bubble above the window, outside the padding. Without it the bubble uses the default corner.",
    params:{corner:{type:"enum",def:"br",options:["br","bl","tr","tl"],label:"Corner"},size:{type:"number",def:.24,min:.12,max:.45,step:.01,label:"Size"},roundness:{type:"number",def:.2,min:0,max:1,step:.05,label:"Roundness"},shadow:{type:"number",def:.5,min:0,max:1,step:.05,label:"Shadow"}}},
  captions:{title:"Caption Style",group:"Captions",applies:"caption",desc:"How captions look and animate: preset, size, position, colors, stroke, safe zone. Set it on the Captions lane for the whole track; a cue overrides single values.",
    params:{preset:{type:"enum",def:"bold",options:CAPTION_PRESETS,label:"Preset"},size:{type:"number",def:.075,min:.03,max:.14,step:.005,label:"Size"},position:{type:"number",def:.68,min:.1,max:.9,step:.01,label:"Position"},
      color:{type:"color",def:"#ffffff",label:"Text"},active:{type:"color",def:"#ffd23f",label:"Highlight"},stroke:{type:"number",def:.14,min:0,max:.3,step:.01,label:"Stroke"},uppercase:{type:"bool",def:true,label:"Uppercase"},safe:{type:"enum",def:"tiktok",options:["tiktok","reels","shorts","none"],label:"Safe zone"}}},
  keystrokes:{title:"Keystroke Chips",group:"Keystrokes",applies:"take",desc:"Styles the chips drawn from recorded keystrokes. Hiding single keystrokes stays a clip edit (keys.hide).",
    params:{size:{type:"number",def:1,min:.5,max:3,step:.1,label:"Size"},position:{type:"enum",def:"bottom-left",options:["bottom-left","bottom-center","top-left"],label:"Position"}}}
};
const PLANNED_EFFECTS = [["motionBlur","Motion Blur","Cursor","Blur on zoom, pan and cursor moves (Recorder Animation)"],["blurRegion","Blur Region","Privacy","Time-ranged blur or cover rectangle (Recorder masks)"],["spotlight","Spotlight","Privacy","Dim everything except a region or the focused window"],["deviceFrame","Device Frame","Frame","MacBook / browser chrome around the window"],["backgroundBlur","Background Blur","Frame","Blurred copy of the content as background"],["clickSound","Click Sound","Cursor","Audio effect on recorded clicks"],["grade","Color","Look","Project-wide color/look, evaluated last"]];
function fxOwner(p,id){
  if(id==="project")return p;
  if(id==="lan_spine")return p.spine;
  if(id?.startsWith("lan_"))return laneById(p,id);
  const x=byId(p,id);return isClip(x)||isItem(x)?x:null;
}
const ownerLabel=(p,id)=>id==="project"?"Project":id==="lan_spine"?"Video lane":id?.startsWith("lan_")?(laneById(p,id)?.name||id)+" lane":nameOf(byId(p,id));
/* The scopes that feed an item, outermost first. Project scope applies to spine clips; overlay items take their lane's. */
function fxChain(p,x){
  if(isClip(x))return [["project",p],["lan_spine",p.spine],[x.id,x]];
  if(isItem(x))return x.kind==="caption"?[["project",p],[x.lane,laneById(p,x.lane)],[x.id,x]]:[[x.lane,laneById(p,x.lane)],[x.id,x]];
  if(x===p.spine)return [["project",p],["lan_spine",p.spine]];
  if(x===p)return [["project",p]];
  if(x&&x.kind)return [[x.id,x]];
  return [];
}
function fxApplies(type,x){
  const a=EFFECT_TYPES[type]?.applies;if(!x)return true;
  if(isClip(x))return a==="take"?isTake(x):a==="caption"?false:x.asset!=="gap";
  if(isItem(x))return a==="caption"?x.kind==="caption":a==="take"?false:x.kind==="media";
  if(x.kind)return a==="caption"?x.kind==="captions":x.kind!=="captions";
  if(x.id==="lan_spine")return a!=="caption";
  return true;
}
/* Merge an effect type down the chain. null when no scope defines it or it does not apply. */
function resolveFx(p,x,type){
  if(!fxApplies(type,x))return null;
  const T=EFFECT_TYPES[type];let found=false,enabled=true;const params={},from={};
  for(const [k] of Object.entries(T.params)){params[k]=T.params[k].def;from[k]="default";}
  let enabledFrom=null;const scopes=[];
  for(const [sid,owner] of fxChain(p,x)){
    const inst=owner?.effects?.find(e=>e.type===type);if(!inst)continue;found=true;scopes.push(sid);
    if(inst.enabled!==undefined){enabled=inst.enabled;enabledFrom=sid;}
    for(const [k,v] of Object.entries(inst.params||{})){params[k]=v;from[k]=sid;}
  }
  return found?{type,enabled,enabledFrom,params,from,scopes}:null;
}
const fxOn=(p,x,type)=>{const r=resolveFx(p,x,type);return r&&r.enabled?r.params:null;};
/* Style effects (cursor, camera, keystrokes) fall back to defaults; window and click zoom are purely additive. */
const fxStyle=(p,x,type)=>{const r=resolveFx(p,x,type);if(r&&!r.enabled)return null;if(r)return r.params;return Object.fromEntries(Object.entries(EFFECT_TYPES[type].params).map(([k,d])=>[k,d.def]));};
function fxById(p,id){
  const owners=[["project",p],["lan_spine",p.spine],...p.lanes.map(l=>[l.id,l]),...p.clips.map(c=>[c.id,c]),...p.items.map(i=>[i.id,i])];
  for(const [oid,o] of owners){const e=o?.effects?.find(e=>e.id===id);if(e)return {owner:o,ownerId:oid,fx:e};}
  return null;
}
function needOwner(p,id,x){
  const o=fxOwner(p,x.refs.resolve(id));
  if(!o)throw fail("NOT_FOUND",`No effect target ${id}.`,"Targets: a clip (clp_), media or title overlay (ovl_), a lane (lan_…, lan_spine) or \"project\".");
  return o;
}
function checkParams(type,params){
  const T=EFFECT_TYPES[type];
  for(const [k,v] of Object.entries(params||{})){
    const d=T.params[k];if(!d)throw fail("BAD_ARGS",`${T.title} has no parameter ${k}.`,`Parameters: ${Object.keys(T.params).join(", ")}.`);
    if(d.type==="number"&&!(typeof v==="number"&&v>=d.min&&v<=d.max))throw fail("BAD_ARGS",`${k} must be a number from ${d.min} to ${d.max}.`,`Default ${d.def}.`);
    if(d.type==="bool"&&typeof v!=="boolean")throw fail("BAD_ARGS",`${k} must be true or false.`,"Pass a boolean.");
    if(d.type==="enum"&&!d.options.includes(v))throw fail("BAD_ARGS",`${k} must be one of ${d.options.join(", ")}.`,`Default ${d.def}.`);
    if(d.type==="color"&&!/^#[0-9a-f]{6}$/i.test(v))throw fail("BAD_ARGS",`${k} must be a #rrggbb color.`,`Default ${d.def}.`);
  }
}
function needType(type){if(!EFFECT_TYPES[type])throw fail(PLANNED_EFFECTS.some(t=>t[0]===type)?"PLANNED":"BAD_ARGS",`No effect type ${type}.`,`Types: ${Object.keys(EFFECT_TYPES).join(", ")}. commands.describe {id:"effect.add"} lists parameters.`);return EFFECT_TYPES[type];}
/* Write params into the owner's instance of a type, creating it if needed. Used by every effect command. */
function upsertFx(p,owner,ownerId,type,params,enabled,x){
  if(!fxApplies(type,owner))throw fail("WRONG_KIND",`${EFFECT_TYPES[type].title} doesn't apply to ${ownerLabel(p,ownerId)}.`,EFFECT_TYPES[type].applies==="take"?"It needs a screen take with recorded events.":EFFECT_TYPES[type].applies==="caption"?"Caption Style applies to a Captions lane, a caption cue or the project.":"Window applies to spine clips and media overlays.");
  checkParams(type,params);
  owner.effects??=[];
  let e=owner.effects.find(e=>e.type===type);
  if(!e){e={id:x.mint("fx"),type,params:{}};owner.effects.push(e);x.created.push(e.id);}
  Object.assign(e.params,params||{});
  if(enabled!==undefined)e.enabled=!!enabled;
  return e;
}
const fxTypesDoc=Object.entries(EFFECT_TYPES).map(([k,t])=>`${k} (${Object.keys(t.params).join(", ")})`).join("; ");
cmd("effect.add",{title:"Add Effect",group:"Effects",shortcut:"",risk:"edit",status:"★",
  ui:["Effects browser ▸ drag onto a clip, lane header or the viewer","Effects browser ▸ Selection / Lane / Project","Inspector ▸ Effects ▸ + Effect","Double-click an effect: apply to selection"],
  params:P({target:"!clp_… | ovl_… | lan_… | lan_spine | project",type:"!"+Object.keys(EFFECT_TYPES).join("|"),params:"object · only the values you want to set; the rest inherit",enabled:"bool = true · false turns an inherited effect off at this scope"}),
  example:p=>({target:"lan_spine",type:"clickZoom",params:{scale:1.4}}),
  run(p,a,x){const id=x.refs.resolve(a.target), o=needOwner(p,a.target,x);needType(a.type);const e=upsertFx(p,o,id,a.type,a.params,a.enabled,x);x.primary=e.id;
    return `${a.enabled===false?"Turned off":"Added"} ${EFFECT_TYPES[a.type].title} on ${ownerLabel(p,id)}`;}});
cmd("effect.set",{title:"Set Effect Parameters",group:"Effects",shortcut:"",risk:"edit",status:"★",
  ui:["Inspector ▸ Effects ▸ parameter controls (editing an inherited value overrides it here)","Inspector ▸ enable checkbox","Parameter dot ▸ Reset to inherited"],
  params:P({effect:"fx_… id, or use target + type",target:"owner id (creates the instance when missing)",type:"effect type with target",params:"object",enabled:"bool",reset:"string[] · parameter names to drop so they inherit again"}),
  example:p=>({target:"clp_take",type:"window",params:{padding:.12}}),
  run(p,a,x){
    let owner,ownerId,e;
    if(a.effect){const f=fxById(p,x.refs.resolve(a.effect));if(!f)throw fail("NOT_FOUND",`No effect ${a.effect}.`,"items.inspect lists effects with IDs.");({owner,ownerId}=f);e=f.fx;checkParams(e.type,a.params);Object.assign(e.params,a.params||{});if(a.enabled!==undefined)e.enabled=!!a.enabled;}
    else{if(!a.target||!a.type)throw fail("BAD_ARGS","Pass effect, or target and type.","effect is an fx_ id.");ownerId=x.refs.resolve(a.target);owner=needOwner(p,a.target,x);needType(a.type);e=upsertFx(p,owner,ownerId,a.type,a.params,a.enabled,x);}
    for(const k of list(a.reset))delete e.params[k];
    if(list(a.reset).length&&!Object.keys(e.params).length&&e.enabled===undefined)owner.effects.splice(owner.effects.indexOf(e),1);
    return `${EFFECT_TYPES[e.type].title} on ${ownerLabel(p,ownerId)}: ${a.enabled===false?"off":a.enabled===true?"on":list(a.reset).length?"reset "+list(a.reset).join(", "):Object.keys(a.params||{}).join(", ")}`;
  }});
cmd("effect.remove",{title:"Remove Effect",group:"Effects",shortcut:"",risk:"edit",status:"★",
  ui:["Inspector ▸ effect menu ▸ Remove / Reset to inherited","Clip menu ▸ Remove Effects"],
  params:P({effects:"fx_…[] ids",target:"owner id · with type, or alone to remove all its effects",type:"effect type"}),
  example:p=>({target:"clp_open",type:"window"}),
  run(p,a,x){
    let n=0;
    for(const id of list(a.effects)){const f=fxById(p,x.refs.resolve(id));if(!f)throw fail("NOT_FOUND",`No effect ${id}.`,"items.inspect lists effect IDs.");f.owner.effects.splice(f.owner.effects.indexOf(f.fx),1);n++;}
    if(a.target){const o=needOwner(p,a.target,x);const before=(o.effects||[]).length;o.effects=(o.effects||[]).filter(e=>a.type&&e.type!==a.type);n+=before-o.effects.length;}
    return `Removed ${n} effect${n===1?"":"s"}`;
  }});
cmd("effect.paste",{title:"Paste Effects",group:"Effects",shortcut:"⌥⌘V",risk:"edit",status:"★",
  ui:["Shortcut ⌥⌘V (after ⌘C)","Clip menu ▸ Paste Effects"],
  params:P({from:"!owner id to copy local effects from",to:"!owner id[]",mode:"merge|replace = merge · merge keeps the target's other values"}),
  example:p=>({from:"clp_open",to:["clp_end"]}),
  run(p,a,x){
    const src=needOwner(p,a.from,x), effs=src.effects||[];if(!effs.length)throw fail("BAD_ARGS",`${ownerLabel(p,x.refs.resolve(a.from))} has no effects of its own.`,"Copy from an item with local effects.");
    const targets=list(a.to).map(id=>[x.refs.resolve(id),needOwner(p,id,x)]);let n=0;
    for(const [tid,t] of targets){if(a.mode==="replace")t.effects=[];for(const e of effs){if(!fxApplies(e.type,t))continue;upsertFx(p,t,tid,e.type,copy(e.params),e.enabled,x);n++;}}
    return `Pasted ${effs.length} effect(s) onto ${targets.length} item(s)`;
  }});
cmd("effect.promote",{title:"Promote Effect",group:"Effects",shortcut:"",risk:"edit",status:"★",
  ui:["Inspector ▸ effect menu ▸ Use for Whole Lane / Use for Project"],
  params:P({effect:"!fx_… on a clip or lane",to:"!lane|project",clearOverrides:"bool = false · also drop the same type's overrides on sibling clips"}),
  example:p=>({effect:"fx_take_win",to:"lane"}),
  run(p,a,x){
    const f=fxById(p,x.refs.resolve(a.effect));if(!f)throw fail("NOT_FOUND",`No effect ${a.effect}.`,"items.inspect lists effect IDs.");
    let tid;if(a.to==="project")tid="project";else if(a.to==="lane")tid=isClip(f.owner)?"lan_spine":isItem(f.owner)?f.owner.lane:null;
    if(!tid||tid===f.ownerId)throw fail("BAD_ARGS","Nothing to promote to.","Promote clip effects to their lane, or lane effects to the project.");
    const t=fxOwner(p,tid);upsertFx(p,t,tid,f.fx.type,copy(f.fx.params),f.fx.enabled,x);
    f.owner.effects.splice(f.owner.effects.indexOf(f.fx),1);
    if(a.clearOverrides){const sibs=tid==="project"||tid==="lan_spine"?p.clips:p.items.filter(i=>i.lane===tid);for(const s of sibs)if(s.effects)s.effects=s.effects.filter(e=>e.type!==f.fx.type);}
    return `${EFFECT_TYPES[f.fx.type].title} now applies to ${ownerLabel(p,tid)}`;
  }});

/* ══ Captions: word-bound cues on a Captions lane ══
   A cue lists transcript word IDs. Its start and length are derived after every batch (syncCaptions) from where
   those words sit on the timeline, so cuts, speed changes and moves of the take carry captions with them, and a cue
   whose words were all cut is removed (a side effect, undoable). Text edits live on the cue, per word, and never
   change the transcript or its hash. cue.cut caches which words are off the timeline (derived, rewritten every batch). Look and animation come from the Caption Style effect: project › lane › cue. */
const wordById = id => WORDS.find(w=>w.id===id);
function wordSpan(p,w){
  if(!w)return null;
  for(const c of p.clips){
    if(!(assets[c.asset].words||[]).includes(w))continue;
    if(w.at>=c.source&&w.at<srcEnd(c)){const f0=Math.round(tlAt(c,w.at)), f1=Math.round(tlAt(c,Math.min(w.at+w.dur,srcEnd(c))));return {clip:c.id,f0,f1:Math.max(f0+1,f1)};}
  }
  return null;
}
function visibleWords(p,clips,fillers){
  const out=[], seen=new Set();
  for(const c of clips)for(const w of assets[c.asset].words||[]){
    if(seen.has(w.id)||(!fillers&&w.filler))continue;
    const s=wordSpan(p,w);if(s&&s.clip===c.id){seen.add(w.id);out.push({w,...s});}
  }
  return out.sort((a,b)=>a.f0-b.f0);
}
/* Break at maxWords, at a sentence end, after a comma once the cue has two words, at a pause over 12 frames, or past 22 characters. */
function cueGroups(p,ws,maxWords){
  const groups=[];let cur=[];
  ws.forEach((o,i)=>{
    const prev=cur.at(-1), chars=cur.reduce((n,q)=>n+q.w.text.length+1,0)+o.w.text.length;
    if(cur.length&&(cur.length>=maxWords||prev.w.end||(prev.w.pause&&cur.length>1)||o.f0-prev.f1>12||chars>22)){groups.push(cur);cur=[];}
    cur.push(o);
  });
  if(cur.length)groups.push(cur);
  return groups;
}
function cueTokens(p,cue){
  return cue.words.map(id=>{const w=wordById(id), s=wordSpan(p,w);if(!s)return null;const e=cue.edits?.[id]||{};
    return {id,text:e.text??w.text,emph:!!e.emph,emoji:e.emoji||"",hidden:!!e.hidden,f0:s.f0,f1:s.f1,clip:s.clip};}).filter(Boolean);
}
const cueText = cue => cue.words.map(id=>{const e=cue.edits?.[id]||{};return e.hidden||cue.cut?.includes(id)?"":(e.text??wordById(id)?.text??"")+(e.emoji?" "+e.emoji:"");}).filter(Boolean).join(" ");
function syncCaptions(p,fx){
  for(const cue of p.items.filter(i=>i.kind==="caption")){
    const t=cueTokens(p,cue);
    if(!t.length){p.items.splice(p.items.indexOf(cue),1);fx.push({id:cue.id,label:nameOf(cue),deleted:true,why:"all of its words were cut"});continue;}
    const cut=cue.words.filter(id=>!t.some(q=>q.id===id));if(cut.length)cue.cut=cut;else delete cue.cut;
    const first=t.reduce((a,b)=>b.f0<a.f0?b:a), end=Math.max(...t.map(q=>q.f1));
    cue.anchor={clip:first.clip,sourceFrame:wordById(first.id).at};
    cue.length=Math.max(3,end-first.f0);
  }
}
function noCaption(x,cmdId){
  if(isItem(x)&&x.kind==="caption")throw fail("WRONG_KIND",`${cmdId} doesn't apply to captions: a caption's timing comes from its words.`,"Use caption.split or caption.merge to regroup words, cut.words to remove them, or item.delete to drop the cue.");
}
function needCue(p,id,x){return needItem(p,x.refs.resolve(id),["caption"]);}
const firstCue = p => p.items.find(i=>i.kind==="caption")?.id||"ovl_cap1";

cmd("captions.generate",{title:"Generate Captions",group:"Captions",shortcut:"",risk:"edit",status:"★",
  ui:["Viewer ▸ CC Captions ▸ Generate / Regenerate","Captions lane menu ▸ Regenerate ▸ 1–6 words","Command palette"],
  params:P({clips:"clp_…[] · takes to caption (default: every clip with a transcript)",maxWords:"int 1–8 = 3 · words per caption",fillers:"bool = false · caption um / uh too",lane:"lan_… captions lane (default: the first, created if missing)",style:"preset · "+CAPTION_PRESETS.join("|")+" · sets the lane's Caption Style"}),
  example:p=>({maxWords:2,style:"pop"}),
  run(p,a,x){
    const maxW=a.maxWords??3;
    if(!Number.isInteger(maxW)||maxW<1||maxW>8)throw fail("BAD_ARGS","maxWords must be an integer from 1 to 8.","3 reads well on vertical video; 1 is one word at a time.");
    if(a.style!==undefined&&!CAPTION_PRESETS.includes(a.style))throw fail("BAD_ARGS",`style must be one of ${CAPTION_PRESETS.join(", ")}.`,"commands.describe {id:\"effect.add\"} lists Caption Style parameters.");
    const clips=a.clips?list(a.clips).map(id=>needClip(p,x.refs.resolve(id))):p.clips.filter(c=>assets[c.asset].words?.length);
    if(!clips.length)throw fail("NOT_FOUND","No clip with a transcript is on the timeline.","Insert a take first: clip.insert {media:\"med_screen\"}.");
    let lane;
    if(a.lane){lane=laneById(p,x.refs.resolve(a.lane));if(!lane)throw fail("NOT_FOUND",`No lane ${a.lane}.`,"project.info lists lanes.");if(lane.kind!=="captions")throw fail("WRONG_KIND",`${lane.name} is a ${lane.kind} lane.`,"Pass a captions lane or omit lane.");}
    else lane=lanesOfKind(p,"captions")[0]||createLane(p,"captions",x.mint);
    const ws=visibleWords(p,clips,!!a.fillers), ids=new Set(ws.map(o=>o.w.id));
    if(!ws.length)throw fail("NOT_FOUND","Those clips have no words on the timeline.","Every word is cut or the clips have no speech.");
    for(const old of p.items.filter(i=>i.kind==="caption"&&i.lane===lane.id&&i.words.some(id=>ids.has(id)))){p.items.splice(p.items.indexOf(old),1);x.fx.push({id:old.id,label:nameOf(old),deleted:true,why:"regenerated"});}
    const groups=cueGroups(p,ws,maxW);
    for(const g of groups){const it=itemOf(x.mint("ovl"),"caption",lane.id,{clip:g[0].clip,sourceFrame:g[0].w.at},3,{words:g.map(o=>o.w.id),edits:{}});p.items.push(it);x.created.push(it.id);}
    if(a.style||!lane.effects?.some(e=>e.type==="captions"))upsertFx(p,lane,lane.id,"captions",{preset:a.style??"bold"},undefined,x);
    x.primary=lane.id;
    return `Generated ${groups.length} captions on ${lane.name} (${maxW} word${maxW>1?"s":""} max)`;
  }});
cmd("caption.set",{title:"Edit Caption",group:"Captions",shortcut:"",risk:"edit",status:"★",
  ui:["Inspector ▸ Caption text","Inspector ▸ word chips: text, emphasis, emoji, hide","Double-click a caption block","Caption menu ▸ Emphasize Word"],
  params:P({item:"!ovl_… caption",text:"string · retype the cue; words map onto its words in order, extra text joins the last word, missing words are hidden",words:"{wordId:{text?, emph?, emoji?, hidden?} | null} · per-word display edits; null resets the word"}),
  example:p=>{const c=byId(p,firstCue(p));return {item:c?.id,words:{[c?.words.at(-1)]:{emph:true,emoji:"⚡"}}};},
  run(p,a,x){
    const it=needCue(p,a.item,x);it.edits??={};
    const tidy=id=>{if(it.edits[id]&&!Object.keys(it.edits[id]).length)delete it.edits[id];};
    if(a.text!==undefined){
      const toks=String(a.text).trim().split(/\s+/).filter(Boolean), ids=it.words.filter(id=>wordSpan(p,wordById(id)));
      if(!toks.length)throw fail("BAD_ARGS","text can't be empty.","Hide words with words:{id:{hidden:true}}, or item.delete the cue.");
      ids.forEach((id,i)=>{const e=it.edits[id]??={}, t=i<ids.length-1?toks[i]:(toks.slice(i).join(" ")||undefined);
        if(t===undefined){e.hidden=true;delete e.text;}else{delete e.hidden;if(t===wordById(id).text)delete e.text;else e.text=t;}tidy(id);});
    }
    for(const [id,v] of Object.entries(a.words||{})){
      if(!it.words.includes(id))throw fail("NOT_FOUND",`${id} isn't in this caption.`,"items.inspect shows the cue's words; transcript.get {captions:\"json\"} lists every cue.");
      if(v===null){delete it.edits[id];continue;}
      const e=it.edits[id]??={};
      for(const k of ["text","emph","emoji","hidden"])if(k in v){const val=v[k];
        if(val===null||val===false||val===""||(k==="text"&&val===wordById(id).text))delete e[k];
        else e[k]=k==="emph"||k==="hidden"?true:String(val).slice(0,k==="emoji"?8:40);}
      tidy(id);
    }
    return `Caption “${nameOf(it)}”`;
  }});
cmd("caption.split",{title:"Split Caption",group:"Captions",shortcut:"",risk:"edit",status:"★",
  ui:["Inspector ▸ word chip ▸ Split Before","Caption menu ▸ Split at Playhead","Shortcut C on a caption"],
  params:P({item:"!ovl_… caption",word:"!word id that starts the new caption"}),
  example:p=>{const c=byId(p,firstCue(p));return {item:c?.id,word:c?.words[1]};},
  run(p,a,x){
    const it=needCue(p,a.item,x), i=it.words.indexOf(a.word);
    if(i<1)throw fail("BAD_ARGS",i===0?"That word already starts the caption.":`${a.word} isn't in this caption.`,"Pass a word after the first one.");
    const right=itemOf(x.mint("ovl"),"caption",it.lane,{...it.anchor},3,{words:it.words.slice(i),edits:Object.fromEntries(Object.entries(it.edits||{}).filter(([k])=>it.words.indexOf(k)>=i)),effects:copy(it.effects||[])});
    for(const k of right.words)delete it.edits?.[k];
    it.words=it.words.slice(0,i);p.items.push(right);x.created.push(right.id);x.primary=it.id;
    return `Split caption before “${wordById(a.word).text}”`;
  }});
cmd("caption.merge",{title:"Merge Captions",group:"Captions",shortcut:"",risk:"edit",status:"★",
  ui:["Caption menu ▸ Merge with Next","Select captions ▸ menu ▸ Merge Captions"],
  params:P({items:"!ovl_…[] · two or more captions on one lane"}),
  example:p=>({items:p.items.filter(i=>i.kind==="caption").slice(0,2).map(i=>i.id)}),
  run(p,a,x){
    const cues=list(a.items).map(id=>needCue(p,id,x)).sort((a,b)=>startOf(p,a)-startOf(p,b));
    if(cues.length<2)throw fail("BAD_ARGS","Pass two or more captions.","items:[first, second]");
    if(cues.some(c=>c.lane!==cues[0].lane))throw fail("WRONG_KIND","Those captions are on different lanes.","Merge captions on one lane.");
    const keep=cues[0];
    for(const c of cues.slice(1)){keep.words.push(...c.words);Object.assign(keep.edits,c.edits);p.items.splice(p.items.indexOf(c),1);}
    keep.words.sort((a,b)=>WORDS.indexOf(wordById(a))-WORDS.indexOf(wordById(b)));x.primary=keep.id;
    return `Merged ${cues.length} captions`;
  }});

/* In the registry, not built in this prototype. Listed so the parity table and commands.describe stay complete. */
const PLANNED = [
  ["clip.join","Join Clips","Spine and any item","⌘J","✗","Join adjacent clips from the same source into one"],
  ["clip.ungroup","Ungroup","Spine and any item","⇧⌘J","✗","Split a joined clip back into its parts"],
  ["clip.restore","Restore Cuts","Spine and any item","","✗","Seam bubble ▸ Restore; Restore All Cuts"],
  ["lane.reorder","Reorder Lane","Lanes and anchoring","","★","Drag lane header"],
  ["crop.keys","Animated Crop","Framing","","◐","Crop overlay ▸ Add keyframe (Recorder reframe / cropKeys)"],
  ["transition.add","Add Transition","Spine and any item","⌘T","✗","Edit point menu ▸ Crossfade; today: Lift to Overlay, overlap, fade in"],
  ["zoom.set","Zoom Settings","Titles, motion and effects","","◐","Context: Enable/Disable, Instant, Auto/Manual"],
  ["mask.add","Add Mask","Titles, motion and effects","","◐","Pencil · Cover/Blur/Highlight"],
  ["mask.set","Edit Mask","Titles, motion and effects","","◐","Inspector rect / opacity"],
  ["captions.translate","Translate Captions","Captions","","◐","Captions lane menu ▸ Translate… (new captions lane, same word timing)"],
  ["captions.import","Import SRT / VTT","Captions","","◐","Captions ▸ Import… (cues aligned to transcript words)"],
  ["audio.link","Link Audio","Take components","⌘L","✗","Link to… / Move into Sync"],
  ["audio.unlink","Unlink Audio","Take components","","✗","Clip menu ▸ Unlink"],
  ["audio.sync","Move into Sync","Take components","","✗","Out-of-sync badge ▸ Move into Sync"],
  ["style.set","Style","Markers, project, cutting","","✅","Background, frame, animation, camera tabs"],
  ["cut.pauses","Cut Pauses","Markers, project, cutting","","✅","Pause eraser"],
  ["intent.pace","Tighten Pace","Markers, project, cutting","","✅","Command palette with preview"],
  ["intent.fitDuration","Fit to Duration","Markers, project, cutting","","✅","Command palette with preview"],
  ["intent.keepShots","Keep Shots","Markers, project, cutting","","✅","Command palette with preview"],
  ["intent.autoZoom","Auto Zoom","Markers, project, cutting","","✅","Command palette with preview"]
];
for(const [id,title,group,shortcut,status,ui] of PLANNED)
  cmd(id,{title,group,shortcut,status,risk:"edit",ui:[ui],planned:true,params:{},example:()=>({}),run(){throw fail("PLANNED",`${id} is in the registry but not built in this prototype.`,"See the Spec tab ▸ Command map for its planned parameters.");}});

/* ══ Apply engine: atomic batches, refs, deterministic IDs, side effects, diff ══ */
let SERIAL = 100;
function minter(recorded){
  const used=[];let i=0;
  const mint=prefix=>{let id;if(recorded&&i<recorded.length)id=recorded[i++];else id=prefix+"_"+(SERIAL++).toString(36);used.push(id);return id;};
  return {mint,used};
}
function applyOps(base,ops,{source="human",replay=null}={}){
  const p=copy(base), fx=[], changes=[], refs={}, pinnedLane=new Set();
  const mints=[];
  const R={resolve:v=>{
    if(typeof v!=="string"||!v.startsWith("$"))return v;
    const [name,field]=v.split(".");const r=refs[name];
    if(!r)throw fail("BAD_ARGS",`Unknown ref ${name}.`,"Refs name items created earlier in the same batch (ref:\"$name\").");
    return field==="right"||field==="new"?r.created[0]:r.primary;
  }};
  ops.forEach((op,i)=>{
    const def=COMMANDS[op.cmd];
    if(!def){
      const close=Object.keys(COMMANDS).filter(k=>k.split(".")[0]===String(op.cmd).split(".")[0]||k.includes(String(op.cmd).split(".").pop()));
      throw fail("UNKNOWN_COMMAND",`No command ${op.cmd}.`,"commands.describe lists the registry.",{op:i,valid:close.length?close:Object.keys(COMMANDS).slice(0,12)});
    }
    const args={...op};delete args.cmd;delete args.ref;
    for(const [k,v] of Object.entries(def.params||{}))if(v.startsWith("!")&&args[k]===undefined)throw fail("BAD_ARGS",`${op.cmd} needs ${k}.`,`${k}: ${v.slice(1)}`,{op:i});
    const m=minter(replay?.[i]);
    const x={mint:m.mint,fx,created:[],primary:null,refs:R,pinnedLane,source};
    let summary;
    try{summary=def.run(p,args,x);}catch(e){e.op=i;throw e;}
    mints.push(m.used);
    if(op.ref)refs[op.ref]={primary:x.primary||x.created[0]||list(args.item??args.items)[0],created:x.created};
    changes.push({op:i,cmd:op.cmd,summary,...(x.created.length?{created:x.created}:{})});
  });
  const m=minter(replay?.[ops.length]);
  syncCaptions(p,fx);fixAnchors(p,fx);settle(p,fx,m.mint,pinnedLane,base);
  {const seen=new Set();for(const o of [p,p.spine,...p.lanes,...p.clips,...p.items])for(const e of o?.effects||[]){if(seen.has(e.id))e.id=m.mint("fx");seen.add(e.id);}}
  mints.push(m.used);
  return {project:p,changes,sideEffects:fx,mints};
}

/* ══ Store: revisions, human undo stack, command log, agent sessions ══ */
const Store = {
  project:sampleProject(), rev:1, at:{}, history:[], future:[], log:[], sessions:{}, idem:{}, listeners:[], lastKey:null, badges:new Map(),
  on(fn){this.listeners.push(fn);},
  emit(ev){for(const fn of this.listeners)fn(ev);},
  /* One entry point for every write. ops: [{cmd,...args}] */
  apply(ops,{source="human",sessionId=null,label=null,dryRun=false,key=null}={}){
    const before=this.project;
    const r=applyOps(before,ops,{source});
    const result={revision:"r"+(dryRun?this.rev:this.rev+1),dryRun,changes:r.changes,sideEffects:r.sideEffects,duration:{before:endOf(before),after:endOf(r.project)},warnings:[]};
    const cascade=r.sideEffects.filter(s=>s.deleted&&s.cascadeFrom);
    if(cascade.length&&source==="agent")result.warnings.push(`${cascade.length} anchored overlay(s) were deleted with their parent clip. history.undo restores them.`);
    if(dryRun){result.preview=r.project;return result;}
    const title=label||(source==="agent"?"Agent: ":"")+r.changes.map(c=>c.summary).join(" · ");
    if(key&&key===this.lastKey&&this.log.at(-1)?.key===key){
      const e=this.log.at(-1);e.ops=copy(ops);e.mints=r.mints;e.title=title;
    }else{
      this.history.push({project:before,logIndex:this.log.length});this.future=[];
      this.log.push({rev:this.rev+1,title,source,sessionId,ops:copy(ops),mints:r.mints,before,key,undone:false,reverted:false,changes:r.changes,sideEffects:r.sideEffects,time:Date.now()});
    }
    this.lastKey=key;
    this.project=r.project;this.rev++;this.at[this.rev]=this.project;
    if(sessionId){const s=this.sessions[sessionId];if(s){s.batches.push(this.log.length-1);for(const id of affectedIds(before,r.project))if(byId(r.project,id))this.badges.set(id,sessionId);}}
    this.emit({type:"commit",source,title,result,cascade});
    return result;
  },
  undo(){
    if(!this.history.length)return false;
    const h=this.history.pop();this.future.push({project:this.project,logIndex:h.logIndex});
    if(this.log[h.logIndex])this.log[h.logIndex].undone=true;
    this.project=h.project;this.rev++;this.at[this.rev]=this.project;this.lastKey=null;this.emit({type:"undo",title:this.log[h.logIndex]?.title});return true;
  },
  redo(){
    if(!this.future.length)return false;
    const f=this.future.pop();this.history.push({project:this.project,logIndex:f.logIndex});
    if(this.log[f.logIndex])this.log[f.logIndex].undone=false;
    this.project=f.project;this.rev++;this.at[this.rev]=this.project;this.lastKey=null;this.emit({type:"redo",title:this.log[f.logIndex]?.title});return true;
  },
  /* Revert a set of log entries: rebuild from the state before the first one and replay everything
     after it that is not reverted (human edits replay on top; ones that can no longer apply are skipped). */
  revert(indexes,title,meta={}){
    const set=new Set(indexes.filter(i=>!this.log[i].undone&&!this.log[i].reverted));
    if(!set.size)return {reverted:0,skipped:[],replayed:0};
    const first=Math.min(...set);
    let p=this.log[first].before;const skipped=[];let replayed=0;
    for(let i=first+1;i<this.log.length;i++){
      const e=this.log[i];if(set.has(i)||e.undone||e.reverted||!e.ops.length)continue;
      try{p=applyOps(p,e.ops,{source:e.source,replay:e.mints}).project;replayed++;}
      catch(err){skipped.push({title:e.title,error:err.code,message:err.message});}
    }
    for(const i of set)this.log[i].reverted=true;
    const before=this.project;
    this.history.push({project:before,logIndex:this.log.length});this.future=[];
    this.log.push({rev:this.rev+1,title,source:meta.source||"human",sessionId:meta.sessionId||null,ops:[],mints:[],before,undone:false,reverted:false,changes:[],sideEffects:[],time:Date.now(),revertOf:[...set]});
    this.project=p;this.rev++;this.at[this.rev]=this.project;this.lastKey=null;
    for(const [id,s] of [...this.badges])if(meta.sessionId===s&&(meta.scope==="session"))this.badges.delete(id);
    this.emit({type:"revert",title,skipped});
    return {reverted:set.size,skipped,replayed};
  },
  startSession(agent="agent"){
    const id="ses_"+(SERIAL++).toString(36);
    this.sessions[id]={id,agent,checkpoint:this.rev,batches:[],open:true,started:Date.now()};
    this.emit({type:"session",id});return this.sessions[id];
  },
  endSession(id){const s=this.sessions[id];if(!s)return;s.open=false;for(const [k,v] of [...this.badges])if(v===id)this.badges.delete(k);this.emit({type:"session",id});},
  openSession(){return Object.values(this.sessions).find(s=>s.open)||null;},
  /* history.undo {scope:"to"}: restore the exact state at revision r as a new revision. Nothing is destroyed. */
  restoreTo(r,sessionId=null){
    const target=this.at[r];if(!target)throw fail("NOT_FOUND",`No revision r${r}.`,"history.list shows revisions; pass one of them.",{current:"r"+this.rev});
    const before=this.project;
    this.history.push({project:before,logIndex:this.log.length});this.future=[];
    this.log.push({rev:this.rev+1,title:`Restore r${r}`,source:sessionId?"agent":"human",sessionId,ops:[],mints:[],before,undone:false,reverted:false,changes:[],sideEffects:[],time:Date.now(),restoreTo:r});
    if(sessionId&&this.sessions[sessionId])this.sessions[sessionId].batches.push(this.log.length-1);
    this.project=target;this.rev++;this.lastKey=null;this.at[this.rev]=this.project;
    this.emit({type:"revert",title:`Restore r${r}`,skipped:[]});
    return {restored:"r"+r,revision:"r"+this.rev,skipped:[]};
  },
  undoSession(id,scope){
    const s=this.sessions[id];if(!s)throw fail("NOT_FOUND",`No session ${id}.`,"session.start returns a sessionId.");
    const live=s.batches.filter(i=>!this.log[i].undone&&!this.log[i].reverted);
    if(!live.length)return {reverted:0,skipped:[],replayed:0,revision:"r"+this.rev};
    const pick=scope==="session"?live:[live.at(-1)];
    const r=this.revert(pick,scope==="session"?`Undo all agent edits (${id})`:`Undo agent batch: ${this.log[pick[0]].title.replace(/^Agent: /,"")}`,{sessionId:id,scope,source:"human"});
    if(scope==="session")for(const [k,v] of [...this.badges])if(v===id)this.badges.delete(k);
    return {...r,revision:"r"+this.rev};
  }
};
Store.at[Store.rev]=Store.project;
function affectedIds(a,b){
  const ids=new Set([...a.clips,...a.items,...b.clips,...b.items].map(x=>x.id)), out=[];
  for(const id of ids){const x=byId(a,id),y=byId(b,id);if(!x||!y||JSON.stringify(x)!==JSON.stringify(y)||startOf(a,x)!==startOf(b,y))out.push(id);}
  return out;
}
function timelineDiff(base,next){
  const ids=new Set([...base.clips,...base.items,...next.clips,...next.items].map(x=>x.id)), out=[];
  for(const id of ids){
    const a=byId(base,id), b=byId(next,id), name=nameOf(b||a);
    const range=(p,x)=>clock(startOf(p,x))+"–"+clock(startOf(p,x)+lengthOf(p,x));
    if(!a){out.push({id,name,before:"absent",after:range(next,b)});continue;}
    if(!b){out.push({id,name,before:range(base,a),after:"removed"});continue;}
    if(range(base,a)!==range(next,b))out.push({id,name,before:range(base,a),after:range(next,b)});
    else if(a.lane!==b.lane)out.push({id,name,before:laneById(base,a.lane)?.name,after:laneById(next,b.lane)?.name});
    else if(JSON.stringify(a)!==JSON.stringify(b))out.push({id,name,before:"",after:"properties changed"});
  }
  return out;
}
