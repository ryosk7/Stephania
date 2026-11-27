import type {Meta, StoryObj} from '@storybook/react-native';
import {View} from 'react-native';
import type {SceneLayer} from '@/shared/device';
import {SceneThumbnail} from './scene-thumbnail';

const layer = (id: string, x: number, y: number, width: number, height: number, source: string): SceneLayer =>
  ({id, type: 'video', source, x, y, width, height, visible: true}) as SceneLayer;

const layouts: Record<string, SceneLayer[]> = {
  'Speaker + slides': [layer('s', 490, 0, 1430, 1080, 'hdmi:0'), layer('c', 0, 545, 470, 535, 'phone:rear')],
  'Slides full': [layer('s', 0, 0, 1920, 1080, 'hdmi:0')],
  'Q&A — both cameras': [layer('a', 0, 0, 952, 1080, 'phone:rear'), layer('b', 968, 0, 952, 1080, 'phone:front')],
};

const meta = {title: 'Control/Layout miniature', component: SceneThumbnail, args: {layers: layouts['Speaker + slides']}} satisfies Meta<typeof SceneThumbnail>;
export default meta;

export const Miniatures: StoryObj<typeof meta> = {
  render: () => (
    <View className="flex-row flex-wrap gap-3">
      {Object.entries(layouts).map(([name, layers]) => (
        <View key={name} className="aspect-video w-[48%] overflow-hidden rounded-tile border border-border bg-surface">
          <SceneThumbnail layers={layers} />
        </View>
      ))}
    </View>
  ),
};
