import { useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Pencil, Trash2, Upload } from "lucide-react";
import { toast } from "sonner";
import { api, Sound, State } from "@/lib/api";
import { SoundTile } from "@/components/common/SoundTile";
import type { ConfirmFn } from "@/App";

export function LibraryView({ state, confirm }: { state: State; confirm: ConfirmFn }) {
  const qc = useQueryClient();
  const { data: sounds = [] } = useQuery({ queryKey: ["sounds"], queryFn: api.listSounds });
  const [playing, setPlaying] = useState<string | null>(null);
  const timer = useRef<number>();
  const used = new Set(Object.values(state.config.tools).flatMap((tl) => tl.targets.filter((t) => t.type === "sound").flatMap((t) => Object.values(t.sounds ?? {}))));

  const play = async (ref: string) => {
    setPlaying(ref);
    try { await api.previewSound(ref, 100); } catch (e) { toast.error("播放失败", { description: String(e) }); }
    window.clearTimeout(timer.current); timer.current = window.setTimeout(() => setPlaying(null), 1600);
  };
  const upload = async (files: FileList | File[]) => {
    for (const f of Array.from(files)) {
      try { await api.uploadSound(f.name, new Uint8Array(await f.arrayBuffer())); toast.success("已上传", { description: f.name }); }
      catch (e) { toast.error(`上传失败：${f.name}`, { description: String(e) }); }
    }
    qc.invalidateQueries({ queryKey: ["sounds"] });
  };
  const pick = () => { const i = document.createElement("input"); i.type = "file"; i.accept = ".mp3,.wav,.aiff,.aif,.m4a,.caf,audio/*"; i.multiple = true; i.onchange = () => i.files && upload(i.files); i.click(); };
  const rename = async (s: Sound) => {
    const name = window.prompt("新的名字", s.name);
    if (!name || name === s.name) return;
    try { await api.renameSound(s.ref, name); qc.invalidateQueries({ queryKey: ["sounds"] }); toast.success("已重命名", { description: "已选用这个声音的提示音需要重新选择" }); } catch (e) { toast.error("重命名失败", { description: String(e) }); }
  };
  const remove = async (s: Sound) => {
    if (!(await confirm({ title: "删除声音", body: `确定删除「${s.name}」吗？正在使用它的提示音会改用 Glass。`, okText: "删除", danger: true }))) return;
    try { await api.deleteSound(s.ref); qc.invalidateQueries({ queryKey: ["sounds"] }); toast.success("已删除"); } catch (e) { toast.error("删除失败", { description: String(e) }); }
  };
  const custom = sounds.filter((s) => s.kind === "custom"), system = sounds.filter((s) => s.kind === "system");
  const tile = (s: Sound) => (
    <SoundTile key={s.ref} sound={s} playing={playing === s.ref} onPlay={() => play(s.ref)}
      right={<span className="flex items-center gap-1">{used.has(s.ref) ? <span className="rounded-md bg-emerald-100 px-1.5 text-[10px] font-semibold text-emerald-700">使用中</span> : <span className="rounded-md bg-slate-200 px-1.5 text-[10px] font-semibold text-slate-700">{s.ext}</span>}
        {s.kind === "custom" && <><button className="rounded p-1 text-muted-foreground hover:bg-muted" title="重命名" onClick={() => rename(s)}><Pencil className="h-3 w-3" /></button><button className="rounded p-1 text-muted-foreground hover:bg-red-50 hover:text-red-500" title="删除" onClick={() => remove(s)}><Trash2 className="h-3 w-3" /></button></>}</span>} />
  );
  return (
    <div className="rounded-xl border bg-card px-5 py-4">
      <div className="mb-2 flex items-center justify-between text-xs font-semibold text-muted-foreground"><span>我的声音 · {custom.length}</span><span className="font-normal">两个工具共用，保存在 {state.meta.data_dir}/sounds</span></div>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(132px,1fr))] gap-2.5">{custom.map(tile)}
        <button type="button" onClick={pick} onDragOver={(e) => e.preventDefault()} onDrop={(e) => { e.preventDefault(); upload(e.dataTransfer.files); }}
          className="flex min-h-[92px] flex-col items-center justify-center gap-1 rounded-lg border-[1.5px] border-dashed border-slate-300 text-xs text-muted-foreground transition-all hover:border-blue-500 hover:bg-blue-500/5 hover:text-blue-500"><Upload className="h-5 w-5" /><span>上传或拖入声音</span><span className="text-[10px]">mp3 / wav / aiff / m4a · ≤ 5 MB</span></button></div>
      <div className="mb-2 mt-3.5 text-xs font-semibold text-muted-foreground">系统音效 · {system.length}</div>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(132px,1fr))] gap-2.5">{system.map(tile)}</div>
    </div>
  );
}
