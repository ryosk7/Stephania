// Miniature of a layout, drawn from the scene's own layers (docs/SCENE.md),
// so the tile shows where the slides and cameras go — no screenshots.
import {View} from 'react-native';
import type {SceneLayer} from '@/shared/device';
import {Icon} from '@/shared/ui';

const W = 1920, H = 1080;

function kind(l: SceneLayer): 'slides' | 'camera' | 'overlay' {
  if ((l.type ?? 'video') !== 'video') return 'overlay';
  return l.source?.startsWith('phone:') ? 'camera' : 'slides';
}

export function SceneThumbnail({layers}: {layers: SceneLayer[] | undefined}) {
  const visible = (layers ?? []).filter(l => l.visible);
  return (
    <View className="absolute inset-[7px]" accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      {visible.map(l => {
        const k = kind(l);
        return (
          <View key={l.id}
            className={`absolute items-center justify-center overflow-hidden rounded ${k === 'slides' ? 'bg-mini-slides' : k === 'camera' ? 'bg-mini-camera' : ''}`}
            style={{
              left: `${(l.x / W) * 100}%`, top: `${(l.y / H) * 100}%`, width: `${(l.width / W) * 100}%`, height: `${(l.height / H) * 100}%`,
              ...(k === 'overlay' ? {backgroundColor: l.color ?? '#3A3A3C', opacity: 0.7} : null),
            }}>
            {k !== 'overlay' && l.width > W / 6 && <Icon name={k === 'camera' ? 'person' : 'slides'} size={14} color="mini-mark" />}
          </View>
        );
      })}
    </View>
  );
}
