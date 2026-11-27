import type {Meta, StoryObj} from '@storybook/react-vite';
import {initialLive} from './live';
import {LiveBar} from './live-bar';

const meta = {title: 'Shell/Live bar', component: LiveBar, args: {conn: 'open', live: {...initialLive, engine: 'connected', streamState: 'native_worker_active', program: {scene_id: 'speaker', revision: 7}}}} satisfies Meta<typeof LiveBar>;
export default meta;
type Story = StoryObj<typeof meta>;

export const OnAir: Story = {};
export const Reconnecting: Story = {args: {conn: 'closed', live: {...initialLive, engine: 'unavailable', lastError: 'engine socket closed', engineRestarted: true}}};
