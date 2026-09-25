import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { FileText, FolderOpen, Info, Plug, Power, RefreshCw, Unplug, Zap } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { checkForUpdate } from "@/lib/updater";
import { Switch } from "@/components/ui/switch";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api, Config, EVENT_KEYS, EventKey, State, Template, ToolKey } from "@/lib/api";
import { cn } from "@/lib/utils";
import { EVENT_ICON } from "@/components/home/HomeView";
import type { ConfirmFn } from "@/App";
import { useIntegration } from "@/components/common/useIntegration";

type Tab = "integrations" | "templates" | "general";
const HOOK_LABEL: Record<string, string> = { permission_request: "请求权限", task_complete: "任务完成", user_prompt: "发新消息", tool_start: "开始执行", tool_done: "工具执行" };

function preview(tpl: Template | undefined, event: EventKey, agent: string) {
  const ctx: Record<string, string> = { agent, session: "修复登录页 bug", project: "my-app", time: new Date().toLocaleTimeString(), detail: event === "permission_request" ? "Bash: npm install" : "已修改 2 个文件，测试全部通过" };
  const r = (s?: string) => (s ?? "").replace(/\{(\w+)\}/g, (m, k) => (k in ctx ? ctx[k] : m));
  return <><b>{r(tpl?.title)}</b>　<span className="text-muted-foreground">{r(tpl?.body)}</span></>;
}

const RowCard = ({ icon, title, desc, right }: { icon: React.ReactNode; title: React.ReactNode; desc: React.ReactNode; right: React.ReactNode }) => (
  <div className="flex items-center justify-between gap-4 rounded-xl border p-4 transition-colors hover:bg-muted/50">
    <div className="flex min-w-0 items-start gap-3"><div className="flex h-8 w-8 flex-none items-center justify-center rounded-lg bg-background ring-1 ring-border">{icon}</div>
      <div className="min-w-0"><div className="text-sm font-medium leading-tight">{title}</div><div className="mt-0.5 break-all text-xs text-muted-foreground">{desc}</div></div></div>
    <div className="flex flex-none items-center gap-1">{right}</div>
  </div>
);

export type SettingsHandle = { savePending: () => Promise<boolean> };

export const SettingsView = forwardRef<SettingsHandle, { state: State; tool: ToolKey; confirm: ConfirmFn }>(function SettingsView({ state, tool, confirm }, ref) {
  const qc = useQueryClient();
  const { install, uninstall, busy: integrationBusy } = useIntegration(state.meta.agents, confirm);
  const [tab, setTab] = useState<Tab>("integrations");
  const [templates, setTemplates] = useState(state.config.templates);
  const { data: autoLaunch } = useQuery({ queryKey: ["autolaunch"], queryFn: () => invoke<boolean>("get_auto_launch") });
  const { data: version } = useQuery({ queryKey: ["version"], queryFn: getVersion });
  const toggleAutoLaunch = async (v: boolean) => {
    try { await invoke("set_auto_launch", { enabled: v }); qc.invalidateQueries({ queryKey: ["autolaunch"] }); toast.success(v ? "已开启开机自启" : "已关闭开机自启"); }
    catch (e) { toast.error("设置失败", { description: String(e) }); }
  };
  const timer = useRef<number>();
  const pending = useRef<Config["templates"]>();
  const saving = useRef<Promise<boolean>>();
  useEffect(() => () => window.clearTimeout(timer.current), []);

  // 离开前保存最新草稿；串行写入，避免慢请求覆盖后续输入。
  const savePending = async (): Promise<boolean> => {
    window.clearTimeout(timer.current);
    while (pending.current) {
      if (!saving.current) {
        const next = pending.current;
        saving.current = api.saveTemplates(next).then(() => {
          if (pending.current === next) pending.current = undefined;
          qc.invalidateQueries({ queryKey: ["state"] });
          toast.success("已保存");
          return true;
        }, (e) => {
          toast.error("保存失败", { description: String(e) });
          return false;
        }).finally(() => { saving.current = undefined; });
      }
      if (!(await saving.current)) return false;
    }
    return true;
  };
  useImperativeHandle(ref, () => ({ savePending }));

  const editTpl = (k: EventKey, part: "title" | "body", v: string) => {
    const next = { ...templates, [k]: { ...templates[k], [part]: v } };
    setTemplates(next);
    pending.current = next;
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => { void savePending(); }, 800);
  };
  const simulate = async (event: EventKey) => {
    try { const r = await api.simulate(tool, event); r.results.length ? toast.success("已触发", { description: r.results.map((x) => `${x.name}${x.pending ? "⏳" : x.ok ? "✓" : "✗"}`).join("  ") }) : toast("没有提醒方式接收这个事件"); qc.invalidateQueries({ queryKey: ["state"] }); }
    catch (e) { toast.error("触发失败", { description: String(e) }); }
  };

  const tabs: [Tab, string, typeof Plug][] = [["integrations", "工具接入", Plug], ["templates", "消息文案", FileText], ["general", "通用", Info]];
  return (
    <div className="flex flex-col gap-3">
      <div className="mb-2 grid grid-cols-3 gap-1 rounded-xl bg-muted p-1">
        {tabs.map(([k, n, I]) => <button key={k} onClick={() => setTab(k)} className={cn("flex h-8 items-center justify-center gap-1.5 rounded-md text-[13px] font-medium transition-all", tab === k ? "bg-background text-foreground shadow-sm" : "text-muted-foreground hover:bg-background/50")}><I className="h-3.5 w-3.5" />{n}</button>)}
      </div>

      {tab === "integrations" && (<>
        {Object.values(state.integrations).map((i) => (
          <RowCard key={i.agent} icon={<Plug className="h-4 w-4" />}
            title={<span className="flex items-center gap-2">{i.name}{i.installed ? <span className="rounded-md bg-emerald-100 px-1.5 text-[10px] font-semibold leading-4 text-emerald-700">{i.agent === "codex" ? "已配置" : "已接入"}</span> : Object.values(i.events).some(Boolean) ? <span className="rounded-md bg-amber-100 px-1.5 text-[10px] font-semibold leading-4 text-amber-700">部分配置</span> : <span className="rounded-md bg-slate-200 px-1.5 text-[10px] font-semibold leading-4 text-slate-700">未接入</span>}</span>}
            desc={<>{i.file}{!i.file_exists && "（尚不存在，接入时会创建）"}{i.error && <div className="text-red-500">{i.error}</div>}
              <div className="mt-2 flex flex-wrap gap-1">{Object.entries(i.events).map(([k, on]) => <span key={k} className={cn("rounded-md px-1.5 text-[11px] font-medium leading-5", on ? "bg-emerald-100 text-emerald-700" : "bg-muted text-muted-foreground")}>{on ? "✓" : "—"} {HOOK_LABEL[k] ?? k}</span>)}</div>
              {i.agent === "codex" && <div className="mt-2">Codex 还需信任 Hook；点接入或重新接入时会请求确认。以上标记仅表示配置已写入，实际提醒需用新会话验证。</div>}</>}
            right={<>{i.installed ? <><Button variant="outline" size="sm" disabled={!!integrationBusy} onClick={() => install(i.agent)}>{integrationBusy?.agent === i.agent ? integrationBusy.label : "重新接入"}</Button><Button variant="ghost" size="sm" className="text-red-500" disabled={!!integrationBusy} onClick={() => uninstall(i.agent)}>断开</Button></> : <Button size="sm" disabled={!i.tool_dir_exists || !!integrationBusy} onClick={() => install(i.agent)}>{integrationBusy?.agent === i.agent ? integrationBusy.label : "接入"}</Button>}
              <Button variant="ghost" size="icon" className="h-8 w-8" title="打开配置文件位置" onClick={() => api.reveal(i.agent)}><FolderOpen className="h-4 w-4" /></Button></>} />))}
        <div className="px-0.5 text-xs text-muted-foreground">接入和断开都会先备份原配置文件，只改动 AgentPulse 自己的条目。Hook 程序：{state.hook_binary}{state.hook_binary_exists ? "" : "（未找到）"}。每个会话启动时读取一次 Hook 配置，接入后要新开会话才生效。</div>
      </>)}

      {tab === "templates" && (
        <div className="rounded-xl border bg-card px-5 py-4">
          <div className="grid grid-cols-[96px_1fr] items-start gap-x-4 gap-y-3.5">
            {EVENT_KEYS.map((k) => { const I = EVENT_ICON[k]; return (<div key={k} className="contents">
              <div className="flex items-center gap-1 pt-2 text-[13px] text-muted-foreground"><I className="h-3.5 w-3.5" />{state.meta.events[k]}</div>
              <div className="flex flex-col gap-2">
                <Input value={templates[k]?.title ?? ""} onChange={(e) => editTpl(k, "title", e.target.value)} placeholder="标题" />
                <Input value={templates[k]?.body ?? ""} onChange={(e) => editTpl(k, "body", e.target.value)} placeholder="正文" />
                <div className="rounded-md bg-muted px-3 py-2 text-xs">{preview(templates[k], k, state.meta.agents[tool])}</div>
              </div></div>); })}
          </div>
          <div className="mt-3.5 text-xs text-muted-foreground">变量：<code>{"{agent}"}</code> 工具名 · <code>{"{session}"}</code> 对话名 · <code>{"{project}"}</code> 项目文件夹 · <code>{"{detail}"}</code> 请求内容或最后一条回复 · <code>{"{time}"}</code> 时间。修改后自动保存，两个工具共用。</div>
        </div>
      )}

      {tab === "general" && (<>
        <div className="px-0.5 text-xs text-muted-foreground">退出 App 后仍可接收后台通知；主动打开 App 才会恢复主界面和桌宠。</div>
        <RowCard icon={<Power className="h-4 w-4" />} title="开机自启" desc="登录后自动在后台运行，Hook 弹窗不需要先手动打开 App" right={<Switch checked={!!autoLaunch} onCheckedChange={toggleAutoLaunch} />} />
        <RowCard icon={<RefreshCw className="h-4 w-4" />} title={`版本 ${version ?? ""}`} desc="每天自动检查一次；更新包来自 GitHub Releases，安装后自动重启" right={<Button variant="outline" size="sm" onClick={() => checkForUpdate(false)}>检查更新</Button>} />
        <RowCard icon={<FolderOpen className="h-4 w-4" />} title="数据目录" desc={`配置、提醒记录、声音库、备份都在 ${state.meta.data_dir}`} right={<Button variant="outline" size="sm" onClick={() => api.reveal("data")}>打开</Button>} />
        <RowCard icon={<FileText className="h-4 w-4" />} title="日志" desc="logs/app.log（App）；Hook 出错记在 hook-errors.log" right={<Button variant="outline" size="sm" onClick={() => api.reveal("logs")}>打开</Button>} />
        <RowCard icon={<Unplug className="h-4 w-4" />} title="卸载提醒" desc="移除 Claude Code 和 Codex 的提醒接入，保留提醒配置，之后可重新接入。已有会话需重启后生效。"
          right={<Button variant="outline" size="sm" className="text-red-500" disabled={!!integrationBusy} onClick={() => uninstall(["claude", "codex"])}>{integrationBusy?.agent === "all" ? integrationBusy.label : "卸载提醒"}</Button>} />
        <RowCard icon={<Zap className="h-4 w-4" />} title="模拟触发" desc="不用真跑工具，按当前工具的提醒方式真实发一次"
          right={EVENT_KEYS.map((k) => <Button key={k} variant="outline" size="sm" onClick={() => simulate(k)}>{state.meta.agents[tool]} · {state.meta.events[k]}</Button>)} />
      </>)}
    </div>
  );
});
