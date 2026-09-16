/* 装饰性波形：按名字生成固定图案，不代表真实音频 */
export function Wave({ seed, active, playing }: { seed: string; active?: boolean; playing?: boolean }) {
  let h = 7;
  for (const c of seed) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  const bars = Array.from({ length: 12 }, () => { h = (h * 1103515245 + 12345) >>> 0; return 4 + (h % 15); });
  return (
    <div className={`flex h-[18px] items-end gap-[2px] ${playing ? "wave-playing" : ""}`}>
      {bars.map((b, i) => <i key={i} style={{ height: b }} className={`block w-[3px] rounded-sm ${active || playing ? "bg-blue-500" : "bg-gray-300"}`} />)}
    </div>
  );
}
