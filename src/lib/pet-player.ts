/** 配置使用 1 起始闭区间，播放器内部使用 0 起始帧号。 */
import { defaultPetAnimations } from "./pet-animation.ts";
import type { PetAnimation, PetPhase } from "./pet-animation.ts";
export type { PetPhase } from "./pet-animation.ts";
export type Playback = { phase: PetPhase; key: string; frame: number; leaving: boolean; profile?: string };
export type Desired = { phase: PetPhase; key: string; profile?: string };
export function nextFrame(current: Playback, desired: Desired, clip: PetAnimation = defaultPetAnimations()[current.phase], nextClip: PetAnimation = defaultPetAnimations()[desired.phase]): Playback {
  const changed = desired.phase !== current.phase || desired.key !== current.key || desired.profile !== current.profile;
  if (current.frame >= clip.exit[1]-1) return { ...desired, frame: nextClip.enter[0]-1, leaving: false };
  if (current.frame === clip.enter[1]-1) return { ...current, frame: clip.cycle[0]-1 };
  if (current.frame === clip.cycle[1]-1 && !current.leaving) {
    if (changed) return { ...current, frame: clip.exit[0]-1, leaving: true };
    if (clip.hold) return current;
    return { ...current, frame: clip.cycle[0]-1 };
  }
  return { ...current, frame: current.frame+1 };
}
