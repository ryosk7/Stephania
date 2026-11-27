// Rename a T3Box on this phone (DESIGN.md §6 Control), in the app's own
// sheet so iOS and Android behave the same.
import {useEffect, useState} from 'react';
import {renameDevice} from '@/shared/device';
import {ButtonRow, Field, Footnote, Notice, PrimaryButton, SecondaryButton, Sheet} from '@/shared/ui';

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
    <Sheet visible={visible} title="Rename" onClose={onClose}>
      <Field label="Name shown on this phone" value={value} onChangeText={setValue} autoCapitalize="words" autoFocus maxLength={40} returnKeyType="done" onSubmitEditing={() => void save()} />
      <Footnote>{host ? `The network name ${host} and` : 'The network name and'} the certificate stay the same, so nothing has to be set up again.</Footnote>
      {error !== '' && <Notice tone="warning">{error}</Notice>}
      <ButtonRow>
        <SecondaryButton title="Cancel" onPress={onClose} />
        <PrimaryButton title="Save" disabled={!value.trim()} onPress={() => void save()} />
      </ButtonRow>
    </Sheet>
  );
}
