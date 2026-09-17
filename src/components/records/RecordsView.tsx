import { useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { CheckCircle2, Eye, History, Lock, X } from "lucide-react";
import { api, EVENT_KEYS, EventKey, State, ToolKey, fmtTime } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Pills } from "@/components/common/Pills";
import { Button } from "@/components/ui/button";

const SRC: Record<string, string> = { simulate: "模拟", test: "测试", delayed: "延迟队列" };

export function RecordsView({ state, tool }: { state: State; tool: ToolKey }) {
  const { data: events = [] } = useQuery({ queryKey: ["events"], queryFn: () => api.readEvents(300), refetchInterval: 5000 });
  const { data: queue = [] } = useQuery({ queryKey: ["queue"], queryFn: api.readQueue, refetchInterval: 5000 });
  const [scope, setScope] = useState<ToolKey | "all">(tool);
  const [event, setEvent] = useState<EventKey | "">("");
  const [failed, setFailed] = useState(false);
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [highlight, setHighlight] = useState<string | null>(null);
  const rows = useRef<Record<string, HTMLDivElement | null>>({});
  const list = events.filter((e) => (scope === "all" || e.agent === scope) && (!event || e.event === event) && (!failed || e.results.some((r) => !r.ok)));
  const queued = queue.filter((q) => scope === "all" || q.agent === scope);
  const toggle = (id: string) => setOpen((s) => { const n = new Set(s); n.has(id) ? n.delete(id) : n.add(id); return n; });
  const jumpTo = (id: string) => { rows.current[id]?.scrollIntoView({ behavior: "smooth", block: "center" }); setHighlight(id); };

  return (
    <div className="flex flex-col gap-3">
      <div className="mb-1 flex flex-wrap items-center justify-between gap-2.5">
        <div className="inline-flex flex-wrap gap-1 rounded-xl bg-muted p-1">
          {(Object.keys(state.meta.agents) as ToolKey[]).map((k) => <button key={k} onClick={() => setScope(k)} className={cn("h-8 rounded-md px-3 text-[13px] font-medium transition-all", scope === k ? "bg-background shadow-sm" : "text-muted-foreground")}>{state.meta.agents[k]}</button>)}
          <button onClick={() => setScope("all")} className={cn("h-8 rounded-md px-3 text-[13px] font-medium transition-all", scope === "all" ? "bg-background shadow-sm" : "text-muted-foreground")}>所有 APP</button>
        </div>
        <Pills options={[{ value: "", label: "全部事件" }, ...EVENT_KEYS.map((k) => ({ value: k, label: state.meta.events[k] })), { value: "__failed", label: "只看失败" }]}
          isOn={(v) => (v === "__failed" ? failed : event === v)} onToggle={(v) => (v === "__failed" ? setFailed(!failed) : setEvent(v as EventKey | ""))} />
      </div>
      {queued.length > 0 && (
        <div className="flex flex-col gap-1.5 rounded-xl border border-dashed border-amber-200 bg-amber-50/50 p-2.5">
          <div className="text-[11px] font-medium text-amber-700">排队中，等无人处理才会外发 · {queued.length}</div>
          <div className="flex flex-wrap gap-1.5">
            {queued.map((q) => {
              const I = q.event === "permission_request" ? Lock : CheckCircle2;
              return (
                <button key={q.ref} onMouseEnter={() => jumpTo(q.ref)} onMouseLeave={() => setHighlight(null)} title={fmtTime(q.created)}
                  className="inline-flex max-w-[240px] items-center gap-1.5 rounded-full border border-amber-200 bg-white px-2.5 py-1 text-xs text-amber-800 transition-colors hover:border-amber-400">
                  <I className="h-3 w-3 flex-none" /><span className="truncate">{q.title}</span>
                  {scope === "all" && <span className="flex-none text-amber-500">· {state.meta.agents[q.agent] ?? q.agent}</span>}
                </button>
              );
            })}
          </div>
        </div>
      )}
      {list.length === 0 ? (
        <div className="rounded-xl border-[1.5px] border-dashed p-10 text-center"><div className="mx-auto mb-2.5 flex h-14 w-14 items-center justify-center rounded-full bg-muted text-muted-foreground"><History className="h-6 w-6" /></div>
          <div className="font-semibold">还没有记录</div><div className="mt-1 text-xs text-muted-foreground">触发一次提醒，或在设置 → 通用里模拟触发</div></div>
      ) : list.map((e) => {
        const I = e.event === "permission_request" ? Lock : CheckCircle2;
        const isOpen = open.has(e.id);
        return (
          <div key={e.id} ref={(el) => { rows.current[e.id] = el; }} className={cn("rounded-lg border px-3.5 py-3 transition-colors", highlight === e.id && "border-amber-400 bg-amber-50")}>
            <div className="flex items-center gap-2.5">
              <div className="w-16 flex-none text-xs tabular-nums text-muted-foreground">{fmtTime(e.ts)}</div>
              <div className={cn("flex h-7 w-7 flex-none items-center justify-center rounded-lg border", e.event === "permission_request" ? "border-green-100 bg-green-50 text-green-600" : "border-blue-100 bg-blue-50 text-blue-500")}><I className="h-3.5 w-3.5" /></div>
              <div className="min-w-0 flex-1">
                <div className="truncate text-[13px] font-medium">{e.title} {SRC[e.source] && <span className="rounded-md bg-slate-200 px-1.5 text-[10px] font-semibold text-slate-700">{SRC[e.source]}</span>} {scope === "all" && <span className="rounded-md bg-sky-100 px-1.5 text-[10px] font-semibold text-sky-700">{state.meta.agents[e.agent] ?? e.agent}</span>}</div>
                <div className="truncate text-xs text-muted-foreground">{e.body}</div>
              </div>
              <div className="flex max-w-[40%] flex-wrap justify-end gap-1">
                {e.results.length ? e.results.map((r, i) => <span key={i} title={r.error || r.info || ""} className={cn("rounded-md px-1.5 text-[11px] font-medium leading-5", r.pending ? "bg-amber-100 text-amber-700" : r.skipped ? "bg-muted text-zinc-600" : r.ok ? "bg-emerald-100 text-emerald-700" : "bg-red-100 text-red-700")}>{r.pending ? "⏳" : r.skipped ? "⏭" : r.ok ? "✓" : "✗"} {r.name}</span>) : <span className="text-xs text-muted-foreground">无匹配的提醒方式</span>}
              </div>
              <Button variant="ghost" size="icon" className="h-8 w-8" title="详情" onClick={() => toggle(e.id)}>{isOpen ? <X className="h-4 w-4" /> : <Eye className="h-4 w-4" />}</Button>
            </div>
            {isOpen && (
              <div className="mt-2.5 flex flex-col gap-1 text-xs">
                {e.results.map((r, i) => <div key={i}><b>{r.name}</b>：{r.pending ? <span className="text-muted-foreground">⏳ {r.info}</span> : r.skipped ? <span className="text-muted-foreground">⏭ {r.info}</span> : r.ok ? <><span className="text-green-600">发送成功</span> <span className="text-muted-foreground">{r.info ?? ""} {r.ms} ms</span></> : <span className="text-red-500">失败 — {r.error}</span>}</div>)}
                <div className="text-muted-foreground">项目：{e.project} · 原始数据：</div>
                <pre className="max-h-72 overflow-auto whitespace-pre-wrap break-all rounded-md bg-zinc-900 p-3 text-[11px] text-zinc-200">{JSON.stringify(e.payload, null, 2)}</pre>
              </div>)}
          </div>);
      })}
    </div>
  );
}
