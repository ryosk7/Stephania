import type {Meta, StoryObj} from '@storybook/react-native';
import {View} from 'react-native';
import {CodeCard} from './code-card';

const meta = {title: 'Maintenance/Code', component: CodeCard, args: {code: '492013', secondsLeft: 18, source: 'device'}} satisfies Meta<typeof CodeCard>;
export default meta;

export const DeviceClock: StoryObj<typeof meta> = {};
export const EndingSoon: StoryObj<typeof meta> = {args: {secondsLeft: 4, copied: true}};
export const PhoneClock: StoryObj<typeof meta> = {
  render: args => <View className="gap-4"><CodeCard {...args} source="phone" /></View>,
};
