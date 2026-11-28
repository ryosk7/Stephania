// What the Wi-Fi step shows for each network in a device scan.
import type {ScanNetwork} from '@stephania/provisioning-client';

export const supported = (n: ScanNetwork) => n.security === 'open' || n.security === 'wpa-psk';

/** An open network named like a secured one in the same scan (possible evil twin). */
export const openTwin = (n: ScanNetwork, all: readonly ScanNetwork[]) =>
  n.security === 'open' && all.some(o => o.ssid === n.ssid && o.security !== 'open');

/** Row detail; `undefined` for an ordinary password network. */
export function securityLabel(n: ScanNetwork, all: readonly ScanNetwork[]): string | undefined {
  if (openTwin(n, all)) return 'Open, same name as a secured network';
  if (n.security === 'open') return 'Open, not encrypted';
  return n.security === 'wpa-psk' ? undefined : 'Not supported';
}
