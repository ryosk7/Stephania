import type {ReactNode} from 'react';
import {Pressable, StyleSheet, Text, View} from 'react-native';

export const s = StyleSheet.create({
  screen: {flex: 1, padding: 20, gap: 14, backgroundColor: '#fff'},
  h: {fontSize: 20, fontWeight: '600'},
  p: {fontSize: 15, lineHeight: 22, color: '#333'},
  error: {fontSize: 14, color: '#b00020'},
  row: {paddingVertical: 12, borderBottomWidth: StyleSheet.hairlineWidth, borderColor: '#ccc'},
  input: {borderWidth: 1, borderColor: '#999', borderRadius: 8, padding: 12, fontSize: 16},
});

export function Button({title, onPress, disabled}: {title: string; onPress: () => void; disabled?: boolean}) {
  return (
    <Pressable accessibilityRole="button" disabled={disabled} onPress={onPress}
      style={({pressed}) => ({padding: 14, borderRadius: 10, alignItems: 'center', backgroundColor: disabled ? '#bbb' : pressed ? '#333' : '#111'})}>
      <Text style={{color: '#fff', fontSize: 16, fontWeight: '600'}}>{title}</Text>
    </Pressable>
  );
}

export const Screen = ({children}: {children: ReactNode}) => <View style={s.screen}>{children}</View>;

/** User-facing message for protocol error codes (contracts/ble-protocol.md). */
export function describeError(e: unknown): string {
  const code = (e as {code?: string})?.code ?? (e instanceof Error ? e.message : String(e));
  const messages: Record<string, string> = {
    bad_proof: 'This QR code is not for this T3Box. Check that it matches the t3box-setup.txt on the card.',
    already_claimed: 'This T3Box already has an owner.',
    locked_out: 'Locked after repeated failures. Try again in 5 minutes.',
    challenge_expired: 'The registration window expired. Please try again.',
    not_claimable: 'This T3Box already has an owner or is not accepting registration.',
    disconnected: 'The connection to the T3Box was lost. Move closer and try again.',
    bad_seal: 'Could not verify the encrypted data. Restart the app.',
    not_owner: 'This phone is not registered as the owner of this T3Box.',
    join_failed: 'Could not join the Wi-Fi network. Check the SSID and password.',
    response_timeout: 'No response from the T3Box. Move closer and try again.',
    bluetooth_off: 'Turn Bluetooth on.',
    storage_failed: 'The T3Box could not save. Check its SD card.',
  };
  return messages[code] ?? `Error: ${code}`;
}
