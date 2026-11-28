import {beforeEach, describe, expect, it, vi} from 'vitest';

// In-memory secure store that records which items need authentication.
const store = vi.hoisted(() => ({
  items: new Map<string, {value: string; auth: boolean}>(),
  biometrics: true,
  prompts: 0,
  next: null as null | 'cancel' | 'invalidated',
}));

vi.mock('react-native', () => ({Platform: {OS: 'ios'}}));
vi.mock('expo-secure-store', () => ({
  WHEN_UNLOCKED_THIS_DEVICE_ONLY: 'when_unlocked_this_device_only',
  canUseBiometricAuthentication: () => store.biometrics,
  setItemAsync: async (key: string, value: string, o?: {requireAuthentication?: boolean}) => {
    if (o?.requireAuthentication && !store.biometrics) throw new Error('No biometrics enrolled');
    store.items.set(key, {value, auth: !!o?.requireAuthentication});
  },
  getItemAsync: async (key: string) => {
    const item = store.items.get(key);
    if (!item) return null;
    if (item.auth) {
      store.prompts++;
      const outcome = store.next;
      store.next = null;
      if (outcome === 'cancel') throw new Error('User canceled the operation.');
      if (outcome === 'invalidated') return null;
    }
    return item.value;
  },
  deleteItemAsync: async (key: string) => { store.items.delete(key); },
}));

import {forgetEnrollment, loadEnrollment, loadOtpSecret, OtpSecretError, saveEnrollment} from './enrollment';

const SECRET = 'AAAAAAAAAAAAAAAAAAAAAAAAAAA=';
const enrollment = {token: 't', tokenId: 'id', urls: ['https://stephania.local:8443'], caSha256: 'ab'.repeat(32)};
const plainItems = () => [...store.items.values()].filter(i => !i.auth).map(i => i.value).join('\n');

describe('maintenance code secret storage', () => {
  beforeEach(() => { store.items.clear(); store.biometrics = true; store.prompts = 0; store.next = null; });

  it('keeps the secret out of the enrollment item, behind authentication', async () => {
    await saveEnrollment('dev-1', {...enrollment, otpSecret: SECRET});
    expect(plainItems()).not.toContain(SECRET);
    expect([...store.items.values()].some(i => i.auth && i.value === SECRET)).toBe(true);
    const e = await loadEnrollment('dev-1');
    expect(e).toMatchObject({...enrollment, maintenance: 'protected'});
    expect(e).not.toHaveProperty('otpSecret');
    expect(store.prompts).toBe(0);
    expect(await loadOtpSecret('dev-1', 'Show')).toBe(SECRET);
    expect(store.prompts).toBe(1);
  });

  it('moves a secret stored by an older version into the protected item', async () => {
    store.items.set('stephania.lan.' + Buffer.from('dev-1').toString('hex'), {value: JSON.stringify({...enrollment, otpSecret: SECRET}), auth: false});
    expect(await loadEnrollment('dev-1')).toMatchObject({maintenance: 'protected'});
    expect(plainItems()).not.toContain(SECRET);
    expect(await loadOtpSecret('dev-1', 'Show')).toBe(SECRET);
  });

  it('falls back to the in-app gate without biometrics, and migrates later', async () => {
    store.biometrics = false;
    await saveEnrollment('dev-1', {...enrollment, otpSecret: SECRET});
    expect(await loadEnrollment('dev-1')).toMatchObject({maintenance: 'app-gated'});
    expect(await loadOtpSecret('dev-1', 'Show')).toBe(SECRET);
    store.biometrics = true;
    expect(await loadEnrollment('dev-1')).toMatchObject({maintenance: 'protected'});
    expect(plainItems()).not.toContain(SECRET);
  });

  it('reports cancellation and invalidated keys', async () => {
    await saveEnrollment('dev-1', {...enrollment, otpSecret: SECRET});
    store.next = 'cancel';
    await expect(loadOtpSecret('dev-1', 'Show')).rejects.toMatchObject({reason: 'cancelled'});
    store.next = 'invalidated';
    const err = await loadOtpSecret('dev-1', 'Show').catch(e => e as OtpSecretError);
    expect(err).toBeInstanceOf(OtpSecretError);
    expect(err).toMatchObject({reason: 'invalidated'});
  });

  it('a device without maintenance has none, and forgetting removes both items', async () => {
    await saveEnrollment('dev-2', enrollment);
    expect(await loadEnrollment('dev-2')).toMatchObject({maintenance: 'none'});
    await saveEnrollment('dev-1', {...enrollment, otpSecret: SECRET});
    await forgetEnrollment('dev-1');
    expect([...store.items.keys()].filter(k => k.includes(Buffer.from('dev-1').toString('hex')))).toEqual([]);
  });
});
