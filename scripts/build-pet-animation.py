#!/usr/bin/env python3
"""从已确认的第一版关键帧生成 50 帧透明动画；不生成或替换角色原画。"""
import hashlib
import json
from pathlib import Path

import cv2
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / 'assets' / 'pet'
SIZE = 384
ROWS = [(0, 310), (310, 620), (620, 949), (949, 1254)]
COLS = [[0, 311, 623, 933, 1254], [0, 311, 621, 940, 1254],
        [0, 310, 624, 942, 1254], [0, 310, 627, 949, 1254]]
TIMELINES = {
    'working': [(5, 1), (12, 0), (19, 1), (24, 2), (29, 3), (36, 0), (44, 1)],
    'sleeping': [(5, 0), (18, 1), (31, 2), (37, 3), (44, 0)],
    'permission': [(5, 0), (14, 2), (20, 0), (24, 1), (28, 0), (37, 3), (44, 0)],
    'complete': [(5, 0), (17, 1), (27, 2), (44, 3)],
}


def normalize(image):
    """按固定比例、地面基线放入同一画布，保留整个草席和举牌。"""
    scale = .88
    image = image.resize((round(image.width * scale), round(image.height * scale)), Image.Resampling.LANCZOS)
    frame = Image.new('RGBA', (SIZE, SIZE))
    frame.paste(image, ((SIZE - image.width) // 2, SIZE - 28 - image.height))
    return frame


def premultiply(image):
    """预乘透明度，让插帧边缘不混入透明像素中的底色。"""
    pixels = np.array(image).astype(np.float32) / 255
    pixels[:, :, :3] *= pixels[:, :, 3:]
    return pixels


def tween(a, b, count):
    """使用双向稠密光流移动像素，再混合遮挡区域，产生关键帧之间的画面。"""
    pa, pb = premultiply(a), premultiply(b)
    def gray(pixels):
        white = pixels[:, :, :3] + (1 - pixels[:, :, 3:])
        return cv2.cvtColor(np.uint8(np.clip(white * 255, 0, 255)), cv2.COLOR_RGB2GRAY)
    estimator = cv2.DISOpticalFlow_create(cv2.DISOPTICAL_FLOW_PRESET_MEDIUM)
    estimator.setFinestScale(0)
    ga, gb = gray(pa), gray(pb)
    forward = estimator.calc(ga, gb, None)
    backward = estimator.calc(gb, ga, None)
    yy, xx = np.mgrid[:SIZE, :SIZE].astype(np.float32)
    def warp(pixels, flow, fraction):
        # 迭代求反向采样位置，RGBA 共用位移，避免透明轮廓与颜色错开。
        mx, my = xx.copy(), yy.copy()
        for _ in range(3):
            displacement = cv2.remap(flow, mx, my, cv2.INTER_LINEAR, borderMode=cv2.BORDER_REPLICATE)
            mx, my = xx - fraction * displacement[:, :, 0], yy - fraction * displacement[:, :, 1]
        return cv2.remap(pixels, mx, my, cv2.INTER_LINEAR, borderMode=cv2.BORDER_CONSTANT)
    result = []
    for i in range(1, count):
        t = i / count
        t = t * t * (3 - 2 * t)
        wa, wb = warp(pa, forward, t), warp(pb, backward, 1 - t)
        # 大姿势变化的遮挡区域不做双影叠加：保留较近关键帧的像素。
        # 能对齐的区域继续混合；不能对齐的爪子、尾巴和面部避免出现第二套轮廓。
        difference = np.max(np.abs(wa - wb), axis=2)
        occluded = cv2.GaussianBlur(np.float32(difference > .18), (5, 5), 0)[:, :, None]
        nearer = wa if t < .5 else wb
        pixels = ((1 - t) * wa + t * wb) * (1 - occluded) + nearer * occluded
        pixels[:, :, :3] /= np.maximum(pixels[:, :, 3:], 1e-6)
        result.append(Image.fromarray(np.uint8(np.clip(pixels * 255 + .5, 0, 255))))
    return result


def build():
    """导出逐帧图集、WebP 动画与校验记录；各段首尾复用同一原图。"""
    source = Image.open(ASSETS / 'cat-atlas.png').convert('RGBA')
    neutral = normalize(Image.open(ASSETS / 'keyframe.png').convert('RGBA'))
    neutral.save(ASSETS / 'neutral.png')
    endpoint = hashlib.sha256(neutral.tobytes()).hexdigest()
    for row, (name, timeline) in enumerate(TIMELINES.items()):
        top, bottom = ROWS[row]
        keys = [normalize(source.crop((COLS[row][i], top, COLS[row][i + 1], bottom))) for i in range(4)]
        anchors = [(0, neutral)] + [(index, keys[key]) for index, key in timeline] + [(49, neutral)]
        frames = [neutral]
        for (start, a), (end, b) in zip(anchors, anchors[1:]):
            frames.extend(tween(a, b, end - start))
            frames.append(b)
        assert len(frames) == 50
        sheet = Image.new('RGBA', (SIZE * 10, SIZE * 5))
        for index, frame in enumerate(frames):
            bounds = frame.getbbox()
            assert bounds and min(bounds[0], bounds[1], SIZE - bounds[2], SIZE - bounds[3]) >= 16, (name, index, bounds)
            sheet.paste(frame, (index % 10 * SIZE, index // 10 * SIZE))
        assert frames[0].tobytes() == frames[-1].tobytes() == neutral.tobytes()
        folder = ASSETS / 'frames' / {'permission': 'permission_request', 'complete': 'task_complete'}.get(name, name)
        folder.mkdir(parents=True, exist_ok=True)
        for i, frame in enumerate(frames):
            frame.save(folder / f'{i + 1:04}.png')
        sheet.save(ASSETS / f'{name}-atlas.webp', lossless=True)
        output = ASSETS / f'{name}.webp'
        frames[0].save(output, save_all=True, append_images=frames[1:], duration=80, loop=0, lossless=True)
        with Image.open(output) as animation:
            assert animation.n_frames == 50, (name, animation.n_frames)
        unique = len({hashlib.sha256(frame.tobytes()).hexdigest() for frame in frames})
        report = {'frames': 50, 'unique_frames': unique, 'frame_ms': 80, 'source_keyframes': 4,
                  'method': 'bidirectional DIS optical flow; confirmed original art',
                  'canvas': [SIZE, SIZE], 'columns': 10, 'body_start': 5, 'body_end': 44,
                  'shared_endpoint_sha256': endpoint, 'anchors': timeline}
        (ASSETS / f'{name}.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
        print(f'{name}: 50 frames, {unique} unique, shared endpoints and margins verified', flush=True)


if __name__ == '__main__':
    build()
