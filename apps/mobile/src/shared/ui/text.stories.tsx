import type {Meta, StoryObj} from '@storybook/react-native';
import {View} from 'react-native';
import {Body, Display, Footnote, Headline, Label, Mono, Overline, Title} from './text';

const meta = {title: 'UI/Text', component: Body, args: {children: 'Body 16'}} satisfies Meta<typeof Body>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Scale: Story = {
  render: () => (
    <View className="gap-2">
      <Display>Display 32</Display>
      <Title>Title 22</Title>
      <Headline>Headline 16</Headline>
      <Body>Body 16</Body>
      <Label>Label 14</Label>
      <Footnote>Footnote 13</Footnote>
      <Overline>Overline 11</Overline>
      <Mono>Mono 13 · stephania-8f21.local</Mono>
    </View>
  ),
};
