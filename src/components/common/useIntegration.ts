import { useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { api, ToolKey } from "@/lib/api";
import type { ConfirmFn } from "@/App";

const MANUAL_TRUST = "可重新接入，或在 Codex CLI 输入 /hooks，手动信任 AgentPulse 条目。";
const CODEX_EVENT_LABEL: Record<string, string> = {
  permissionrequest: "请求权限", stop: "任务完成", userpromptsubmit: "提交消息", pretooluse: "开始执行", posttooluse: "工具完成",
};

export function useIntegration(agents: Record<ToolKey, string>, confirm: ConfirmFn) {
  const qc = useQueryClient();
  const locked = useRef(false);
  const [busy, setBusy] = useState<{ agent: ToolKey | "all"; label: string } | null>(null);

  const install = async (agent: ToolKey) => {
    if (locked.current) return;
    locked.current = true;
    setBusy({ agent, label: "接入中…" });
    let configured = false;
    try {
      const result = await api.install(agent);
      configured = true;
      void qc.invalidateQueries({ queryKey: ["state"] });
      if (agent === "codex") {
        setBusy({ agent, label: "检查信任…" });
        const review = await api.reviewCodexHooks();
        if (!review.hooks.length) throw new Error("未找到可核验的 AgentPulse Hook。");
        const pending = review.hooks.filter((hook) => !hook.trusted);
        if (pending.length) {
          const commands = [...new Set(pending.map((hook) => hook.command))].join("\n");
          const events = [...new Set(pending.map((hook) => CODEX_EVENT_LABEL[hook.event.toLowerCase()] ?? hook.event))].join("、");
          const approved = await confirm({
            title: "信任 AgentPulse Hook？",
            body: `允许以下 Hook 将 Codex 的任务状态和消息交给本机 AgentPulse 提醒。本次只信任列出的 ${pending.length} 条 Hook。\n\n命令：\n${commands}\n\n事件：${events}`,
            okText: "信任",
          });
          if (!approved) {
            toast("Hook 已写入，尚未信任", { description: `未信任的 Hook 暂不能发送 Codex 提醒。${MANUAL_TRUST}` });
            return;
          }
          setBusy({ agent, label: "信任中…" });
          const verified = await api.trustCodexHooks(review.hooks);
          if (verified.hooks.length !== review.hooks.length || verified.hooks.some((hook) => !hook.trusted)
            || review.hooks.some((expected) => !verified.hooks.some((hook) => hook.key === expected.key && hook.hash === expected.hash && hook.trusted))) {
            throw new Error("未能确认这些 Hook 已全部信任。");
          }
        }
        toast.success("Codex Hook 已配置并信任", { description: "新开 Codex 会话后，用真实任务确认提醒是否收到。" });
      } else {
        toast.success(`已接入 ${agents[agent]}`, { description: result.backup ? "原文件已备份，新开会话后生效" : "新开会话后生效" });
      }
    } catch (e) {
      const trustIncomplete = configured && agent === "codex";
      toast.error(trustIncomplete ? "Hook 已写入，信任未完成" : "接入失败", {
        description: `${String(e)}${trustIncomplete ? ` ${MANUAL_TRUST}` : ""}`,
      });
    } finally {
      locked.current = false;
      setBusy(null);
    }
  };

  const uninstall = async (agent: ToolKey | ToolKey[]) => {
    if (locked.current) return;
    const selected = typeof agent === "string" ? [agent] : [...new Set(agent)];
    if (!selected.length) return;
    const all = Array.isArray(agent);
    locked.current = true;
    setBusy({ agent: all ? "all" : selected[0], label: all ? "卸载中…" : "断开中…" });
    try {
      if (!(await confirm({
        title: all ? "卸载提醒？" : `断开 ${agents[selected[0]]}`,
        body: `将移除 ${selected.map(key => agents[key]).join("、")} 中 AgentPulse 的提醒接入，保留其他 Hook 和已保存的提醒配置。\n\n正在运行的会话可能仍使用旧接入，请重启这些会话后生效；已进入延迟队列的消息不受此操作影响。`,
        okText: all ? "卸载提醒" : "断开", danger: true,
      }))) return;
      const removed: string[] = [];
      const failed: string[] = [];
      for (const key of selected) {
        try {
          const result = await api.uninstall(key);
          if (result.error || result.installed || Object.values(result.events).some(Boolean)) {
            throw new Error(result.error || "仍检测到提醒接入，请重试");
          }
          removed.push(agents[key]);
        } catch (e) { failed.push(`${agents[key]}：${String(e)}`); }
      }
      void qc.invalidateQueries({ queryKey: ["state"] });
      if (failed.length) {
        toast.error(removed.length ? "部分提醒卸载失败" : "卸载失败", {
          description: [...(removed.length ? [`已移除：${removed.join("、")}`] : []), ...failed].join("\n"),
        });
      } else {
        toast.success(all ? "提醒接入已卸载" : `已断开 ${removed[0]}`, { description: "重启已有 Claude Code / Codex 会话后生效。" });
      }
    } catch (e) {
      toast.error("卸载失败", { description: String(e) });
    } finally {
      locked.current = false;
      setBusy(null);
    }
  };

  return { install, uninstall, busy };
}
