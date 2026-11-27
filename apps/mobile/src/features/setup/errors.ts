/** User-facing message for provisioning error codes (contracts/ble-protocol.md). */
export function describeSetupError(e: unknown): string {
  const code = (e as {code?: string})?.code ?? (e instanceof Error ? e.message : String(e));
  const messages: Record<string, string> = {
    bad_proof: 'This QR code is not for this Stephania device. Check that it matches the stephania-setup.txt on the card.',
    already_claimed: 'This Stephania device already has an owner.',
    locked_out: 'Locked after repeated failures. Try again in 5 minutes.',
    challenge_expired: 'The registration window expired. Try again.',
    not_claimable: 'This Stephania device already has an owner or is not accepting registration.',
    disconnected: 'The connection to the Stephania device was lost. Move closer and try again.',
    bad_seal: 'Could not verify the encrypted data. Restart the app.',
    not_owner: 'This phone is not registered as the owner of this Stephania device.',
    need_qr: 'This Stephania device has no owner yet. Scan its setup QR code first.',
    join_failed: 'Could not join the Wi-Fi network. Check the network name and password.',
    response_timeout: 'No response from the Stephania device. Move closer and try again.',
    bluetooth_off: 'Turn Bluetooth on.',
    storage_failed: 'The Stephania device could not save. Check its SD card.',
  };
  return messages[code] ?? `Error: ${code}`;
}
