import type {Meta, StoryObj} from '@storybook/react-vite';
import {Dashboard} from './dashboard';
import {initialLive} from './live';
import {StudioContext, type StudioContextValue} from './studio-context';

const context = {
  api: {} as StudioContextValue['api'],
  me: {kind: 'user', username: 'olga', role: 'admin'} as StudioContextValue['me'],
  live: initialLive, conn: 'open', canEdit: true, onMessage: () => {}, logout: () => {},
} satisfies StudioContextValue;

const meta = {
  title: 'Pages/Dashboard',
  component: Dashboard,
  decorators: [Story => <StudioContext value={context}><Story/></StudioContext>],
} satisfies Meta<typeof Dashboard>;
export default meta;
export const Stream: StoryObj<typeof meta> = {};
