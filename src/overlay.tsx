/* 透明网页叠在原生毛玻璃面板上，独立样式避免主窗口的背景覆盖透明度。 */
import { useState } from "react";
import ReactDOM from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import fallbackIcon from "../src-tauri/icons/128x128.png";
import "./overlay.css";

const p = new URLSearchParams(location.search);
declare global {
  interface Window {
    __AGENTPULSE_OVERLAY__?: { title: string; body: string; host: string; icon: string | null };
  }
}
const content = window.__AGENTPULSE_OVERLAY__;
const title = content?.title ?? p.get("title") ?? "";
const body = content?.body ?? p.get("body") ?? "";
const host = content?.host ?? p.get("host") ?? "";
const icon = content?.icon ?? p.get("icon") ?? "";

async function close() {
  await getCurrentWindow().close();
}
function Overlay() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function activate() {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      if (host) {
        // Rust 成功返回来源后会关闭这一组窗口；不要再对已销毁的窗口调用 close。
        await invoke("activate_app", { bundle: host });
      } else {
        await close();
      }
    } catch (e) {
      console.error("返回来源失败", e);
      setError("无法返回来源 App，点击重试");
      setBusy(false);
    }
  }

  return (
    <article className="overlay-card" aria-label="AgentPulse 提醒">
      <button className="overlay-content" onClick={activate} disabled={busy}
        title={host ? "返回来源 App 并清除它的悬浮提醒" : "关闭这条提醒"}>
        <span className="overlay-copy">
          <span className="overlay-heading">
            <img src={icon || fallbackIcon} alt="" className="overlay-icon"
              onError={(e) => { e.currentTarget.onerror = null; e.currentTarget.src = fallbackIcon; }} />
            <span className="overlay-title">{title}</span>
          </span>
          <span className="overlay-body">{body}</span>
          {error && <span className="overlay-error" role="alert">{error}</span>}
        </span>
      </button>
      <button onClick={close} title="关闭这条提醒" aria-label="关闭这条提醒" className="overlay-close">
        <svg width="9" height="9" viewBox="0 0 10 10" fill="none" aria-hidden="true">
          <path d="m1 1 8 8M9 1 1 9" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
      </button>
    </article>
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(<Overlay />);
