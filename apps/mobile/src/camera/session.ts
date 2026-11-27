// Phone camera session (docs/PHONE.md): one phone, one or two camera
// tracks, sent to the T3Box over WebRTC. Platform independent: the native
// sender (modules/t3box-camera) captures, encodes and transports; this
// class decides when to connect, when to retry and at what quality.
//
// Lifecycle
//   idle → starting → connecting → live
//   live ⇄ reconnecting   peer failed/disconnected, capture interrupted
//   any  → paused         app in the background (iOS stops the cameras)
//   starting → busy       another phone holds the session (HTTP 409);
//                         takeOver() replaces it
//   any  → error          non-retryable (rejected offer, no permission, …)
import type {CameraMode, Capabilities, NativeCamera, PeerState, ThermalState} from '../../modules/t3box-camera';
import {DeviceError} from '../net/device';
import {capturePlan, planFor, rolesFor} from './plan';
import {checkOffer} from './sdp';

export type CameraClient = {
  cameraOffer(sdp: string, tracks: Record<string, 'rear' | 'front'>, replace?: boolean): Promise<{sdp: string}>;
  cameraClose(): Promise<void>;
};

export type SessionState = 'idle' | 'starting' | 'connecting' | 'live' | 'reconnecting' | 'busy' | 'paused' | 'error';

export type Snapshot = {
  state: SessionState;
  mode: CameraMode;
  thermal: ThermalState;
  /** Front camera switched off to protect the phone (dual mode, critical). */
  degraded: boolean;
  message: string;
  attempt: number;
};

export type Timers = {
  setTimeout(f: () => void, ms: number): unknown;
  clearTimeout(t: unknown): void;
};

/** Delays before reconnect attempts; after the last one the session gives up. */
export const BACKOFF_MS = [500, 1000, 2000, 4000, 8000, 8000];
/** A `disconnected` peer often recovers by itself (Wi-Fi roam); wait this long first. */
export const DISCONNECT_GRACE_MS = 3000;

export class CameraSession {
  private snap: Snapshot;
  private listeners = new Set<(s: Snapshot) => void>();
  private subs: {remove(): void}[] = [];
  private retryTimer: unknown;
  private graceTimer: unknown;
  /** Bumped on every (re)start/stop; stale async continuations compare and bail out. */
  private generation = 0;
  private capturing = false;
  private caps?: Capabilities;

  constructor(private readonly client: CameraClient, private readonly native: NativeCamera, private readonly timers: Timers = globalThis) {
    this.snap = {state: 'idle', mode: 'rear', thermal: native.thermalState(), degraded: false, message: '', attempt: 0};
    this.subs.push(
      native.addListener('onPeerState', e => this.onPeer(e.state)),
      native.addListener('onThermalState', e => void this.onThermal(e.state)),
      native.addListener('onInterruption', e => this.onInterruption(e.active)),
      native.addListener('onCaptureError', e => this.fail(`Camera error: ${e.message}`)),
    );
  }

  get snapshot(): Snapshot { return this.snap; }

  subscribe(f: (s: Snapshot) => void): () => void {
    this.listeners.add(f);
    f(this.snap);
    return () => this.listeners.delete(f);
  }

  /** Start sending `mode`. Refused with state `busy` if another phone is connected. */
  start(mode: CameraMode): Promise<void> {
    return this.begin(mode, false);
  }

  /** Replace the other phone's session (after the user confirmed `busy`). */
  takeOver(): Promise<void> {
    return this.begin(this.snap.mode, true);
  }

  /**
   * Local preview without sending: opens the cameras of `mode` so the user
   * can frame the shot before going live. Ignored while sending.
   */
  async preview(mode: CameraMode): Promise<void> {
    if (!['idle', 'error', 'busy'].includes(this.snap.state)) return;
    const gen = ++this.generation;
    try {
      const caps = this.caps ??= await this.native.capabilities();
      if (!rolesFor(mode).every(r => caps.roles.includes(r))) return;
      if (this.capturing && mode === this.snap.mode) return;
      if (this.capturing) { this.capturing = false; await this.native.stopCapture(); }
      if (gen !== this.generation) return;
      this.set({mode});
      await this.native.startCapture(mode, capturePlan(mode, caps));
      this.capturing = true;
    } catch (e) {
      if (gen === this.generation) this.set({message: describe(e)});
    }
  }

  /** Stop sending. The cameras keep running for the preview until release(). */
  async stop(): Promise<void> {
    this.generation++;
    this.clearTimers();
    const wasConnected = !['idle', 'busy', 'error'].includes(this.snap.state);
    this.set({state: 'idle', message: '', attempt: 0, degraded: false});
    await this.native.closePeer().catch(() => {});
    // Best effort: the device also notices the closed peer by itself.
    if (wasConnected) await this.client.cameraClose().catch(() => {});
  }

  /** Stop sending and close the cameras (leaving the screen). */
  async release(): Promise<void> {
    await this.stop();
    if (this.capturing) { this.capturing = false; await this.native.stopCapture().catch(() => {}); }
  }

  /** App moved to the background (true) or back to the foreground (false). */
  async setBackground(background: boolean): Promise<void> {
    if (background) {
      if (['idle', 'busy', 'error', 'paused'].includes(this.snap.state)) return;
      this.generation++;
      this.clearTimers();
      this.set({state: 'paused', message: 'Paused while the app is in the background'});
      await this.native.closePeer().catch(() => {});
      if (this.capturing) { this.capturing = false; await this.native.stopCapture().catch(() => {}); }
    } else if (this.snap.state === 'paused') {
      // Coming back: replace our own (now dead) session without asking.
      await this.begin(this.snap.mode, true);
    }
  }

  dispose(): void {
    this.generation++;
    this.clearTimers();
    for (const s of this.subs) s.remove();
    this.subs = [];
    this.listeners.clear();
  }

  // --- internals ---------------------------------------------------------

  private async begin(mode: CameraMode, replace: boolean, attempt = 0): Promise<void> {
    const gen = ++this.generation;
    this.clearTimers();
    const modeChanged = mode !== this.snap.mode;
    this.set({state: attempt ? 'reconnecting' : 'starting', mode, message: '', attempt});
    try {
      const caps = this.caps ??= await this.native.capabilities();
      if (!rolesFor(mode).every(r => caps.roles.includes(r))) throw new Error('This phone does not have that camera.');
      await this.native.closePeer();
      if (this.capturing && modeChanged) { this.capturing = false; await this.native.stopCapture(); }
      if (!this.capturing) {
        await this.native.startCapture(mode, capturePlan(mode, caps));
        this.capturing = true;
      }
      if (gen !== this.generation) return;
      const plans = planFor(mode, this.snap.thermal, caps);
      const offer = await this.native.createOffer(plans);
      checkOffer(offer);
      if (gen !== this.generation) return;
      this.set({state: attempt ? 'reconnecting' : 'connecting'});
      const answer = await this.client.cameraOffer(offer.sdp, offer.tracks, replace);
      if (gen !== this.generation) return;
      await this.native.setAnswer(answer.sdp);
      this.set({degraded: plans.some(p => !p.active)});
    } catch (e) {
      if (gen !== this.generation) return;
      if (e instanceof DeviceError && e.code === 'busy') {
        await this.native.closePeer().catch(() => {});
        this.set({state: 'busy', message: 'Another phone is sending to this T3Box.'});
        return;
      }
      if (retryable(e)) { this.scheduleRetry(attempt, describe(e)); return; }
      this.fail(describe(e));
    }
  }

  private onPeer(state: PeerState) {
    const s = this.snap.state;
    if (state === 'connected') {
      if (this.graceTimer !== undefined) { this.timers.clearTimeout(this.graceTimer); this.graceTimer = undefined; }
      if (s === 'connecting' || s === 'reconnecting' || s === 'live') this.set({state: 'live', message: '', attempt: 0});
      return;
    }
    if (s !== 'live' && s !== 'connecting' && s !== 'reconnecting') return;
    if (state === 'failed' || state === 'closed') {
      this.scheduleRetry(s === 'live' ? 0 : this.snap.attempt, 'Connection lost');
    } else if (state === 'disconnected' && this.graceTimer === undefined) {
      const gen = this.generation;
      this.graceTimer = this.timers.setTimeout(() => {
        this.graceTimer = undefined;
        if (gen === this.generation) this.scheduleRetry(0, 'Connection lost');
      }, DISCONNECT_GRACE_MS);
    }
  }

  private async onThermal(thermal: ThermalState) {
    this.set({thermal});
    if (!['live', 'connecting', 'reconnecting'].includes(this.snap.state)) return;
    if (!this.caps) return;
    const plans = planFor(this.snap.mode, thermal, this.caps);
    await this.native.applyPlans(plans).catch(() => {});
    this.set({degraded: plans.some(p => !p.active)});
  }

  private onInterruption(active: boolean) {
    const s = this.snap.state;
    if (s === 'idle' || s === 'busy' || s === 'error' || s === 'paused') return;
    if (active) {
      // The capture session stopped (phone call, other app, system pressure):
      // no frames reach the encoder, the device shows its fallback. Keep the
      // peer; frames resume when the interruption ends.
      this.set({message: 'Camera interrupted by the system'});
    } else {
      this.set({message: ''});
    }
  }

  private scheduleRetry(attempt: number, reason: string) {
    if (attempt >= BACKOFF_MS.length) { this.fail(`${reason}. Gave up after ${attempt} attempts.`); return; }
    const gen = ++this.generation;
    this.clearTimers();
    this.set({state: 'reconnecting', message: reason, attempt: attempt + 1});
    // Our own session is what the device still holds: replace it.
    this.retryTimer = this.timers.setTimeout(() => {
      if (gen === this.generation) void this.begin(this.snap.mode, true, attempt + 1);
    }, BACKOFF_MS[attempt]!);
  }

  private fail(message: string) {
    this.generation++;
    this.clearTimers();
    this.set({state: 'error', message});
    void this.native.closePeer().catch(() => {});
  }

  private clearTimers() {
    if (this.retryTimer !== undefined) this.timers.clearTimeout(this.retryTimer);
    if (this.graceTimer !== undefined) this.timers.clearTimeout(this.graceTimer);
    this.retryTimer = this.graceTimer = undefined;
  }

  private set(patch: Partial<Snapshot>) {
    this.snap = {...this.snap, ...patch};
    for (const f of this.listeners) f(this.snap);
  }
}

/** Network trouble and a briefly unavailable phone service are worth retrying; a rejected offer or a missing permission is not. */
function retryable(e: unknown): boolean {
  if (e instanceof DeviceError) return e.code === 'unreachable' || e.code === 'phone_unavailable' || (e.status ?? 0) >= 500;
  return false;
}

function describe(e: unknown): string {
  if (e instanceof DeviceError) {
    const m: Record<string, string> = {
      unreachable: 'Cannot reach the T3Box.',
      invalid_offer: 'The T3Box rejected the camera stream.',
      forbidden_role: 'This phone may not send camera video to this T3Box.',
      unauthorized: 'This phone\'s access is no longer valid.',
      phone_unavailable: 'The camera service on the T3Box is not running.',
    };
    return m[e.code] ?? `${e.code}: ${e.message}`;
  }
  return e instanceof Error ? e.message : String(e);
}
