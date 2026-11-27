import type {Meta, StoryObj} from '@storybook/react-native';
import {View} from 'react-native';
import {ButtonRow, DestructiveButton, IconButton, PrimaryButton, SecondaryButton, TextButton} from './buttons';

const noop = () => {};

const meta = {
  title: 'UI/Buttons',
  component: PrimaryButton,
  args: {title: 'Control', icon: 'control', onPress: noop},
} satisfies Meta<typeof PrimaryButton>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Primary: Story = {};
export const Busy: Story = {args: {busy: true}};
export const Disabled: Story = {args: {disabled: true}};

/** Every variant as used on the screens: icon plus one word, icon-only beside it. */
export const AllVariants: Story = {
  render: () => (
    <View className="gap-3">
      <ButtonRow>
        <IconButton shape="square" icon="camera" label="Send this phone's camera" onPress={noop} />
        <PrimaryButton title="Control" icon="control" onPress={noop} />
      </ButtonRow>
      <SecondaryButton title="Nearby" icon="bluetooth" onPress={noop} />
      <DestructiveButton title="Stop" icon="stop" accessibilityLabel="Stop stream" onPress={noop} />
      <PrimaryButton title="Join" icon="arrow" onPress={noop} />
      <View className="flex-row items-center gap-3">
        <IconButton icon="back" label="Back" onPress={noop} />
        <IconButton icon="settings" label="Settings" onPress={noop} />
        <IconButton shape="inline" icon="pencil" label="Rename" onPress={noop} />
        <IconButton icon="close" label="Close" disabled onPress={noop} />
      </View>
      <TextButton title="Already on this network? Set up access only" onPress={noop} />
    </View>
  ),
};
