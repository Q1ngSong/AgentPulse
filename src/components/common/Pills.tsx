import { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";

export interface PillOption<T extends string | number> { value: T; label: string; icon?: LucideIcon }

/* 胶囊单选/多选按钮组 */
export function Pills<T extends string | number>({ options, isOn, onToggle }: { options: PillOption<T>[]; isOn: (v: T) => boolean; onToggle: (v: T) => void }) {
  return (
    <div className="flex flex-wrap gap-1.5">
      {options.map(({ value, label, icon: Icon }) => {
        const on = isOn(value);
        return (
          <button key={String(value)} type="button" onClick={() => onToggle(value)}
            className={cn("inline-flex h-[30px] items-center gap-1.5 rounded-full border px-3 text-xs font-medium transition-all",
              on ? "border-blue-500 bg-blue-500 text-white shadow-[0_4px_10px_-4px_rgba(10,132,255,.6)]" : "border-border bg-background text-muted-foreground hover:border-blue-500/40 hover:text-foreground")}>
            {Icon && <Icon className="h-3.5 w-3.5" />}{label}
          </button>
        );
      })}
    </div>
  );
}
