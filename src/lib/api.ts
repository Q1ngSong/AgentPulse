/* 前端唯一的后端入口：全部通过 Tauri 命令调用 Rust 核心 */
import { defaultPetAnimations } from "./pet-animation";
import type { PetAnimations } from "./pet-animation";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type EventKey = "permission_request" | "task_complete";
export type ToolKey = "claude" | "codex";
export type TargetType = "desktop" | "sound" | "wxtest" | "feishu";

export interface Target {
  id: string;
  type: TargetType;
  name: string;
  enabled: boolean;
  events: EventKey[];
  // 类型相关字段
  mode?: "overlay" | "system" | "both";
  click?: "activate" | "close";
  sounds?: Partial<Record<EventKey, string>>;
  volume?: number;
  appid?: string;
  secret?: string;
  openid?: string;
  template_id?: string;
  webhook?: string;
  delay_minutes?: number;
  batch?: boolean;
  [k: string]: unknown;
}

export interface Template { title: string; body: string }
export interface PetSource { agent: ToolKey; host: string }
export interface PetEnabledChange { id: string; enabled: boolean }
export interface PetConfig {
  id: string; number: number; name: string; enabled: boolean;
  events: EventKey[]; animations: PetAnimations; sources: PetSource[];
}
export interface Config { templates: Record<EventKey, Template>; tools: Record<ToolKey, { targets: Target[] }>; pets: PetConfig[] }
export interface Integration {
  agent: ToolKey; name: string; file: string; file_exists: boolean; tool_dir_exists: boolean;
  events: Record<string, boolean>; installed: boolean; error: string | null; backup: string | null;
}
export interface CodexHook { key: string; hash: string; event: string; command: string; trusted: boolean }
export interface ToolStats { today: number; failed_today: number; queued: number }
export interface State {
  config: Config;
  integrations: Record<ToolKey, Integration>;
  stats: Record<ToolKey, ToolStats>;
  icons: Record<ToolKey, string | null>;
  pet_sources: (PetSource & { name: string })[];
  hook_binary: string;
  hook_binary_exists: boolean;
  meta: { events: Record<EventKey, string>; agents: Record<ToolKey, string>; data_dir: string };
}
export interface SendResult { target_id?: string; name: string; type?: string; ok: boolean; error?: string; info?: string; ms: number; pending?: boolean; skipped?: boolean }
export interface EventRecord {
  id: string; ts: number; source: "hook" | "simulate" | "test" | "delayed"; agent: ToolKey; event: EventKey;
  project?: string; title: string; body: string; results: SendResult[]; payload?: unknown;
}
export interface QueueItem {
  ref: string; created: number; agent: ToolKey; event: EventKey; title: string; body: string; targets: string[];
}
export interface Sound { ref: string; name: string; kind: "system" | "custom"; ext: string; size: number | null }

export const api = {
  getState: () => invoke<State>("get_state"),
  onPetEnabledChanged: (handler: (change: PetEnabledChange) => void) => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen<PetEnabledChange>("pet-enabled-changed", ({ payload }) => { if (!disposed) handler(payload); })
      .then(unlisten => { if (disposed) unlisten(); else stop = unlisten; })
      .catch(error => console.error("监听桌宠状态失败", error));
    return () => { disposed = true; stop?.(); };
  },
  saveToolTargets: (agent: ToolKey, targets: Target[]) => invoke<void>("save_tool_targets", { agent, targets }),
  copyTarget: (agent: ToolKey, id: string, to: ToolKey) => invoke<string>("copy_target", { agent, id, to }),
  saveTemplates: (templates: Record<EventKey, Template>) => invoke<void>("save_templates", { templates }),
  savePets: (pets: PetConfig[], expected: PetConfig[]) => invoke<void>("save_pets", { pets, expected }),
  testPet: (pet: PetConfig, event: EventKey) => invoke<EventRecord>("test_pet", { pet, event }),
  testTarget: (agent: ToolKey, target: Target, event: EventKey) => invoke<EventRecord>("test_target", { agent, target, event }),
  simulate: (agent: ToolKey, event: EventKey) => invoke<EventRecord>("simulate", { agent, event }),
  inspectPetFolder: (folder: string) => invoke<{count:number;width:number;height:number}>("inspect_pet_folder", {folder}),
  listSounds: () => invoke<Sound[]>("list_sounds"),
  uploadSound: (name: string, data: Uint8Array) => invoke<string>("upload_sound", { name, data: Array.from(data) }),
  renameSound: (ref: string, name: string) => invoke<string>("rename_sound", { ref, name }),
  deleteSound: (ref: string) => invoke<void>("delete_sound", { ref }),
  previewSound: (ref: string, volume = 100) => invoke<void>("preview_sound", { ref, volume }),
  install: (agent: ToolKey) => invoke<Integration>("install_integration", { agent }),
  reviewCodexHooks: () => invoke<{ hooks: CodexHook[] }>("review_codex_hooks"),
  trustCodexHooks: (hooks: CodexHook[]) => invoke<{ hooks: CodexHook[] }>("trust_codex_hooks", { hooks }),
  uninstall: (agent: ToolKey) => invoke<Integration>("uninstall_integration", { agent }),
  readEvents: (limit = 300) => invoke<EventRecord[]>("read_events", { limit }),
  readQueue: () => invoke<QueueItem[]>("read_queue"),
  clearEvents: () => invoke<void>("clear_events"),
  reveal: (key: string) => invoke<void>("reveal", { key }),
  uploadToolIcon: (agent: ToolKey, filename: string, data: Uint8Array) => invoke<void>("upload_tool_icon", { agent, filename, data: Array.from(data) }),
  deleteToolIcon: (agent: ToolKey) => invoke<void>("delete_tool_icon", { agent }),
};

export const TYPE_META: Record<TargetType, { name: string; desc: string; singleton: boolean }> = {
  desktop: { name: "屏幕弹窗", desc: "悬浮窗或系统通知", singleton: true },
  sound: { name: "提示音", desc: "声音库里挑声音", singleton: true },
  wxtest: { name: "微信测试号", desc: "推送到你的微信", singleton: false },
  feishu: { name: "飞书机器人", desc: "推送到飞书群", singleton: false },
};
export const EVENT_KEYS: EventKey[] = ["permission_request", "task_complete"];
export const DELAYS = [0, 1, 3, 5, 10, 15, 30];

export function newTarget(type: TargetType): Target {
  const rand = () => Math.random().toString(36).slice(2, 8);
  const base: Target = { id: TYPE_META[type].singleton ? type : `${type}-${rand()}`, type, name: TYPE_META[type].name, enabled: true, events: [...EVENT_KEYS] };
  if (type === "desktop") Object.assign(base, { mode: "overlay", click: "activate" });
  if (type === "sound") Object.assign(base, { sounds: { permission_request: "system:Ping", task_complete: "system:Glass" }, volume: 100 });
  if (type === "wxtest") Object.assign(base, { name: "我的微信", appid: "", secret: "", openid: "", template_id: "", delay_minutes: 5, batch: true });
  if (type === "feishu") Object.assign(base, { name: "飞书群", webhook: "", secret: "", delay_minutes: 0, batch: true });
  return base;
}

export function newPet(pets: PetConfig[]): PetConfig {
  const number = Math.max(0, ...pets.map((pet) => pet.number)) + 1;
  return { id: `pet-${crypto.randomUUID()}`, number, name: "", enabled: true,
    events: [...EVENT_KEYS], animations: defaultPetAnimations(), sources: [] };
}

export const soundLabel = (ref?: string) => {
  if (!ref) return "未选择";
  const i = ref.indexOf(":");
  const kind = ref.slice(0, i), name = ref.slice(i + 1);
  return kind === "custom" ? name.replace(/\.[^.]+$/, "") : name;
};
export const fmtTime = (ts: number) => {
  const d = new Date(ts * 1000);
  return d.toDateString() === new Date().toDateString()
    ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })
    : d.toLocaleString([], { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" });
};
