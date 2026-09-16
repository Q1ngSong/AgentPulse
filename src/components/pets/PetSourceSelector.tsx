import type { PetSource, State, ToolKey } from "@/lib/api";

export function selectPetSource(sources: PetSource[], agent: ToolKey, host: string, selected: boolean): PetSource[] {
  const other = sources.filter((source) => source.agent !== agent);
  const current = sources.filter((source) => source.agent === agent);
  if (!selected) return [...other, ...current.filter((source) => source.host !== host)];
  if (!host) return [...other, { agent, host: "" }];
  return [...other, ...current.filter((source) => source.host && source.host !== host), { agent, host }];
}

export function PetSourceSelector({ value, state, onChange }: {
  value: PetSource[]; state: State; onChange: (sources: PetSource[]) => void;
}) {
  return <section className="rounded-xl border bg-card p-5">
    <h2 className="text-sm font-semibold">同步消息来源</h2>
    <p className="mt-1 text-xs text-muted-foreground">勾选这只桌宠要同步的 Agent 和来源 App，可跨工具多选。选具体 App 时会取消该工具的「所有来源 App」。</p>
    <div className="mt-4 grid gap-4 sm:grid-cols-2">
      {(Object.keys(state.meta.agents) as ToolKey[]).map((agent) => {
        const options = new Map<string, string>();
        for (const source of state.pet_sources ?? []) {
          if (source.agent === agent && source.host) options.set(source.host, source.name || source.host);
        }
        for (const source of value) {
          if (source.agent === agent && source.host && !options.has(source.host)) options.set(source.host, source.host);
        }
        const selected = (host: string) => value.some((source) => source.agent === agent && source.host === host);
        return <fieldset key={agent} className="min-w-0 rounded-lg border px-3.5 pb-3.5 pt-2">
          <legend className="px-1 text-sm font-semibold">{state.meta.agents[agent]}</legend>
          <label className="flex cursor-pointer items-center gap-2 py-1.5 text-[13px] font-medium">
            <input type="checkbox" className="h-4 w-4 accent-blue-500" checked={selected("")} onChange={(e) => onChange(selectPetSource(value, agent, "", e.target.checked))} />
            所有来源 App
          </label>
          <div className="mt-1 border-t pt-1">
            {[...options].sort((a, b) => a[1].localeCompare(b[1], "zh-CN")).map(([host, name]) => <label key={host} className="flex cursor-pointer items-start gap-2 py-1.5 text-[13px]">
              <input type="checkbox" className="mt-0.5 h-4 w-4 flex-none accent-blue-500" checked={selected(host)} onChange={(e) => onChange(selectPetSource(value, agent, host, e.target.checked))} />
              <span className={name === host ? "min-w-0 break-all" : "min-w-0 break-words"} title={name !== host ? host : undefined}>{name}</span>
            </label>)}
            {!options.size && <p className="py-2 text-xs text-muted-foreground">尚未发现来源 App。收到该工具的真实消息后会列在这里，也可先选择所有来源。</p>}
          </div>
        </fieldset>;
      })}
    </div>
    {!value.length && <p className="mt-3 text-xs text-amber-700">尚未选择同步来源；可以保存，但这只桌宠暂不会收到任务状态或消息。</p>}
  </section>;
}
