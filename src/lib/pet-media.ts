import { invoke, isTauri } from "@tauri-apps/api/core";
import manifest from "../../src-tauri/pet-assets.json";
import { PET_NAMES, defaultPetAnimations } from "./pet-animation";
import type { PetAnimations, PetPhase } from "./pet-animation";
export type PetFrame = { image: HTMLImageElement; x: number; y: number; width: number; height: number };
export type PetMedia = { clips: PetAnimations; images: Record<PetPhase, PetFrame[]> };
let builtin: Promise<PetMedia> | undefined;
async function decode(src: string) {
  const image = new Image(); image.src = src; await image.decode(); return image;
}
export function builtinMedia(): Promise<PetMedia> {
  return builtin ??= (async () => {
    // 浏览器演示直接读取同一下载源；桌面端由 Rust 校验并缓存，不把图片编入前端。
    const urls = isTauri() ? await invoke<Record<string, string>>("load_builtin_pet")
      : Object.fromEntries(manifest.files.map(file => [file.name, `${manifest.base_url}/${file.name}`]));
    const images = {} as PetMedia["images"];
    await Promise.all((Object.keys(PET_NAMES) as PetPhase[]).map(async phase => {
      const file = manifest.files.find(file => file.phase === phase)!;
      const image = await decode(urls[file.name]);
      if (image.naturalWidth !== 3840 || image.naturalHeight !== 1920) throw new Error(`${PET_NAMES[phase]}图集尺寸不正确`);
      images[phase] = Array.from({ length: 50 }, (_, i) => ({ image, x: i % 10 * 384, y: Math.floor(i / 10) * 384, width: 384, height: 384 }));
    }));
    return { clips: defaultPetAnimations(), images };
  })().catch(error => { builtin = undefined; throw error; });
}
export async function loadPetMedia(clips: PetAnimations): Promise<PetMedia> {
  const images = Object.values(clips).some(clip => !clip.folder) ? { ...(await builtinMedia()).images } : {} as PetMedia["images"];
  for (const phase of Object.keys(PET_NAMES) as PetPhase[]) {
    if (!clips[phase].folder) continue;
    const result = await invoke<{ count: number; frames: string[] }>("load_pet_frames", { folder: clips[phase].folder });
    if (result.count !== clips[phase].frames) throw new Error(`${PET_NAMES[phase]}素材帧数已变化，请在桌宠设置中重新检查文件夹`);
    images[phase] = await Promise.all(result.frames.map(async src => {
      const image = await decode(src);
      return { image, x: 0, y: 0, width: image.naturalWidth, height: image.naturalHeight };
    }));
  }
  return { clips, images };
}
