import {describe, expect, it} from 'vitest';
import {openTwin, securityLabel, supported} from './networks';

describe('Wi-Fi scan presentation', () => {
  const hallOpen = {ssid: 'Hall', rssi: -40, security: 'open'};
  const hallWpa = {ssid: 'Hall', rssi: -70, security: 'wpa-psk'};
  const cafe = {ssid: 'Cafe', rssi: -60, security: 'open'};
  const all = [hallOpen, cafe, hallWpa];

  it('flags an open network that shares its name with a secured one', () => {
    expect(openTwin(hallOpen, all)).toBe(true);
    expect(openTwin(cafe, all)).toBe(false);
    expect(openTwin(hallWpa, all)).toBe(false);
  });

  it('labels open networks as open and leaves password networks plain', () => {
    expect(securityLabel(hallOpen, all)).toBe('Open, same name as a secured network');
    expect(securityLabel(cafe, all)).toBe('Open, not encrypted');
    expect(securityLabel(hallWpa, all)).toBeUndefined();
    expect(securityLabel({ssid: 'Corp', rssi: -50, security: 'enterprise'}, all)).toBe('Not supported');
    expect(supported({ssid: 'Corp', rssi: -50, security: 'enterprise'})).toBe(false);
  });
});
