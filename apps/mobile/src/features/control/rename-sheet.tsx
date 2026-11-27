// Rename a Stephania device on this phone (DESIGN.md §6 Control), in the app's own
// sheet so iOS and Android behave the same.
import {useEffect, useState} from 'react';
import {View} from 'react-native';
import {renameDevice} from '@/shared/device';
import {Field, Footnote, Icon, Mono, Notice, PrimaryButton, Sheet} from '@/shared/ui';

export function RenameSheet({deviceId, name, host, visible, onClose, onRenamed}: {
  deviceId: string; name: string; host: string; visible: boolean; onClose(): void; onRenamed(name: string): void;
}) {
  const [value, setValue] = useState(name);
  const [error, setError] = useState('');
  useEffect(() => { if (visible) { setValue(name); setError(''); } }, [visible, name]);
  const save = async () => {
    try { onRenamed(await renameDevice(deviceId, value)); onClose(); } catch (e) { setError((e as Error).message); }
  };
  return (
    <Sheet visible={visible} title="Rename" icon="pencil" onClose={onClose}>
      <Field label="Name shown on this phone" value={value} onChangeText={setValue} autoCapitalize="words" autoFocus maxLength={40} returnKeyType="done" onSubmitEditing={() => void save()} />
      {host !== '' && (
        <View className="flex-row items-center gap-2 px-1" accessible accessibilityLabel={`The network name ${host} and the certificate stay the same`}>
          <Icon name="lock" size={15} color="secondary" />
          <Mono tone="secondary">{host}</Mono>
          <Footnote>stays the same</Footnote>
        </View>
      )}
      {error !== '' && <Notice tone="warning">{error}</Notice>}
      <PrimaryButton title="Save" icon="check" disabled={!value.trim()} onPress={() => void save()} />
    </Sheet>
  );
}
