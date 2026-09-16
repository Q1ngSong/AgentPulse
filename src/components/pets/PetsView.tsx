import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Cat, ChevronLeft, Pencil, Play, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { PetAnimationEditor } from "@/components/edit/PetAnimationEditor";
import { api, EVENT_KEYS, newPet } from "@/lib/api";
import type { EventKey, PetConfig, State, ToolKey } from "@/lib/api";
import type { ConfirmFn } from "@/App";
import { PetSourceSelector } from "./PetSourceSelector";

function sourceSummary(pet: PetConfig, state: State): string {
  const groups = (Object.keys(state.meta.agents) as ToolKey[]).flatMap((agent) => {
    const sources = pet.sources.filter((source) => source.agent === agent);
    if (!sources.length) return [];
    const names = sources.some((source) => !source.host) ? "所有来源 App"
      : sources.map((source) => state.pet_sources?.find((known) => known.agent === agent && known.host === source.host)?.name || source.host).join("、");
    return [`${state.meta.agents[agent]}：${names}`];
  });
  return groups.join("；") || "未选择同步来源";
}

export function PetsView({ state, confirm }: { state: State; confirm: ConfirmFn }) {
  const qc = useQueryClient();
  const pets = state.config.pets;
  const [editor, setEditor] = useState<{ pet: PetConfig; isNew: boolean; original?: PetConfig } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const locked = useRef(false);
  useEffect(() => {
    setEditor(current => {
      if (!current || current.isNew || !current.original) return current;
      const saved = pets.find(pet => pet.id === current.pet.id);
      if (!saved || saved.enabled === current.original.enabled) return current;
      // 外部关闭只同步开关和编辑基线，保留其他尚未保存的设置。
      return { ...current, pet: { ...current.pet, enabled: saved.enabled }, original: { ...current.original, enabled: saved.enabled } };
    });
  }, [pets]);
  const currentPets = () => qc.getQueryData<State>(["state"])?.config.pets ?? pets;
  const patch = (changes: Partial<PetConfig>) => setEditor((current) => current && { ...current, pet: { ...current.pet, ...changes } });
  const perform = async (label: string, task: () => Promise<void>) => {
    if (locked.current) return;
    locked.current = true;
    setBusy(label);
    try { await task(); }
    catch (e) { toast.error(`${label.replace(/中…$/, "")}失败`, { description: String(e) }); }
    finally { locked.current = false; setBusy(null); }
  };
  const persist = async (transform: (latest: PetConfig[]) => PetConfig[]) => {
    try {
      const latest = (await api.getState()).config.pets;
      const next = transform(latest);
      if (next.some(pet => pet.enabled && Object.values(pet.animations).some(clip => !clip.folder) && !latest.some(saved => saved.id === pet.id && saved.enabled && JSON.stringify(saved.animations) === JSON.stringify(pet.animations)))) setBusy("正在准备桌宠素材，首次使用需下载…");
      await api.savePets(next, latest);
      qc.setQueryData<State>(["state"], (current) => current && { ...current, config: { ...current.config, pets: next } });
      void qc.invalidateQueries({ queryKey: ["state"] });
      return next;
    } catch (e) {
      void qc.invalidateQueries({ queryKey: ["state"] });
      throw new Error(`${String(e)} 请刷新后重试。`);
    }
  };
  const save = () => perform("保存中…", async () => {
    if (!editor) return;
    const draft = editor.pet;
    const saved = await persist((latest) => {
      if (editor.isNew) {
        const number = latest.some((item) => item.number === draft.number) ? Math.max(0, ...latest.map((item) => item.number)) + 1 : draft.number;
        return [...latest, { ...draft, name: draft.name.trim(), number }];
      }
      const current = latest.find((item) => item.id === draft.id);
      if (!current) throw new Error("这只桌宠已被删除，请返回列表。");
      const original = editor.original;
      if (!original) throw new Error("缺少编辑前的桌宠设置，请重新打开编辑页。");
      const merged = { ...current };
      if (draft.name !== original.name) merged.name = draft.name.trim();
      if (draft.enabled !== original.enabled) merged.enabled = draft.enabled;
      if (JSON.stringify(draft.events) !== JSON.stringify(original.events)) merged.events = draft.events;
      if (JSON.stringify(draft.animations) !== JSON.stringify(original.animations)) merged.animations = draft.animations;
      if (JSON.stringify(draft.sources) !== JSON.stringify(original.sources)) merged.sources = draft.sources;
      return latest.map((item) => item.id === draft.id ? merged : item);
    });
    const pet = saved.find((item) => item.id === draft.id)!;
    setEditor(null);
    toast.success(`已${editor.isNew ? "添加" : "保存"} ${pet.number} 号桌宠`, { description: !pet.sources.length ? "尚未选择来源，暂不接收任务状态或消息。" : undefined });
  });
  const toggle = (pet: PetConfig, enabled: boolean) => perform("保存中…", async () => {
    await persist((latest) => {
      if (!latest.some((item) => item.id === pet.id)) throw new Error("这只桌宠已被删除。");
      return latest.map((item) => item.id === pet.id ? { ...item, enabled } : item);
    });
    toast.success(`${pet.number} 号桌宠已${enabled ? "启用" : "停用"}`);
  });
  const remove = (pet: PetConfig) => perform("删除中…", async () => {
    if (!(await confirm({ title: `删除 ${pet.number} 号桌宠`, body: `删除「${pet.name || `桌宠 ${pet.number}`}」及它的动作和来源设置。素材文件会保留。`, okText: "删除", danger: true }))) return;
    await persist((latest) => latest.filter((item) => item.id !== pet.id));
    toast.success("已删除桌宠");
  });
  const test = (pet: PetConfig, event: EventKey) => perform("测试中…", async () => {
    setBusy("正在准备桌宠素材，首次使用需下载…");
    const result = await api.testPet(pet, event);
    if (!result.results.length) throw new Error("桌宠未返回测试结果。");
    const failed = result.results.filter((item) => !item.ok);
    if (failed.length) throw new Error(failed.map((item) => item.error || "未能显示测试提醒").join("；"));
    toast.success("已显示桌宠测试", { description: `${pet.number} 号 · ${state.meta.events[event]}` });
  });

  if (editor) {
    const pet = editor.pet;
    return <div className="flex flex-col gap-4">
      <div className="flex items-center gap-2">
        <Button variant="ghost" size="sm" disabled={!!busy} onClick={() => setEditor(null)}><ChevronLeft className="h-3.5 w-3.5" />桌宠列表</Button>
        <span className="text-sm font-semibold">{editor.isNew ? "添加" : "编辑"} {pet.number} 号桌宠</span>
      </div>
      <fieldset disabled={!!busy} className="contents">
        <section className="rounded-xl border bg-card p-5">
          <div className="flex flex-wrap items-center gap-3">
            <span className="flex h-10 min-w-10 items-center justify-center rounded-lg bg-amber-100 px-2 text-sm font-semibold text-amber-800">#{pet.number}</span>
            <label className="min-w-0 flex-1 text-xs text-muted-foreground">名字
              <Input className="mt-1" value={pet.name} maxLength={80} onChange={(e) => patch({ name: e.target.value })} placeholder={`桌宠 ${pet.number}`} />
            </label>
            <label className="flex items-center gap-2 text-[13px]">启用<Switch checked={pet.enabled} onCheckedChange={(enabled) => patch({ enabled })} /></label>
          </div>
          <div className="mt-4 border-t pt-3">
            <div className="mb-2 text-[13px] text-muted-foreground">接收提醒</div>
            <div className="flex flex-wrap gap-4">{EVENT_KEYS.map((event) => <label key={event} className="flex cursor-pointer items-center gap-2 text-[13px]">
              <input type="checkbox" className="h-4 w-4 accent-blue-500" checked={pet.events.includes(event)}
                onChange={(e) => patch({ events: EVENT_KEYS.filter((item) => item === event ? e.target.checked : pet.events.includes(item)) })} />
              {state.meta.events[event]}
            </label>)}</div>
            {!pet.events.length && <p className="mt-2 text-xs text-amber-700">未选择提醒类型，不会显示请求权限或任务完成提醒。</p>}
          </div>
        </section>
        <PetSourceSelector value={pet.sources} state={state} onChange={(sources) => patch({ sources })} />
        <PetAnimationEditor value={pet.animations} onChange={(animations) => patch({ animations })} />
        <section className="rounded-xl border bg-card p-5">
          <h2 className="text-sm font-semibold">测试当前动作设置</h2>
          <p className="mb-3 mt-1 text-xs text-muted-foreground">使用当前草稿真实显示桌宠测试，保存后才用于同步所选来源的消息。</p>
          <div className="flex flex-wrap gap-2">{EVENT_KEYS.map((event) => <Button key={event} type="button" size="sm" variant="outline" onClick={() => test(pet, event)}>
            <Play className="h-3.5 w-3.5" />测试{state.meta.events[event]}
          </Button>)}</div>
        </section>
      </fieldset>
      <div className="fixed inset-x-0 bottom-0 z-10 border-t bg-background/90 backdrop-blur-md">
        <div className="mx-auto flex max-w-[860px] items-center justify-end gap-3 px-6 py-3.5">
          <Button variant="outline" disabled={!!busy} onClick={() => setEditor(null)}>取消</Button>
          <Button disabled={!!busy} onClick={save}>{busy ?? (editor.isNew ? "添加桌宠" : "保存")}</Button>
        </div>
      </div>
    </div>;
  }

  return <div className="flex flex-col gap-3">
    <div className="mb-1 flex items-start justify-between gap-4">
      <p className="max-w-lg text-[13px] leading-relaxed text-muted-foreground">每只桌宠有独立编号、动作和同步来源。可以让一只桌宠接收多个 Agent、多个 App 的消息，也可以分别配置。</p>
      <Button size="sm" disabled={!!busy} onClick={() => setEditor({ pet: newPet(currentPets()), isNew: true })}><Plus className="h-3.5 w-3.5" />添加桌宠</Button>
    </div>
    <p className="text-xs text-muted-foreground">默认猫咪首次启用时下载，之后可离线使用。安装包不包含桌宠图片。</p>
    {busy && <p role="status" className="text-sm text-muted-foreground">{busy}</p>}
    {!pets.length ? <div className="rounded-xl border-[1.5px] border-dashed p-10 text-center">
      <div className="mx-auto mb-3 flex h-14 w-14 items-center justify-center rounded-full bg-amber-50 text-amber-600"><Cat className="h-7 w-7" /></div>
      <h2 className="font-semibold">添加第一只桌宠</h2>
      <p className="mt-1 text-sm text-muted-foreground">给它起个名字，再勾选要同步的消息来源。</p>
      <Button className="mt-4" disabled={!!busy} onClick={() => setEditor({ pet: newPet(currentPets()), isNew: true })}><Plus className="h-4 w-4" />添加桌宠</Button>
    </div> : pets.map((pet) => <section key={pet.id} className="flex flex-wrap items-center gap-3 rounded-xl border bg-card p-4">
      <span className="flex h-10 min-w-10 items-center justify-center rounded-lg bg-amber-100 px-2 text-sm font-semibold text-amber-800">#{pet.number}</span>
      <div className="min-w-0 flex-1">
        <h2 className="break-words text-[15px] font-semibold">{pet.name || `桌宠 ${pet.number}`}</h2>
        <p className={`mt-1 break-words text-xs ${pet.sources.length ? "text-muted-foreground" : "text-amber-700"}`}>{sourceSummary(pet, state)}</p>
        <p className="mt-1 text-xs text-muted-foreground">{pet.events.map((event) => state.meta.events[event]).join(" · ") || "未选择提醒类型"}</p>
      </div>
      <div className="flex items-center gap-1">
        <Button variant="ghost" size="icon" className="h-8 w-8" title={`编辑 ${pet.number} 号桌宠`} disabled={!!busy} onClick={() => setEditor({ pet: structuredClone(pet), original: structuredClone(pet), isNew: false })}><Pencil className="h-4 w-4" /></Button>
        <Button variant="ghost" size="icon" className="h-8 w-8 text-red-500" title={`删除 ${pet.number} 号桌宠`} disabled={!!busy} onClick={() => remove(pet)}><Trash2 className="h-4 w-4" /></Button>
        <Switch checked={pet.enabled} disabled={!!busy} onCheckedChange={(enabled) => toggle(pet, enabled)} aria-label={`${pet.number} 号桌宠${pet.enabled ? "已启用" : "已停用"}`} />
      </div>
    </section>)}
  </div>;
}
