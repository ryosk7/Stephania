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
    bad_proof: 'QRコードがこのT3Boxのものではありません。SDに入れた t3box-setup.txt と同じQRか確認してください。',
    already_claimed: 'このT3Boxはすでに所有者が登録されています。',
    not_claimable: 'このT3Boxは登録を受け付けていません（要復旧状態の可能性があります）。',
    locked_out: '失敗が続いたため一時的にロックされています。5分後に再試行してください。',
    pairing_expired: '登録の有効時間が切れました。もう一度お試しください。',
    not_owner: 'このスマホはこのT3Boxの所有者として登録されていません。',
    join_failed: 'Wi-Fiに接続できませんでした。SSIDとパスワードを確認してください。',
    response_timeout: 'T3Boxからの応答がありません。近づいてもう一度お試しください。',
    bluetooth_off: 'Bluetoothをオンにしてください。',
    storage_failed: 'T3Box側で保存に失敗しました。SDカードの状態を確認してください。',
  };
  return messages[code] ?? `エラー: ${code}`;
}
