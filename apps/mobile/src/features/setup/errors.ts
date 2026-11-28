/** User-facing message for provisioning error codes (contracts/ble-protocol.md). */
export function describeSetupError(e: unknown): string {
  const code = (e as {code?: string})?.code ?? (e instanceof Error ? e.message : String(e));
  const messages: Record<string, string> = {
    already_claimed: 'Another phone set up this Stephania device first. To start over, write the microSD card again.',
    management_unavailable: 'The Stephania device is still preparing its certificate. Try again in a few minutes.',
    locked_out: 'Locked after repeated failures. Wait a few minutes and try again.',
    challenge_expired: 'The registration window expired. Try again.',
    not_claimable: 'Another phone set up this Stephania device first. To start over, write the microSD card again.',
    bad_key: 'The Stephania device refused this phone\'s key. Try again.',
    claim_failed: 'The Stephania device answered with a different key. Try again.',
    disconnected: 'The connection to the Stephania device was lost. Move closer and try again.',
    bad_seal: 'Could not verify the encrypted data. Restart the app.',
    not_owner: 'Another phone owns this Stephania device. To start over, write the microSD card again.',
    bad_proof: 'The Stephania device did not accept this phone\'s key. If another phone set it up, write the microSD card again to start over.',
    owner_key_exists: 'This phone already owns a device with this Bluetooth identity, but it now says it is not set up. That is expected only if you wrote its microSD card again; otherwise another device may be imitating it.',
    join_failed: 'Could not join the Wi-Fi network. Check the network name and password.',
    response_timeout: 'No response from the Stephania device. Move closer and try again.',
    bluetooth_off: 'Turn Bluetooth on.',
    storage_failed: 'The Stephania device could not save. Check its SD card.',
  };
  return messages[code] ?? `Error: ${code}`;
}
