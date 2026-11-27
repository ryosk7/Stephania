import type {Meta, StoryObj} from '@storybook/react-native';
import {useState} from 'react';
import {View} from 'react-native';
import {PrimaryButton, SecondaryButton} from './buttons';
import {Toggle} from './controls';
import {Card, List, ListIcon, SectionHeading, Sheet, StatusView, TopBar} from './layout';
import {Footnote} from './text';

const noop = () => {};
const meta = {title: 'UI/Layout', component: Card, args: {children: null}} satisfies Meta<typeof Card>;
export default meta;
type Story = StoryObj<typeof meta>;

export const TopBars: Story = {
  render: () => (
    <View className="-mx-4 gap-2">
      <TopBar onBack={noop} title="Settings" />
      <TopBar onBack={noop} backIcon="close" />
    </View>
  ),
};

export const Cards: Story = {
  render: () => (
    <View className="gap-4">
      <Card>
        <Card.Header title="Hall A" />
        <Footnote>Speaker + slides</Footnote>
        <PrimaryButton title="Control" icon="control" onPress={noop} />
      </Card>
      <Card outlined="warning"><Card.Header title="Studio B" /></Card>
    </View>
  ),
};

function Audio() {
  const [hdmi, setHdmi] = useState(true);
  const [mic, setMic] = useState(false);
  return (
    <List.Section heading={<SectionHeading icon="speaker" label="Audio" />}>
      <List.Row title="HDMI" leading={<ListIcon icon={hdmi ? 'speaker' : 'speakerOff'} color={hdmi ? 'text' : 'tertiary'} />} accessory={<Toggle value={hdmi} onChange={setHdmi} label="HDMI audio on air" />} />
      <List.Row title="Mic" leading={<ListIcon icon={mic ? 'mic' : 'micOff'} color={mic ? 'text' : 'tertiary'} />} accessory={<Toggle value={mic} onChange={setMic} label="Microphone on air" />} />
    </List.Section>
  );
}
export const Lists: Story = {
  render: () => (
    <View className="gap-6">
      <Audio />
      <List.Section>
        <List.Row title="Venue-Staff" selected onPress={noop} />
        <List.Row title="Venue-Guest" onPress={noop} />
        <List.Row title="Old-Office-AP" detail="Not supported" disabled onPress={noop} />
      </List.Section>
    </View>
  ),
};

export const Status: Story = {render: () => <View className="h-80"><StatusView icon="done" tone="success" title="Hall A is ready" body="Reachable at https://stephania-8f21.local/" /></View>};

function SheetDemo() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <SecondaryButton title="Open sheet" onPress={() => setOpen(true)} />
      <Sheet visible={open} title="Stop the stream?" icon="stop" onClose={() => setOpen(false)}>
        <Footnote>Viewers will see the stream end.</Footnote>
      </Sheet>
    </>
  );
}
export const Sheets: Story = {render: () => <SheetDemo />};
