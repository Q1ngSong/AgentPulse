/* 检查并安装更新：更新包由 GitHub Releases 的 latest.json 描述，minisign 签名校验 */
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { toast } from "sonner";

export async function checkForUpdate(silent = false): Promise<void> {
  let update;
  try { update = await check(); }
  catch (e) { if (!silent) toast.error("检查更新失败", { description: String(e) }); return; }
  if (!update) { if (!silent) toast.success("已是最新版本"); return; }
  const go = window.confirm(`发现新版本 ${update.version}${update.body ? `\n\n${update.body}` : ""}\n\n现在下载并安装？安装后会自动重启。`);
  if (!go) return;
  const id = toast.loading(`正在下载 ${update.version}…`);
  try {
    await update.downloadAndInstall();
    toast.success("安装完成，正在重启", { id });
    await relaunch();
  } catch (e) { toast.error("更新失败", { description: String(e), id }); }
}

/* 每天最多自动检查一次 */
export function autoCheckOncePerDay() {
  const key = "ap.lastUpdateCheck";
  const last = Number(localStorage.getItem(key) || 0);
  if (Date.now() - last < 24 * 3600 * 1000) return;
  localStorage.setItem(key, String(Date.now()));
  checkForUpdate(true);
}
