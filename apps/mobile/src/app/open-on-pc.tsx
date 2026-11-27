// Opening the Studio on a PC. The PC has no BLE-delivered pin, so the user
// installs the device CA there and compares its fingerprint with the one
// this phone received sealed from the device.
import {useLocalSearchParams} from 'expo-router';
import {useEffect, useState} from 'react';
import {Share, Text} from 'react-native';
import type {ManagementEnrollment} from '@t3box/provisioning-client';
import {formatFingerprint} from '../net/device';
import {loadEnrollment} from '../state/session';
import {Button, Screen, s} from '../ui';

export default function OpenOnPc() {
  const {device} = useLocalSearchParams<{device: string}>();
  const [e, setE] = useState<ManagementEnrollment | null>();
  useEffect(() => { loadEnrollment(device).then(setE).catch(() => setE(null)); }, [device]);
  if (e === undefined) return <Screen><Text style={s.p}>読み込み中…</Text></Screen>;
  if (e === null) return <Screen><Text style={s.p}>このT3Boxのアクセス権がありません。</Text></Screen>;
  const url = e.urls[0]!;
  return (
    <Screen>
      <Text style={s.p}>PCで次のアドレスを開きます（同じWi-Fiに接続してください）。</Text>
      <Text selectable style={s.h}>{url}</Text>
      <Text style={s.p}>
        初回はブラウザが証明書の警告を出します。T3Box の証明書（{url}ca.pem）を PC に「信頼するルート証明書」として登録し、
        その指紋が下の値と一致することを確認してください。一致しない場合は登録せず、ネットワークを確認してください。
      </Text>
      <Text selectable style={[s.p, {fontFamily: 'Menlo', fontWeight: '600'}]}>SHA-256{'\n'}{formatFingerprint(e.caSha256)}</Text>
      <Text style={s.p}>サインインには Studio のアカウント（管理者が作成）を使います。</Text>
      <Button title="アドレスと指紋を共有" onPress={() => void Share.share({message: `${url}\nSHA-256: ${formatFingerprint(e.caSha256)}`})} />
    </Screen>
  );
}
