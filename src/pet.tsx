import { useEffect, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalPosition } from "@tauri-apps/api/window";
import keyframe from "../assets/pet/keyframe.png";
import { nextFrame } from "./lib/pet-player";
import type { PetPhase as Phase, Playback, Desired } from "./lib/pet-player";
import { defaultPetAnimations, PET_NAMES } from "./lib/pet-animation";
import type { PetAnimations } from "./lib/pet-animation";
import { builtinMedia, loadPetMedia } from "./lib/pet-media";
import type { PetMedia } from "./lib/pet-media";
import "./pet.css";

type Notice = { id: string; phase: Phase; title: string; detail: string; host: string; agent?: string };
type Snapshot = { instance?: string; number?: number; name?: string; enabled: boolean; current: Notice | null; pending: number; animations?:PetAnimations; revision?:number };
type Point = { x: number; y: number };
type Drag = { pointer: number; sequence: number; start: Point; base: Promise<Point>; moved: boolean; pending: Point | null; writing: boolean };
const params = new URLSearchParams(location.search);
const demo = params.has("demo");
const instance = params.get("instance") ?? "";
const keyframePreview = params.has("keyframe");
const defaults=defaultPetAnimations();

function CatFrame({ image }: { image?:HTMLImageElement }) {
  const canvas=useRef<HTMLCanvasElement>(null);
  useEffect(()=>{
    const ctx=canvas.current?.getContext("2d"); if(!ctx || !image) return;
    ctx.clearRect(0,0,384,384);
    const scale=Math.min(384/image.naturalWidth,384/image.naturalHeight);
    const w=image.naturalWidth*scale,h=image.naturalHeight*scale;
    ctx.drawImage(image,(384-w)/2,(384-h)/2,w,h);
  },[image]);
  return <canvas ref={canvas} width={384} height={384} style={{width:280,height:280}} aria-hidden="true"/>;
}
const names: Record<Phase, string> = { working: "工作中", sleeping: "休息中", permission_request: "等你批准", task_complete: "任务完成" };
const sample = (phase: Phase, id: string = phase, host = "com.microsoft.VSCode"): Notice => ({ id, phase, host, title: host === "com.apple.Terminal" ? "Terminal · 测试运行" : "VS Code · AgentPulse", detail: phase === "working" ? "正在使用 Read" : phase === "permission_request" ? "Bash：安装项目依赖" : "修改已完成，等你查看" });

function Pet() {
  const [state, setState] = useState<Snapshot>({ enabled: true, current: null, pending: 0, ...(demo ? { number: 1, name: "桌宠 1" } : {}) });
  const [playback, setPlayback] = useState<Playback>({ phase: "sleeping", key: "sleeping", frame: 0, leaving: false });
  const [busy, setBusy] = useState(false);
  const [ready, setReady] = useState(false);
  const media=useRef(new Map<string,PetMedia>());
  const [profile,setProfile]=useState("builtin");
  const [paused, setPaused] = useState(false);
  const [error, setError] = useState("");
  const [demoQueue, setDemoQueue] = useState<Notice[]>([]);
  const [demoResult, setDemoResult] = useState("");
  const [demoPosition, setDemoPosition] = useState<Point>({ x: 0, y: 0 });
  const [menu, setMenu] = useState<Point | null>(null);
  const [dragging, setDragging] = useState(false);
  const scene = useRef<HTMLElement>(null);
  const menuElement = useRef<HTMLDivElement>(null);
  const drag = useRef<Drag | null>(null);
  const dragSequence = useRef(0);
  const suppressClick = useRef(false);
  const phase = state.current?.phase ?? "sleeping";
  const petName = state.name ?? (state.number ? `桌宠 ${state.number}` : "猫咪桌宠");
  const canActivate = !!state.current?.host && !busy;
  const desired = useRef<Desired>({ phase, key: phase });
  desired.current = { phase, key: state.pending ? state.current?.id ?? phase : phase, profile };
  const currentMedia=media.current.get(playback.profile ?? "builtin");
  const currentClip=currentMedia?.clips[playback.phase] ?? defaults[playback.phase];
  const animationSignature=JSON.stringify(state.animations ?? defaults)+String(state.revision ?? 0);

  useEffect(() => {
    if (demo) return;
    let alive = true;
    let timer: number;
    const refresh = async () => {
      try {
        const next = await invoke<Snapshot>("get_pet_state", { instance });
        if (alive) setState(next);
      } catch { if (alive) setError("状态读取失败，正在重试"); }
      if (alive) timer = window.setTimeout(refresh, 1000);
    };
    void refresh();
    return () => { alive = false; clearTimeout(timer); };
  }, []);

  useEffect(()=>{
    let alive=true;
    const signature=demo ? "builtin" : animationSignature;
    (demo ? builtinMedia() : loadPetMedia(state.animations ?? defaults)).then(bundle=>{
      if(!alive) return;
      media.current.set(signature,bundle);
      // 首次加载还没有旧动画，直接使用新素材的进入首帧。
      if(!ready) setPlayback(current=>({...current,frame:bundle.clips[current.phase].enter[0]-1,profile:signature}));
      setProfile(signature);setReady(true);
    }).catch(e=>{if(alive) setError(`动画素材加载失败：${String(e)}`);});
    return ()=>{alive=false;};
  },[animationSignature]);

  useEffect(()=>{
    const active=playback.profile ?? "builtin";
    for(const key of media.current.keys()) if(key!==active && key!==profile) media.current.delete(key);
  },[playback.profile,profile]);

  useEffect(() => {
    if (!ready || (demo && paused) || keyframePreview) return;
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)");
    const timer = window.setInterval(() => {
      setPlayback(current => {
        const target=desired.current;
        const clip=media.current.get(current.profile ?? "builtin")!.clips[current.phase];
        const nextClip=media.current.get(target.profile ?? "builtin")!.clips[target.phase];
        return reduce.matches ? {...target,frame:nextClip.cycle[1]-1,leaving:false} : nextFrame(current,target,clip,nextClip);
      });
    }, currentClip.frame_ms);
    return () => clearInterval(timer);
  }, [ready, paused, currentClip.frame_ms]);

  useEffect(() => { setError(""); }, [state.current?.id]);

  useEffect(() => {
    if (!menu) return;
    menuElement.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    const outside = (event: PointerEvent) => {
      if (!menuElement.current?.contains(event.target as Node)) setMenu(null);
    };
    const keyboard = (event: KeyboardEvent) => {
      if (event.key === "Escape" || event.key === "Tab") setMenu(null);
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
      event.preventDefault();
      const items = [...(menuElement.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
      const step = event.key === "ArrowDown" ? 1 : -1;
      items[(items.indexOf(document.activeElement as HTMLButtonElement) + step + items.length) % items.length]?.focus();
    };
    document.addEventListener("pointerdown", outside);
    document.addEventListener("keydown", keyboard);
    return () => { document.removeEventListener("pointerdown", outside); document.removeEventListener("keydown", keyboard); };
  }, [menu]);

  function updateDemo(queue: Notice[], working: Notice | null = null) {
    setDemoQueue(queue);
    if (paused) {
      const next = queue[0] ?? working;
      const nextPhase = next?.phase ?? "sleeping";
      setPlayback({ profile, phase: nextPhase, key: queue.length ? next!.id : nextPhase, frame: 0, leaving: false });
    }
    setState(current => ({ ...current, enabled: true, current: queue[0] ?? working, pending: queue.length }));
  }

  async function activate() {
    setMenu(null);
    if (!canActivate) return;
    setError(""); setBusy(true);
    try {
      if (demo) {
        const host = state.current!.host;
        setDemoResult(`演示跳转：${host}；其他来源继续排队`);
        if (state.pending) updateDemo(demoQueue.filter(n => n.host !== host));
      } else {
        await invoke("activate_pet_source", { instance });
        setState(await invoke<Snapshot>("get_pet_state", { instance }));
      }
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }

  async function closePet() {
    setMenu(null);
    try {
      if (demo) {
        setState(current => ({ ...current, enabled: false }));
        setDemoResult("已关闭这只桌宠；其他桌宠继续运行");
      } else await invoke("close_pet", { instance });
    } catch (e) { setError(String(e)); }
  }

  // 串行写入最新位置，避免慢 IPC 将窗口拉回过时的鼠标位置。
  async function moveWindow(gesture: Drag) {
    if (gesture.writing) return;
    gesture.writing = true;
    try {
      const base = await gesture.base;
      while (gesture.pending && gesture.sequence === dragSequence.current) {
        const delta = gesture.pending;
        gesture.pending = null;
        await getCurrentWindow().setPosition(new LogicalPosition(base.x + delta.x, base.y + delta.y));
      }
    } catch (e) { setError(`拖动失败：${String(e)}`); }
    finally { gesture.writing = false; }
  }

  function beginDrag(event: ReactPointerEvent<HTMLButtonElement>) {
    if (event.button !== 0 || event.ctrlKey) return;
    setMenu(null);
    suppressClick.current = false;
    event.currentTarget.setPointerCapture(event.pointerId);
    const base = demo ? Promise.resolve(demoPosition) : Promise.all([
      getCurrentWindow().outerPosition(), getCurrentWindow().scaleFactor(),
    ]).then(([position, scale]) => position.toLogical(scale));
    // 即使只是单击，也立即处理位置读取错误，避免未使用的 Promise 拒绝。
    void base.catch(() => {});
    drag.current = { pointer: event.pointerId, sequence: ++dragSequence.current,
      start: { x: event.screenX, y: event.screenY }, base, moved: false, pending: null, writing: false };
  }

  function continueDrag(event: ReactPointerEvent<HTMLButtonElement>) {
    const gesture = drag.current;
    if (!gesture || gesture.pointer !== event.pointerId) return;
    const delta = { x: event.screenX - gesture.start.x, y: event.screenY - gesture.start.y };
    // 一旦越过拖动阈值，移回原点也仍是拖动，不能变成一次来源跳转。
    if (!gesture.moved && Math.hypot(delta.x, delta.y) < 5) return;
    gesture.moved = true;
    suppressClick.current = true;
    setDragging(true);
    if (demo) {
      void gesture.base.then(base => setDemoPosition({ x: base.x + delta.x, y: base.y + delta.y }));
    } else {
      gesture.pending = delta;
      void moveWindow(gesture);
    }
  }

  function finishDrag(event: ReactPointerEvent<HTMLButtonElement>, cancelled = false) {
    const gesture = drag.current;
    if (!gesture || gesture.pointer !== event.pointerId) return;
    suppressClick.current = gesture.moved || cancelled;
    drag.current = null;
    setDragging(false);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
  }

  if (keyframePreview) return <main className="demo-page">
    <header><h1>第一帧确认</h1><p>第一版原图。后续动作以这只猫为基准。</p></header>
    <img src={keyframe} width="311" height="310" alt="第一版手绘橘猫坐在电脑前，桌椅位于完整草席上" />
  </main>;

  return <main className={demo ? "demo-page" : "pet-page"}>
    {demo && <header><h1>猫咪桌宠</h1><p>按住猫咪拖动，单击返回消息来源，右键打开菜单。此页使用模拟消息。</p></header>}
    {state.enabled && <section ref={scene} className="pet-scene" aria-label={petName} data-instance={instance} data-playing={playback.phase} data-frame={playback.frame} data-target={phase}
      style={demo ? { transform: `translate(${demoPosition.x}px, ${demoPosition.y}px)` } : undefined}>
      {state.current && phase !== "sleeping" && <div className="pet-status" aria-live="polite"><b>{phase === "permission_request" && state.current.agent === "codex" ? "权限处理中" : names[phase]}</b>{state.pending > 0 && <span>{state.pending} 条待处理</span>}
        {state.current && <><strong>{state.current.title}</strong><small>{state.current.detail}</small></>}
      </div>}
      <button className={`pet-character${dragging ? " is-dragging" : ""}`} aria-label={canActivate ? `${petName}：返回当前消息的来源 App` : `${petName}：按住拖动，右键打开菜单`}
        onPointerDown={beginDrag} onPointerMove={continueDrag} onPointerUp={event => finishDrag(event)} onPointerCancel={event => finishDrag(event, true)}
        onLostPointerCapture={event => finishDrag(event, true)} onDragStart={event => event.preventDefault()}
        onClick={event => { if (event.detail !== 0 && suppressClick.current) { suppressClick.current = false; return; } void activate(); }}
        onContextMenu={event => {
          event.preventDefault();
          suppressClick.current = true;
          const bounds = scene.current!.getBoundingClientRect();
          setMenu({ x: Math.max(8, Math.min(event.clientX - bounds.left, 132)), y: Math.max(8, Math.min(event.clientY - bounds.top, 266)) });
        }}
        title={canActivate ? "单击返回来源 · 按住拖动 · 右键菜单" : "按住拖动 · 右键菜单"}>
        <CatFrame image={currentMedia?.images[playback.phase][playback.frame]} />
      </button>
      {state.number !== undefined && <span className="pet-number" title={petName}>#{state.number}</span>}
      {menu && <div ref={menuElement} className="pet-menu" role="menu" aria-label={`${petName}菜单`} style={{ left: menu.x, top: menu.y }}>
        <button role="menuitem" disabled={!canActivate} onClick={() => void activate()}>返回来源 App</button>
        <button role="menuitem" onClick={() => void closePet()}>关闭这只桌宠</button>
      </div>}
      {error && <div className="pet-error" role="alert">{error}</div>}
    </section>}
    {demo && <section className="demo-controls">
      {!state.enabled && <button onClick={() => { setState(current => ({ ...current, enabled: true })); setDemoResult(""); }}>恢复这只桌宠</button>}
      <div>{(Object.keys(PET_NAMES) as Phase[]).map(p => <button key={p} aria-pressed={phase === p} onClick={() => { setDemoResult(""); updateDemo(p === "permission_request" || p === "task_complete" ? [sample(p)] : [], p === "working" ? sample(p) : null); }}>{names[p]}</button>)}</div>
      <button onClick={() => { setDemoResult(""); updateDemo([sample("task_complete", "first", "com.apple.Terminal"), sample("permission_request", "second"), sample("task_complete", "third", "com.apple.Terminal")]); }}>模拟 3 条消息：Terminal → VS Code → Terminal</button>
      <p>点击猫咪先返回 Terminal，清理它的两条消息，留下 VS Code 的请求。</p>
      <p>正在播放：{names[playback.phase]} · 第 {playback.frame + 1} / {currentClip.frames} 帧{playback.phase !== phase ? `；随后切换到${names[phase]}` : ""}</p>
      <div>
        <button onClick={() => setPaused(value => !value)}>{paused ? "继续播放" : "暂停逐帧查看"}</button>
        <label>逐帧 <input aria-label="动画帧" type="range" min="0" max={currentClip.frames-1} value={playback.frame}
          onChange={event => { setPaused(true); setPlayback(current => ({ ...current, frame: Number(event.target.value) })); }} /></label>
      </div>
      <output aria-live="polite">{demoResult}</output>
    </section>}
  </main>;
}

ReactDOM.createRoot(document.getElementById("root")!).render(<Pet />);
