/* 视图切换与顶部栏，结构参考 cc-switch 的 App.tsx */
import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { AnimatePresence, motion } from "framer-motion";
import { Cat, ChevronLeft, FolderOpen, History, Music, Plus, RefreshCw, Settings, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { Toaster } from "@/components/ui/sonner";
import { Button } from "@/components/ui/button";
import { api, newTarget, State, Target, ToolKey, TYPE_META } from "@/lib/api";
import { cn } from "@/lib/utils";
import { HomeView } from "@/components/home/HomeView";
import { EditView } from "@/components/edit/EditView";
import { SettingsHandle, SettingsView } from "@/components/settings/SettingsView";
import { RecordsView } from "@/components/records/RecordsView";
import { LibraryView } from "@/components/library/LibraryView";
import { PetsView } from "@/components/pets/PetsView";
import { useConfirm } from "@/components/common/useConfirm";
import { autoCheckOncePerDay } from "@/lib/updater";
import { GITHUB_URL } from "@/lib/repo";
import { openUrl } from "@tauri-apps/plugin-opener";

export type View =
  | { name: "home" }
  | { name: "edit"; draft: Target; isNew: boolean; index: number }
  | { name: "settings" }
  | { name: "records" }
  | { name: "pets" }
  | { name: "library"; from?: View };

export default function App() {
  const qc = useQueryClient();
  const [tool, setTool] = useState<ToolKey>(() => (localStorage.getItem("ap.tool") as ToolKey) || "claude");
  const [view, setView] = useState<View>({ name: "home" });
  const settings = useRef<SettingsHandle>(null);
  const { data: state, error } = useQuery({ queryKey: ["state"], queryFn: api.getState, refetchInterval: view.name === "home" || view.name === "pets" ? 5000 : false });
  const { confirm, dialog } = useConfirm();

  useEffect(() => localStorage.setItem("ap.tool", tool), [tool]);
  useEffect(() => { autoCheckOncePerDay(); }, []);
  useEffect(() => api.onPetEnabledChanged(async ({ id, enabled }) => {
    // 先取消旧查询，避免关闭前发出的读取结果把开关重新覆盖为开启。
    await qc.cancelQueries({ queryKey: ["state"] });
    qc.setQueryData<State>(["state"], current => current && { ...current, config: { ...current.config,
      pets: current.config.pets.map(pet => pet.id === id ? { ...pet, enabled } : pet),
    } });
    void qc.invalidateQueries({ queryKey: ["state"] });
  }), [qc]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || view.name === "home") return;
      if (["INPUT", "TEXTAREA"].includes(document.activeElement?.tagName ?? "")) { (document.activeElement as HTMLElement).blur(); return; }
      goBack();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  });

  const goBack = async () => {
    if (view.name === "settings" && (await settings.current?.savePending()) === false) return;
    if (view.name === "library" && view.from) setView(view.from);
    else setView({ name: "home" });
    qc.invalidateQueries({ queryKey: ["state"] });
  };

  if (error) return <div className="p-10 text-center text-sm text-red-500">无法读取状态：{String(error)}</div>;
  if (!state) return null;

  const agents = state.meta.agents;
  const targets = state.config.tools[tool].targets;

  const startAdd = () => {
    const addable = (Object.keys(TYPE_META) as Target["type"][]).filter((k) => !(TYPE_META[k].singleton && targets.some((t) => t.type === k)));
    const type = addable[0] ?? "wxtest";
    setView({ name: "edit", draft: newTarget(type), isNew: true, index: targets.length });
  };

  const title = view.name === "edit" ? (view.isNew ? `为 ${agents[tool]} 添加提醒方式` : `${TYPE_META[view.draft.type]?.name ?? ""} · ${agents[tool]}`)
    : { settings: "设置", records: "提醒记录", library: "声音库", pets: "桌宠", home: "" }[view.name];

  const clearRecords = async () => {
    if (!(await confirm({ title: "清空提醒记录", body: "所有工具的提醒记录都会被清空，不可恢复。", okText: "清空", danger: true }))) return;
    await api.clearEvents();
    qc.invalidateQueries({ queryKey: ["events"] });
    toast.success("已清空");
  };

  return (
    <div className="min-h-screen bg-background">
      <header className="sticky top-0 z-20 flex h-16 items-center justify-between gap-3 border-b border-border/60 bg-background/80 px-6 backdrop-blur-md">
        {view.name === "home" ? (
          <>
            <div className="flex items-center gap-2">
              <button onClick={() => openUrl(GITHUB_URL)} title={`在浏览器打开 ${GITHUB_URL}`}
                className="text-xl font-bold tracking-tight text-blue-500 transition-opacity hover:opacity-80">AgentPulse</button>
              <Button variant="ghost" size="icon" className="h-8 w-8" title="设置" onClick={() => setView({ name: "settings" })}><Settings className="h-5 w-5" /></Button>
            </div>
            <div className="flex items-center gap-2">
              <div className="inline-flex gap-1 rounded-xl bg-muted p-1">
                {(Object.keys(agents) as ToolKey[]).map((k) => {
                  const on = state.integrations[k]?.installed;
                  return (
                    <button key={k} onClick={() => setTool(k)} title={on ? (k === "codex" ? "Hook 已配置" : "已接入") : "未接入"}
                      className={cn("flex h-8 items-center gap-2 rounded-md px-3 text-[13px] font-medium transition-all", k === tool ? "bg-background text-foreground shadow-sm" : "text-muted-foreground hover:bg-background/50")}>
                      <span className={cn("h-[7px] w-[7px] rounded-full", on ? "bg-green-500 shadow-[0_0_0_3px_rgba(16,185,129,.15)]" : "bg-gray-300")} />{agents[k]}
                    </button>
                  );
                })}
              </div>
              <div className="inline-flex gap-1 rounded-xl bg-muted p-1">
                <Button variant="ghost" size="icon" className="h-8 w-8" title="桌宠" aria-label="桌宠" onClick={() => setView({ name: "pets" })}><Cat className="h-4 w-4" /></Button>
                <Button variant="ghost" size="icon" className="h-8 w-8" title="提醒记录" onClick={() => setView({ name: "records" })}><History className="h-4 w-4" /></Button>
                <Button variant="ghost" size="icon" className="h-8 w-8" title="声音库" onClick={() => setView({ name: "library" })}><Music className="h-4 w-4" /></Button>
              </div>
              <button onClick={startAdd} title="添加提醒方式"
                className="inline-flex h-8 w-8 items-center justify-center rounded-full bg-orange-500 text-white shadow-lg shadow-orange-500/30 transition hover:-translate-y-px hover:bg-orange-600"><Plus className="h-5 w-5" /></button>
            </div>
          </>
        ) : (
          <>
            <div className="flex min-w-0 items-center gap-3">
              <Button variant="outline" size="icon" className="h-9 w-9 rounded-lg" title="返回（Esc）" onClick={goBack}><ChevronLeft className="h-4 w-4" /></Button>
              <h1 className="truncate text-lg font-semibold">{title}</h1>
            </div>
            <div className="flex items-center gap-1">
              {view.name === "records" && (<>
                <Button variant="ghost" size="sm" onClick={() => { qc.invalidateQueries({ queryKey: ["events"] }); toast.success("已刷新"); }}><RefreshCw className="h-3.5 w-3.5" />刷新</Button>
                <Button variant="ghost" size="sm" className="text-red-500 hover:text-red-600" onClick={clearRecords}><Trash2 className="h-3.5 w-3.5" />清空</Button>
              </>)}
              {view.name === "library" && <Button variant="ghost" size="sm" onClick={() => api.reveal("sounds")}><FolderOpen className="h-3.5 w-3.5" />打开文件夹</Button>}
            </div>
          </>
        )}
      </header>

      <main className="mx-auto max-w-[860px] px-6 pb-28 pt-6">
        <AnimatePresence mode="wait">
          <motion.div key={view.name + (view.name === "edit" ? view.draft.id : "")} initial={{ opacity: 0, x: view.name === "home" ? 0 : 16 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0 }} transition={{ duration: 0.2 }}>
            {view.name === "home" && <HomeView state={state} tool={tool} onEdit={(t, i) => setView({ name: "edit", draft: structuredClone(t), isNew: false, index: i })} onAdd={startAdd} confirm={confirm} />}
            {view.name === "edit" && <EditView state={state} tool={tool} view={view} onDone={goBack} onLibrary={(draft) => setView({ name: "library", from: { ...view, draft } })} confirm={confirm} />}
            {view.name === "settings" && <SettingsView ref={settings} state={state} tool={tool} confirm={confirm} />}
            {view.name === "records" && <RecordsView state={state} tool={tool} />}
            {view.name === "library" && <LibraryView state={state} confirm={confirm} />}
            {view.name === "pets" && <PetsView state={state} confirm={confirm} />}
          </motion.div>
        </AnimatePresence>
      </main>
      {dialog}
      <Toaster />
    </div>
  );
}

export type ConfirmFn = ReturnType<typeof useConfirm>["confirm"];
