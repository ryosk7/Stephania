// Miniature of a layout, drawn from the scene's own layers (docs/SCENE.md),
// so the tile shows where the slides and cameras go — no screenshots.
import {StyleSheet, View} from 'react-native';
import type {SceneLayer} from '@/shared/device';
import {color, Icon} from '@/shared/ui';

const W = 1920, H = 1080;

function kind(l: SceneLayer): 'slides' | 'camera' | 'overlay' {
  if ((l.type ?? 'video') !== 'video') return 'overlay';
  return l.source?.startsWith('phone:') ? 'camera' : 'slides';
}

export function SceneThumbnail({layers}: {layers: SceneLayer[] | undefined}) {
  const visible = (layers ?? []).filter(l => l.visible);
  return (
    <View style={st.canvas} accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
      {visible.map(l => {
        const k = kind(l);
        return (
          <View key={l.id} style={[st.layer, {
            left: `${(l.x / W) * 100}%`, top: `${(l.y / H) * 100}%`, width: `${(l.width / W) * 100}%`, height: `${(l.height / H) * 100}%`,
            backgroundColor: k === 'slides' ? '#48484A' : k === 'camera' ? '#636366' : (l.color ?? '#3A3A3C'),
            opacity: k === 'overlay' ? 0.7 : 1,
          }]}>
            {k !== 'overlay' && l.width > W / 6 && <Icon name={k === 'camera' ? 'person' : 'slides'} size={14} tint="rgba(235,235,245,0.5)" />}
          </View>
        );
      })}
    </View>
  );
}

const st = StyleSheet.create({
  canvas: {position: 'absolute', left: 8, right: 8, top: 8, bottom: 8},
  layer: {position: 'absolute', borderRadius: 3, alignItems: 'center', justifyContent: 'center', overflow: 'hidden'},
});
