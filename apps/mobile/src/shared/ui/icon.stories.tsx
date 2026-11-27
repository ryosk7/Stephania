import type {Meta, StoryObj} from '@storybook/react-native';
import {View} from 'react-native';
import {Icon, icons, type IconName} from './icon';
import {Footnote} from './text';

const meta = {title: 'UI/Icons', component: Icon, args: {name: 'broadcast'}} satisfies Meta<typeof Icon>;
export default meta;
type Story = StoryObj<typeof meta>;

/** The whole set: what each name means on screen. */
export const All: Story = {
  render: () => (
    <View className="flex-row flex-wrap gap-3">
      {(Object.keys(icons) as IconName[]).map(n => (
        <View key={n} className="w-20 items-center gap-1.5 rounded-control border border-border bg-surface py-3">
          <Icon name={n} size={22} />
          <Footnote numberOfLines={1}>{n}</Footnote>
        </View>
      ))}
    </View>
  ),
};
