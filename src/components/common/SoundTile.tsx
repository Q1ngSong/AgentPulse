import { Check, Pause, Play } from "lucide-react";
import { Sound } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Wave } from "./Wave";

export function SoundTile({ sound, selected, playing, onPick, onPlay, right }: {
  sound: Sound; selected?: boolean; playing?: boolean; onPick?: () => void; onPlay: () => void; right?: React.ReactNode;
}) {
  return (
    <div role={onPick ? "button" : undefined} tabIndex={onPick ? 0 : undefined} onClick={onPick}
      className={cn("flex flex-col gap-2 rounded-lg border p-2.5 text-left transition-all",
        onPick && "cursor-pointer hover:border-blue-500/40 hover:shadow-sm",
        selected && "border-blue-500 bg-blue-500/5 shadow-[0_0_0_3px_rgba(10,132,255,.12)]")}>
      <div className="flex items-center justify-between">
        <button type="button" onClick={(e) => { e.stopPropagation(); onPlay(); }} title="试听"
          className={cn("inline-flex h-7 w-7 items-center justify-center rounded-full bg-muted transition-colors hover:bg-gray-200", (selected || playing) && "bg-blue-500 text-white hover:bg-blue-600")}>
          {playing ? <Pause className="h-3 w-3" /> : <Play className="h-3 w-3" />}
        </button>
        {right ?? (selected
          ? <span className="inline-flex h-4 w-4 items-center justify-center rounded-full bg-blue-500 text-white"><Check className="h-3 w-3" /></span>
          : <span className="rounded-md bg-slate-200 px-1.5 text-[10px] font-semibold text-slate-700">{sound.ext}</span>)}
      </div>
      <Wave seed={sound.ref} active={selected} playing={playing} />
      <div className="truncate text-xs font-medium" title={sound.name}>{sound.name}</div>
    </div>
  );
}
