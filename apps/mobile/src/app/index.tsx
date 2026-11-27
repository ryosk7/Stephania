import {router} from 'expo-router';
import {useEffect, useState} from 'react';
import {Text} from 'react-native';
import {enrolledDevices, type EnrolledDevice} from '../state/session';
import {Button, Screen, s} from '../ui';

export default function Home() {
  const [enrolled, setEnrolled] = useState<EnrolledDevice[]>([]);
  useEffect(() => { enrolledDevices().then(setEnrolled).catch(() => undefined); }, []);
  return (
    <Screen>
      <Text style={s.h}>T3Boxを使い始める</Text>
      <Text style={s.p}>
        1. PCでセットアップページを開き「秘密値を作成」を押す{'\n'}
        2. t3box-setup.txt をOSを書き込んだmicroSDのboot領域にコピー{'\n'}
        3. microSDをT3Boxに入れて電源を入れる{'\n'}
        4. 下のボタンからPC画面のQRを読み取る
      </Text>
      <Button title="セットアップQRを読む" onPress={() => router.push('/scan')} />
      <Button title="登録済みのT3Boxに接続" onPress={() => router.push('/devices')} />
      {enrolled.length > 0 && <Text style={s.h}>操作パネル（Wi-Fi経由）</Text>}
      {enrolled.map(d => (
        <Button key={d.deviceId} title={`${d.name} を操作`} onPress={() => router.push({pathname: '/dashboard', params: {device: d.deviceId}})} />
      ))}
    </Screen>
  );
}
