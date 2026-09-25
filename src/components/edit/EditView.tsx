/* 整页添加 / 编辑一张提醒方式卡片 */
import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Bell, Check, ChevronLeft, Clock, Eye, Image as ImageIcon, Info, MessageSquare, Monitor, Play, Plus, Send, Upload, Volume2, X, Zap } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api, DELAYS, EVENT_KEYS, EventKey, newTarget, State, Target, TargetType, ToolKey, TYPE_META, soundLabel } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Pills } from "@/components/common/Pills";
import { SoundTile } from "@/components/common/SoundTile";
import { EVENT_ICON } from "@/components/home/HomeView";
import type { ConfirmFn, View } from "@/App";

type EditProps = { state: State; tool: ToolKey; view: Extract<View, { name: "edit" }>; onDone: () => void; onLibrary: (draft: Target) => void; confirm: ConfirmFn };

const Panel = ({ children, className }: { children: React.ReactNode; className?: string }) => <div className={cn("rounded-xl border bg-card px-5 py-4", className)}>{children}</div>;
const Row = ({ label, children, top }: { label: React.ReactNode; children: React.ReactNode; top?: boolean }) => (
  <div className="grid grid-cols-[96px_1fr] items-center gap-x-4 gap-y-3"><div className={cn("text-[13px] text-muted-foreground", top && "self-start pt-2")}>{label}</div><div>{children}</div></div>
);
const Hint = ({ children }: { children: React.ReactNode }) => <div className="mt-1 text-xs text-muted-foreground">{children}</div>;
// 微信只显示「名称：{{字段.DATA}}」这样的行；字段与 wxtest.rs 的 template_data 一致。
const WX_TEMPLATE = ["设备：{{device.DATA}}", "应用：{{app.DATA}}", "项目：{{project.DATA}}", "标题：{{title.DATA}}", "内容：{{body.DATA}}", "时间：{{time.DATA}}"].join("\n");

export function EditView({ state, tool, view, onDone, onLibrary, confirm }: EditProps) {
  const qc = useQueryClient();
  const [t, setT] = useState<Target>(view.draft);
  const [soundEvent, setSoundEvent] = useState<EventKey>(view.draft.events[0] ?? "permission_request");
  const [playing, setPlaying] = useState<string | null>(null);
  const events = state.meta.events;
  const agentName = state.meta.agents[tool];
  const targets = state.config.tools[tool].targets;
  const { data: sounds = [] } = useQuery({ queryKey: ["sounds"], queryFn: api.listSounds, enabled: t.type === "sound" });
  const patch = (p: Partial<Target>) => setT((x) => ({ ...x, ...p }));
  const playTimer = useRef<number>();

  const play = async (ref: string) => {
    setPlaying(ref);
    try { await api.previewSound(ref, t.volume ?? 100); } catch (e) { toast.error("播放失败", { description: String(e) }); }
    window.clearTimeout(playTimer.current);
    playTimer.current = window.setTimeout(() => setPlaying(null), 1600);
  };
  useEffect(() => () => window.clearTimeout(playTimer.current), []);

  const upload = async (files: FileList | File[]) => {
    let last: string | null = null;
    for (const f of Array.from(files)) {
      try { last = await api.uploadSound(f.name, new Uint8Array(await f.arrayBuffer())); toast.success("已上传", { description: f.name }); }
      catch (e) { toast.error(`上传失败：${f.name}`, { description: String(e) }); }
    }
    qc.invalidateQueries({ queryKey: ["sounds"] });
    if (last) { patch({ sounds: { ...t.sounds, [soundEvent]: last } }); play(last); }
  };
  const pickFiles = () => {
    const input = document.createElement("input");
    input.type = "file"; input.accept = ".mp3,.wav,.aiff,.aif,.m4a,.caf,audio/*"; input.multiple = true;
    input.onchange = () => input.files && upload(input.files);
    input.click();
  };

  // 悬浮窗图标按工具保存、上传即生效，不跟卡片的「保存」走；系统通知由 macOS 画，用不上它。
  const icon = state.icons[tool];
  const pickIcon = () => {
    const i = document.createElement("input");
    i.type = "file"; i.accept = ".png,.jpg,.jpeg,.gif,image/png,image/jpeg,image/gif";
    i.onchange = async () => {
      const f = i.files?.[0]; if (!f) return;
      try { await api.uploadToolIcon(tool, f.name, new Uint8Array(await f.arrayBuffer())); qc.invalidateQueries({ queryKey: ["state"] }); toast.success("已上传"); }
      catch (e) { toast.error("上传失败", { description: String(e) }); }
    };
    i.click();
  };
  const removeIcon = async () => {
    try { await api.deleteToolIcon(tool); qc.invalidateQueries({ queryKey: ["state"] }); toast.success("已恢复自动图标"); }
    catch (e) { toast.error("删除失败", { description: String(e) }); }
  };

  const testDraft = async (event: EventKey) => {
    try {
      const r = await api.testTarget(tool, t, event); const x = r.results[0];
      if (x) x.ok ? toast.success(`${x.name}：已发送`, { description: x.info }) : toast.error(`${x.name}：发送失败`, { description: x.error });
    } catch (e) { toast.error("测试失败", { description: String(e) }); }
  };
  const save = async () => {
    if (!t.events.length && !(await confirm({ title: "没有选择接收的提醒", body: "这张卡片不会收到任何提醒，确定保存吗？", okText: "仍然保存" }))) return;
    const next = structuredClone(targets);
    if (view.isNew) next.push(t); else next[view.index] = t;
    try { await api.saveToolTargets(tool, next); await qc.invalidateQueries({ queryKey: ["state"] }); toast.success(view.isNew ? "已添加" : "已保存"); onDone(); }
    catch (e) { toast.error("保存失败", { description: String(e) }); }
  };

  const eventPills = <Pills options={EVENT_KEYS.map((k) => ({ value: k, label: events[k], icon: EVENT_ICON[k] }))} isOn={(v) => t.events.includes(v)}
    onToggle={(v) => patch({ events: EVENT_KEYS.filter((k) => (k === v ? !t.events.includes(v) : t.events.includes(k))) })} />;
  const addable = (Object.keys(TYPE_META) as TargetType[]).filter((k) => !(TYPE_META[k].singleton && targets.some((x) => x.type === k)));
  const typeIcon = { desktop: Monitor, sound: Bell, wxtest: MessageSquare, feishu: Send } as const;

  const delivery = (
    <Panel><div className="flex flex-col gap-3.5">
      <Row label="接收提醒">{eventPills}</Row>
      <Row label="发送时机" top><Pills options={DELAYS.map((m) => ({ value: m, label: m ? `${m} 分钟` : "立即", icon: m ? Clock : Zap }))} isOn={(v) => Number(t.delay_minutes ?? 0) === v} onToggle={(v) => patch({ delay_minutes: v })} />
        {Number(t.delay_minutes) > 0 && <Hint><Info className="mr-1 inline h-3 w-3" />本机先提醒；{t.delay_minutes} 分钟内你切回 {agentName}、在里面有操作、发了新消息或批准了权限，就不发</Hint>}</Row>
      {Number(t.delay_minutes) > 0 && <Row label="多条提醒"><Pills options={[{ value: "1", label: "合并成一条" }, { value: "0", label: "逐条发送" }]} isOn={(v) => (t.batch !== false) === (v === "1")} onToggle={(v) => patch({ batch: v === "1" })} /></Row>}
    </div></Panel>
  );

  return (
    <div className="flex flex-col gap-4">
      {view.isNew && addable.length > 1 && (
        <Panel>
          <div className="mb-3 flex items-center gap-2 border-b border-border/60 pb-2 text-sm font-semibold"><Plus className="h-4 w-4 text-blue-500" />选择类型</div>
          <div className="grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-2.5">
            {addable.map((k) => { const I = typeIcon[k]; const on = t.type === k; return (
              <button key={k} type="button" onClick={() => setT(newTarget(k))}
                className={cn("flex items-center gap-2.5 rounded-lg p-3 text-left font-medium transition-all", on ? "bg-blue-500 text-white shadow-[0_8px_18px_-8px_rgba(10,132,255,.7)]" : "bg-muted text-muted-foreground hover:bg-gray-200 hover:text-foreground")}>
                <div className={cn("flex h-8 w-8 items-center justify-center rounded-lg border", on ? "border-transparent bg-white/20 text-white" : "border-border bg-background")}><I className="h-4 w-4" /></div>
                <div><div>{TYPE_META[k].name}</div><div className="text-[11px] font-normal opacity-80">{TYPE_META[k].desc}</div></div>
              </button>); })}
          </div>
        </Panel>
      )}

      {t.type === "desktop" && (<>
        <Panel>
          <div className="mb-3 flex items-center gap-2 border-b border-border/60 pb-2 text-sm font-semibold"><Eye className="h-4 w-4 text-blue-500" />弹窗样式</div>
          <div className="grid grid-cols-3 gap-3">
            {([["overlay", "强制悬浮窗", "专注模式下也显示，返回来源后清除"], ["system", "系统通知", "走通知中心，受专注模式控制"], ["both", "两者都发", "悬浮窗 + 通知中心留档"]] as const).map(([v, n, d]) => (
              <button key={v} type="button" onClick={() => patch({ mode: v })} className={cn("rounded-lg border-[1.5px] p-2.5 text-left transition-all hover:border-blue-500/40", t.mode === v && "border-blue-500 shadow-[0_0_0_3px_rgba(10,132,255,.15)]")}>
                <div className="relative mb-2 h-[84px] overflow-hidden rounded-md bg-gradient-to-br from-indigo-200 via-purple-200 to-pink-200">
                  <div className="absolute inset-x-0 top-0 h-[9px] bg-white/55" />
                  {(v === "overlay" || v === "both") && <div className="absolute right-[7px] top-[14px] h-[30px] w-[62%] rounded-[7px] bg-white/50 shadow-lg backdrop-blur-md after:absolute after:right-[5px] after:top-[5px] after:h-1.5 after:w-1.5 after:rounded-full after:bg-black/10" />}
                  {v === "overlay" && <div className="absolute right-[7px] top-[50px] h-[30px] w-[52%] rounded-[7px] bg-white/40 backdrop-blur-md" />}
                  {v === "system" && <div className="absolute right-[7px] top-[14px] h-[22px] w-[58%] rounded-[7px] bg-white/90 shadow-lg" />}
                  {v === "both" && <div className="absolute right-[7px] top-[50px] h-[22px] w-[45%] rounded-[7px] bg-white/70" />}
                </div>
                <div className="flex items-center justify-between text-[13px] font-semibold">{n}{t.mode === v && <span className="inline-flex h-4 w-4 items-center justify-center rounded-full bg-blue-500 text-white"><Check className="h-3 w-3" /></span>}</div>
                <div className="mt-0.5 text-[11px] text-muted-foreground">{d}</div>
              </button>))}
          </div>
          {t.mode !== "system" && <p className="mt-3 text-xs text-muted-foreground">返回来源 App 时，清除它已显示的全部悬浮提醒；关闭按钮只移除单条。</p>}
          {(t.mode === "overlay" || t.mode === "both") && (
            <div className="mt-3 flex items-center gap-2.5 rounded-lg border px-3.5 py-2.5 text-[13px]">
              <div className="flex h-7 w-7 flex-none items-center justify-center overflow-hidden rounded-md border bg-muted">
                {icon ? <img src={icon} alt="" className="h-full w-full object-cover" /> : <ImageIcon className="h-3.5 w-3.5 text-muted-foreground" />}
              </div>
              <span className="text-muted-foreground">悬浮窗图标：{icon ? "自定义" : "未设置，自动使用来源 App 的图标"}</span>
              <span className="flex-1" />
              <Button size="sm" variant="ghost" onClick={pickIcon}>{icon ? "更换" : "上传自定义图标"}</Button>
              {icon && <Button size="sm" variant="ghost" className="hover:bg-red-50 hover:text-red-500" onClick={removeIcon}>删除</Button>}
            </div>
          )}
        </Panel>
        <Panel><div className="flex flex-col gap-3.5">
          <Row label="接收提醒">{eventPills}</Row>
          <Row label="点击弹窗后"><Pills options={[{ value: "activate", label: `切回 ${agentName}`, icon: ChevronLeft }, { value: "close", label: "只关闭", icon: X }]} isOn={(v) => (t.click ?? "activate") === v} onToggle={(v) => patch({ click: v as Target["click"] })} /></Row>
        </div></Panel>
        <Panel className="flex items-center gap-3"><div className="flex h-9 w-9 items-center justify-center rounded-lg border border-blue-100 bg-blue-50 text-blue-500"><Eye className="h-4 w-4" /></div>
          <div className="min-w-0 flex-1"><div className="font-semibold">预览</div><div className="text-xs text-muted-foreground">{agentName}: AgentPulse 测试对话 · ✅ 任务完成 · 这是一条测试提醒</div></div>
          <Button size="sm" onClick={() => testDraft("task_complete")}><Play className="h-3.5 w-3.5" />弹一个看看</Button></Panel>
      </>)}

      {t.type === "sound" && (<>
        <Panel>
          <div className="grid grid-cols-2 gap-1 rounded-lg bg-muted p-1">
            {EVENT_KEYS.map((k) => { const I = EVENT_ICON[k]; return (
              <button key={k} type="button" onClick={() => setSoundEvent(k)} className={cn("flex h-[38px] items-center justify-center gap-1.5 rounded-md text-[13px] font-medium transition-all", soundEvent === k ? "bg-background text-foreground shadow-sm" : "text-muted-foreground")}>
                <I className="h-3.5 w-3.5" />{events[k]}时<span className="text-[11px] font-semibold text-blue-500">{soundLabel(t.sounds?.[k])}</span></button>); })}
          </div>
          {t.sounds?.[soundEvent] && sounds.length > 0 && !sounds.some((s) => s.ref === t.sounds?.[soundEvent]) && (
            <div className="mt-3 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-900">选中的声音「{soundLabel(t.sounds[soundEvent])}」已被删除，提醒时会改用 Glass</div>)}
          <div className="mb-2 mt-3.5 flex items-center justify-between text-xs font-semibold text-muted-foreground"><span>我的声音</span><button type="button" className="text-blue-500 hover:underline" onClick={() => onLibrary(t)}>管理声音库</button></div>
          <div className="grid grid-cols-[repeat(auto-fill,minmax(132px,1fr))] gap-2.5">
            {sounds.filter((s) => s.kind === "custom").map((s) => <SoundTile key={s.ref} sound={s} selected={s.ref === t.sounds?.[soundEvent]} playing={playing === s.ref} onPick={() => { patch({ sounds: { ...t.sounds, [soundEvent]: s.ref } }); play(s.ref); }} onPlay={() => play(s.ref)} />)}
            <button type="button" onClick={pickFiles} onDragOver={(e) => e.preventDefault()} onDrop={(e) => { e.preventDefault(); upload(e.dataTransfer.files); }}
              className="flex min-h-[92px] flex-col items-center justify-center gap-1 rounded-lg border-[1.5px] border-dashed border-slate-300 text-xs text-muted-foreground transition-all hover:border-blue-500 hover:bg-blue-500/5 hover:text-blue-500">
              <Upload className="h-5 w-5" /><span>上传声音</span><span className="text-[10px]">mp3 / wav / aiff / m4a · ≤ 5 MB</span></button>
          </div>
          <div className="mb-2 mt-3.5 text-xs font-semibold text-muted-foreground">系统音效</div>
          <div className="grid grid-cols-[repeat(auto-fill,minmax(132px,1fr))] gap-2.5">
            {sounds.filter((s) => s.kind === "system").map((s) => <SoundTile key={s.ref} sound={s} selected={s.ref === t.sounds?.[soundEvent]} playing={playing === s.ref} onPick={() => { patch({ sounds: { ...t.sounds, [soundEvent]: s.ref } }); play(s.ref); }} onPlay={() => play(s.ref)} />)}
          </div>
        </Panel>
        <Panel><div className="flex flex-col gap-3.5">
          <Row label="接收提醒">{eventPills}</Row>
          <Row label="音量"><div className="flex items-center gap-3"><Volume2 className="h-4 w-4 text-muted-foreground" />
            <input type="range" min={0} max={100} step={5} value={t.volume ?? 100} className="h-1 flex-1 accent-blue-500" onChange={(e) => patch({ volume: Number(e.target.value) })} onMouseUp={() => play(t.sounds?.[soundEvent] || "system:Glass")} />
            <span className="w-10 text-right text-xs tabular-nums text-muted-foreground">{t.volume ?? 100}%</span></div></Row>
        </div></Panel>
      </>)}

      {t.type === "wxtest" && (<>
        <Panel><div className="flex flex-col gap-3.5">
          <Row label="名称"><Input value={t.name} onChange={(e) => patch({ name: e.target.value })} placeholder="比如：我的手机" /></Row>
          <Row label="appID"><Input className="font-mono text-xs" value={t.appid ?? ""} onChange={(e) => patch({ appid: e.target.value })} placeholder="测试号信息里的 appID" /></Row>
          <Row label="appsecret"><Input type="password" className="font-mono text-xs" value={t.secret ?? ""} onChange={(e) => patch({ secret: e.target.value })} placeholder="测试号信息里的 appsecret" /></Row>
          <Row label="openid"><Input className="font-mono text-xs" value={t.openid ?? ""} onChange={(e) => patch({ openid: e.target.value })} placeholder="用户列表里的微信号，多个用逗号分隔" /></Row>
          <Row label="模板 ID" top><Input className="font-mono text-xs" value={t.template_id ?? ""} onChange={(e) => patch({ template_id: e.target.value })} placeholder="新增测试模板后得到的 ID" />
            <Hint>微信只显示「名称：{"{{字段.DATA}}"}」这样的行，模板内容照下面填（点一下全选） · <a className="text-blue-500 hover:underline" href="https://mp.weixin.qq.com/debug/cgi-bin/sandbox?t=sandbox/login" target="_blank" rel="noreferrer">打开测试号页面</a>
              <pre className="mt-1.5 select-all whitespace-pre rounded-md bg-muted px-3 py-2 font-mono text-[11px] leading-5 text-foreground">{WX_TEMPLATE}</pre></Hint></Row>
        </div></Panel>
        {delivery}
      </>)}

      {t.type === "feishu" && (<>
        <Panel><div className="flex flex-col gap-3.5">
          <Row label="名称"><Input value={t.name} onChange={(e) => patch({ name: e.target.value })} placeholder="比如：项目群" /></Row>
          <Row label="Webhook" top><Input className="font-mono text-xs" value={t.webhook ?? ""} onChange={(e) => patch({ webhook: e.target.value })} placeholder="https://open.feishu.cn/open-apis/bot/v2/hook/..." /><Hint>飞书群 → 设置 → 群机器人 → 添加机器人 → 自定义机器人，复制 Webhook 地址</Hint></Row>
          <Row label="签名密钥" top><Input type="password" className="font-mono text-xs" value={t.secret ?? ""} onChange={(e) => patch({ secret: e.target.value })} placeholder="机器人开启了「签名校验」才需要填" /><Hint>建议开启签名校验；如果改用「自定义关键词」，关键词要出现在消息标题或正文里</Hint></Row>
        </div></Panel>
        {delivery}
      </>)}

      <div className="fixed inset-x-0 bottom-0 z-10 border-t bg-background/90 backdrop-blur-md">
        <div className="mx-auto flex max-w-[860px] items-center gap-3 px-6 py-3.5">
          {(t.type === "wxtest" || t.type === "feishu") && <Button variant="ghost" size="sm" onClick={() => testDraft("task_complete")}><Send className="h-3.5 w-3.5" />发送测试消息</Button>}
          {t.type === "sound" && <Button variant="ghost" size="sm" onClick={() => testDraft(soundEvent)}><Play className="h-3.5 w-3.5" />试听当前设置</Button>}
          <span className="flex-1" />
          <Button variant="outline" onClick={onDone}>取消</Button>
          <Button onClick={save}>{view.isNew ? <><Plus className="h-4 w-4" />添加</> : "保存"}</Button>
        </div>
      </div>
    </div>
  );
}
