import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, Bell, Clock, Copy, GripVertical, Lock, MessageSquare, Monitor, Pencil, Plug, Plus, Send, Trash2, Volume2, CheckCircle2, Zap } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { api, EventKey, State, Target, ToolKey, TYPE_META, soundLabel } from "@/lib/api";
import { cn } from "@/lib/utils";
import type { ConfirmFn } from "@/App";
import { useIntegration } from "@/components/common/useIntegration";

const TYPE_ICON = { desktop: Monitor, sound: Bell, wxtest: MessageSquare, feishu: Send } as const;
const TYPE_TONE = { desktop: "border-blue-100 bg-blue-50 text-blue-500", sound: "border-orange-100 bg-orange-50 text-orange-500", wxtest: "border-green-100 bg-green-50 text-green-600", feishu: "border-indigo-100 bg-indigo-50 text-indigo-500" } as const;
export const EVENT_ICON: Record<EventKey, typeof Lock> = { permission_request: Lock, task_complete: CheckCircle2 };

const Tag = ({ tone, children }: { tone: "sky" | "emerald" | "amber" | "slate"; children: React.ReactNode }) => (
  <span className={cn("inline-flex items-center gap-1 rounded-md px-1.5 text-[10px] font-semibold leading-4",
    { sky: "bg-sky-100 text-sky-700", emerald: "bg-emerald-100 text-emerald-700", amber: "bg-amber-100 text-amber-700", slate: "bg-slate-200 text-slate-700" }[tone])}>{children}</span>
);

function summary(t: Target, events: Record<EventKey, string>) {
  const evs = t.events.map((e) => { const I = EVENT_ICON[e]; return <span key={e} className="inline-flex items-center gap-1"><I className="h-3 w-3" />{events[e]}</span>; });
  if (t.type === "desktop") {
    const mode = { overlay: "强制悬浮窗", system: "系统通知", both: "悬浮窗 + 通知" }[t.mode ?? "system"];
    return { tag: <Tag tone="sky">{mode}</Tag>, sub: <>{evs}{t.click === "close" && <Tag tone="slate">点击只关闭</Tag>}</> };
  }
  if (t.type === "sound") {
    return { tag: null, sub: <>{t.events.map((e) => { const I = EVENT_ICON[e]; return <span key={e} className="inline-flex items-center gap-1"><I className="h-3 w-3" />{soundLabel(t.sounds?.[e])}</span>; })}<Tag tone="slate"><Volume2 className="h-3 w-3" />{t.volume ?? 100}%</Tag></> };
  }
  const d = Number(t.delay_minutes ?? 0);
  return { tag: d ? <Tag tone="amber"><Clock className="h-3 w-3" />无人值守 {d} 分钟</Tag> : <Tag tone="emerald">立即发送</Tag>, sub: <>{evs}{d ? <Tag tone="slate">{t.batch === false ? "逐条发送" : "合并发送"}</Tag> : null}</> };
}

export function HomeView({ state, tool, onEdit, onAdd, confirm }: { state: State; tool: ToolKey; onEdit: (t: Target, i: number) => void; onAdd: () => void; confirm: ConfirmFn }) {
  const qc = useQueryClient();
  const { install, busy: integrationBusy } = useIntegration(state.meta.agents, confirm);
  const targets = state.config.tools[tool].targets;
  const integ = state.integrations[tool];
  const st = state.stats[tool];
  const name = state.meta.agents[tool];
  const other: ToolKey = tool === "claude" ? "codex" : "claude";
  const [drag, setDrag] = useState<{ from: number; over: number | null } | null>(null);

  const save = useMutation({
    mutationFn: (list: Target[]) => api.saveToolTargets(tool, list),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["state"] }),
    onError: (e) => { toast.error("保存失败", { description: String(e) }); qc.invalidateQueries({ queryKey: ["state"] }); },
  });

  const toggle = (i: number, on: boolean) => {
    const list = structuredClone(targets); list[i].enabled = on;
    qc.setQueryData<State>(["state"], (s) => s && { ...s, config: { ...s.config, tools: { ...s.config.tools, [tool]: { targets: list } } } });
    save.mutate(list, { onSuccess: () => toast.success(on ? "已启用" : "已停用") });
  };
  const remove = async (i: number) => {
    if (!(await confirm({ title: "删除提醒方式", body: `确定删除「${targets[i].name}」吗？此操作只影响 ${name}。`, okText: "删除", danger: true }))) return;
    save.mutate(targets.filter((_, k) => k !== i), { onSuccess: () => toast.success("已删除") });
  };
  const copy = async (t: Target) => {
    const exists = TYPE_META[t.type].singleton && state.config.tools[other].targets.some((x) => x.type === t.type);
    if (!(await confirm({ title: `复制到 ${state.meta.agents[other]}`, body: exists ? `${state.meta.agents[other]} 已经有一个${TYPE_META[t.type].name}，复制后会被替换。` : `会在 ${state.meta.agents[other]} 下新建一张相同的「${t.name}」，包括密钥。之后两边互不影响。`, okText: "复制" }))) return;
    try { await api.copyTarget(tool, t.id, other); qc.invalidateQueries({ queryKey: ["state"] }); toast.success(`已复制到 ${state.meta.agents[other]}`); }
    catch (e) { toast.error("复制失败", { description: String(e) }); }
  };
  const test = async (t: Target) => {
    try {
      const r = await api.testTarget(tool, t, t.events[0] ?? "task_complete");
      const x = r.results[0];
      if (x) x.ok ? toast.success(`${x.name}：已发送`, { description: x.info }) : toast.error(`${x.name}：发送失败`, { description: x.error });
    } catch (e) { toast.error("测试失败", { description: String(e) }); }
  };
  const drop = () => {
    if (!drag || drag.over === null || drag.over === drag.from) { setDrag(null); return; }
    const list = structuredClone(targets); const [m] = list.splice(drag.from, 1); list.splice(drag.over > drag.from ? drag.over - 1 : drag.over, 0, m);
    setDrag(null); save.mutate(list);
  };

  return (
    <div className="flex flex-col gap-3">
      {integ.installed ? (
        <div className="flex items-center gap-2.5 rounded-lg border border-green-500/25 bg-green-500/10 px-3.5 py-2.5 text-[13px] text-emerald-900">
          <Plug className="h-4 w-4" /><span title={tool === "codex" ? "配置已写入；信任及真实提醒接收需另行确认" : undefined}>{tool === "codex" ? "已配置" : "已接入"} {name}</span><i className="h-3 w-px bg-current opacity-25" /><span>今日 {st.today} 条提醒</span>
          {st.queued > 0 && <><i className="h-3 w-px bg-current opacity-25" /><span>待发微信 {st.queued} 条</span></>}
          {st.failed_today > 0 && <><i className="h-3 w-px bg-current opacity-25" /><span className="text-red-500">失败 {st.failed_today}</span></>}
        </div>
      ) : (
        <div className="flex items-center gap-2.5 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3.5 py-2.5 text-[13px] text-amber-900">
          <AlertTriangle className="h-4 w-4" /><span>{name} 还没接入，下面的提醒方式不会生效</span><span className="flex-1" />
          <Button size="sm" onClick={() => install(tool)} disabled={!integ.tool_dir_exists || !!integrationBusy}>{integrationBusy?.agent === tool ? integrationBusy.label : "立即接入"}</Button>
        </div>
      )}
      {!state.hook_binary_exists && (
        <div className="flex items-center gap-2.5 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3.5 py-2.5 text-[13px] text-amber-900"><AlertTriangle className="h-4 w-4" />找不到 Hook 程序 {state.hook_binary}，接入会失败</div>
      )}
      {targets.length === 0 ? (
        <div className="rounded-xl border-[1.5px] border-dashed p-10 text-center">
          <div className="mx-auto mb-2.5 flex h-14 w-14 items-center justify-center rounded-full bg-muted text-muted-foreground"><Bell className="h-6 w-6" /></div>
          <div className="text-base font-semibold">{name} 还没有提醒方式</div>
          <div className="mt-1 text-sm text-muted-foreground">点右上角橙色 + 添加屏幕弹窗、提示音或微信</div>
          <Button className="mt-4" onClick={onAdd}><Plus className="h-4 w-4" />添加提醒方式</Button>
        </div>
      ) : targets.map((t, i) => {
        const Icon = TYPE_ICON[t.type] ?? Bell;
        const { tag, sub } = summary(t, state.meta.events);
        return (
          <div key={t.id} draggable={drag?.from === i} onDragOver={(e) => { if (drag) { e.preventDefault(); setDrag({ ...drag, over: i }); } }} onDrop={drop} onDragEnd={drop}
            className={cn("group relative flex items-center gap-2.5 overflow-hidden rounded-xl border bg-card px-4 py-3.5 transition-all hover:border-blue-500/50 hover:shadow-sm",
              t.enabled && "before:pointer-events-none before:absolute before:inset-0 before:bg-gradient-to-r before:from-blue-500/[.07] before:to-transparent before:to-60%",
              drag?.from === i && "scale-[1.01] border-blue-500 opacity-90 shadow-lg", drag?.over === i && drag.from !== i && "shadow-[0_-3px_0_0_#0A84FF]")}>
            <span className="-ml-1.5 cursor-grab p-1 text-muted-foreground/40 hover:text-muted-foreground" title="拖动排序" onPointerDown={() => setDrag({ from: i, over: null })}><GripVertical className="h-4 w-4" /></span>
            <div className={cn("flex h-9 w-9 flex-none items-center justify-center rounded-lg border transition-transform group-hover:scale-105", TYPE_TONE[t.type], !t.enabled && "opacity-50")}><Icon className="h-4 w-4" /></div>
            <div className={cn("relative min-w-0 flex-1", !t.enabled && "opacity-50")}>
              <div className="flex flex-wrap items-center gap-1.5 text-[15px] font-semibold leading-tight">{t.name || TYPE_META[t.type]?.name} {tag}</div>
              <div className="mt-1 flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">{sub}</div>
            </div>
            <div className="relative flex items-center gap-0.5 opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
              <Button size="sm" onClick={() => test(t)} title="用当前设置真实发一次"><Send className="h-3.5 w-3.5" />测试</Button>
              <Button variant="ghost" size="icon" className="h-8 w-8" title="编辑" onClick={() => onEdit(t, i)}><Pencil className="h-4 w-4" /></Button>
              <Button variant="ghost" size="icon" className="h-8 w-8" title={`复制到 ${state.meta.agents[other]}`} onClick={() => copy(t)}><Copy className="h-4 w-4" /></Button>
              <Button variant="ghost" size="icon" className="h-8 w-8 hover:bg-red-50 hover:text-red-500" title="删除" onClick={() => remove(i)}><Trash2 className="h-4 w-4" /></Button>
            </div>
            <Switch checked={t.enabled} onCheckedChange={(v) => toggle(i, v)} title={t.enabled ? "已启用" : "已停用"} />
          </div>
        );
      })}
      <div className="px-0.5 text-xs text-muted-foreground"><Zap className="mr-1 inline h-3 w-3" />每个会话启动时读取一次 Hook 配置，接入后要新开会话才生效。</div>
    </div>
  );
}
