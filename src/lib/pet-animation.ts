export type PetPhase = "working" | "sleeping" | "permission_request" | "task_complete";
export const PET_NAMES: Record<PetPhase,string> = { working:"工作中", sleeping:"休息中", permission_request:"等你批准", task_complete:"任务完成" };
export type PetAnimation = { folder: string; frames: number; frame_ms: number; enter: [number,number]; cycle: [number,number]; exit: [number,number]; hold: boolean };
export type PetAnimations = Record<PetPhase,PetAnimation>;
export function defaultPetAnimations(): PetAnimations {
  return Object.fromEntries(Object.keys(PET_NAMES).map(p=>[p,{folder:"",frames:50,frame_ms:80,enter:[1,5],cycle:[6,45],exit:[46,50],hold:p==="task_complete"}])) as PetAnimations;
}
