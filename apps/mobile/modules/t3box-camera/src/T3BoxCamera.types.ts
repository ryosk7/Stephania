// Platform contract between the shared TypeScript layer (src/camera/*) and
// the native sender. Native code owns capture, hardware H.264 encoding and
// WebRTC transport; it never decides policy. Everything that decides
// (signalling, retries, thermal plans) lives in TypeScript so a second
// platform only has to implement this interface.

export type CameraRole = 'rear' | 'front';
/** What the user picked: one camera or both at once (MultiCam). */
export type CameraMode = 'rear' | 'front' | 'dual';
/** Mirrors ProcessInfo.ThermalState / Android PowerManager thermal status buckets. */
export type ThermalState = 'nominal' | 'fair' | 'serious' | 'critical';

/** §12 concurrent rear+front profiles (rear first). */
export type DualProfile = '720p30+720p30' | '720p30+540p24' | '540p24+540p24';

export type Capabilities = {
  /** Cameras this phone has. */
  roles: CameraRole[];
  /**
   * Rear+front profiles this phone can capture concurrently, measured by the
   * native side (iOS: an AVCaptureMultiCamSession configured with the
   * formats has hardwareCost and systemPressureCost < 1). Empty when the
   * phone cannot run both cameras at once.
   */
  dualProfiles: DualProfile[];
  /** Hardware H.264 encoder available (VideoToolbox / MediaCodec). */
  hardwareH264: boolean;
};

/** Encoding target for one track; `active: false` stops sending it (the capture keeps running for the preview). */
export type TrackPlan = {
  role: CameraRole;
  active: boolean;
  width: number;
  height: number;
  fps: number;
  maxBitrateKbps: number;
};

export type Offer = {
  sdp: string;
  /** MediaStreamTrack id in the SDP (a=msid) → camera role. */
  tracks: Record<string, CameraRole>;
};

export type PeerState = 'new' | 'connecting' | 'connected' | 'disconnected' | 'failed' | 'closed';

export type InterruptionReason =
  | 'background'           // app left the foreground; iOS stops camera access
  | 'in_use_by_other_app'
  | 'multitasking'         // Split View / Stage Manager without multitasking camera access
  | 'pressure'             // system pressure (thermal / power) shut the capture down
  | 'unknown';

export type CameraEvents = {
  onPeerState: (e: {state: PeerState}) => void;
  onThermalState: (e: {state: ThermalState}) => void;
  onInterruption: (e: {active: boolean; reason: InterruptionReason}) => void;
  onCaptureError: (e: {message: string}) => void;
};

export interface NativeCamera {
  capabilities(): Promise<Capabilities>;
  thermalState(): ThermalState;
  /** Opens the cameras of `mode` and configures capture for `plans` (sizes/fps). */
  startCapture(mode: CameraMode, plans: TrackPlan[]): Promise<void>;
  stopCapture(): Promise<void>;
  /** New peer connection with one H.264 (42e01f, packetization-mode 1) track per captured camera; resolves after ICE gathering completes (non-trickle). */
  createOffer(plans: TrackPlan[]): Promise<Offer>;
  setAnswer(sdp: string): Promise<void>;
  /** Changes bitrate/fps/resolution (and active) of running tracks without renegotiation. */
  applyPlans(plans: TrackPlan[]): Promise<void>;
  closePeer(): Promise<void>;
  addListener<E extends keyof CameraEvents>(event: E, listener: CameraEvents[E]): {remove(): void};
}
