// Rename a T3Box on this phone (DESIGN.md §6 Control). A bottom sheet, not
// Alert.prompt, which iOS has and Android lacks.
import {useState} from 'react';
import {KeyboardAvoidingView, Modal, Platform, Pressable, StyleSheet, View} from 'react-native';
import {renameDevice} from '@/shared/device';
import {ButtonRow, Field, Footnote, Notice, PrimaryButton, SecondaryButton, Title, color, radius, space} from '@/shared/ui';

export function RenameSheet({deviceId, name, host, visible, onClose, onRenamed}: {
  deviceId: string; name: string; host: string; visible: boolean; onClose(): void; onRenamed(name: string): void;
}) {
  const [value, setValue] = useState(name);
  const [error, setError] = useState('');
  const save = async () => {
    try { onRenamed(await renameDevice(deviceId, value)); onClose(); } catch (e) { setError((e as Error).message); }
  };
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose} onShow={() => { setValue(name); setError(''); }}>
      <Pressable style={st.scrim} onPress={onClose} accessibilityLabel="Close" />
      <KeyboardAvoidingView behavior={Platform.OS === 'ios' ? 'padding' : undefined} style={st.wrap}>
        <View style={st.sheet}>
          <View style={st.grabber} />
          <Title>Rename this T3Box</Title>
          <Field label="Name shown on this phone" value={value} onChangeText={setValue} autoCapitalize="words" autoFocus maxLength={40} returnKeyType="done" onSubmitEditing={() => void save()} />
          <Footnote>{host ? `The network name ${host} and` : 'The network name and'} the certificate stay the same, so nothing has to be set up again.</Footnote>
          {error !== '' && <Notice tone="warning">{error}</Notice>}
          <ButtonRow>
            <SecondaryButton title="Cancel" onPress={onClose} />
            <PrimaryButton title="Save" disabled={!value.trim()} onPress={() => void save()} />
          </ButtonRow>
        </View>
      </KeyboardAvoidingView>
    </Modal>
  );
}

const st = StyleSheet.create({
  scrim: {position: 'absolute', left: 0, right: 0, top: 0, bottom: 0, backgroundColor: 'rgba(0,0,0,0.6)'},
  wrap: {flex: 1, justifyContent: 'flex-end'},
  sheet: {backgroundColor: color.surface, borderTopLeftRadius: radius.card + 4, borderTopRightRadius: radius.card + 4, padding: space.l, paddingBottom: space.xxl, gap: space.l},
  grabber: {alignSelf: 'center', width: 36, height: 5, borderRadius: 3, backgroundColor: '#48484A'},
});
