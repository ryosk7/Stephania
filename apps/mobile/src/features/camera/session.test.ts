import {readFileSync} from 'node:fs';
import {beforeEach, describe, expect, it} from 'vitest';
import type {CameraEvents, CameraMode, Capabilities, NativeCamera, Offer, ThermalState, TrackPlan} from '@modules/t3box-camera';
import {DeviceError} from '@/shared/device/client';
import {BACKOFF_MS, CameraSession, DISCONNECT_GRACE_MS, type CameraClient, type Timers} from './session';

const SDP = readFileSync(new URL('./fixtures/offer-dual.sdp', import.meta.url), 'utf8');
// Single-camera offer: the first media section of the dual fixture.
const SINGLE_SDP = SDP.slice(0, SDP.indexOf('m=video', SDP.indexOf('m=video') + 1));

class FakeNative implements NativeCamera {
  caps: Capabilities = {roles: ['rear', 'front'], dualProfiles: ['720p30+720p30', '720p30+540p24', '540p24+540p24'], hardwareH264: true};
  thermal: ThermalState = 'nominal';
  log: string[] = [];
  applied: TrackPlan[][] = [];
  answer?: string;
  offerOverride?: () => Promise<Offer>;
  private handlers: {[K in keyof CameraEvents]?: Set<CameraEvents[K]>} = {};

  capabilities = async () => this.caps;
  thermalState = () => this.thermal;
  async startCapture(mode: CameraMode) { this.log.push(`capture ${mode}`); }
  async stopCapture() { this.log.push('stop capture'); }
  async createOffer(plans: TrackPlan[]): Promise<Offer> {
    this.log.push(`offer ${plans.map(p => p.role).join('+')}`);
    if (this.offerOverride) return this.offerOverride();
    return plans.length === 2 ? {sdp: SDP, tracks: {'rear-1': 'rear', 'front-1': 'front'}} : {sdp: SINGLE_SDP, tracks: {'rear-1': plans[0]!.role}};
  }
  async setAnswer(sdp: string) { this.answer = sdp; this.log.push('answer'); }
  async applyPlans(plans: TrackPlan[]) { this.applied.push(plans); }
  async closePeer() { this.log.push('close'); }
  addListener<E extends keyof CameraEvents>(event: E, f: CameraEvents[E]) {
    const set = (this.handlers[event] ??= new Set() as never) as Set<CameraEvents[E]>;
    set.add(f);
    return {remove: () => set.delete(f)};
  }
  emit<E extends keyof CameraEvents>(event: E, e: Parameters<CameraEvents[E]>[0]) {
    for (const f of (this.handlers[event] ?? []) as Set<(x: typeof e) => void>) f(e);
  }
}

class FakeTimers implements Timers {
  now = 0;
  private queue: {at: number; f: () => void; id: number}[] = [];
  private next = 1;
  setTimeout(f: () => void, ms: number) { const id = this.next++; this.queue.push({at: this.now + ms, f, id}); return id; }
  clearTimeout(id: unknown) { this.queue = this.queue.filter(t => t.id !== id); }
  pending() { return this.queue.map(t => t.at - this.now).sort((a, b) => a - b); }
  async advance(ms: number) {
    const end = this.now + ms;
    for (;;) {
      this.queue.sort((a, b) => a.at - b.at);
      const t = this.queue[0];
      if (!t || t.at > end) break;
      this.queue.shift();
      this.now = t.at;
      t.f();
      await flush();
    }
    this.now = end;
  }
}

const flush = () => new Promise(r => setTimeout(r, 0));

class FakeClient implements CameraClient {
  offers: {tracks: Record<string, string>; replace?: boolean}[] = [];
  closed = 0;
  respond: (replace?: boolean) => Promise<{sdp: string}> = async () => ({sdp: 'v=0 answer'});
  async cameraOffer(_sdp: string, tracks: Record<string, 'rear' | 'front'>, replace?: boolean) {
    this.offers.push({tracks, replace});
    return this.respond(replace);
  }
  async cameraClose() { this.closed++; }
}

let native: FakeNative, client: FakeClient, timers: FakeTimers, session: CameraSession;
beforeEach(() => {
  native = new FakeNative();
  client = new FakeClient();
  timers = new FakeTimers();
  session = new CameraSession(client, native, timers);
});

async function live(mode: CameraMode = 'rear') {
  await session.start(mode);
  native.emit('onPeerState', {state: 'connected'});
  expect(session.snapshot.state).toBe('live');
}

describe('CameraSession', () => {
  it('captures, offers non-replacing, applies the answer and goes live', async () => {
    const states: string[] = [];
    session.subscribe(s => states.push(s.state));
    await live('dual');
    expect(native.log).toEqual(['close', 'capture dual', 'offer rear+front', 'answer']);
    expect(client.offers).toEqual([{tracks: {'rear-1': 'rear', 'front-1': 'front'}, replace: false}]);
    expect(native.answer).toBe('v=0 answer');
    expect(states).toEqual(['idle', 'starting', 'connecting', 'connecting', 'live']);
  });

  it('a second phone gets busy; takeOver replaces the session', async () => {
    client.respond = async replace => {
      if (!replace) throw new DeviceError('busy', 'a phone session is active', 409);
      return {sdp: 'v=0 answer'};
    };
    await session.start('rear');
    expect(session.snapshot).toMatchObject({state: 'busy', message: expect.stringMatching(/Another phone/)});
    expect(native.log.at(-1)).toBe('close');
    await session.takeOver();
    expect(client.offers.map(o => o.replace)).toEqual([false, true]);
    native.emit('onPeerState', {state: 'connected'});
    expect(session.snapshot.state).toBe('live');
  });

  it('reconnects with backoff after the peer fails, replacing its own session', async () => {
    await live();
    native.emit('onPeerState', {state: 'failed'});
    expect(session.snapshot).toMatchObject({state: 'reconnecting', attempt: 1});
    expect(timers.pending()).toEqual([BACKOFF_MS[0]]);
    await timers.advance(BACKOFF_MS[0]!);
    expect(client.offers.at(-1)!.replace).toBe(true);
    // Capture keeps running across reconnects; only the peer is rebuilt.
    expect(native.log.filter(l => l.startsWith('capture'))).toHaveLength(1);
    native.emit('onPeerState', {state: 'connected'});
    expect(session.snapshot).toMatchObject({state: 'live', attempt: 0});
  });

  it('gives up after the last backoff step while the T3Box is unreachable', async () => {
    await live();
    client.respond = async () => { throw new DeviceError('unreachable', 'no route'); };
    native.emit('onPeerState', {state: 'failed'});
    for (const ms of BACKOFF_MS) await timers.advance(ms);
    expect(session.snapshot.state).toBe('error');
    expect(session.snapshot.message).toMatch(/Gave up after 6 attempts/);
    expect(client.offers).toHaveLength(1 + BACKOFF_MS.length);
    expect(timers.pending()).toEqual([]);
  });

  it('waits out a short disconnect before renegotiating', async () => {
    await live();
    native.emit('onPeerState', {state: 'disconnected'});
    await timers.advance(DISCONNECT_GRACE_MS - 1);
    native.emit('onPeerState', {state: 'connected'});
    await timers.advance(10_000);
    expect(client.offers).toHaveLength(1);
    native.emit('onPeerState', {state: 'disconnected'});
    await timers.advance(DISCONNECT_GRACE_MS);
    expect(session.snapshot.state).toBe('reconnecting');
  });

  it('does not retry an offer the T3Box rejected', async () => {
    client.respond = async () => { throw new DeviceError('invalid_offer', 'no H264', 422); };
    await session.start('rear');
    expect(session.snapshot).toMatchObject({state: 'error', message: 'The T3Box rejected the camera stream.'});
    expect(timers.pending()).toEqual([]);
  });

  it('refuses an offer that t3box-phone would not accept before sending it', async () => {
    native.offerOverride = async () => ({sdp: SINGLE_SDP.replaceAll('42e01f', '640c1f'), tracks: {'rear-1': 'rear'}});
    await session.start('rear');
    expect(session.snapshot.state).toBe('error');
    expect(client.offers).toHaveLength(0);
  });

  it('pauses in the background and resumes by replacing its own session', async () => {
    await live('dual');
    await session.setBackground(true);
    expect(session.snapshot.state).toBe('paused');
    expect(native.log.slice(-2)).toEqual(['close', 'stop capture']);
    // A failure event from the torn-down peer must not trigger a retry.
    native.emit('onPeerState', {state: 'failed'});
    expect(timers.pending()).toEqual([]);
    await session.setBackground(false);
    expect(native.log.slice(-3)).toEqual(['capture dual', 'offer rear+front', 'answer']);
    expect(client.offers.at(-1)!.replace).toBe(true);
  });

  it('applies thermal plans live: dual drops the front camera when critical', async () => {
    await live('dual');
    native.emit('onThermalState', {state: 'fair'});
    await flush();
    expect(native.applied.at(-1)!.map(p => `${p.role}:${p.active ? p.height : 'off'}`)).toEqual(['rear:720', 'front:540']);
    native.emit('onThermalState', {state: 'critical'});
    await flush();
    expect(native.applied.at(-1)!.map(p => `${p.role}:${p.active ? p.height : 'off'}`)).toEqual(['rear:540', 'front:off']);
    expect(session.snapshot).toMatchObject({thermal: 'critical', degraded: true});
    native.emit('onThermalState', {state: 'nominal'});
    await flush();
    expect(session.snapshot.degraded).toBe(false);
  });

  it('starts at the current thermal level', async () => {
    native.thermal = 'serious';
    session = new CameraSession(client, native, timers);
    let plans: TrackPlan[] = [];
    const orig = native.createOffer.bind(native);
    native.createOffer = async p => { plans = p; return orig(p); };
    await session.start('dual');
    expect(plans.map(p => p.height)).toEqual([540, 540]);
  });

  it('refuses dual mode on a phone without concurrent cameras', async () => {
    native.caps = {...native.caps, dualProfiles: []};
    await session.start('dual');
    expect(session.snapshot).toMatchObject({state: 'error', message: expect.stringMatching(/both cameras/)});
  });

  it('keeps the peer through a capture interruption and shows it', async () => {
    await live();
    native.emit('onInterruption', {active: true, reason: 'in_use_by_other_app'});
    expect(session.snapshot).toMatchObject({state: 'live', message: expect.stringMatching(/interrupted/)});
    native.emit('onInterruption', {active: false, reason: 'in_use_by_other_app'});
    expect(session.snapshot.message).toBe('');
    expect(client.offers).toHaveLength(1);
  });

  it('release closes the peer, the capture and the device session; late answers are ignored', async () => {
    let release!: () => void;
    client.respond = () => new Promise(r => { release = () => r({sdp: 'late'}); });
    const pending = session.start('rear');
    await flush();
    await session.release();
    release();
    await pending;
    expect(native.answer).toBeUndefined();
    expect(session.snapshot.state).toBe('idle');
    expect(native.log).toContain('stop capture');
    expect(client.closed).toBe(1);
  });

  it('stop keeps the preview running; preview switches cameras only while not sending', async () => {
    await session.preview('front');
    expect(native.log).toEqual(['capture front']);
    await session.preview('front');
    expect(native.log).toEqual(['capture front']);
    await live('front');
    await session.preview('dual');
    expect(native.log.filter(l => /capture/.test(l))).toEqual(['capture front']);
    await session.stop();
    expect(native.log).not.toContain('stop capture');
    expect(client.closed).toBe(1);
    await session.preview('dual');
    expect(native.log.slice(-2)).toEqual(['stop capture', 'capture dual']);
  });

  it('switching mode restarts the capture with the new cameras', async () => {
    await live('rear');
    await session.start('dual');
    expect(native.log.filter(l => /capture/.test(l))).toEqual(['capture rear', 'stop capture', 'capture dual']);
  });
});
