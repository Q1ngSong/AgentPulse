import assert from "node:assert/strict";
import { test } from "node:test";
import { nextFrame } from "./pet-player.ts";
import type { Playback } from "./pet-player.ts";

test("切换请求不会中断当前动作，先经过共同结束帧", () => {
  let current: Playback = { phase: "working", key: "working", frame: 12, leaving: false };
  const desired = { phase: "permission_request" as const, key: "message-a" };
  for (let frame = 13; frame <= 49; frame++) {
    current = nextFrame(current, desired);
    assert.equal(current.phase, "working");
    assert.equal(current.frame, frame);
  }
  current = nextFrame(current, desired);
  assert.deepEqual(current, { ...desired, frame: 0, leaving: false });
});
test("同一状态只循环主体，完成状态保留稿纸等待", () => {
  const working: Playback = { phase: "working", key: "working", frame: 44, leaving: false };
  assert.equal(nextFrame(working, working).frame, 5);
  const complete: Playback = { phase: "task_complete", key: "a", frame: 44, leaving: false };
  assert.equal(nextFrame(complete, complete).frame, 44);
  assert.equal(nextFrame(complete, { phase: "task_complete", key: "b" }).frame, 45);
});
test("收尾期间再次改变状态，在共同起点接最新状态", () => {
  const current: Playback = { phase: "sleeping", key: "sleeping", frame: 49, leaving: true };
  assert.deepEqual(nextFrame(current, { phase: "working", key: "working" }),
    { phase: "working", key: "working", frame: 0, leaving: false });
});

test("自定义片段支持间隔和不同帧数，切入下一素材的指定起点", () => {
  const clip={folder:"/tmp/frames",frames:90,frame_ms:50,enter:[2,8] as [number,number],cycle:[12,65] as [number,number],exit:[75,88] as [number,number],hold:false};
  const next={...clip,enter:[3,8] as [number,number]};
  let p:Playback={phase:"working",key:"a",frame:7,leaving:false};
  assert.equal(nextFrame(p,p,clip,next).frame,11);
  p={...p,frame:64};assert.equal(nextFrame(p,p,clip,next).frame,11);
  const desired={phase:"sleeping" as const,key:"b"};
  p=nextFrame(p,desired,clip,next);assert.equal(p.frame,74);assert.equal(p.leaving,true);
  while(p.frame<87) p=nextFrame(p,desired,clip,next);
  assert.deepEqual(nextFrame(p,desired,clip,next),{...desired,frame:2,leaving:false});
});

test("修改同一状态的素材配置，也先退出旧素材", () => {
  const p:Playback={phase:"working",key:"a",profile:"old",frame:44,leaving:false};
  const desired={phase:"working" as const,key:"a",profile:"new"};
  const result=nextFrame(p,desired);assert.equal(result.profile,"old");assert.equal(result.frame,45);
  assert.equal(nextFrame({...result,frame:49},desired).profile,"new");
});
