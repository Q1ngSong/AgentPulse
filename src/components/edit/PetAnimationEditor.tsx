import { useState, useRef, useEffect } from "react";
import { FolderOpen } from "lucide-react";
import { toast } from "sonner";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { PET_NAMES } from "@/lib/pet-animation";
import type { PetAnimations, PetPhase, PetAnimation } from "@/lib/pet-animation";

export function PetAnimationEditor({ value, onChange }: { value:PetAnimations; onChange:(value:PetAnimations)=>void }) {
  const [busy,setBusy]=useState<PetPhase|null>(null);
  const current=useRef(value);current.current=value;
  const alive=useRef(true);useEffect(()=>{alive.current=true;return ()=>{alive.current=false;};},[]);
  const patch=(phase:PetPhase, changes:Partial<PetAnimation>)=>onChange({...current.current,[phase]:{...current.current[phase],...changes}});
  async function inspect(phase:PetPhase) {
    setBusy(phase);
    try { const folder=value[phase].folder;const info=await api.inspectPetFolder(folder);if(!alive.current || current.current[phase].folder!==folder)return;patch(phase,{frames:info.count});toast.success(`${info.count} 帧 · ${info.width} × ${info.height}`,{description:"请核对三个片段的起止帧号"}); }
    catch(e){toast.error("素材检查失败",{description:String(e)});}finally{if(alive.current)setBusy(null);}
  }
  return <section className="rounded-xl border bg-card p-5">
    <h2 className="font-semibold">动作素材与播放片段</h2>
    <p className="mt-1 text-xs text-muted-foreground">每个状态一个文件夹，PNG 从 0001.png 连续编号。帧号从 1 开始，起止帧均包含在片段内。四个状态的首尾图须一致。</p>
    {(Object.keys(PET_NAMES) as PetPhase[]).map(phase=>{
      const a=value[phase]; return <fieldset key={phase} className="mt-5 border-t pt-4">
        <legend className="px-1 text-sm font-semibold">{PET_NAMES[phase]}</legend>
        <label className="text-xs text-muted-foreground" htmlFor={`${phase}-folder`}>素材文件夹</label>
        <div className="mt-1 flex gap-2"><Input id={`${phase}-folder`} value={a.folder} placeholder={`内置：assets/pet/frames/${phase}`} onChange={e=>patch(phase,{folder:e.target.value})}/>
          <Button type="button" size="sm" variant="outline" disabled={!a.folder || busy!==null} onClick={()=>inspect(phase)}><FolderOpen className="h-3.5 w-3.5"/>检查文件夹</Button></div>
        <p className="mt-1 text-xs text-muted-foreground">留空使用内置素材；自定义目录请填完整路径并检查。{a.frames} 帧。</p>
        <div className="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-3">
          {([["enter","进入（过渡）"],["cycle","循环主体"],["exit","退出（过渡）"]] as const).map(([key,label])=><div key={key}>
            <div className="mb-1 text-xs text-muted-foreground">{label}</div><div className="flex items-center gap-1">
            {[0,1].map(index=><Input key={index} aria-label={`${PET_NAMES[phase]} ${label}${index===0?"起始":"结束"}帧`} type="number" min="1" max={a.frames} value={a[key][index]}
              onChange={e=>{const range:[number,number]=[...a[key]];range[index]=Number(e.target.value);patch(phase,{[key]:range});}}/>)}</div>
          </div>)}
        </div>
        <div className="mt-3 flex flex-wrap items-center gap-3 text-xs">
          <label className="flex items-center gap-2">每帧毫秒<Input className="w-20" aria-label={`${PET_NAMES[phase]} 每帧毫秒`} type="number" min="16" max="1000" value={a.frame_ms} onChange={e=>patch(phase,{frame_ms:Number(e.target.value)})}/></label>
          <label className="flex items-center gap-2"><input type="checkbox" checked={a.hold} onChange={e=>patch(phase,{hold:e.target.checked})}/>主体播完停在末帧</label>
          {!a.hold && <span className="text-muted-foreground">保持此状态时重复循环主体</span>}
        </div>
      </fieldset>;
    })}
  </section>;
}
