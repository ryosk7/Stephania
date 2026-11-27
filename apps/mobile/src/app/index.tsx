import {router} from 'expo-router';
import {Text} from 'react-native';
import {Button, Screen, s} from '../ui';

export default function Home() {
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
    </Screen>
  );
}
