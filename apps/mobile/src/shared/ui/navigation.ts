// Dark navigation chrome matching the tokens (DESIGN.md §3.3).
import {DarkTheme, type Theme} from 'expo-router';
import {color} from './tokens';

export const navigationTheme: Theme = {
  ...DarkTheme,
  colors: {...DarkTheme.colors, background: color.bg, card: color.bg, text: color.text, border: color.separator, primary: color.info, notification: color.live},
};

export const stackScreenOptions = {
  headerStyle: {backgroundColor: color.bg},
  headerTintColor: color.text,
  headerShadowVisible: false,
  headerBackButtonDisplayMode: 'minimal' as const,
  contentStyle: {backgroundColor: color.bg},
};
