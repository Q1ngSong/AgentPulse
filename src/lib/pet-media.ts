import { invoke } from "@tauri-apps/api/core";
import { PET_NAMES, defaultPetAnimations } from "./pet-animation";
import type { PetAnimations, PetPhase } from "./pet-animation";
const files = import.meta.glob<string>("../../assets/pet/frames/*/*.png", { eager:true, query:"?url", import:"default" });
export type PetMedia = { clips:PetAnimations; images:Record<PetPhase,HTMLImageElement[]> };
let builtin: Promise<PetMedia> | undefined;
async function decode(urls:string[]) {
  return Promise.all(urls.map(async src=>{const image=new Image();image.src=src;await image.decode();return image;}));
}
export function builtinMedia():Promise<PetMedia> {
  return builtin ??= (async()=>{
    const images={} as PetMedia["images"];
    for(const phase of Object.keys(PET_NAMES) as PetPhase[]) {
      const urls=Object.entries(files).filter(([path])=>path.includes(`/frames/${phase}/`)).sort(([a],[b])=>a.localeCompare(b)).map(([,url])=>url);
      if(urls.length!==50) throw new Error(`${phase} 内置帧不完整`);
      images[phase]=await decode(urls);
    }
    return {clips:defaultPetAnimations(),images};
  })();
}
export async function loadPetMedia(clips:PetAnimations):Promise<PetMedia> {
  const defaults=await builtinMedia(); const images={...defaults.images};
  for(const phase of Object.keys(PET_NAMES) as PetPhase[]) {
    if(!clips[phase].folder) continue;
    const result=await invoke<{count:number;frames:string[]}>("load_pet_frames",{folder:clips[phase].folder});
    if(result.count!==clips[phase].frames) throw new Error(`${PET_NAMES[phase]}素材帧数已变化，请在提醒卡片中重新检查文件夹`);
    images[phase]=await decode(result.frames);
  }
  return {clips,images};
}
